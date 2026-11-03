use crate::adaptor::presenter::error::AppError;

use crate::domain::local_event::StreamVersion;
use crate::domain::workspace_state::{WorkspaceState, WorkspaceStateRepository};

pub(crate) async fn save_workspace_state_shared(
    store: &dyn WorkspaceStateRepository,
    publisher: Option<&crate::usecase::state_subscription::StateSubscriptionUsecase>,
    worktree_name: String,
    state: WorkspaceState,
    expected: StreamVersion,
) -> Result<(StreamVersion, WorkspaceState), AppError> {
    crate::usecase::workspace_state::usecase::save_workspace_state(
        store,
        publisher,
        &worktree_name,
        state,
        expected,
    )
    .await
    .map_err(AppError::from)
}

pub(crate) async fn save_repository_group_state_shared(
    store: &dyn WorkspaceStateRepository,
    publisher: Option<&crate::usecase::state_subscription::StateSubscriptionUsecase>,
    path: &str,
    collapsed: bool,
) -> Result<(), AppError> {
    crate::usecase::workspace_state::usecase::save_repository_group_state(
        store, publisher, path, collapsed,
    )
    .await
    .map_err(AppError::from)
}
