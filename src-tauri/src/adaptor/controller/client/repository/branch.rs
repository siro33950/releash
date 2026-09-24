use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;

pub(crate) async fn git_create_branch_shared(
    state: &AppState,
    repo_path: String,
    branch_name: String,
) -> Result<(), AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.create_branch(&repo_path, &branch_name)).await
}

pub(crate) async fn delete_branch_shared(
    state: &AppState,
    runtime: std::sync::Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
    repo_path: String,
    branch_name: String,
    force: bool,
) -> Result<(), AppError> {
    let uc = state.repository_usecase.clone();
    let handle = tokio::runtime::Handle::current();
    run_blocking(move || {
        handle.block_on(uc.delete_branch(runtime.as_ref(), &repo_path, &branch_name, force))
    })
    .await
}
