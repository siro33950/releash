use super::run_blocking;
use crate::adaptor::presenter::error::AppError;
use crate::usecase::git_host::GitHostUsecase;

pub(crate) async fn fetch_issues_shared(
    usecase: &GitHostUsecase,
    repo_path: String,
) -> Result<(), AppError> {
    let uc = usecase.clone();
    run_blocking(move || {
        uc.fetch_issues(&repo_path)
            .map(|_| ())
            .map_err(AppError::from_failure)
    })
    .await?
}
