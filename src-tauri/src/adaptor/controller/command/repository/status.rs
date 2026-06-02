use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::domain::repository::{FileDiffStat, FileStatus};
use crate::other::AppError;

#[tauri::command]
pub async fn get_git_status(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<Vec<FileStatus>, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.get_git_status(&repo_path)).await
}

#[tauri::command]
pub async fn get_status_diff_stats(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<Vec<FileDiffStat>, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.get_status_diff_stats(&repo_path)).await
}
