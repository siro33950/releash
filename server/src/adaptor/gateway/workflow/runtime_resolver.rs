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
        _workflow_name: &str,
    ) -> Result<crate::domain::workflow::WorkflowDefinitionContent, WorkflowDefinitionResolverError>
    {
        todo!()
    }
}

/// Runtime start の外部識別子はファイル名ではなく WorkflowDefinitionContent.name である。
///
/// ユーザーが YAML を直接編集・rename した場合も同じ規則を保つため、全 definition を
/// load した後に name で一意解決する。複数ファイル（または builtin）が同じ name を
/// 宣言した状態を先勝ちにすると実行対象が directory iteration order に依存するため、
/// §7 の閉じた code 表で workflow name の妥当性にも使われる WFS006 Diagnostic として
/// 明示的に拒否する（表にない新 code は追加しない）。
pub fn resolve_workflow_by_name(
    workflows_dir: &Path,
    facets_base_dir: &Path,
    workflow_name: &str,
) -> Result<WorkflowDefinitionYaml, WorkflowDefinitionResolverError> {
    todo!()
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
