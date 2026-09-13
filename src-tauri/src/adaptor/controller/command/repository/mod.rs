pub(crate) const COMMAND_NAMES: &[&str] = &[
    "get_current_branch",
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
    router.register_domain(
        COMMAND_NAMES,
        Box::new(super::client::handle_registered_invoke),
    );
}
