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
}
