use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;
use crate::usecase::git_host::PrStatusDto;

#[tauri::command]
pub async fn fetch_pr_status(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<PrStatusDto, AppError> {
    let uc = state.git_host_usecase.clone();
    run_blocking(move || PrStatusDto::from(uc.fetch_pr_status(&repo_path))).await
}

#[tauri::command]
pub async fn get_cached_pr_status(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<PrStatusDto, AppError> {
    let uc = state.git_host_usecase.clone();
    run_blocking(move || PrStatusDto::from(uc.get_cached_pr_status(&repo_path))).await
}
