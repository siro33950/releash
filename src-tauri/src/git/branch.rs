use super::error::GitError;
use super::types::BranchInfo;
use git2::{build::CheckoutBuilder, BranchType, Repository};
use std::path::Path;

pub fn list_branches(repo_path: String) -> Result<Vec<BranchInfo>, GitError> {
    let path = Path::new(&repo_path);
    let repo = Repository::discover(path)?;

    let mut result = Vec::new();
    let mut local_names = std::collections::HashSet::new();

    let local_branches = repo.branches(Some(BranchType::Local))?;
    for branch in local_branches {
        let (branch, _) = branch?;
        if let Some(name) = branch.name()? {
            local_names.insert(name.to_string());
            result.push(BranchInfo {
                name: name.to_string(),
                is_remote: false,
            });
        }
    }

    let remote_branches = repo.branches(Some(BranchType::Remote))?;
    for branch in remote_branches {
        let (branch, _) = branch?;
        if let Some(full_name) = branch.name()? {
            // "origin/branch-name" → "branch-name"
            let short = full_name
                .split_once('/')
                .map(|(_, b)| b)
                .unwrap_or(full_name);
            if short == "HEAD" || local_names.contains(short) {
                continue;
            }
            result.push(BranchInfo {
                name: short.to_string(),
                is_remote: true,
            });
        }
    }

    Ok(result)
}

pub fn get_current_branch(repo_path: String) -> Result<String, GitError> {
    let repo = Repository::open(&repo_path)?;

    let head = match repo.head() {
        Ok(h) => h,
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => {
            return Ok("(no commits)".to_string())
        }
        Err(e) => return Err(e.into()),
    };

    if head.is_branch() {
        Ok(head.shorthand().unwrap_or("HEAD").to_string())
    } else {
        let oid = head
            .target()
            .ok_or_else(|| GitError::Custom("HEAD has no target".to_string()))?;
        let short = &oid.to_string()[..7];
        Ok(format!("({short})"))
    }
}

pub fn git_create_branch(repo_path: String, branch_name: String) -> Result<(), GitError> {
    let repo = Repository::open(&repo_path)?;

    let head = repo.head()?;
    let commit = head.peel_to_commit()?;

    repo.branch(&branch_name, &commit, false)?;
    repo.set_head(&format!("refs/heads/{branch_name}"))?;
    repo.checkout_head(Some(CheckoutBuilder::new().safe()))?;

    Ok(())
}

pub fn delete_branch(repo_path: String, branch_name: String, force: bool) -> Result<(), GitError> {
    let repo = Repository::open(&repo_path)?;

    // デフォルトブランチの削除を拒否
    if let Some(default) = detect_default_branch(&repo) {
        if default == branch_name {
            return Err(GitError::Custom(
                "cannot delete the default branch".to_string(),
            ));
        }
    }

    // 現在の HEAD ブランチの削除を拒否
    if let Ok(head) = repo.head() {
        if head.is_branch() {
            if let Some(current) = head.shorthand() {
                if current == branch_name {
                    return Err(GitError::Custom(
                        "cannot delete the current HEAD branch".to_string(),
                    ));
                }
            }
        }
    }

    // ブランチの存在確認
    repo.find_branch(&branch_name, BranchType::Local)?;

    // 対象ブランチに紐づく worktree を検索 → 存在すれば先に削除
    if let Ok(wt_names) = repo.worktrees() {
        for i in 0..wt_names.len() {
            let wt_name = match wt_names.get(i) {
                Some(name) => name.to_string(),
                None => continue,
            };
            if let Ok(wt) = repo.find_worktree(&wt_name) {
                if wt.validate().is_err() {
                    continue;
                }
                let wt_path = wt.path();
                if let Ok(wt_repo) = Repository::open(wt_path) {
                    let wt_branch = match wt_repo.head() {
                        Ok(h) if h.is_branch() => h.shorthand().unwrap_or("").to_string(),
                        _ => continue,
                    };
                    if wt_branch == branch_name {
                        let wt_path_str = wt_path
                            .to_str()
                            .ok_or_else(|| {
                                GitError::Custom("invalid worktree path encoding".to_string())
                            })?
                            .trim_end_matches('/')
                            .to_string();
                        drop(wt_repo);
                        drop(wt);
                        super::worktree::remove_worktree(repo_path.clone(), wt_path_str, force)?;
                        break;
                    }
                }
            }
        }
    }

    // Repository を再 open してブランチを削除
    let repo = Repository::open(&repo_path)?;
    let mut branch = repo.find_branch(&branch_name, BranchType::Local)?;
    branch.delete()?;

    Ok(())
}

pub(crate) fn detect_default_branch(repo: &Repository) -> Option<String> {
    for name in &["main", "master"] {
        if repo.find_branch(name, BranchType::Local).is_ok() {
            return Some(name.to_string());
        }
    }
    None
}

pub fn get_default_branch(repo_path: String) -> Result<String, GitError> {
    let repo = Repository::open(&repo_path)?;
    detect_default_branch(&repo)
        .ok_or_else(|| GitError::Custom("no default branch found".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_helpers::*;
    use git2::WorktreeAddOptions;
    use std::path::{Path, PathBuf};

    fn create_test_repo_with_parent() -> (tempfile::TempDir, PathBuf, Repository) {
        let parent = tempfile::TempDir::new().unwrap();
        let repo_dir = parent.path().join("main-repo");
        std::fs::create_dir(&repo_dir).unwrap();
        let repo = Repository::init(&repo_dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        (parent, repo_dir, repo)
    }

    fn create_worktree_helper(
        repo: &Repository,
        parent_dir: &Path,
        wt_name: &str,
        branch_name: &str,
    ) -> PathBuf {
        let wt_path = parent_dir.join(wt_name);
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let branch = repo.branch(branch_name, &head, false).unwrap();
        let reference = branch.into_reference();
        let mut opts = WorktreeAddOptions::new();
        opts.reference(Some(&reference));
        repo.worktree(wt_name, &wt_path, Some(&opts)).unwrap();
        wt_path
    }

    #[test]
    fn test_get_current_branch() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let result = get_current_branch(dir.path().to_str().unwrap().to_string()).unwrap();
        assert!(result == "main" || result == "master");
    }

    #[test]
    fn test_get_current_branch_empty_repo() {
        let (dir, _repo) = create_test_repo();

        let result = get_current_branch(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(result, "(no commits)");
    }

    #[test]
    fn test_get_current_branch_detached_head() {
        let (dir, repo) = create_test_repo();
        let oid = create_initial_commit(&repo);

        repo.set_head_detached(oid).unwrap();

        let result = get_current_branch(dir.path().to_str().unwrap().to_string()).unwrap();
        assert!(result.starts_with('('));
        assert!(result.ends_with(')'));
        assert_eq!(result.len(), 9); // "(1234567)"
    }

    #[test]
    fn test_create_branch() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        git_create_branch(
            dir.path().to_str().unwrap().to_string(),
            "feature".to_string(),
        )
        .unwrap();

        let branch = get_current_branch(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(branch, "feature");
    }

    #[test]
    fn test_create_branch_already_exists() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        git_create_branch(
            dir.path().to_str().unwrap().to_string(),
            "feature".to_string(),
        )
        .unwrap();

        let result = git_create_branch(
            dir.path().to_str().unwrap().to_string(),
            "feature".to_string(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_get_default_branch() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let repo_path = dir.path().to_str().unwrap().to_string();
        let branch = get_default_branch(repo_path).unwrap();
        assert!(
            branch == "main" || branch == "master",
            "expected main or master, got {}",
            branch
        );

        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        let other = if branch == "main" { "master" } else { "main" };
        repo.branch(other, &head_commit, false).unwrap();
        repo.set_head(&format!("refs/heads/{}", other)).unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        let mut old_branch = repo.find_branch(&branch, BranchType::Local).unwrap();
        old_branch.delete().unwrap();

        let repo_path = dir.path().to_str().unwrap().to_string();
        let new_default = get_default_branch(repo_path).unwrap();
        assert_eq!(new_default, other);
    }

    #[test]
    fn test_delete_merged_branch() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feat-done", &head, false).unwrap();

        let repo_path = dir.path().to_str().unwrap().to_string();
        delete_branch(repo_path.clone(), "feat-done".to_string(), false).unwrap();

        assert!(repo.find_branch("feat-done", BranchType::Local).is_err());
    }

    #[test]
    fn test_delete_branch_with_worktree() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-del", "feat-del");
        assert!(wt_path.exists());

        let repo_path = repo_dir.to_str().unwrap().to_string();
        delete_branch(repo_path, "feat-del".to_string(), false).unwrap();

        assert!(!wt_path.exists());
        let repo = Repository::open(&repo_dir).unwrap();
        assert!(repo.find_branch("feat-del", BranchType::Local).is_err());
    }

    #[test]
    fn test_delete_default_branch_rejected() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let default = detect_default_branch(&repo).unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let result = delete_branch(repo_path, default.clone(), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("default branch"));
    }

    #[test]
    fn test_delete_head_branch_rejected() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        git_create_branch(
            dir.path().to_str().unwrap().to_string(),
            "current".to_string(),
        )
        .unwrap();

        let repo_path = dir.path().to_str().unwrap().to_string();
        let result = delete_branch(repo_path, "current".to_string(), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("current HEAD"));
    }

    #[test]
    fn test_delete_nonexistent_branch() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let repo_path = dir.path().to_str().unwrap().to_string();
        let result = delete_branch(repo_path, "no-such-branch".to_string(), false);
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_branch_dirty_worktree_no_force() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-dirty", "feat-dirty");
        std::fs::write(wt_path.join("dirty.txt"), "uncommitted").unwrap();

        let repo_path = repo_dir.to_str().unwrap().to_string();
        let result = delete_branch(repo_path, "feat-dirty".to_string(), false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("uncommitted change"));
        assert!(wt_path.exists());
    }

    #[test]
    fn test_delete_branch_dirty_worktree_force() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-dirtyf", "feat-dirtyf");
        std::fs::write(wt_path.join("dirty.txt"), "uncommitted").unwrap();

        let repo_path = repo_dir.to_str().unwrap().to_string();
        delete_branch(repo_path, "feat-dirtyf".to_string(), true).unwrap();

        assert!(!wt_path.exists());
        let repo = Repository::open(&repo_dir).unwrap();
        assert!(repo.find_branch("feat-dirtyf", BranchType::Local).is_err());
    }
}
