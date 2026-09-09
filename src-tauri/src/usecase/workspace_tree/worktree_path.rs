use crate::domain::workflow::WorkflowError;
use std::sync::Arc;

pub(crate) trait WorkspaceWorktreePathQuery: Send + Sync {
    fn workspace_worktree_path(&self, path: &str) -> Result<String, WorkflowError>;
}

pub(crate) struct WorkspaceWorktreePathUsecase {
    query: Arc<dyn WorkspaceWorktreePathQuery>,
}

impl WorkspaceWorktreePathUsecase {
    pub(crate) fn new(query: Arc<dyn WorkspaceWorktreePathQuery>) -> Self {
        Self { query }
    }

    pub(crate) fn workspace_worktree_path(&self, path: &str) -> Result<String, WorkflowError> {
        self.query.workspace_worktree_path(path)
    }
}

#[cfg(test)]
#[path = "worktree_path_test.rs"]
mod worktree_path_tests;
