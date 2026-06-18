use std::sync::Arc;

use crate::adaptor::gateway::workflow::resolver::{
    ManagedWorktreeResolver, ManagedWorktreeResolverError, WorkflowDefinitionResolver,
    WorkflowDefinitionResolverError,
};
use crate::adaptor::gateway::workflow::schema::Workflow;
use crate::config::AppConfig;
use crate::usecase::repository_usecase::RepositoryUsecase;

pub(crate) struct DefaultWorkflowDefinitionResolver;

#[async_trait::async_trait]
impl WorkflowDefinitionResolver for DefaultWorkflowDefinitionResolver {
    async fn resolve(&self, file_stem: &str) -> Result<Workflow, WorkflowDefinitionResolverError> {
        let load_stem = file_stem.to_string();
        tokio::task::spawn_blocking(move || {
            let dir = crate::adaptor::gateway::workflow::storage::workflows_dir();
            let facets_base = crate::adaptor::gateway::workflow::facet::facets_base_dir();
            let file_path = dir.join(format!("{load_stem}.yml"));
            if file_path.exists() {
                match crate::adaptor::gateway::workflow::storage::load_workflow(
                    &file_path,
                    &facets_base,
                ) {
                    Ok(wf) => return Ok(wf),
                    Err(e)
                        if crate::adaptor::gateway::workflow::builtin::is_builtin_workflow(
                            &load_stem,
                        ) =>
                    {
                        log::warn!(
                            "user-side workflow '{load_stem}' failed to load ({e}); falling back to builtin"
                        );
                    }
                    Err(e) => {
                        return Err(WorkflowDefinitionResolverError::InvalidWorkflow(
                            e.to_string(),
                        ));
                    }
                }
            }
            crate::adaptor::gateway::workflow::builtin::load_builtin_workflow_resolved(&load_stem)
                .map_err(|e| WorkflowDefinitionResolverError::InvalidWorkflow(e.to_string()))?
                .ok_or_else(|| {
                    WorkflowDefinitionResolverError::InvalidWorkflow(format!(
                        "ワークフロー '{load_stem}' が見つかりません"
                    ))
                })
        })
        .await
        .map_err(|e| {
            WorkflowDefinitionResolverError::Infrastructure(format!("task join error: {e}"))
        })?
    }
}

pub(crate) struct AppConfigManagedWorktreeResolver {
    usecase: Arc<RepositoryUsecase>,
    config: Arc<AppConfig>,
}

impl AppConfigManagedWorktreeResolver {
    pub(crate) fn new(usecase: Arc<RepositoryUsecase>, config: Arc<AppConfig>) -> Self {
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
