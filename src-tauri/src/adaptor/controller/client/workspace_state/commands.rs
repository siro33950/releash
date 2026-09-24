use crate::other::AppError;
use std::sync::Arc;

use crate::adaptor::gateway::workspace_state::WorkspaceStateStore;
use crate::usecase::workspace_state::dto::WorkspaceStateDto;

pub(crate) fn load_workspace_state_shared(
    store: &Arc<WorkspaceStateStore>,
    worktree_name: String,
    worktree_root: String,
) -> Option<WorkspaceStateDto> {
    crate::usecase::workspace_state::usecase::load_workspace_state(
        store.as_ref(),
        &worktree_name,
        &worktree_root,
    )
    .map(WorkspaceStateDto::from)
}

pub(crate) fn save_workspace_state_shared(
    store: &Arc<WorkspaceStateStore>,
    worktree_name: String,
    state: WorkspaceStateDto,
) -> Result<(), AppError> {
    crate::usecase::workspace_state::usecase::save_workspace_state(
        store.as_ref(),
        &worktree_name,
        state.into(),
    )
    .map_err(AppError::from_failure)
}
