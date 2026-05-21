use std::sync::Arc;

use crate::config::AppConfig;
use crate::workflow::resolver::{
    ManagedWorktreeResolver, ManagedWorktreeResolverError, WorkflowDefinitionResolver,
    WorkflowDefinitionResolverError,
};
use crate::workflow::schema::Workflow;

pub(crate) struct DefaultWorkflowDefinitionResolver;

#[async_trait::async_trait]
impl WorkflowDefinitionResolver for DefaultWorkflowDefinitionResolver {
    async fn resolve(&self, file_stem: &str) -> Result<Workflow, WorkflowDefinitionResolverError> {
        let load_stem = file_stem.to_string();
        tokio::task::spawn_blocking(move || {
            let dir = crate::workflow::storage::workflows_dir();
            let facets_base = crate::workflow::facet::facets_base_dir();
            let file_path = dir.join(format!("{load_stem}.yml"));
            if file_path.exists() {
                crate::workflow::storage::load_workflow(&file_path, &facets_base)
                    .map_err(|e| WorkflowDefinitionResolverError::InvalidWorkflow(e.to_string()))
            } else {
                crate::workflow::builtin::load_builtin_workflow_resolved(&load_stem)
                    .map_err(|e| WorkflowDefinitionResolverError::InvalidWorkflow(e.to_string()))?
                    .ok_or_else(|| {
                        WorkflowDefinitionResolverError::InvalidWorkflow(format!(
                            "ワークフロー '{load_stem}' が見つかりません"
                        ))
                    })
            }
        })
        .await
        .map_err(|e| {
            WorkflowDefinitionResolverError::Infrastructure(format!("task join error: {e}"))
        })?
    }
}

pub(crate) struct AppConfigManagedWorktreeResolver {
    config: Arc<AppConfig>,
}

impl AppConfigManagedWorktreeResolver {
    pub(crate) fn new(config: Arc<AppConfig>) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl ManagedWorktreeResolver for AppConfigManagedWorktreeResolver {
    async fn resolve(&self, worktree_path: String) -> Result<String, ManagedWorktreeResolverError> {
        crate::workflow::worktree::canonicalize_managed_worktree_path(
            Arc::clone(&self.config),
            worktree_path,
        )
        .await
        .map_err(ManagedWorktreeResolverError::Validation)
    }
}

#[cfg(test)]
pub(crate) struct PassthroughManagedWorktreeResolver;

#[cfg(test)]
#[async_trait::async_trait]
impl ManagedWorktreeResolver for PassthroughManagedWorktreeResolver {
    async fn resolve(&self, worktree_path: String) -> Result<String, ManagedWorktreeResolverError> {
        Ok(worktree_path)
    }
}
