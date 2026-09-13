use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;
use crate::usecase::git_host::PrStatusDto;

pub(crate) async fn fetch_pr_status_shared(
    state: &AppState,
    repo_path: String,
) -> Result<PrStatusDto, AppError> {
    let uc = state.git_host_usecase.clone();
    run_blocking(move || PrStatusDto::from(uc.fetch_pr_status(&repo_path))).await
}

pub(crate) async fn get_cached_pr_status_shared(
    state: &AppState,
    repo_path: String,
) -> Result<PrStatusDto, AppError> {
    let uc = state.git_host_usecase.clone();
    run_blocking(move || PrStatusDto::from(uc.get_cached_pr_status(&repo_path))).await
}
