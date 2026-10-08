use crate::adaptor::presenter::error::AppError;
use crate::usecase::git_host::GitHostUsecase;

pub(crate) async fn fetch_issues_shared(
    usecase: &GitHostUsecase,
    repo_path: String,
) -> Result<(), AppError> {
    usecase
        .fetch_issues(&repo_path)
        .await
        .map(|_| ())
        .map_err(AppError::from_failure)
}
