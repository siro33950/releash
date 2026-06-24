//! repository 責務の Tauri コマンド（薄い入口）。
//!
//! 引数の受け渡しと型変換のみを行い、ビジネスロジックは usecase /
//! query service に委ねる。git2 のブロッキング呼び出しを非同期境界へ
//! 載せるため、各コマンドは `spawn_blocking` でユースケースを呼ぶ。

pub(crate) mod branch;
pub(crate) mod git_config;
pub(crate) mod log;
pub(crate) mod repo_paths;
pub(crate) mod status;
pub(crate) mod util;
pub(crate) mod worktree;

use crate::other::AppError;
use crate::usecase::repository_error::UsecaseError;
use crate::usecase::repository_state::RepositoryStateError;

/// ユースケースエラー → アプリエラーの集約変換（adaptor 層が担う）。
/// `#[error(transparent)]` な `UsecaseError` の `Display` を保持するため、
/// serialize 表現は移行前と等価に保たれる。
impl From<UsecaseError> for AppError {
    fn from(e: UsecaseError) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl From<RepositoryStateError> for AppError {
    fn from(e: RepositoryStateError) -> Self {
        AppError::Internal(e.to_string())
    }
}

/// ユースケース呼び出しを `spawn_blocking` 上で実行し、結果を `AppError`
/// に集約する共通ヘルパー。join 失敗時のメッセージは移行前と等価に保つ。
pub(super) async fn run_blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, UsecaseError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::new(format!("task join error: {e}")))?
        .map_err(AppError::from)
}

pub(super) async fn run_repository_state<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, RepositoryStateError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::new(format!("task join error: {e}")))?
        .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::repository::RepositoryError;

    // RepositoryError → UsecaseError → AppError の 3 ホップ変換でメッセージ文字列が
    // 保持され、serialize 表現が移行前（gateway が返していたプレーン文字列）と
    // 等価であることをガードする（behavior.md: 失敗は移行前後で等価に観測される）。

    #[test]
    fn rule違反のdisplayが変換チェーンを通じて保持される() {
        let usecase_err = UsecaseError::Rule("既定ブランチは削除できません".to_string());
        let app_err = AppError::from(usecase_err);
        assert_eq!(app_err.to_string(), "既定ブランチは削除できません");
        assert_eq!(
            serde_json::to_string(&app_err).unwrap(),
            "\"既定ブランチは削除できません\""
        );
    }

    #[test]
    fn repository_external由来のdisplayが変換チェーンを通じて保持される() {
        let usecase_err =
            UsecaseError::Repository(RepositoryError::External("git2 boom".to_string()));
        let app_err = AppError::from(usecase_err);
        assert_eq!(app_err.to_string(), "git2 boom");
        assert_eq!(serde_json::to_string(&app_err).unwrap(), "\"git2 boom\"");
    }
}
