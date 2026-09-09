//! External interaction ports for workflow.

use crate::domain::workflow::WorkflowError;

pub trait ManagedWorktreeGateway: Send + Sync {
    fn resolve(&self, worktree_path: &str) -> Result<String, WorkflowError>;
}

pub trait IsolatedWorktreeGateway: Send + Sync {
    fn repository_root(&self, worktree_path: &str) -> Result<String, WorkflowError>;
    fn is_created(
        &self,
        parent_worktree_path: &str,
        worktree: &crate::domain::workflow::IsolatedWorktree,
    ) -> Result<bool, WorkflowError>;
    fn create(
        &self,
        parent_worktree_path: &str,
        worktree: &crate::domain::workflow::IsolatedWorktree,
    ) -> Result<(), WorkflowError>;
}

pub trait SecretSourceGateway: Send + Sync {
    fn configured_secret_values(&self) -> Result<Vec<String>, WorkflowError>;
}
