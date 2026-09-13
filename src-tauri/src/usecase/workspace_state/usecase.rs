use crate::domain::workspace_state::{
    WorkspaceState, WorkspaceStateError, WorkspaceStateRepository,
};

pub fn save_workspace_state(
    repository: &dyn WorkspaceStateRepository,
    worktree_name: &str,
    state: WorkspaceState,
) -> Result<(), WorkspaceStateError> {
    repository.set(worktree_name, state);
    repository.save(worktree_name)
}

pub fn load_workspace_state(
    repository: &dyn WorkspaceStateRepository,
    worktree_name: &str,
    worktree_root: &str,
) -> Option<WorkspaceState> {
    super::query_service::load_workspace_state(repository, worktree_name, worktree_root)
}
