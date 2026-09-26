//! code 責務の Tauri コマンド（薄い入口）。
//!
//! 引数の受け渡しと型変換のみを行い、ビジネスロジックは usecase / query service に
//! 委ねる。git2 等のブロッキング呼び出しを非同期境界へ載せるため、各コマンドは
//! `run_blocking` でユースケースを呼ぶ（移行前 `git/commands.rs` の `blocking` と等価）。

mod shared;
pub(crate) use shared::register_shared;

pub(crate) mod diff;
pub(crate) mod hunk;
pub(crate) mod language;
pub(crate) mod markdown;
pub(crate) mod review;
pub(crate) mod staging;

use crate::adaptor::presenter::error::AppError;
use crate::usecase::code_error::CodeUsecaseError;

/// ユースケース呼び出しを `spawn_blocking` 上で実行し、結果を `AppError` に集約する
/// 共通ヘルパー。join 失敗時のメッセージは移行前と等価に保つ。
pub(crate) async fn run_blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CodeUsecaseError> + Send + 'static,
{
    crate::adaptor::controller::client::worktree_mutation::spawn_blocking(f)
        .await
        .map_err(|e| AppError::new(format!("task join error: {e}")))?
        .map_err(AppError::from)
}
