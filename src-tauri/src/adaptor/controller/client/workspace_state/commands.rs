use crate::other::AppError;
use std::sync::Arc;

use crate::adaptor::gateway::workspace_state::WorkspaceStateStore;
use crate::usecase::workspace_state::dto::WorkspaceStateDto;

pub(crate) fn save_workspace_state_shared(
    store: &Arc<WorkspaceStateStore>,
    publisher: Option<&crate::usecase::state_subscription::StateSubscriptionPublisher>,
    worktree_name: String,
    state: WorkspaceStateDto,
) -> Result<(), AppError> {
    crate::usecase::workspace_state::usecase::save_workspace_state(
        store.as_ref(),
        publisher,
        &worktree_name,
        state.into(),
    )
    .map_err(AppError::from_failure)
}
