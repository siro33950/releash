use std::sync::Arc;

use crate::domain::app_config::RepositoryConfigRepository;
use crate::domain::git::{Branch, GitError, GitService, RepositoryStatusScan, Worktree};

use super::{branch, git_config, status, worktree};

/// Git の状態と repository の設定に接続する。
pub struct GitGateway {
    repository_config: Arc<dyn RepositoryConfigRepository>,
}

impl GitGateway {
    /// repository の設定の読み取り先を受け取る。
    pub fn new(repository_config: Arc<dyn RepositoryConfigRepository>) -> Self {
        Self { repository_config }
    }
}

impl GitService for GitGateway {
    fn list_branches(&self, repo_path: &str) -> Result<Vec<Branch>, GitError> {
        branch::list_branches(repo_path)
    }
    fn current(&self, repo_path: &str) -> Result<String, GitError> {
        branch::get_current_branch(repo_path)
    }
    fn create_branch(&self, repo_path: &str, branch_name: &str) -> Result<(), GitError> {
        branch::git_create_branch(repo_path, branch_name)
    }

    fn status_scan(&self, repo_path: &str) -> Result<RepositoryStatusScan, GitError> {
        status::get_repository_status_scan(repo_path)
    }

    fn find_main_repo_path(&self, path: &str) -> Result<Option<String>, GitError> {
        worktree::find_main_repo_path(path)
    }
    fn main_repo_path(&self, any_path: &str) -> Result<String, GitError> {
        worktree::get_main_repo_path(any_path)
    }
    fn list_worktrees(&self, repo_path: &str) -> Result<Vec<Worktree>, GitError> {
        worktree::list_worktrees(repo_path)
    }
    fn create_worktree(
        &self,
        repo_path: &str,
        worktree_path: &str,
        branch: &str,
        create_branch: bool,
        base_branch: Option<&str>,
    ) -> Result<Worktree, GitError> {
        worktree::create_worktree(repo_path, worktree_path, branch, create_branch, base_branch)
    }
    fn validate_removal(
        &self,
        repo_path: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<String, GitError> {
        worktree::validate_removal(repo_path, worktree_path, force)
    }
    fn remove(
        &self,
        repo_path: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<Option<String>, GitError> {
        worktree::remove_worktree(repo_path, worktree_path, force)
    }

    fn set_branch_base_override(
        &self,
        repo_path: &str,
        branch_name: &str,
        base: Option<&str>,
    ) -> Result<(), GitError> {
        git_config::set_branch_base_override(repo_path, branch_name, base)
    }
    fn remove_branch_base_overrides(
        &self,
        repo_path: &str,
        branches: &[String],
    ) -> Result<(), GitError> {
        git_config::remove_branch_base_overrides(repo_path, branches)
    }
    fn resolve_current_base_branch(&self, path_hint: &str) -> Result<Option<String>, GitError> {
        use crate::adaptor::gateway::shared::git_operation;
        use crate::domain::app_config::AppConfigError;

        let repo = git_config::discover_repo(std::path::Path::new(path_hint))?;
        let head = match git_operation::run(|| repo.head()) {
            Ok(head) => head,
            Err(error) if error.code() == git2::ErrorCode::UnbornBranch => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !head.is_branch() {
            return Ok(None);
        }
        let branch = git_operation::run(|| head.shorthand())?;
        let repo_path = repo
            .workdir()
            .unwrap_or(repo.path())
            .to_str()
            .ok_or_else(|| GitError::rule("invalid path encoding"))?;
        let config = self
            .repository_config
            .load(repo_path)
            .map_err(|error| match error {
                AppConfigError::Technical(failure) => GitError::Technical(failure),
                AppConfigError::InvalidInput(message) => GitError::rule(message),
            })?;
        Ok(config.resolve_base_branch(branch).map(str::to_owned))
    }
    fn resolve_base_commit_oid(
        &self,
        path_hint: &str,
        base_name: &str,
    ) -> Result<Option<String>, GitError> {
        git_config::resolve_base_commit_oid(path_hint, base_name)
    }
}
