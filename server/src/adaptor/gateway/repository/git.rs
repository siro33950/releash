use std::sync::Arc;

use crate::domain::app_config::RepositoryConfigRepository;
use crate::domain::git::{
    Branch, CommitMessage, GitError, GitService, LineChanges, OperationKind, Push,
    RepositoryStatusScan, Worktree, WorktreeBranch,
};

use crate::adaptor::gateway::shared::git_operation;
use crate::infrastructure::git::client;

use super::{branch, git_config, source_control, status, worktree};

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
    fn default_branch(&self, repo_path: &str) -> Result<Option<String>, GitError> {
        branch::default_branch(repo_path)
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

    fn worktree_branch(&self, worktree: &str) -> Result<WorktreeBranch, GitError> {
        source_control::worktree_branch(worktree)
    }
    fn line_changes(&self, worktree: &str) -> Result<LineChanges, GitError> {
        source_control::line_changes(worktree)
    }
    fn commit(&self, worktree: &str, message: &CommitMessage) -> Result<(), GitError> {
        source_control::git(worktree, &["commit", "-m", message.as_str()]).map(|_| ())
    }
    fn push(&self, worktree: &str, push: Push) -> Result<(), GitError> {
        let repo = git_operation::run(|| client::open(worktree))?;
        let head = git_operation::run(|| repo.head())?;
        let name = git_operation::run(|| head.shorthand())?;
        if !head.is_branch() {
            return Err(GitError::rule("Cannot push detached HEAD"));
        }
        match push {
            Push::Publish => source_control::git(
                worktree,
                &[
                    "push",
                    "--set-upstream",
                    "origin",
                    &format!("refs/heads/{name}:refs/heads/{name}"),
                ],
            ),
            Push::ToUpstream | Push::ForceWithLease => {
                let config = git_operation::run(|| repo.config())?;
                let merge =
                    git_operation::run(|| config.get_string(&format!("branch.{name}.merge")))?;
                let remote =
                    git_operation::run(|| config.get_string(&format!("branch.{name}.remote")))?;
                let mut args = vec!["push"];
                if push == Push::ForceWithLease {
                    args.push("--force-with-lease");
                }
                let refspec = format!("HEAD:{merge}");
                args.extend([remote.as_str(), &refspec]);
                source_control::git(worktree, &args)
            }
        }
        .map(|_| ())
    }
    fn pull(&self, worktree: &str) -> Result<(), GitError> {
        source_control::git(worktree, &["pull", "--no-edit"]).map(|_| ())
    }
    fn fetch(&self, worktree: &str) -> Result<(), GitError> {
        let repo = git_operation::run(|| client::open(worktree))?;
        let remote = match git_operation::run(|| repo.head()) {
            Ok(head) if head.is_branch() => {
                let name = git_operation::run(|| head.shorthand())?;
                let local = git_operation::run(|| repo.find_branch(name, git2::BranchType::Local))?;
                match git_operation::run(|| local.upstream()) {
                    Ok(_) => {
                        let config = git_operation::run(|| repo.config())?;
                        git_operation::run(|| config.get_string(&format!("branch.{name}.remote")))?
                    }
                    Err(error) if error.code() == git2::ErrorCode::NotFound => "origin".into(),
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(_) => "origin".into(),
            Err(error) if error.code() == git2::ErrorCode::UnbornBranch => "origin".into(),
            Err(error) => return Err(error.into()),
        };
        source_control::git(worktree, &["fetch", &remote]).map(|_| ())
    }
    fn restore_worktree_files(&self, worktree: &str, paths: &[String]) -> Result<(), GitError> {
        if paths.is_empty() {
            return Ok(());
        }
        source_control::validate_paths(worktree, paths)?;
        let mut args = vec!["--literal-pathspecs", "restore", "--worktree", "--"];
        args.extend(paths.iter().map(String::as_str));
        source_control::git(worktree, &args).map(|_| ())
    }
    fn trash_files(&self, worktree: &str, paths: &[String]) -> Result<(), GitError> {
        source_control::trash_files(worktree, paths)
    }
    fn continue_operation(
        &self,
        worktree: &str,
        operation: &OperationKind,
        message: Option<&CommitMessage>,
    ) -> Result<(), GitError> {
        match (operation, message) {
            (OperationKind::Merge, Some(message)) => {
                source_control::git(worktree, &["commit", "-m", message.as_str()]).map(|_| ())
            }
            (OperationKind::Rebase(_), Some(message)) => {
                source_control::continue_rebase(worktree, message)
            }
            (_, None) => source_control::git(worktree, &["rebase", "--continue"]).map(|_| ()),
        }
    }
    fn abort_operation(&self, worktree: &str, operation: &OperationKind) -> Result<(), GitError> {
        let command = match operation {
            OperationKind::Merge => "merge",
            OperationKind::Rebase(_) => "rebase",
        };
        source_control::git(worktree, &[command, "--abort"]).map(|_| ())
    }

    fn origin_url(&self, repo_path: &str) -> Result<Option<String>, GitError> {
        crate::adaptor::gateway::git_host::discovery::get_origin_url(repo_path).map_err(Into::into)
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
