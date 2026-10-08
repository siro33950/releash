use crate::domain::workflow::WorkflowError;

#[async_trait::async_trait]
pub trait WorkspaceWorktreePathQuery: Send + Sync {
    async fn workspace_worktree_path(&self, path: &str) -> Result<String, WorkflowError>;
}
