//! ファイル内容参照（at_ref / at_branch_base / staged、テキスト／バイナリ）の Tauri コマンド。

use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;

#[tauri::command]
pub async fn get_file_at_ref(
    state: State<'_, AppState>,
    file_path: String,
    git_ref: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || uc.get_file_at_ref(&file_path, &git_ref),
        )
    })
    .await
}

#[tauri::command]
pub async fn get_staged_content(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || uc.get_staged_content(&file_path),
        )
    })
    .await
}

#[tauri::command]
pub async fn get_binary_staged_content(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || uc.get_binary_staged_content(&file_path),
        )
    })
    .await
}

#[tauri::command]
pub async fn get_file_at_branch_base(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || uc.get_file_at_branch_base(&file_path),
        )
    })
    .await
}

#[tauri::command]
pub async fn get_binary_file_at_branch_base(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || uc.get_binary_file_at_branch_base(&file_path),
        )
    })
    .await
}

#[tauri::command]
pub async fn get_binary_file_at_ref(
    state: State<'_, AppState>,
    file_path: String,
    git_ref: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || uc.get_binary_file_at_ref(&file_path, &git_ref),
        )
    })
    .await
}
