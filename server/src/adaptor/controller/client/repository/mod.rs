//! repository 責務の ClientService の入口。
//!
//! 引数の受け渡しと型変換のみを行い、ビジネスロジックは usecase /
//! query service に委ねる。git2 のブロッキング呼び出しを非同期境界へ
//! 載せるため、各コマンドは `spawn_blocking` でユースケースを呼ぶ。

pub(crate) mod shared;
pub(crate) use shared::register_shared;

pub(crate) mod branch;
pub(crate) mod git_config;
pub(crate) mod repo_paths;
pub(crate) mod worktree;

use crate::adaptor::presenter::error::AppError;
use crate::usecase::repository_error::UsecaseError;

/// ユースケースエラー → アプリエラーの集約変換（adaptor 層が担う）。
/// `#[error(transparent)]` な `UsecaseError` の `Display` を保持するため、
/// serialize 表現は移行前と等価に保たれる。
///
/// commit を作った後の失敗（`UsecaseError::AfterCommit`）は、中身の失敗の分類と文を保ち、
/// code `GIT_COMMIT_FOLLOW_UP_FAILED` を付ける。
impl From<UsecaseError> for AppError {
    fn from(e: UsecaseError) -> Self {
        match e {
            UsecaseError::AfterCommit(error) => {
                AppError::from(*error).with_code("GIT_COMMIT_FOLLOW_UP_FAILED")
            }
            e => AppError::from_failure(e),
        }
    }
}

/// ユースケース呼び出しを `spawn_blocking` 上で実行し、結果を `AppError`
/// に集約する共通ヘルパー。join 失敗時のメッセージは移行前と等価に保つ。
pub async fn run_blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, UsecaseError> + Send + 'static,
{
    crate::adaptor::controller::client::worktree_mutation::spawn_blocking(f)
        .await
        .map_err(|e| AppError::from_failure(crate::domain::failure::TechnicalFailure::from(e)))?
        .map_err(AppError::from)
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod mod_tests;

mod source_control;
