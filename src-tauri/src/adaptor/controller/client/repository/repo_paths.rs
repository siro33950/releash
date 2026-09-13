use crate::adaptor::controller::state::AppState;
use crate::other::AppError;

pub(crate) fn get_repo_paths_shared(state: &AppState) -> Vec<String> {
    state.repo_paths_usecase.get()
}

// 変更通知（repo-paths-changed）の gating は usecase が担うため、controller は
// usecase を呼ぶ薄い入口に徹する（emit は注入された NotifyGateway 経由）。

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
