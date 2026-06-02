use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::domain::repository::Commit;
use crate::other::AppError;

#[tauri::command]
pub async fn get_git_log(
    state: State<'_, AppState>,
    repo_path: String,
    limit: Option<usize>,
) -> Result<Vec<Commit>, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.get_git_log(&repo_path, limit)).await
}
