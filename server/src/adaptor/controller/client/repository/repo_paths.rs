use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::error::AppError;

pub(crate) async fn add_repo_path_shared(state: &AppState, path: String) -> Result<bool, AppError> {
    let uc = state.repo_paths_usecase.clone();
    super::run_blocking(move || uc.add(&path)).await
}

pub(crate) async fn remove_repo_path_shared(
    state: &AppState,
    path: String,
) -> Result<bool, AppError> {
    let uc = state.repo_paths_usecase.clone();
    super::run_blocking(move || uc.remove(&path)).await
}
