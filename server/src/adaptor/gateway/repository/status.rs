//! status 責務の gateway 実装。git2 による作業ツリー状態取得を封じ込める。

use crate::adaptor::gateway::shared::git_operation;
use crate::domain::repository::{
    FileDiffStat, FileStatus, RepositoryError, RepositoryStatusScan, StatusRepository,
};
use crate::infrastructure::git::client;
use git2::{ErrorCode, Repository, StatusOptions};
use std::collections::HashMap;

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
pub fn get_git_status(repo_path: &str) -> Result<Vec<FileStatus>, RepositoryError> {
    collect_git_status(&git_operation::run(|| client::open(repo_path))?)
}

fn collect_git_status(repo: &Repository) -> Result<Vec<FileStatus>, RepositoryError> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);

    #[cfg(any(test, feature = "test-support"))]
    STATUS_WALK_COUNT.with(|count| count.set(count.get() + 1));
    let statuses = git_operation::run(|| repo.statuses(Some(&mut opts)))?;

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
                path,
                index_status: idx.to_string(),
                worktree_status: wt.to_string(),
            })
        })
        .collect();

    Ok(result)
}

fn count_patch_lines(diff: &git2::Diff, idx: usize) -> Result<(u32, u32), RepositoryError> {
    let patch =
        match git_operation::optional(git_operation::run(|| git2::Patch::from_diff(diff, idx)))? {
            Some(Some(p)) => p,
            _ => return Ok((0, 0)),
        };
    let mut adds = 0u32;
    let mut dels = 0u32;
    for h in 0..patch.num_hunks() {
        let lines =
            match git_operation::optional(git_operation::run(|| patch.num_lines_in_hunk(h)))? {
                Some(n) => n,
                None => continue,
            };
        for l in 0..lines {
            if let Some(line) =
                git_operation::optional(git_operation::run(|| patch.line_in_hunk(h, l)))?
            {
                match line.origin() {
                    '+' => adds += 1,
                    '-' => dels += 1,
                    _ => {}
                }
            }
        }
    }
    Ok((adds, dels))
}

fn collect_diff_stats(diff: &git2::Diff) -> Result<HashMap<String, (u32, u32)>, RepositoryError> {
    let mut map = HashMap::new();
    let num_deltas = diff.deltas().len();
    for i in 0..num_deltas {
        let delta = diff.get_delta(i).unwrap();
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().to_string());
        if let Some(path) = path {
            if delta.new_file().is_binary() || delta.old_file().is_binary() {
                map.insert(path, (0, 0));
            } else {
                let (adds, dels) = count_patch_lines(diff, i)?;
                map.insert(path, (adds, dels));
            }
        }
    }
    Ok(map)
}

#[cfg(any(test, feature = "test-support"))]
pub fn get_status_diff_stats(repo_path: &str) -> Result<Vec<FileDiffStat>, RepositoryError> {
    crate::infrastructure::telemetry::metrics::measure_result(
        crate::infrastructure::telemetry::metrics::HotPath::DiffStats,
        || get_status_diff_stats_inner(repo_path),
    )
}

#[cfg(any(test, feature = "test-support"))]
fn get_status_diff_stats_inner(repo_path: &str) -> Result<Vec<FileDiffStat>, RepositoryError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    collect_status_diff_stats(&repo)
}

fn collect_status_diff_stats(repo: &Repository) -> Result<Vec<FileDiffStat>, RepositoryError> {
    // HEAD tree (may not exist for unborn branch)
    let head_tree = match git_operation::run(|| repo.head()) {
        Ok(head) => Some(git_operation::run(|| head.peel_to_tree())?),
        Err(err) if err.code() == ErrorCode::UnbornBranch => None,
        Err(err) => return Err(err.into()),
    };

    // Index diff stats: HEAD → index (staged changes)
    let index_diff =
        git_operation::run(|| repo.diff_tree_to_index(head_tree.as_ref(), None, None))?;
    let index_stats = collect_diff_stats(&index_diff)?;

    // Worktree diff stats: index → worktree (unstaged changes)
    let mut wt_opts = git2::DiffOptions::new();
    wt_opts
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);
    let wt_diff = git_operation::run(|| repo.diff_index_to_workdir(None, Some(&mut wt_opts)))?;
    let wt_stats = collect_diff_stats(&wt_diff)?;

    // Merge all paths
    let mut all_paths = std::collections::BTreeSet::new();
    for path in index_stats.keys() {
        all_paths.insert(path.clone());
    }
    for path in wt_stats.keys() {
        all_paths.insert(path.clone());
    }

    let result = all_paths
        .into_iter()
        .map(|path| {
            let (ia, id) = index_stats.get(&path).copied().unwrap_or((0, 0));
            let (wa, wd) = wt_stats.get(&path).copied().unwrap_or((0, 0));
            FileDiffStat {
                path,
                index_additions: ia,
                index_deletions: id,
                wt_additions: wa,
                wt_deletions: wd,
            }
        })
        .collect();

    Ok(result)
}

pub fn get_repository_status_scan(
    repo_path: &str,
) -> Result<RepositoryStatusScan, RepositoryError> {
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

fn get_repository_status_scan_inner(
    repo_path: &str,
) -> Result<RepositoryStatusScan, RepositoryError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    let status = collect_git_status(&repo)?;
    let dirty_count = status
        .iter()
        .filter(|entry| entry.worktree_status != "ignored")
        .count();
    let diff_stats = crate::infrastructure::telemetry::metrics::measure_result(
        crate::infrastructure::telemetry::metrics::HotPath::DiffStats,
        || collect_status_diff_stats(&repo),
    )?;

    Ok(RepositoryStatusScan {
        status,
        diff_stats,
        dirty_count,
    })
}

/// `StatusRepository` の git2 実装。
pub struct StatusGateway;

impl StatusRepository for StatusGateway {
    fn status_scan(&self, repo_path: &str) -> Result<RepositoryStatusScan, RepositoryError> {
        get_repository_status_scan(repo_path)
    }
}
