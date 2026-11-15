use std::collections::HashMap;
use std::path::{Component, Path};

use super::branch;
use crate::adaptor::gateway::shared::git_operation;
use crate::domain::git::{
    BranchState, CommitMessage, GitError, LineChanges, LineCount, Operation, OperationKind,
    RebaseProgress, Tracking, WorktreeBranch,
};
use crate::infrastructure::git::client;

pub(super) fn git(worktree: &str, args: &[&str]) -> Result<String, GitError> {
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-C")
        .arg(worktree)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true");
    let output = tokio::runtime::Handle::current()
        .block_on(crate::infrastructure::process::output::output(
            command,
            Vec::new(),
        ))
        .map_err(|error| match error {
            crate::infrastructure::process::output::ProcessError::Io(error) => {
                GitError::from(error)
            }
            crate::infrastructure::process::output::ProcessError::Stopped(error) => {
                GitError::from(error)
            }
        })?;
    if !output.status.success() {
        return Err(GitError::External(format!(
            "Git operation failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout).map_err(|error| GitError::External(error.to_string()))
}

pub(super) fn worktree_branch(worktree: &str) -> Result<WorktreeBranch, GitError> {
    let repo = git_operation::run(|| client::open(worktree))?;
    let mut name = branch::get_current_branch(worktree)?;
    for directory in ["rebase-merge", "rebase-apply"] {
        let path = repo.path().join(directory);
        if path.is_dir() {
            name = read(&path.join("head-name"))?
                .trim_start_matches("refs/heads/")
                .into();
            let onto = read(&path.join("onto"))?;
            let mut onto_name = None;
            let mut references = git_operation::run(|| repo.references())?;
            while let Some(reference) = git_operation::run(|| references.next().transpose())? {
                if reference.is_branch()
                    && reference.target().is_some_and(|id| id.to_string() == onto)
                {
                    onto_name = Some(git_operation::run(|| reference.shorthand())?.to_owned());
                    break;
                }
            }
            let onto = onto_name.unwrap_or_else(|| onto.chars().take(7).collect());
            let (step, total) = if directory == "rebase-merge" {
                (
                    read_number(&path.join("msgnum"))?,
                    read_number(&path.join("end"))?,
                )
            } else {
                (
                    read_number(&path.join("next"))?,
                    read_number(&path.join("last"))?,
                )
            };
            let message = match git_operation::run(|| repo.revparse_single("REBASE_HEAD")) {
                Ok(object) => {
                    let commit = git_operation::run(|| object.peel_to_commit())?;
                    Some(git_operation::run(|| commit.message())?.trim().to_owned())
                }
                Err(error) if error.code() == git2::ErrorCode::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            return Ok(WorktreeBranch {
                name,
                state: BranchState::Operating(Operation {
                    kind: OperationKind::Rebase(RebaseProgress { onto, step, total }),
                    message,
                }),
            });
        }
    }
    if repo.path().join("MERGE_HEAD").exists() {
        return Ok(WorktreeBranch {
            name,
            state: BranchState::Operating(Operation {
                kind: OperationKind::Merge,
                message: Some(read(&repo.path().join("MERGE_MSG"))?),
            }),
        });
    }
    let head = match git_operation::run(|| repo.head()) {
        Ok(head) => head,
        Err(error) if error.code() == git2::ErrorCode::UnbornBranch => {
            return Ok(WorktreeBranch {
                name,
                state: BranchState::Unpublished { unpushed: 0 },
            })
        }
        Err(error) => return Err(error.into()),
    };
    if head.is_branch() {
        let local = git_operation::run(|| repo.find_branch(&name, git2::BranchType::Local))?;
        match git_operation::run(|| local.upstream()) {
            Ok(upstream) => {
                let head_commit = git_operation::run(|| head.peel_to_commit())?;
                let upstream_commit = git_operation::run(|| upstream.get().peel_to_commit())?;
                let (ahead, behind) = git_operation::run(|| {
                    repo.graph_ahead_behind(head_commit.id(), upstream_commit.id())
                })?;
                return Ok(WorktreeBranch {
                    name,
                    state: BranchState::Tracking(Tracking {
                        upstream: git_operation::run(|| upstream.name())?
                            .ok_or_else(|| GitError::rule("Invalid upstream name"))?
                            .into(),
                        ahead: ahead as u32,
                        behind: behind as u32,
                    }),
                });
            }
            Err(error) if error.code() == git2::ErrorCode::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let unpushed = git(
        worktree,
        &["rev-list", "--count", "HEAD", "--not", "--remotes=origin"],
    )?
    .trim()
    .parse()
    .map_err(|error: std::num::ParseIntError| GitError::External(error.to_string()))?;
    Ok(WorktreeBranch {
        name,
        state: BranchState::Unpublished { unpushed },
    })
}

fn read(path: &Path) -> Result<String, GitError> {
    Ok(std::fs::read_to_string(path)?.trim().to_owned())
}
fn read_number(path: &Path) -> Result<u32, GitError> {
    read(path)?
        .parse()
        .map_err(|error: std::num::ParseIntError| GitError::External(error.to_string()))
}

pub(super) fn line_changes(worktree: &str) -> Result<LineChanges, GitError> {
    let repo = git_operation::run(|| client::open(worktree))?;
    let tree = match git_operation::run(|| repo.head()) {
        Ok(head) => Some(git_operation::run(|| head.peel_to_tree())?),
        Err(error) if error.code() == git2::ErrorCode::UnbornBranch => None,
        Err(error) => return Err(error.into()),
    };
    let index = git_operation::run(|| repo.index())?;
    let staged = git_operation::run(|| repo.diff_tree_to_index(tree.as_ref(), Some(&index), None))?;
    let mut options = git2::DiffOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);
    let unstaged =
        git_operation::run(|| repo.diff_index_to_workdir(Some(&index), Some(&mut options)))?;
    Ok(LineChanges {
        staged: counts(&staged)?,
        unstaged: counts(&unstaged)?,
    })
}
fn counts(diff: &git2::Diff<'_>) -> Result<HashMap<String, LineCount>, GitError> {
    let mut result = HashMap::new();
    for (index, delta) in diff.deltas().enumerate() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .and_then(Path::to_str)
            .ok_or_else(|| GitError::rule("Invalid diff path"))?
            .to_owned();
        let mut lines = LineCount::default();
        if let Some(patch) = git_operation::run(|| git2::Patch::from_diff(diff, index))? {
            let (_, additions, deletions) = git_operation::run(|| patch.line_stats())?;
            lines = LineCount {
                additions: additions as u32,
                deletions: deletions as u32,
            };
        }
        let total = result.entry(path).or_insert_with(LineCount::default);
        total.additions += lines.additions;
        total.deletions += lines.deletions;
    }
    Ok(result)
}

pub(super) fn validate_paths(worktree: &str, paths: &[String]) -> Result<(), GitError> {
    let root = std::fs::canonicalize(worktree)?;
    for value in paths {
        let path = Path::new(value);
        if value.is_empty()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(GitError::rule("Expected a relative file path"));
        }
        let mut parent = root
            .join(path)
            .parent()
            .ok_or_else(|| GitError::rule("Missing parent"))?
            .to_owned();
        loop {
            match std::fs::canonicalize(&parent) {
                Ok(parent) if parent.starts_with(&root) => break,
                Ok(_) => return Err(GitError::rule("Path escapes worktree")),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if !parent.pop() {
                        return Err(error.into());
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

pub(super) fn trash_files(worktree: &str, paths: &[String]) -> Result<(), GitError> {
    if paths.is_empty() {
        return Ok(());
    }
    validate_paths(worktree, paths)?;
    #[cfg(target_os = "macos")]
    {
        let mut command = tokio::process::Command::new("osascript");
        command.args(["-l", "JavaScript", "-e", "ObjC.import('Foundation'); function run(paths) { for (const path of paths) { const error = Ref(); if (!$.NSFileManager.defaultManager.trashItemAtURLResultingItemURLError($.NSURL.fileURLWithPath(path), null, error)) throw Error(ObjC.unwrap(error[0].localizedDescription)); } }"]);
        for path in paths {
            command.arg(Path::new(worktree).join(path));
        }
        let output = tokio::runtime::Handle::current()
            .block_on(crate::infrastructure::process::output::output(
                command,
                Vec::new(),
            ))
            .map_err(|error| match error {
                crate::infrastructure::process::output::ProcessError::Io(error) => {
                    GitError::from(error)
                }
                crate::infrastructure::process::output::ProcessError::Stopped(error) => {
                    GitError::from(error)
                }
            })?;
        if !output.status.success() {
            return Err(GitError::External(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(GitError::rule("OS trash is only supported on macOS"))
    }
}

pub(super) fn continue_rebase(worktree: &str, message: &CommitMessage) -> Result<(), GitError> {
    let repo = git_operation::run(|| client::open(worktree))?;
    let path = if repo.path().join("rebase-merge").is_dir() {
        repo.path().join("rebase-merge/message")
    } else {
        repo.path().join("rebase-apply/final-commit")
    };
    std::fs::write(path, format!("{}\n", message.as_str()))?;
    git(worktree, &["rebase", "--continue"]).map(|_| ())
}
