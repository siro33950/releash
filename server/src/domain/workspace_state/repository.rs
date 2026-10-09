use crate::domain::workspace_state::{WorkspaceState, WorkspaceStateError};

pub trait WorkspaceStateRepository: Send + Sync {
    fn load_repository_group(&self, repository_path: &str) -> Result<bool, WorkspaceStateError>;
    fn save_repository_group(
        &self,
        repository_path: &str,
        collapsed: bool,
    ) -> Result<(), WorkspaceStateError>;
    fn load(
        &self,
        worktree_name: &str,
        worktree_root: &str,
    ) -> Result<Option<WorkspaceState>, crate::domain::workspace_state::WorkspaceStateError>;
    fn check_readable(&self, worktree_name: &str) -> Result<(), WorkspaceStateError>;
    fn save(&self, worktree_name: &str) -> Result<(), WorkspaceStateError>;
    fn set(&self, worktree_name: &str, state: WorkspaceState);
}
