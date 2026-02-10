use super::error::GitError;
use super::types::GitFileStatus;
use git2::{Repository, StatusOptions};

fn index_status_from_flags(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::CONFLICTED) {
        "modified"
    } else if status.contains(git2::Status::INDEX_NEW) {
        "new"
    } else if status.contains(git2::Status::INDEX_MODIFIED) {
        "modified"
    } else if status.contains(git2::Status::INDEX_DELETED) {
        "deleted"
    } else if status.contains(git2::Status::INDEX_RENAMED) {
        "renamed"
    } else if status.contains(git2::Status::INDEX_TYPECHANGE) {
        "modified"
    } else {
        "none"
    }
}

fn worktree_status_from_flags(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::IGNORED) {
        "ignored"
    } else if status.contains(git2::Status::CONFLICTED) {
        "modified"
    } else if status.contains(git2::Status::WT_NEW) {
        "new"
    } else if status.contains(git2::Status::WT_MODIFIED) {
        "modified"
    } else if status.contains(git2::Status::WT_DELETED) {
        "deleted"
    } else if status.contains(git2::Status::WT_RENAMED)
        || status.contains(git2::Status::WT_TYPECHANGE)
    {
        "modified"
    } else {
        "none"
    }
}

pub fn get_git_status(repo_path: String) -> Result<Vec<GitFileStatus>, GitError> {
    let repo = Repository::open(&repo_path)?;

    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(true);

    let statuses = repo.statuses(Some(&mut opts))?;

    let result: Vec<GitFileStatus> = statuses
        .iter()
        .filter_map(|entry| {
            let path = entry.path()?.to_string();
            let path = path.trim_end_matches('/').to_string();
            let status = entry.status();
            let idx = index_status_from_flags(status);
            let wt = worktree_status_from_flags(status);
            if idx == "none" && wt == "none" {
                return None;
            }
            Some(GitFileStatus {
                path,
                index_status: idx.to_string(),
                worktree_status: wt.to_string(),
            })
        })
        .collect();

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_helpers::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn test_get_git_status_untracked() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("new_file.txt"), "hello").unwrap();

        let result = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "new_file.txt");
        assert_eq!(result[0].worktree_status, "new");
        assert_eq!(result[0].index_status, "none");
    }

    #[test]
    fn test_get_git_status_staged() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        fs::write(dir.path().join("staged.txt"), "content").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("staged.txt")).unwrap();
        index.write().unwrap();

        let result = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "staged.txt");
        assert_eq!(result[0].index_status, "new");
    }

    #[test]
    fn test_get_git_status_modified() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "original", "add file");

        fs::write(dir.path().join("file.txt"), "modified content").unwrap();

        let result = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "file.txt");
        assert_eq!(result[0].worktree_status, "modified");
    }

    #[test]
    fn test_get_git_status_empty_repo() {
        let (dir, _repo) = create_test_repo();

        let result = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_get_git_status_ignored_file() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        fs::write(dir.path().join(".gitignore"), "ignored.txt\nbuild/\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(".gitignore")).unwrap();
        index.write().unwrap();
        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add gitignore", &tree, &[&parent])
            .unwrap();

        fs::write(dir.path().join("ignored.txt"), "should be ignored").unwrap();
        fs::create_dir(dir.path().join("build")).unwrap();
        fs::write(dir.path().join("build").join("output.js"), "built").unwrap();

        let result = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();

        let ignored_file = result.iter().find(|e| e.path == "ignored.txt");
        assert!(
            ignored_file.is_some(),
            "ignored.txt should appear in status"
        );
        assert_eq!(ignored_file.unwrap().worktree_status, "ignored");
        assert_eq!(ignored_file.unwrap().index_status, "none");

        let ignored_dir = result.iter().find(|e| e.path == "build");
        assert!(ignored_dir.is_some(), "build dir should appear in status");
        assert_eq!(ignored_dir.unwrap().worktree_status, "ignored");
    }
}
