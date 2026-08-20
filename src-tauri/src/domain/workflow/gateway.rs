//! External interaction ports for workflow.

use crate::domain::workflow::{RepositoryWorktreeInventory, WorkflowError};

pub trait ManagedWorktreeGateway: Send + Sync {
    fn resolve(&self, worktree_path: &str) -> Result<String, WorkflowError>;
}

/// Git worktree の実体を変更能力なしで照会する port。
pub trait WorktreeInventoryGateway: Send + Sync {
    fn snapshot(&self) -> Result<Vec<RepositoryWorktreeInventory>, WorkflowError>;
}

pub trait SecretSourceGateway: Send + Sync {
    fn configured_secret_values(&self) -> Result<Vec<String>, WorkflowError>;
}
