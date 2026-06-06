//! ファイルメンション候補列挙の Tauri コマンド。
//!
//! 移行前 `file_mention::list_mentionable_files` は同期コマンドであったため、観測可能な
//! 振る舞いを保つよう同期コマンドのまま usecase へ委譲する。

use tauri::State;

use crate::adaptor::controller::state::AppState;
use crate::other::AppError;

#[tauri::command]
pub fn list_mentionable_files(
    state: State<'_, AppState>,
    worktree_path: String,
    query: String,
) -> Result<Vec<String>, AppError> {
    state
        .code_usecase
        .list_mentionable_files(&worktree_path, &query)
        .map_err(AppError::from)
}
