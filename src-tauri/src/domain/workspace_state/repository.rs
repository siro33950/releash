use crate::domain::workspace_state::{WorkspaceState, WorkspaceStateError};

pub trait WorkspaceStateRepository: Send + Sync {
    fn load(&self, worktree_name: &str, worktree_root: &str) -> Option<WorkspaceState>;
    fn save(&self, worktree_name: &str) -> Result<(), WorkspaceStateError>;
    fn set(&self, worktree_name: &str, state: WorkspaceState);
}
