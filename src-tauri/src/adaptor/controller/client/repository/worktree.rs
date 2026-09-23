use super::{run_blocking, run_repository_state};
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;
use crate::usecase::repository_dto::WorktreeEntryDto;
use crate::usecase::repository_state::snapshot::RepositoryBranchCardsSnapshotDto;

pub(crate) async fn get_main_repo_path_shared(
    state: &AppState,
    any_path: String,
) -> Result<String, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.get_main_repo_path(&any_path)).await
}

pub(crate) async fn get_worktree_dirty_count_shared(
    state: &AppState,
    worktree_path: String,
) -> Result<u32, AppError> {
    let service = state.repository_state.clone();
    run_repository_state(move || service.get_worktree_dirty_count(&worktree_path)).await
}

pub(crate) async fn list_worktrees_shared(
    state: &AppState,
    repo_path: String,
) -> Result<Vec<WorktreeEntryDto>, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.list_worktrees(&repo_path)).await
}

pub(crate) async fn list_branches_with_status_snapshot_shared(
    state: &AppState,
    repo_path: String,
) -> Result<RepositoryBranchCardsSnapshotDto, AppError> {
    let service = state.repository_state.clone();
    run_repository_state(move || service.list_branches_with_status_snapshot(&repo_path)).await
}

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
