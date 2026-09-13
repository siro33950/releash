use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;
use crate::usecase::repository_dto::CommitDto;

pub(crate) async fn get_git_log_shared(
    state: &AppState,
    repo_path: String,
    limit: Option<usize>,
) -> Result<Vec<CommitDto>, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || {
        uc.get_git_log(&repo_path, limit)
            .map(|commits| commits.into_iter().map(Into::into).collect())
    })
    .await
}
