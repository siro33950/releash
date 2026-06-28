use super::repo_path::normalize_repo_path;

pub fn worktree_dir(repo_path: &str) -> String {
    let normalized_repo_path = normalize_repo_path(repo_path);
    let parent = match normalized_repo_path.rfind('/') {
        Some(index) => &normalized_repo_path[..index],
        None => &normalized_repo_path,
    };
    let repo_name = normalized_repo_path
        .split('/')
        .rfind(|segment| !segment.is_empty())
        .unwrap_or("repo");
    normalize_repo_path(&format!("{parent}/{repo_name}-worktrees"))
}

pub fn branch_to_dir(branch: &str) -> String {
    branch.replace('/', "-")
}

pub fn worktree_path(repo_path: &str, branch: &str) -> String {
    normalize_repo_path(&format!(
        "{}/{}",
        worktree_dir(repo_path),
        branch_to_dir(branch)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
