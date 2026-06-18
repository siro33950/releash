use crate::domain::workspace_state::{WorkspaceState, WorkspaceStateRepository};

pub fn save_workspace_state(
    repository: &dyn WorkspaceStateRepository,
    worktree_name: &str,
    state: WorkspaceState,
) -> Result<(), String> {
    repository.set(worktree_name, state);
    repository.save(worktree_name)
}
