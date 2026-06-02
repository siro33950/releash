use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;

#[tauri::command]
pub async fn get_cwd(state: State<'_, AppState>) -> Result<String, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.get_cwd()).await
}

#[tauri::command]
pub async fn get_repo_git_dir(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<String, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.get_repo_git_dir(&file_path)).await
}
