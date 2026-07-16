pub(crate) fn to_canonical_forward_slash(path: &str) -> String {
    path.replace('\\', "/")
}

/// Compare native filesystem paths for worktree ownership without treating a valid Unix
/// backslash as a separator. `Path::components` ignores repeated/trailing native separators,
/// so `/repo` and `/repo/` are equal while `/repo` and `/repository` remain distinct.
pub(crate) fn same_worktree_path(left: &str, right: &str) -> bool {
    std::path::Path::new(left)
        .components()
        .eq(std::path::Path::new(right).components())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_backslashes_to_forward_slashes_only() {
        assert_eq!(to_canonical_forward_slash(r"C:\repo\wt"), "C:/repo/wt");
        assert_eq!(
            to_canonical_forward_slash(r"\\server\share\\wt"),
            "//server/share//wt"
        );
        assert_eq!(to_canonical_forward_slash("/repo//wt/"), "/repo//wt/");
    }

    #[test]
    fn is_idempotent_after_conversion() {
        let once = to_canonical_forward_slash(r"C:\repo\wt");
        let twice = to_canonical_forward_slash(&once);

        assert_eq!(twice, once);
    }

    #[test]
    fn worktree_identity_ignores_only_native_separator_aliases() {
        assert!(same_worktree_path("/repo", "/repo/"));
        assert!(same_worktree_path("/repo//wt", "/repo/wt/"));
        assert!(!same_worktree_path("/repo", "/repository"));
    }

    #[cfg(not(windows))]
    #[test]
    fn worktree_identity_does_not_treat_unix_backslash_as_a_separator() {
        assert!(!same_worktree_path(r"/tmp/a\b", "/tmp/a/b"));
    }
}
