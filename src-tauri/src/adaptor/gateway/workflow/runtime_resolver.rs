use std::path::Path;
use std::sync::Arc;

use crate::adaptor::gateway::workflow::resolver::{
    ManagedWorktreeResolver, ManagedWorktreeResolverError, WorkflowDefinitionResolver,
    WorkflowDefinitionResolverError,
};
use crate::adaptor::gateway::workflow::schema::Workflow;
use crate::domain::app_config::ConfigRepository;
use crate::usecase::repository_usecase::RepositoryUsecase;

pub(crate) struct DefaultWorkflowDefinitionResolver;

#[async_trait::async_trait]
impl WorkflowDefinitionResolver for DefaultWorkflowDefinitionResolver {
    async fn resolve(
        &self,
        workflow_name: &str,
    ) -> Result<Workflow, WorkflowDefinitionResolverError> {
        let workflow_name = workflow_name.to_string();
        tokio::task::spawn_blocking(move || {
            let dir = crate::adaptor::gateway::workflow::storage::workflows_dir();
            let facets_base = crate::adaptor::gateway::workflow::facet::facets_base_dir();
            resolve_workflow_by_name(&dir, &facets_base, &workflow_name)
        })
        .await
        .map_err(|e| {
            WorkflowDefinitionResolverError::Infrastructure(format!("task join error: {e}"))
        })?
    }
}

/// Runtime start の外部識別子はファイル名ではなく WorkflowDefinition.name である。
///
/// ユーザーが YAML を直接編集・rename した場合も同じ規則を保つため、全 definition を
/// load した後に name で一意解決する。複数ファイル（または builtin）が同じ name を
/// 宣言した状態を先勝ちにすると実行対象が directory iteration order に依存するため、
/// §7 の閉じた code 表で workflow name の妥当性にも使われる WFS006 Diagnostic として
/// 明示的に拒否する（表にない新 code は追加しない）。
pub(crate) fn resolve_workflow_by_name(
    workflows_dir: &Path,
    facets_base_dir: &Path,
    workflow_name: &str,
) -> Result<Workflow, WorkflowDefinitionResolverError> {
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
            .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("yml"))
            .collect::<Vec<_>>();
        paths.sort();

        for path in paths {
            match crate::adaptor::gateway::workflow::storage::load_workflow(&path, facets_base_dir)
            {
                Ok(workflow) if workflow.name == workflow_name => {
                    matches.push((path.display().to_string(), workflow));
                }
                Ok(_) => {}
                Err(error)
                    if path.file_stem().and_then(|stem| stem.to_str()) == Some(workflow_name) =>
                {
                    exact_file_error = Some(error.to_string());
                }
                Err(error) => {
                    log::warn!("workflow definition skipped: {}: {error}", path.display());
                }
            }
        }
    }

    for summary in crate::adaptor::gateway::workflow::builtin::list_builtin_workflows() {
        match crate::adaptor::gateway::workflow::builtin::load_builtin_workflow_resolved(
            &summary.name,
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
            Err(WorkflowDefinitionResolverError::InvalidWorkflow(format!(
                "workflow_diagnostics: WFS006: workflow name '{workflow_name}' is duplicated: {sources}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::resolve_workflow_by_name;
    use crate::adaptor::gateway::workflow::schema::{
        CommandSpec, NodeDefinition, NodeKind, Workflow,
    };
    use crate::adaptor::gateway::workflow::storage;

    fn workflow(name: &str) -> Workflow {
        Workflow {
            name: name.to_string(),
            description: "test workflow".to_string(),
            nodes: vec![NodeDefinition {
                name: "node1".to_string(),
                kind: NodeKind::Command(CommandSpec {
                    command: "true".to_string(),
                }),
                ..NodeDefinition::default()
            }],
            ..Workflow::default()
        }
    }

    #[test]
    fn resolves_definition_name_when_filename_differs() {
        let tmp = TempDir::new().unwrap();
        storage::save_workflow(tmp.path(), &workflow("declared-name")).unwrap();
        std::fs::rename(
            tmp.path().join("declared-name.yml"),
            tmp.path().join("different-filename.yml"),
        )
        .unwrap();

        let resolved = resolve_workflow_by_name(tmp.path(), tmp.path(), "declared-name").unwrap();

        assert_eq!(resolved.name, "declared-name");
    }

    #[test]
    fn duplicate_definition_names_are_reported_as_diagnostic() {
        let tmp = TempDir::new().unwrap();
        storage::save_workflow(tmp.path(), &workflow("duplicate-name")).unwrap();
        std::fs::copy(
            tmp.path().join("duplicate-name.yml"),
            tmp.path().join("second-file.yml"),
        )
        .unwrap();

        let error = resolve_workflow_by_name(tmp.path(), tmp.path(), "duplicate-name")
            .expect_err("duplicate names must not be selected by directory order");

        assert!(error.to_string().contains("WFS006"));
        assert!(error.to_string().contains("duplicate-name"));
    }
}

pub(crate) struct AppConfigManagedWorktreeResolver {
    usecase: Arc<RepositoryUsecase>,
    config: Arc<dyn ConfigRepository>,
}

impl AppConfigManagedWorktreeResolver {
    pub(crate) fn new(usecase: Arc<RepositoryUsecase>, config: Arc<dyn ConfigRepository>) -> Self {
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
        .map_err(ManagedWorktreeResolverError::Validation)
    }
}
