use std::collections::HashMap;

use crate::domain::code::DiffFileEntry;
use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};

use super::error::RepositoryStateError;
use super::snapshot::RepositorySnapshotParts;
use super::status_membership::{changed_statuses, staged_statuses};

pub trait RepositoryScanner: Send + Sync {
    fn scan(&self, repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError>;

    fn status_with_ignored(
        &self,
        repo_path: &str,
    ) -> Result<Vec<FileStatusDto>, RepositoryStateError>;

    fn prune_stale_branch_bases(
        &self,
        repo_path: &str,
        existing_branches: &[String],
    ) -> Result<(), RepositoryStateError>;
}

fn stats_by_path(diff_stats: &[FileDiffStatDto]) -> HashMap<&str, &FileDiffStatDto> {
    diff_stats
        .iter()
        .map(|stat| (stat.path.as_str(), stat))
        .collect()
}

pub(crate) fn diff_tree_entries(
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

pub(crate) fn staged_diff_tree_entries(
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

pub(crate) fn changes_diff_tree_entries(
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
mod tests {
    use super::*;

    #[test]
    fn diff_tree_entries_use_index_status_before_worktree_status() {
        let status = vec![FileStatusDto {
            path: "src/lib.rs".to_string(),
            index_status: "modified".to_string(),
            worktree_status: "deleted".to_string(),
        }];
        let stats = vec![FileDiffStatDto {
            path: "src/lib.rs".to_string(),
            index_additions: 2,
            index_deletions: 1,
            wt_additions: 3,
            wt_deletions: 4,
        }];

        let entries = diff_tree_entries(&status, &stats);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "modified");
        assert_eq!(entries[0].additions, 5);
        assert_eq!(entries[0].deletions, 5);
    }

    #[test]
    fn diff_tree_entries_skip_ignored_status() {
        let status = vec![FileStatusDto {
            path: "target".to_string(),
            index_status: "none".to_string(),
            worktree_status: "ignored".to_string(),
        }];

        assert!(diff_tree_entries(&status, &[]).is_empty());
    }

    #[test]
    fn staged_and_changes_entries_are_split_from_same_status_and_stats() {
        let status = vec![
            FileStatusDto {
                path: "both.rs".to_string(),
                index_status: "modified".to_string(),
                worktree_status: "deleted".to_string(),
            },
            FileStatusDto {
                path: "ignored".to_string(),
                index_status: "none".to_string(),
                worktree_status: "ignored".to_string(),
            },
        ];
        let stats = vec![FileDiffStatDto {
            path: "both.rs".to_string(),
            index_additions: 2,
            index_deletions: 1,
            wt_additions: 0,
            wt_deletions: 4,
        }];

        let staged = staged_diff_tree_entries(&status, &stats);
        let changes = changes_diff_tree_entries(&status, &stats);

        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].status, "modified");
        assert_eq!(staged[0].additions, 2);
        assert_eq!(staged[0].deletions, 1);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].status, "deleted");
        assert_eq!(changes[0].additions, 0);
        assert_eq!(changes[0].deletions, 4);
    }
}
