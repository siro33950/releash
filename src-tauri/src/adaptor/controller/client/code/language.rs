//! 言語判定の Tauri コマンド。

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;

pub(crate) async fn get_language_from_path_shared(
    state: &AppState,
    file_path: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || Ok(uc.get_language_from_path(&file_path))).await
}
