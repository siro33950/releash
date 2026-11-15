//! status 責務の gateway 実装。git2 による作業ツリー状態取得を封じ込める。

use crate::adaptor::gateway::shared::git_operation;
use crate::domain::git::{FileStatus, GitError, RepositoryStatusScan};
use crate::infrastructure::git::client;
use git2::{Repository, StatusOptions};

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static STATUS_WALK_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn reset_status_walk_count_for_tests() {
    STATUS_WALK_COUNT.with(|count| count.set(0));
}

#[cfg(any(test, feature = "test-support"))]
pub fn status_walk_count_for_tests() -> usize {
    STATUS_WALK_COUNT.with(|count| count.get())
}

fn index_status_from_flags(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::CONFLICTED) {
        "modified"
    } else if status.contains(git2::Status::INDEX_NEW) {
        "new"
    } else if status.contains(git2::Status::INDEX_MODIFIED) {
        "modified"
    } else if status.contains(git2::Status::INDEX_DELETED) {
        "deleted"
    } else if status.contains(git2::Status::INDEX_RENAMED) {
        "renamed"
    } else if status.contains(git2::Status::INDEX_TYPECHANGE) {
        "modified"
    } else {
        "none"
    }
}

fn worktree_status_from_flags(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::IGNORED) {
        "ignored"
    } else if status.contains(git2::Status::CONFLICTED) {
        "modified"
    } else if status.contains(git2::Status::WT_NEW) {
        "new"
    } else if status.contains(git2::Status::WT_MODIFIED) {
        "modified"
    } else if status.contains(git2::Status::WT_DELETED) {
        "deleted"
    } else if status.contains(git2::Status::WT_RENAMED)
        || status.contains(git2::Status::WT_TYPECHANGE)
    {
        "modified"
    } else {
        "none"
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn get_git_status(repo_path: &str) -> Result<Vec<FileStatus>, GitError> {
    collect_git_status(&git_operation::run(|| client::open(repo_path))?)
}

fn collect_git_status(repo: &Repository) -> Result<Vec<FileStatus>, GitError> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);

    #[cfg(any(test, feature = "test-support"))]
    STATUS_WALK_COUNT.with(|count| count.set(count.get() + 1));
    let statuses = git_operation::run(|| repo.statuses(Some(&mut opts)))?;

    let index = git_operation::run(|| repo.index())?;
    let mut conflicts = std::collections::HashMap::new();
    for conflict in git_operation::run(|| index.conflicts())? {
        let conflict = conflict?;
        use crate::domain::git::ConflictKind;
        let kind = match (
            conflict.ancestor.is_some(),
            conflict.our.is_some(),
            conflict.their.is_some(),
        ) {
            (true, true, true) => ConflictKind::BothModified,
            (false, true, true) => ConflictKind::BothAdded,
            (true, false, false) => ConflictKind::BothDeleted,
            (false, true, false) => ConflictKind::AddedByUs,
            (false, false, true) => ConflictKind::AddedByThem,
            (true, false, true) => ConflictKind::DeletedByUs,
            (true, true, false) => ConflictKind::DeletedByThem,
            (false, false, false) => return Err(GitError::rule("Empty index conflict")),
        };
        for entry in [conflict.ancestor, conflict.our, conflict.their]
            .into_iter()
            .flatten()
        {
            let path = String::from_utf8(entry.path)
                .map_err(|error| GitError::External(error.to_string()))?;
            conflicts.insert(path, kind);
        }
    }
    let result: Vec<FileStatus> = statuses
        .iter()
        .filter_map(|entry| {
            let path = entry.path().ok()?.to_string();
            let path = path.trim_end_matches('/').to_string();
            let status = entry.status();
            let idx = index_status_from_flags(status);
            let wt = worktree_status_from_flags(status);
            if idx == "none" && wt == "none" {
                return None;
            }
            Some(FileStatus {
                conflict: conflicts.get(&path).copied(),
                path,
                index_status: idx.to_string(),
                worktree_status: wt.to_string(),
            })
        })
        .collect();

    Ok(result)
}

pub fn get_repository_status_scan(repo_path: &str) -> Result<RepositoryStatusScan, GitError> {
    crate::common::telemetry::observe_result(
        || {
            crate::infrastructure::telemetry::metrics::measure_result(
                crate::infrastructure::telemetry::metrics::HotPath::GitStatusScan,
                || get_repository_status_scan_inner(repo_path),
            )
        },
        |result, _| {
            if result.is_ok() {
                crate::infrastructure::telemetry::metrics::record_first_repo_snapshot_ready();
            }
        },
    )
}

fn get_repository_status_scan_inner(repo_path: &str) -> Result<RepositoryStatusScan, GitError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    let status = collect_git_status(&repo)?;
    let dirty_count = status
        .iter()
        .filter(|entry| entry.worktree_status != "ignored")
        .count();
    Ok(RepositoryStatusScan {
        status,
        dirty_count,
    })
}
