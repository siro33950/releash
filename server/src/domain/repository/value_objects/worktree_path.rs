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
#[path = "worktree_path_test.rs"]
mod worktree_path_tests;
