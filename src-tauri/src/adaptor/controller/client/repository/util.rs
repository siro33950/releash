use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;

pub(crate) async fn get_cwd_shared(state: &AppState) -> Result<String, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.get_cwd()).await
}

pub(crate) async fn get_repo_git_dir_shared(
    state: &AppState,
    file_path: String,
) -> Result<String, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.get_repo_git_dir(&file_path)).await
}
