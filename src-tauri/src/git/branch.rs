use super::error::GitError;
use super::types::BranchInfo;
use super::worktree::{get_branch_name_for_repo, remove_worktree};
use git2::{build::CheckoutBuilder, BranchType, Repository, WorktreePruneOptions};
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

pub(crate) fn detect_default_branch(repo: &Repository) -> Option<String> {
    // remote HEAD (refs/remotes/origin/HEAD) を最優先で確認
    if let Ok(reference) = repo.find_reference("refs/remotes/origin/HEAD") {
        if let Ok(resolved) = reference.resolve() {
            if let Some(name) = resolved.shorthand() {
                // "origin/main" → "main"
                let short = name.strip_prefix("origin/").unwrap_or(name);
                if repo.find_branch(short, BranchType::Local).is_ok() {
                    return Some(short.to_string());
                }
            }
        }
    }

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

    // HEADブランチの削除を拒否
    let head_branch = get_branch_name_for_repo(&repo);
    if head_branch == branch_name {
        return Err(GitError::Custom(
            "cannot delete the branch currently checked out in the main worktree".to_string(),
        ));
    }

    // 紐づくworktreeがあれば先に削除
    let mut had_worktree = false;
    if let Ok(wt_names) = repo.worktrees() {
        for i in 0..wt_names.len() {
            let wt_name = match wt_names.get(i) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let wt = match repo.find_worktree(&wt_name) {
                Ok(wt) => wt,
                Err(_) => continue,
            };

            // 壊れたworktreeはpruneして継続
            if wt.validate().is_err() {
                let mut prune_opts = WorktreePruneOptions::new();
                prune_opts.working_tree(true);
                let _ = wt.prune(Some(&mut prune_opts));
                continue;
            }

            let wt_path = wt.path().to_path_buf();
            let wt_branch = match Repository::open(&wt_path) {
                Ok(wt_repo) => get_branch_name_for_repo(&wt_repo),
                Err(_) => continue,
            };

            if wt_branch == branch_name {
                let wt_path_str = wt_path.to_str().unwrap_or("").trim_end_matches('/').to_string();
                remove_worktree(repo_path.clone(), wt_path_str, force)?;
                had_worktree = true;
            }
        }
    }

    // worktreeを削除した場合、Repositoryを再openする
    let repo = if had_worktree {
        Repository::open(&repo_path)?
    } else {
        repo
    };

    let mut branch = repo.find_branch(&branch_name, BranchType::Local)?;
    branch.delete()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_helpers::*;
    use git2::WorktreeAddOptions;
    use std::path::{Path, PathBuf};

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

    // ── delete_branch tests ──

    fn create_repo_with_parent() -> (tempfile::TempDir, PathBuf, Repository) {
        let parent = tempfile::TempDir::new().unwrap();
        let repo_dir = parent.path().join("main-repo");
        std::fs::create_dir(&repo_dir).unwrap();
        let repo = Repository::init(&repo_dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        (parent, repo_dir, repo)
    }

    fn create_worktree_for_branch(
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
    fn test_delete_branch_basic() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feat-delete", &head, false).unwrap();

        let repo_path = dir.path().to_str().unwrap().to_string();
        delete_branch(repo_path.clone(), "feat-delete".to_string(), false).unwrap();

        let repo = Repository::open(&repo_path).unwrap();
        assert!(repo.find_branch("feat-delete", BranchType::Local).is_err());
    }

    #[test]
    fn test_delete_branch_rejects_default() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let default = detect_default_branch(&repo).unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let result = delete_branch(repo_path, default, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("default branch"));
    }

    #[test]
    fn test_delete_branch_rejects_head() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head_branch = get_branch_name_for_repo(&repo);
        // Create another branch so default != head to isolate the HEAD check
        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("other-default", &head_commit, false).unwrap();

        let repo_path = dir.path().to_str().unwrap().to_string();
        let result = delete_branch(repo_path, head_branch, false);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("default branch") || err_msg.contains("currently checked out"),
            "unexpected error: {}",
            err_msg
        );
    }

    #[test]
    fn test_delete_branch_with_worktree() {
        let (parent, repo_dir, repo) = create_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path =
            create_worktree_for_branch(&repo, parent.path(), "wt-del", "feat-wt-delete");
        assert!(wt_path.exists());

        let repo_path = repo_dir.to_str().unwrap().to_string();
        delete_branch(repo_path.clone(), "feat-wt-delete".to_string(), false).unwrap();

        assert!(!wt_path.exists());
        let repo = Repository::open(&repo_path).unwrap();
        assert!(repo
            .find_branch("feat-wt-delete", BranchType::Local)
            .is_err());
    }

    #[test]
    fn test_delete_branch_force() {
        let (parent, repo_dir, repo) = create_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path =
            create_worktree_for_branch(&repo, parent.path(), "wt-force", "feat-force-del");
        std::fs::write(wt_path.join("dirty.txt"), "uncommitted").unwrap();

        let repo_path = repo_dir.to_str().unwrap().to_string();
        // Without force should fail due to dirty worktree
        let result = delete_branch(repo_path.clone(), "feat-force-del".to_string(), false);
        assert!(result.is_err());

        // With force should succeed
        delete_branch(repo_path.clone(), "feat-force-del".to_string(), true).unwrap();
        let repo = Repository::open(&repo_path).unwrap();
        assert!(repo
            .find_branch("feat-force-del", BranchType::Local)
            .is_err());
    }

    #[test]
    fn test_delete_branch_no_default_detected() {
        let parent = tempfile::TempDir::new().unwrap();
        let repo_dir = parent.path().join("no-default");
        std::fs::create_dir(&repo_dir).unwrap();
        let repo = Repository::init(&repo_dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        // Create initial commit on a non-standard branch name
        let sig = repo.signature().unwrap();
        let tree_id = {
            let mut index = repo.index().unwrap();
            let path = repo_dir.join("init.txt");
            std::fs::write(&path, "init").unwrap();
            index.add_path(std::path::Path::new("init.txt")).unwrap();
            index.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        // Rename to non-standard branch
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("develop", &head, false).unwrap();
        repo.set_head("refs/heads/develop").unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();

        // Delete original branch
        let default_name = detect_default_branch(&repo);
        if let Some(name) = &default_name {
            let mut b = repo.find_branch(name, BranchType::Local).unwrap();
            b.delete().unwrap();
        }

        // Create a branch to delete
        repo.branch("feat-x", &head, false).unwrap();

        let repo_path = repo_dir.to_str().unwrap().to_string();
        delete_branch(repo_path.clone(), "feat-x".to_string(), false).unwrap();

        let repo = Repository::open(&repo_path).unwrap();
        assert!(repo.find_branch("feat-x", BranchType::Local).is_err());
    }

    #[test]
    fn test_detect_default_branch_remote_head() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        // Create "develop" as the only branch
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("develop", &head, false).unwrap();

        // Simulate remote HEAD pointing to origin/develop
        repo.reference(
            "refs/remotes/origin/develop",
            head.id(),
            true,
            "test remote branch",
        )
        .unwrap();
        repo.reference_symbolic(
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/develop",
            true,
            "test remote HEAD",
        )
        .unwrap();

        // Remove main/master so fallback won't find them
        let _current = detect_default_branch(&repo);
        // remote HEAD should resolve to "develop" now
        // but only if main/master are removed
        let default_name = repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().map(|s| s.to_string()));
        if let Some(name) = default_name {
            if name == "main" || name == "master" {
                // Switch HEAD to develop first
                repo.set_head("refs/heads/develop").unwrap();
                repo.checkout_head(Some(CheckoutBuilder::new().force()))
                    .unwrap();
                let mut b = repo.find_branch(&name, BranchType::Local).unwrap();
                b.delete().unwrap();
            }
        }

        let result = detect_default_branch(&repo);
        assert_eq!(result, Some("develop".to_string()));
    }

    #[test]
    fn test_delete_branch_broken_worktree() {
        let (parent, repo_dir, repo) = create_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path =
            create_worktree_for_branch(&repo, parent.path(), "wt-broken", "feat-broken");

        // Break the worktree by removing its directory
        std::fs::remove_dir_all(&wt_path).unwrap();

        // Also create a normal branch to delete
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feat-normal", &head, false).unwrap();

        let repo_path = repo_dir.to_str().unwrap().to_string();
        // Should succeed — broken worktree is pruned, feat-normal is deleted
        delete_branch(repo_path.clone(), "feat-normal".to_string(), false).unwrap();

        let repo = Repository::open(&repo_path).unwrap();
        assert!(repo
            .find_branch("feat-normal", BranchType::Local)
            .is_err());
    }
}
