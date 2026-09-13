use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;
use crate::usecase::repository_dto::BranchDto;

pub(crate) async fn list_branches_shared(
    state: &AppState,
    repo_path: String,
) -> Result<Vec<BranchDto>, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || {
        uc.list_branches(&repo_path)
            .map(|branches| branches.into_iter().map(Into::into).collect())
    })
    .await
}

pub(crate) async fn get_default_branch_shared(
    state: &AppState,
    repo_path: String,
) -> Result<String, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.get_default_branch(&repo_path)).await
}

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
    repo_path: String,
    branch_name: String,
    force: bool,
) -> Result<(), AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.delete_branch(&repo_path, &branch_name, force)).await
}
