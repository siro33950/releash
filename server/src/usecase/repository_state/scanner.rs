use std::collections::HashMap;

use crate::domain::code::DiffFileEntry;
use crate::domain::repository::Worktree;
use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};

use super::error::RepositoryStateError;
use super::snapshot::RepositorySnapshotParts;
use super::status_membership::{changed_statuses, staged_statuses};

#[async_trait::async_trait]
pub trait RepositoryScanner: Send + Sync {
    /// worktree の変更の状態を読む。
    fn scan(&self, repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError>;

    async fn scan_async(
        &self,
        repo_path: &str,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError>;

    /// Repository の worktree の並びを読む。
    fn scan_worktrees(&self, repo_path: &str) -> Result<Vec<Worktree>, RepositoryStateError>;

    /// 現存しないブランチの base 設定を掃除する。
    fn prune_stale_branch_bases(&self, repo_path: &str) -> Result<(), RepositoryStateError>;
}

fn stats_by_path(diff_stats: &[FileDiffStatDto]) -> HashMap<&str, &FileDiffStatDto> {
    diff_stats
        .iter()
        .map(|stat| (stat.path.as_str(), stat))
        .collect()
}

pub fn diff_tree_entries(
    status: &[FileStatusDto],
    diff_stats: &[FileDiffStatDto],
) -> Vec<DiffFileEntry> {
    let stats_by_path = stats_by_path(diff_stats);

    status
        .iter()
        .filter(|entry| entry.worktree_status != "ignored")
        .map(|entry| {
            let stat = stats_by_path.get(entry.path.as_str()).copied();
            DiffFileEntry {
                path: entry.path.clone(),
                status: diff_tree_status(entry).to_string(),
                additions: stat
                    .map(|stat| stat.index_additions + stat.wt_additions)
                    .unwrap_or(0),
                deletions: stat
                    .map(|stat| stat.index_deletions + stat.wt_deletions)
                    .unwrap_or(0),
            }
        })
        .collect()
}

pub fn staged_diff_tree_entries(
    status: &[FileStatusDto],
    diff_stats: &[FileDiffStatDto],
) -> Vec<DiffFileEntry> {
    let stats_by_path = stats_by_path(diff_stats);

    staged_statuses(status)
        .map(|entry| {
            let stat = stats_by_path.get(entry.path.as_str()).copied();
            DiffFileEntry {
                path: entry.path.clone(),
                status: entry.index_status.clone(),
                additions: stat.map(|stat| stat.index_additions).unwrap_or(0),
                deletions: stat.map(|stat| stat.index_deletions).unwrap_or(0),
            }
        })
        .collect()
}

pub fn changes_diff_tree_entries(
    status: &[FileStatusDto],
    diff_stats: &[FileDiffStatDto],
) -> Vec<DiffFileEntry> {
    let stats_by_path = stats_by_path(diff_stats);

    changed_statuses(status)
        .map(|entry| {
            let stat = stats_by_path.get(entry.path.as_str()).copied();
            DiffFileEntry {
                path: entry.path.clone(),
                status: entry.worktree_status.clone(),
                additions: stat.map(|stat| stat.wt_additions).unwrap_or(0),
                deletions: stat.map(|stat| stat.wt_deletions).unwrap_or(0),
            }
        })
        .collect()
}

fn diff_tree_status(entry: &FileStatusDto) -> &str {
    if entry.index_status != "none" {
        entry.index_status.as_str()
    } else {
        entry.worktree_status.as_str()
    }
}

#[cfg(test)]
#[path = "scanner_test.rs"]
pub(crate) mod scanner_tests;
