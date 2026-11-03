//! staging（差分 Approve）責務の gateway 実装。git2 index 操作と `git apply --cached`
//! を封じ込める。

use crate::adaptor::gateway::shared::git_operation;
use git2::{ErrorCode, Repository, StatusOptions};
use std::path::Path;
use tokio::process::Command;

use crate::domain::code::{CodeError, StagingRepository};

pub fn git_stage(repo_path: &str, paths: Vec<String>) -> Result<(), CodeError> {
    let repo = git_operation::run(|| Repository::open(repo_path))?;
    let _lock = crate::infrastructure::file_lock::acquire(&repo.path().join("releash-index.lock"))?;
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
            .filter(|path| {
                index.get_path(Path::new(path), 0).is_none_or(|entry| {
                    entry.flags_extended & git2::IndexEntryExtendedFlag::SKIP_WORKTREE.bits() == 0
                })
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
        match full_path.symlink_metadata() {
            Ok(_) => git_operation::run(|| index.add_path(Path::new(p)))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                git_operation::run(|| index.remove_path(Path::new(p)))?;
            }
            Err(error) => return Err(error.into()),
        }
    }

    git_operation::run(|| index.write())?;
    Ok(())
}

pub fn git_unstage(repo_path: &str, paths: Vec<String>) -> Result<(), CodeError> {
    let repo = git_operation::run(|| Repository::open(repo_path))?;
    let _lock = crate::infrastructure::file_lock::acquire(&repo.path().join("releash-index.lock"))?;

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

        if paths.is_empty() {
            let tree = git_operation::run(|| head_obj.peel_to_tree())?;
            let mut index = git_operation::run(|| repo.index())?;
            let mut options = git2::DiffOptions::new();
            options.reverse(true).include_typechange(true);
            let diff = git_operation::run(|| {
                repo.diff_tree_to_index(Some(&tree), Some(&index), Some(&mut options))
            })?;
            for delta in diff.deltas() {
                let file = delta.new_file();
                let path = file
                    .path()
                    .ok_or_else(|| CodeError::Rule("missing index path".into()))?;
                if let Err(error) = git_operation::run(|| index.conflict_remove(path)) {
                    if error.code() != ErrorCode::NotFound {
                        return Err(error.into());
                    }
                }
                if !file.exists() {
                    if index.get_path(path, 0).is_some() {
                        git_operation::run(|| index.remove_path(path))?;
                    }
                } else {
                    let entry = git2::IndexEntry {
                        ctime: git2::IndexTime::new(0, 0),
                        mtime: git2::IndexTime::new(0, 0),
                        dev: 0,
                        ino: 0,
                        mode: file.mode().into(),
                        uid: 0,
                        gid: 0,
                        file_size: 0,
                        id: file.id(),
                        flags: 0,
                        flags_extended: 0,
                        path: file.path_bytes().unwrap().to_vec(),
                    };
                    git_operation::run(|| index.add(&entry))?;
                }
            }
            git_operation::run(|| index.write())?;
        } else {
            git_operation::run(|| repo.reset_default(Some(&head_obj), &paths))?;
        }
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
    let repo = git_operation::run(|| Repository::open(repo_path))?;
    let _lock =
        crate::infrastructure::file_lock::acquire_async(&repo.path().join("releash-index.lock"))
            .await?;
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
