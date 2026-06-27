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

pub(super) const COMMAND_NAMES: &[&str] = &[
    "list_branches",
    "get_current_branch",
    "get_default_branch",
    "git_create_branch",
    "delete_branch",
    "get_git_status",
    "get_git_status_snapshot",
    "get_status_diff_stats",
    "get_status_diff_stats_snapshot",
    "get_git_log",
    "get_main_repo_path",
    "get_worktree_dirty_count",
    "list_worktrees",
    "list_branches_with_status",
    "list_branches_with_status_snapshot",
    "create_worktree",
    "remove_worktree",
    "get_cwd",
    "get_repo_git_dir",
    "get_releash_base",
    "set_releash_base",
    "get_branch_base",
    "set_branch_base",
    "get_repo_paths",
    "add_repo_path",
    "remove_repo_path",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        branch::list_branches,
        branch::get_current_branch,
        branch::get_default_branch,
        branch::git_create_branch,
        branch::delete_branch,
        status::get_git_status,
        status::get_git_status_snapshot,
        status::get_status_diff_stats,
        status::get_status_diff_stats_snapshot,
        log::get_git_log,
        worktree::get_main_repo_path,
        worktree::get_worktree_dirty_count,
        worktree::list_worktrees,
        worktree::list_branches_with_status,
        worktree::list_branches_with_status_snapshot,
        worktree::create_worktree,
        worktree::remove_worktree,
        util::get_cwd,
        util::get_repo_git_dir,
        git_config::get_releash_base,
        git_config::set_releash_base,
        git_config::get_branch_base,
        git_config::set_branch_base,
        repo_paths::get_repo_paths,
        repo_paths::add_repo_path,
        repo_paths::remove_repo_path,
    ]
}

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
