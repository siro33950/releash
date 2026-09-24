use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;

pub(crate) async fn get_releash_base_shared(
    state: &AppState,
    repo_path: String,
) -> Result<Option<String>, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.get_releash_base(&repo_path)).await
}

pub(crate) async fn set_releash_base_shared(
    state: &AppState,
    repo_path: String,
    base: Option<String>,
) -> Result<(), AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.set_releash_base(&repo_path, base.as_deref())).await
}

// Tauri コマンド登録名は外部契約のため `set_branch_base` のまま据え置く。
// 内部の usecase / domain trait は per-branch override を書く責務を明示する
// `set_branch_base_override` へリネーム済み。

pub(crate) async fn set_branch_base_shared(
    state: &AppState,
    repo_path: String,
    branch_name: String,
    base: Option<String>,
) -> Result<(), AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.set_branch_base_override(&repo_path, &branch_name, base.as_deref()))
        .await
}
