//! staging（差分 Approve）の Tauri コマンド。

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::error::AppError;

pub(crate) async fn git_stage_shared(
    state: &AppState,
    repo_path: String,
    paths: Vec<String>,
) -> Result<(), AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || uc.git_stage(&repo_path, paths)).await
}

pub(crate) async fn git_unstage_shared(
    state: &AppState,
    repo_path: String,
    paths: Vec<String>,
) -> Result<(), AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || uc.git_unstage(&repo_path, paths)).await
}
