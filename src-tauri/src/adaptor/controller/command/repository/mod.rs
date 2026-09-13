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

pub(super) use crate::adaptor::controller::client::repository::{
    run_blocking, run_repository_state,
};

pub(super) const COMMAND_NAMES: &[&str] = &[
    "list_branches",
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
