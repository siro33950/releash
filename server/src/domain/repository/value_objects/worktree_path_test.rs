pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn worktree_dir_uses_repo_parent_and_repo_name() {
        assert_eq!(
            worktree_dir("/home/user/projects/my-repo"),
            "/home/user/projects/my-repo-worktrees"
        );
    }

    #[test]
    fn worktree_dir_handles_trailing_slash() {
        assert_eq!(
            worktree_dir("/home/user/projects/my-repo/"),
            "/home/user/projects/my-repo-worktrees"
        );
    }

    #[test]
    fn worktree_dir_handles_windows_path() {
        assert_eq!(
            worktree_dir(r"C:\Users\test\my-repo"),
            "C:/Users/test/my-repo-worktrees"
        );
    }

    #[test]
    fn worktree_dir_preserves_unc_prefix() {
        assert_eq!(
            worktree_dir(r"\\server\share\my-repo"),
            "//server/share/my-repo-worktrees"
        );
    }

    #[test]
    fn branch_to_dir_replaces_slashes() {
        assert_eq!(branch_to_dir("feat/issues/1302"), "feat-issues-1302");
        assert_eq!(branch_to_dir("main"), "main");
    }

    #[test]
    fn worktree_path_combines_derived_dir_and_branch_dir() {
        assert_eq!(
            worktree_path("/home/user/projects/my-repo", "feat/issues/1302"),
            "/home/user/projects/my-repo-worktrees/feat-issues-1302"
        );
    }

    #[test]
    fn worktree_path_preserves_unc_prefix() {
        assert_eq!(
            worktree_path(r"\\server\share\my-repo", "feat/issues/1302"),
            "//server/share/my-repo-worktrees/feat-issues-1302"
        );
    }
}
