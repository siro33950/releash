use crate::domain::workspace_state::{WorkspaceState, WorkspaceStateRepository};

pub fn load_workspace_state(
    repository: &dyn WorkspaceStateRepository,
    worktree_name: &str,
    worktree_root: &str,
) -> Result<Option<WorkspaceState>, crate::domain::workspace_state::WorkspaceStateError> {
    repository.load(worktree_name, worktree_root)
}
