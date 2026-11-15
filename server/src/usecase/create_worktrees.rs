use std::sync::Arc;

use crate::domain::git::GitError;
use crate::usecase::git_usecase::GitUsecase;
use crate::usecase::repository_error::UsecaseError;

/// ブランチごとに worktree を順に作り、作った worktree の path を返す。
///
/// ローカルに無いブランチは `base_branch` から作り、そのブランチの base として記録する。
/// ローカルにあるブランチには `base_branch` を使わず、base の記録も変えない。
/// 途中で失敗したら、残りのブランチの worktree は作らない。
pub async fn create_worktrees(
    repository: Arc<GitUsecase>,
    repo_path: String,
    branches: Vec<String>,
    base_branch: Option<String>,
) -> Result<Vec<String>, UsecaseError> {
    crate::domain::git::validate_worktree_branches(&branches)?;
    let mut paths = Vec::with_capacity(branches.len());
    for branch in branches {
        let repository = repository.clone();
        let repo_path = repo_path.clone();
        let base_branch = base_branch.clone();
        let path = crate::common::operation_context::spawn_blocking(move || {
            let existing = repository.list_branches_with_worktree(&repo_path)?;
            let create_branch = crate::domain::git::Branch::needs_creation(&branch, &existing);
            let base = base_branch.as_deref().filter(|_| create_branch);
            repository
                .create_worktree(&repo_path, &branch, create_branch, base)
                .map(|worktree| worktree.path)
        })
        .await
        .map_err(|error| GitError::Technical(error.into()))??;
        paths.push(path);
    }
    Ok(paths)
}

#[cfg(test)]
#[path = "create_worktrees_test.rs"]
mod create_worktrees_tests;

pub struct CreateWorktreesUsecase {
    repository: Arc<GitUsecase>,
}
impl CreateWorktreesUsecase {
    pub fn new(repository: Arc<GitUsecase>) -> Self {
        Self { repository }
    }
    pub async fn execute(
        &self,
        repo: String,
        branches: Vec<String>,
        base: Option<String>,
    ) -> Result<Vec<String>, UsecaseError> {
        create_worktrees(self.repository.clone(), repo, branches, base).await
    }
}
