use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;
use crate::usecase::git_host::IssueInfoDto;

#[tauri::command]
pub async fn fetch_issues(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<Vec<IssueInfoDto>, AppError> {
    let uc = state.git_host_usecase.clone();
    run_blocking(move || {
        uc.fetch_issues(&repo_path)
            .into_iter()
            .map(Into::into)
            .collect()
    })
    .await
}

#[tauri::command]
pub async fn get_cached_issues(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<Vec<IssueInfoDto>, AppError> {
    let uc = state.git_host_usecase.clone();
    run_blocking(move || {
        uc.get_cached_issues(&repo_path)
            .into_iter()
            .map(Into::into)
            .collect()
    })
    .await
}
