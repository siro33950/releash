use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::error::AppError;

pub(crate) async fn remove_worktree_shared(
    state: &AppState,
    runtime: std::sync::Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
    repo_path: String,
    worktree_path: String,
    force: bool,
) -> Result<(), AppError> {
    state
        .git_usecase
        .remove_worktree(runtime.as_ref(), &repo_path, &worktree_path, force)
        .await
        .map_err(AppError::from)
}
