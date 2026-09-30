use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::error::AppError;

pub(crate) async fn git_create_branch_shared(
    state: &AppState,
    repo_path: String,
    branch_name: String,
) -> Result<(), AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.create_branch(&repo_path, &branch_name)).await
}
