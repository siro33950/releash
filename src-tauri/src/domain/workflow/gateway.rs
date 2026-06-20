//! External interaction ports for workflow.

use crate::domain::workflow::WorkflowError;

pub trait ManagedWorktreeGateway: Send + Sync {
    fn resolve(&self, worktree_path: &str) -> Result<String, WorkflowError>;
}

pub trait SecretSourceGateway: Send + Sync {
    fn configured_secret_values(&self) -> Result<Vec<String>, WorkflowError>;
}
