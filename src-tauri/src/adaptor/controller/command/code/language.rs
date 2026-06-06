//! 言語判定の Tauri コマンド。

use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;

#[tauri::command]
pub async fn get_language_from_path(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || Ok(uc.get_language_from_path(&file_path))).await
}
