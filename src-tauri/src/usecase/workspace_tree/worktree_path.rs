use crate::domain::workflow::WorkflowError;
use std::sync::Arc;

#[async_trait::async_trait]
pub(crate) trait WorkspaceWorktreePathQuery: Send + Sync {
    async fn workspace_worktree_path(&self, path: &str) -> Result<String, WorkflowError>;
}

pub(crate) struct WorkspaceWorktreePathUsecase {
    query: Arc<dyn WorkspaceWorktreePathQuery>,
}

impl WorkspaceWorktreePathUsecase {
    pub(crate) fn new(query: Arc<dyn WorkspaceWorktreePathQuery>) -> Self {
        Self { query }
    }

    pub(crate) async fn workspace_worktree_path(
        &self,
        path: &str,
    ) -> Result<String, WorkflowError> {
        self.query.workspace_worktree_path(path).await
    }
}

#[cfg(test)]
#[path = "worktree_path_test.rs"]
mod worktree_path_tests;
