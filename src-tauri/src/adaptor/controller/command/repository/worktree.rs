use tauri::State;

use super::{run_blocking, run_repository_state};
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;
use crate::usecase::repository_dto::{BranchCardDto, WorktreeEntryDto};
use crate::usecase::repository_state::snapshot::RepositoryBranchCardsSnapshotDto;

#[tauri::command]
pub async fn get_main_repo_path(
    state: State<'_, AppState>,
    any_path: String,
) -> Result<String, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.get_main_repo_path(&any_path)).await
}

#[tauri::command]
pub async fn get_worktree_dirty_count(
    state: State<'_, AppState>,
    worktree_path: String,
) -> Result<u32, AppError> {
    let service = state.repository_state.clone();
    run_repository_state(move || service.get_worktree_dirty_count(&worktree_path)).await
}

#[tauri::command]
pub async fn list_worktrees(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<Vec<WorktreeEntryDto>, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.list_worktrees(&repo_path)).await
}

#[tauri::command]
pub async fn list_branches_with_status(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<Vec<BranchCardDto>, AppError> {
    let service = state.repository_state.clone();
    run_repository_state(move || service.list_branches_with_status(&repo_path)).await
}

#[tauri::command]
pub async fn list_branches_with_status_snapshot(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<RepositoryBranchCardsSnapshotDto, AppError> {
    let service = state.repository_state.clone();
    run_repository_state(move || service.list_branches_with_status_snapshot(&repo_path)).await
}

#[tauri::command]
pub async fn create_worktree(
    state: State<'_, AppState>,
    repo_path: String,
    worktree_path: String,
    branch: String,
    create_branch: bool,
    base_branch: Option<String>,
) -> Result<WorktreeEntryDto, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || {
        uc.create_worktree(
            &repo_path,
            &worktree_path,
            &branch,
            create_branch,
            base_branch.as_deref(),
        )
    })
    .await
}

#[tauri::command]
pub async fn remove_worktree(
    state: State<'_, AppState>,
    repo_path: String,
    worktree_path: String,
    force: bool,
) -> Result<(), AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.remove_worktree(&repo_path, &worktree_path, force)).await
}
