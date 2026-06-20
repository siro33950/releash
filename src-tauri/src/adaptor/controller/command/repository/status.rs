use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;
use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};

#[tauri::command]
pub async fn get_git_status(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<Vec<FileStatusDto>, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || {
        uc.get_git_status(&repo_path)
            .map(|statuses| statuses.into_iter().map(Into::into).collect())
    })
    .await
}

#[tauri::command]
pub async fn get_status_diff_stats(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<Vec<FileDiffStatDto>, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || {
        uc.get_status_diff_stats(&repo_path)
            .map(|stats| stats.into_iter().map(Into::into).collect())
    })
    .await
}
