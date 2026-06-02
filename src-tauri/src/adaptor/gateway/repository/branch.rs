//! branch 責務の gateway 実装。git2 によるブランチ操作を封じ込める。

use crate::domain::repository::{Branch, BranchRepository, RepositoryError};
use crate::infrastructure::git::client;
use crate::infrastructure::git::helpers::detect_default_branch;
use git2::{build::CheckoutBuilder, BranchType};

pub(crate) fn list_branches(repo_path: &str) -> Result<Vec<Branch>, RepositoryError> {
    let repo = client::discover(repo_path)?;

    let mut result = Vec::new();
    let mut local_names = std::collections::HashSet::new();

    let local_branches = repo.branches(Some(BranchType::Local))?;
    for branch in local_branches {
        let (branch, _) = branch?;
        if let Some(name) = branch.name()? {
            local_names.insert(name.to_string());
            result.push(Branch::local(name));
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
            result.push(Branch::remote(short));
        }
    }

    Ok(result)
}

pub(crate) fn get_current_branch(repo_path: &str) -> Result<String, RepositoryError> {
    let repo = client::open(repo_path)?;

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
            .ok_or_else(|| RepositoryError::rule("HEAD has no target"))?;
        let short = &oid.to_string()[..7];
        Ok(format!("({short})"))
    }
}

pub(crate) fn git_create_branch(repo_path: &str, branch_name: &str) -> Result<(), RepositoryError> {
    let repo = client::open(repo_path)?;

    let head = repo.head()?;
    let commit = head.peel_to_commit()?;

    repo.branch(branch_name, &commit, false)?;
    repo.set_head(&format!("refs/heads/{branch_name}"))?;
    repo.checkout_head(Some(CheckoutBuilder::new().safe()))?;

    Ok(())
}

pub(crate) fn get_default_branch(repo_path: &str) -> Result<String, RepositoryError> {
    let repo = client::open(repo_path)?;
    detect_default_branch(&repo).ok_or_else(|| RepositoryError::rule("no default branch found"))
}

/// 単一ローカルブランチを削除する純粋プリミティブ。
///
/// 既定/チェックアウト中ブランチの拒否、紐づく worktree の事前削除、
/// releash-base config の後始末といった業務手順は usecase が担う
/// （[`RepositoryUsecase::delete_branch`](crate::usecase::repository_usecase::RepositoryUsecase::delete_branch)）。
pub(crate) fn delete_branch(repo_path: &str, branch_name: &str) -> Result<(), RepositoryError> {
    let repo = client::open(repo_path)?;
    let mut branch = repo.find_branch(branch_name, BranchType::Local)?;
    branch.delete()?;
    Ok(())
}

/// `BranchRepository` の git2 実装。
pub struct BranchGateway;

impl BranchRepository for BranchGateway {
    fn list(&self, repo_path: &str) -> Result<Vec<Branch>, RepositoryError> {
        list_branches(repo_path)
    }
    fn current(&self, repo_path: &str) -> Result<String, RepositoryError> {
        get_current_branch(repo_path)
    }
    fn default(&self, repo_path: &str) -> Result<String, RepositoryError> {
        get_default_branch(repo_path)
    }
    fn create(&self, repo_path: &str, branch_name: &str) -> Result<(), RepositoryError> {
        git_create_branch(repo_path, branch_name)
    }
    fn delete(&self, repo_path: &str, branch_name: &str) -> Result<(), RepositoryError> {
        delete_branch(repo_path, branch_name)
    }
}

#[cfg(test)]
mod branch_gateway_tests {
    use super::*;
    use crate::git::test_helpers::*;
    use git2::Repository;
    use std::path::Path;

    fn path_str(p: &Path) -> String {
        p.to_str().unwrap().to_string()
    }

    #[test]
    fn test_現在ブランチ取得_初期コミット後() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let result = get_current_branch(&path_str(dir.path())).unwrap();
        assert!(result == "main" || result == "master");
    }

    #[test]
    fn test_現在ブランチ取得_空リポジトリ() {
        let (dir, _repo) = create_test_repo();

        let result = get_current_branch(&path_str(dir.path())).unwrap();
        assert_eq!(result, "(no commits)");
    }

    #[test]
    fn test_現在ブランチ取得_detached_head() {
        let (dir, repo) = create_test_repo();
        let oid = create_initial_commit(&repo);

        repo.set_head_detached(oid).unwrap();

        let result = get_current_branch(&path_str(dir.path())).unwrap();
        assert!(result.starts_with('('));
        assert!(result.ends_with(')'));
        assert_eq!(result.len(), 9); // "(1234567)"
    }

    #[test]
    fn test_ブランチ作成() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        git_create_branch(&path_str(dir.path()), "feature").unwrap();

        let branch = get_current_branch(&path_str(dir.path())).unwrap();
        assert_eq!(branch, "feature");
    }

    #[test]
    fn test_ブランチ作成_既存名でエラー() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        git_create_branch(&path_str(dir.path()), "feature").unwrap();

        let result = git_create_branch(&path_str(dir.path()), "feature");
        assert!(result.is_err());
    }

    #[test]
    fn test_既定ブランチ取得() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let branch = get_default_branch(&path_str(dir.path())).unwrap();
        assert!(
            branch == "main" || branch == "master",
            "expected main or master, got {branch}"
        );

        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        let other = if branch == "main" { "master" } else { "main" };
        repo.branch(other, &head_commit, false).unwrap();
        repo.set_head(&format!("refs/heads/{other}")).unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        let mut old_branch = repo.find_branch(&branch, BranchType::Local).unwrap();
        old_branch.delete().unwrap();

        let new_default = get_default_branch(&path_str(dir.path())).unwrap();
        assert_eq!(new_default, other);
    }

    #[test]
    fn test_ブランチ削除_基本() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feat-delete", &head, false).unwrap();

        let repo_path = path_str(dir.path());
        // gateway は単一ブランチ削除のプリミティブ（拒否ポリシー・worktree 連鎖は
        // usecase の業務手順であり repository_usecase のテストで検証する）。
        delete_branch(&repo_path, "feat-delete").unwrap();

        let repo = Repository::open(&repo_path).unwrap();
        assert!(repo.find_branch("feat-delete", BranchType::Local).is_err());
    }

    #[test]
    fn test_既定ブランチ検出_remote_head() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("develop", &head, false).unwrap();

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

        let default_name = repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().ok().map(|s| s.to_string()));
        if let Some(name) = default_name {
            if name == "main" || name == "master" {
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
}
