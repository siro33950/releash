use tauri::State;

use super::run_repository_state;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;
use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};
use crate::usecase::repository_state::snapshot::{
    RepositoryDiffStatsSnapshotDto, RepositoryStatusSnapshotDto,
};

#[tauri::command]
pub async fn get_git_status(
    state: State<'_, AppState>,
    repo_path: String,
    include_ignored: Option<bool>,
) -> Result<Vec<FileStatusDto>, AppError> {
    let service = state.repository_state.clone();
    run_repository_state(move || service.get_status(&repo_path, include_ignored.unwrap_or(false)))
        .await
}

#[tauri::command]
pub async fn get_git_status_snapshot(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<RepositoryStatusSnapshotDto, AppError> {
    let service = state.repository_state.clone();
    run_repository_state(move || service.get_status_snapshot(&repo_path)).await
}

#[tauri::command]
pub async fn get_status_diff_stats(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<Vec<FileDiffStatDto>, AppError> {
    let service = state.repository_state.clone();
    run_repository_state(move || service.get_diff_stats(&repo_path)).await
}

#[tauri::command]
pub async fn get_status_diff_stats_snapshot(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<RepositoryDiffStatsSnapshotDto, AppError> {
    let service = state.repository_state.clone();
    run_repository_state(move || service.get_diff_stats_snapshot(&repo_path)).await
}
