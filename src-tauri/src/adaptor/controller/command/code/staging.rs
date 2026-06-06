//! staging（差分 Approve）の Tauri コマンド。

use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;

#[tauri::command]
pub async fn git_stage(
    state: State<'_, AppState>,
    repo_path: String,
    paths: Vec<String>,
) -> Result<(), AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || uc.git_stage(&repo_path, paths)).await
}

#[tauri::command]
pub async fn git_unstage(
    state: State<'_, AppState>,
    repo_path: String,
    paths: Vec<String>,
) -> Result<(), AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || uc.git_unstage(&repo_path, paths)).await
}

#[tauri::command]
pub async fn git_stage_hunk(
    state: State<'_, AppState>,
    repo_path: String,
    patch: String,
) -> Result<(), AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || uc.git_stage_hunk(&repo_path, &patch)).await
}

#[tauri::command]
pub async fn git_unstage_hunk(
    state: State<'_, AppState>,
    repo_path: String,
    patch: String,
) -> Result<(), AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || uc.git_unstage_hunk(&repo_path, &patch)).await
}
