use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::error::AppError;
use crate::usecase::repository_dto::WorktreeEntryDto;

pub(crate) async fn create_worktree_shared(
    state: &AppState,
    repo_path: String,
    branch: String,
    create_branch: bool,
    base_branch: Option<String>,
) -> Result<WorktreeEntryDto, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || {
        uc.create_worktree(&repo_path, &branch, create_branch, base_branch.as_deref())
    })
    .await
}

pub(crate) async fn remove_worktree_shared(
    state: &AppState,
    runtime: std::sync::Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
    repo_path: String,
    worktree_path: String,
    force: bool,
) -> Result<(), AppError> {
    let uc = state.repository_usecase.clone();
    let handle = tokio::runtime::Handle::current();
    run_blocking(move || {
        handle.block_on(uc.remove_worktree(runtime.as_ref(), &repo_path, &worktree_path, force))
    })
    .await
}
