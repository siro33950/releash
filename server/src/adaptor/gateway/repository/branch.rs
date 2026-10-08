//! branch 責務の gateway 実装。git2 によるブランチ操作を封じ込める。

use crate::adaptor::gateway::shared::git_operation;
use crate::domain::repository::{Branch, BranchRepository, RepositoryError};
use crate::infrastructure::git::client;
use git2::BranchType;

pub fn list_branches(repo_path: &str) -> Result<Vec<Branch>, RepositoryError> {
    let repo = git_operation::run(|| client::discover(repo_path))?;

    let mut result = Vec::new();
    let mut local_names = std::collections::HashSet::new();

    let local_branches = git_operation::run(|| repo.branches(Some(BranchType::Local)))?;
    for branch in local_branches {
        let (branch, _) = branch?;
        if let Some(name) = git_operation::run(|| branch.name())? {
            local_names.insert(name.to_string());
            result.push(Branch::local(name));
        }
    }

    let remote_branches = git_operation::run(|| repo.branches(Some(BranchType::Remote)))?;
    for branch in remote_branches {
        let (branch, _) = branch?;
        if let Some(full_name) = git_operation::run(|| branch.name())? {
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

pub fn get_current_branch(repo_path: &str) -> Result<String, RepositoryError> {
    let repo = git_operation::run(|| client::open(repo_path))?;

    let head = match git_operation::run(|| repo.head()) {
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

pub fn git_create_branch(repo_path: &str, branch_name: &str) -> Result<(), RepositoryError> {
    let repo = git_operation::run(|| client::open(repo_path))?;

    let head = git_operation::run(|| repo.head())?;
    let commit = git_operation::run(|| head.peel_to_commit())?;

    git_operation::run(|| repo.branch(branch_name, &commit, false))?;
    git_operation::run(|| repo.set_head(&format!("refs/heads/{branch_name}")))?;
    git_operation::run(|| repo.checkout_head(Some(git_operation::checkout().safe())))?;

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
    fn create(&self, repo_path: &str, branch_name: &str) -> Result<(), RepositoryError> {
        git_create_branch(repo_path, branch_name)
    }
}
