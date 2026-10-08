//! staging（差分 Approve）責務の gateway 実装。git2 index 操作と `git apply --cached`
//! を封じ込める。

use crate::adaptor::gateway::shared::git_operation;
use git2::{ErrorCode, Repository, StatusOptions};
use std::path::Path;
use tokio::process::Command;

use crate::domain::code::{CodeError, StagingRepository};

pub fn git_stage(repo_path: &str, paths: Vec<String>) -> Result<(), CodeError> {
    let repo = git_operation::run(|| Repository::open(repo_path))?;
    let mut index = git_operation::run(|| repo.index())?;

    let targets: Vec<String> = if paths.is_empty() {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .renames_index_to_workdir(true);
        let statuses = git_operation::run(|| repo.statuses(Some(&mut opts)))?;
        statuses
            .iter()
            .filter_map(|entry| {
                let s = entry.status();
                if s.contains(git2::Status::WT_NEW)
                    || s.contains(git2::Status::WT_MODIFIED)
                    || s.contains(git2::Status::WT_DELETED)
                    || s.contains(git2::Status::WT_RENAMED)
                    || s.contains(git2::Status::WT_TYPECHANGE)
                {
                    entry.path().ok().map(|p| p.to_string())
                } else {
                    None
                }
            })
            .collect()
    } else {
        paths
    };

    let workdir = repo
        .workdir()
        .ok_or_else(|| CodeError::Rule("bare repository".to_string()))?;

    for p in &targets {
        let full_path = workdir.join(p);
        if full_path.exists() {
            git_operation::run(|| index.add_path(Path::new(p)))?;
        } else {
            git_operation::run(|| index.remove_path(Path::new(p)))?;
        }
    }

    git_operation::run(|| index.write())?;
    Ok(())
}

pub fn git_unstage(repo_path: &str, paths: Vec<String>) -> Result<(), CodeError> {
    let repo = git_operation::run(|| Repository::open(repo_path))?;

    let head_result = git_operation::run(|| repo.head());
    let is_unborn = matches!(&head_result, Err(e) if e.code() == ErrorCode::UnbornBranch);

    if is_unborn {
        let mut index = git_operation::run(|| repo.index())?;
        if paths.is_empty() {
            git_operation::run(|| index.clear())?;
        } else {
            for p in &paths {
                git_operation::run(|| index.remove_path(Path::new(p)))?;
            }
        }
        git_operation::run(|| index.write())?;
    } else {
        let head_ref = head_result?;
        let head_obj = git_operation::run(|| head_ref.peel(git2::ObjectType::Any))?;

        let targets: Vec<String> = if paths.is_empty() {
            let mut opts = StatusOptions::new();
            opts.include_untracked(true).recurse_untracked_dirs(true);
            let statuses = git_operation::run(|| repo.statuses(Some(&mut opts)))?;
            statuses
                .iter()
                .filter_map(|entry| {
                    let s = entry.status();
                    if s.contains(git2::Status::INDEX_NEW)
                        || s.contains(git2::Status::INDEX_MODIFIED)
                        || s.contains(git2::Status::INDEX_DELETED)
                        || s.contains(git2::Status::INDEX_RENAMED)
                    {
                        entry.path().ok().map(|p| p.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            paths
        };

        let path_specs: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
        git_operation::run(|| repo.reset_default(Some(&head_obj), &path_specs))?;
    }

    Ok(())
}

pub async fn git_stage_hunk(repo_path: &str, patch: &str) -> Result<(), CodeError> {
    apply_patch(repo_path, patch, false).await
}

pub async fn git_unstage_hunk(repo_path: &str, patch: &str) -> Result<(), CodeError> {
    apply_patch(repo_path, patch, true).await
}

async fn apply_patch(repo_path: &str, patch: &str, reverse: bool) -> Result<(), CodeError> {
    git_operation::run(|| Repository::open(repo_path))?;
    #[cfg(any(test, feature = "test-support"))]
    let program = crate::adaptor::gateway::code::test_helpers::git_program();
    #[cfg(not(any(test, feature = "test-support")))]
    let program = "git";
    let mut command = Command::new(program);
    command.args(["apply", "--cached"]).current_dir(repo_path);
    if reverse {
        command.arg("--reverse");
    }
    let output = crate::infrastructure::process::output::output(command, patch.as_bytes().to_vec())
        .await
        .map_err(|error| match error {
            crate::infrastructure::process::output::ProcessError::Io(error) => {
                CodeError::from(error)
            }
            crate::infrastructure::process::output::ProcessError::Stopped(error) => {
                CodeError::from(error)
            }
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CodeError::Rule(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

/// `StagingRepository` の git2 / git CLI 実装。
pub struct StagingGateway;

#[async_trait::async_trait]

impl StagingRepository for StagingGateway {
    fn stage(&self, repo_path: &str, paths: Vec<String>) -> Result<(), CodeError> {
        git_stage(repo_path, paths)
    }
    fn unstage(&self, repo_path: &str, paths: Vec<String>) -> Result<(), CodeError> {
        git_unstage(repo_path, paths)
    }
    async fn stage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeError> {
        git_stage_hunk(repo_path, patch).await
    }
    async fn unstage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeError> {
        git_unstage_hunk(repo_path, patch).await
    }
}
