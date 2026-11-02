use std::path::Path;
use std::sync::Arc;

use crate::adaptor::gateway::workflow::schema::WorkflowDefinitionYaml;
use crate::domain::app_config::AppConfigRepository;
use crate::usecase::git_usecase::GitUsecase;
use crate::usecase::workflow::runtime_resolver::{
    ManagedWorktreeResolver, ManagedWorktreeResolverError, WorkflowDefinitionResolver,
    WorkflowDefinitionResolverError,
};

pub struct DefaultWorkflowDefinitionResolver {
    workflows_dir: std::path::PathBuf,
}

impl DefaultWorkflowDefinitionResolver {
    pub fn new(workflows_dir: std::path::PathBuf) -> Self {
        Self { workflows_dir }
    }
}

#[async_trait::async_trait]
impl WorkflowDefinitionResolver for DefaultWorkflowDefinitionResolver {
    async fn resolve(
        &self,
        workflow_name: &str,
    ) -> Result<crate::domain::workflow::WorkflowDefinitionContent, WorkflowDefinitionResolverError>
    {
        let workflow_name = workflow_name.to_string();
        let dir = self.workflows_dir.clone();
        crate::common::operation_context::spawn_blocking(move || {
            resolve_workflow_by_name(&dir, &dir, &workflow_name)
        })
        .await
        .map_err(|e| {
            WorkflowDefinitionResolverError::Infrastructure(format!("task join error: {e}"))
        })?
    }
}

/// Runtime start の外部識別子はファイル名ではなく WorkflowDefinitionContent.name である。
///
/// ユーザーが YAML を直接編集・rename した場合も同じ規則を保つため、全 definition の
/// name を読んで候補を絞り、一致した定義だけを完全に読み込む。複数ファイル（または builtin）が同じ name を
/// 宣言した状態を先勝ちにすると実行対象が directory iteration order に依存するため、
/// §7 の閉じた code 表で workflow name の妥当性にも使われる WFS006 Diagnostic として
/// 明示的に拒否する（表にない新 code は追加しない）。
pub fn resolve_workflow_by_name(
    workflows_dir: &Path,
    facets_base_dir: &Path,
    workflow_name: &str,
) -> Result<WorkflowDefinitionYaml, WorkflowDefinitionResolverError> {
    let mut matches = Vec::new();
    let mut exact_file_error = None;

    if workflows_dir.exists() {
        let entries = std::fs::read_dir(workflows_dir).map_err(|error| {
            WorkflowDefinitionResolverError::Infrastructure(format!(
                "workflow directory read failed: {error}"
            ))
        })?;
        let mut paths = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                crate::adaptor::gateway::workflow::storage::workflow_source_format(path).is_some()
            })
            .collect::<Vec<_>>();
        paths.sort();

        for path in paths {
            let result = (|| {
                let content =
                    std::fs::read_to_string(&path).map_err(super::storage::StorageError::from)?;
                let diagnosis = super::storage::diagnose_workflow_file(
                    &path,
                    &content,
                    workflows_dir,
                    facets_base_dir,
                );
                let workflow = super::storage::diagnosed_workflow(diagnosis)?;
                if workflow.name != workflow_name {
                    return Ok(None);
                }
                super::storage::resolve_and_validate_workflow_facets(&workflow, facets_base_dir)?;
                Ok::<_, super::storage::StorageError>(Some(workflow))
            })();
            match result {
                Ok(Some(workflow)) if workflow.name == workflow_name => {
                    matches.push((path.display().to_string(), workflow))
                }
                Ok(_) => {}
                Err(error) => {
                    if path.file_stem().and_then(|stem| stem.to_str()) == Some(workflow_name) {
                        log::warn!("workflow definition skipped: {}: {error}", path.display());
                        exact_file_error = Some(error.to_string());
                    } else {
                        log::debug!("workflow definition skipped: {}: {error}", path.display());
                    }
                }
            }
        }
    }

    for summary in crate::adaptor::gateway::workflow::builtin::list_builtin_workflows() {
        if summary.name != workflow_name {
            continue;
        }
        match crate::adaptor::gateway::workflow::builtin::load_builtin_workflow_resolved(
            &summary.name,
            facets_base_dir,
        ) {
            Ok(Some(workflow)) if workflow.name == workflow_name => {
                matches.push((format!("builtin:{}", summary.name), workflow));
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!(
                    "builtin workflow definition skipped: {}: {error}",
                    summary.name
                );
            }
        }
    }

    match matches.len() {
        0 => Err(WorkflowDefinitionResolverError::InvalidWorkflow(
            exact_file_error
                .unwrap_or_else(|| format!("ワークフロー '{workflow_name}' が見つかりません")),
        )),
        1 => Ok(matches.pop().expect("one workflow match").1),
        _ => {
            let sources = matches
                .iter()
                .map(|(source, _)| source.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(WorkflowDefinitionResolverError::InvalidWorkflow(
                crate::domain::workflow::WorkflowError::Diagnostics(vec![
                    crate::domain::workflow::value_objects::diagnostic::DiagnosticItem::new(
                        "WFS006",
                        crate::domain::workflow::value_objects::diagnostic::Severity::Error,
                        crate::domain::workflow::value_objects::diagnostic::DiagnosticStage::ParseShape,
                        None,
                        format!("workflow name '{workflow_name}' is duplicated: {sources}"),
                    ),
                ]).to_string(),
            ))
        }
    }
}

pub struct AppConfigManagedWorktreeResolver {
    usecase: Arc<GitUsecase>,
    config: Arc<dyn AppConfigRepository>,
}

impl AppConfigManagedWorktreeResolver {
    pub fn new(usecase: Arc<GitUsecase>, config: Arc<dyn AppConfigRepository>) -> Self {
        Self { usecase, config }
    }
}

#[async_trait::async_trait]
impl ManagedWorktreeResolver for AppConfigManagedWorktreeResolver {
    async fn resolve(&self, worktree_path: String) -> Result<String, ManagedWorktreeResolverError> {
        super::worktree_gateway::canonicalize_managed_worktree_path(
            Arc::clone(&self.usecase),
            Arc::clone(&self.config),
            worktree_path,
        )
        .await
        .map_err(|error| match error {
            crate::domain::workflow::WorkflowError::Technical(stopped) => {
                ManagedWorktreeResolverError::Technical(stopped)
            }
            error => ManagedWorktreeResolverError::Validation(error.to_string()),
        })
    }
}
