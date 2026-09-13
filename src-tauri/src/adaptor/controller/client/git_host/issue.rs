use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;
use crate::usecase::git_host::IssueInfoDto;

pub(crate) async fn fetch_issues_shared(
    state: &AppState,
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

pub(crate) async fn get_cached_issues_shared(
    state: &AppState,
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
