use serde::{Deserialize, Serialize};

use crate::usecase::code_dto::DiffTreeNodeDto;
use crate::usecase::repository_dto::{
    BranchCardDto, FileDiffStatDto, FileStatusDto, WorktreeDisplayGroupsDto,
};

use super::status_membership::{changed_statuses, staged_statuses};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotFlags {
    pub stale: bool,
    pub loading: bool,
    pub limited: bool,
}

impl SnapshotFlags {
    pub fn loading() -> Self {
        Self {
            stale: false,
            loading: true,
            limited: false,
        }
    }

    pub fn ready() -> Self {
        Self {
            stale: false,
            loading: false,
            limited: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RepositorySnapshot {
    pub version: u64,
    pub flags: SnapshotFlags,
    pub status: Vec<FileStatusDto>,
    pub diff_stats: Vec<FileDiffStatDto>,
    pub branch_cards: Vec<BranchCardDto>,
    pub diff_file_tree: Vec<DiffTreeNodeDto>,
    pub staged_diff_file_tree: Vec<DiffTreeNodeDto>,
    pub changes_diff_file_tree: Vec<DiffTreeNodeDto>,
}

impl RepositorySnapshot {
    pub fn loading() -> Self {
        Self {
            version: 0,
            flags: SnapshotFlags::loading(),
            status: Vec::new(),
            diff_stats: Vec::new(),
            branch_cards: Vec::new(),
            diff_file_tree: Vec::new(),
            staged_diff_file_tree: Vec::new(),
            changes_diff_file_tree: Vec::new(),
        }
    }

    pub fn with_read_flags(&self, stale: bool, loading: bool) -> Self {
        let mut snapshot = self.clone();
        snapshot.flags.stale = stale;
        snapshot.flags.loading = loading;
        snapshot
    }
}

#[derive(Debug, Clone)]
pub struct RepositorySnapshotParts {
    pub status: Vec<FileStatusDto>,
    pub diff_stats: Vec<FileDiffStatDto>,
    pub branch_cards: Vec<BranchCardDto>,
    pub diff_file_tree: Vec<DiffTreeNodeDto>,
    pub staged_diff_file_tree: Vec<DiffTreeNodeDto>,
    pub changes_diff_file_tree: Vec<DiffTreeNodeDto>,
    pub limited: bool,
}

impl RepositorySnapshotParts {
    pub fn into_snapshot(self, version: u64) -> RepositorySnapshot {
        RepositorySnapshot {
            version,
            flags: SnapshotFlags {
                limited: self.limited,
                ..SnapshotFlags::ready()
            },
            status: self.status,
            diff_stats: self.diff_stats,
            branch_cards: self.branch_cards,
            diff_file_tree: self.diff_file_tree,
            staged_diff_file_tree: self.staged_diff_file_tree,
            changes_diff_file_tree: self.changes_diff_file_tree,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RepositoryStatusSnapshotDto {
    pub version: u64,
    pub stale: bool,
    pub loading: bool,
    pub limited: bool,
    pub status: Vec<FileStatusDto>,
}

impl RepositoryStatusSnapshotDto {
    pub fn from_snapshot(snapshot: &RepositorySnapshot) -> Self {
        Self {
            version: snapshot.version,
            stale: snapshot.flags.stale,
            loading: snapshot.flags.loading,
            limited: snapshot.flags.limited,
            status: snapshot.status.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RepositoryDiffStatsSnapshotDto {
    pub version: u64,
    pub stale: bool,
    pub loading: bool,
    pub limited: bool,
    pub diff_stats: Vec<FileDiffStatDto>,
}

impl RepositoryDiffStatsSnapshotDto {
    pub fn from_snapshot(snapshot: &RepositorySnapshot) -> Self {
        Self {
            version: snapshot.version,
            stale: snapshot.flags.stale,
            loading: snapshot.flags.loading,
            limited: snapshot.flags.limited,
            diff_stats: snapshot.diff_stats.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RepositoryBranchCardsSnapshotDto {
    pub version: u64,
    pub stale: bool,
    pub loading: bool,
    pub limited: bool,
    pub branches: Vec<BranchCardDto>,
    /// 管理 UI の表示先ごとに振り分けた worktree card。
    pub worktree_display_groups: WorktreeDisplayGroupsDto,
}

impl RepositoryBranchCardsSnapshotDto {
    pub fn from_snapshot(snapshot: &RepositorySnapshot) -> Self {
        Self {
            version: snapshot.version,
            stale: snapshot.flags.stale,
            loading: snapshot.flags.loading,
            limited: snapshot.flags.limited,
            branches: snapshot.branch_cards.clone(),
            worktree_display_groups: WorktreeDisplayGroupsDto::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RepositoryHeadDiffFileTreeSnapshotDto {
    pub version: u64,
    pub stale: bool,
    pub loading: bool,
    pub limited: bool,
    pub combined_tree: Vec<DiffTreeNodeDto>,
    pub staged_tree: Vec<DiffTreeNodeDto>,
    pub changes_tree: Vec<DiffTreeNodeDto>,
    pub staged_file_count: usize,
    pub changes_file_count: usize,
}

impl RepositoryHeadDiffFileTreeSnapshotDto {
    pub fn from_snapshot(snapshot: &RepositorySnapshot) -> Self {
        let staged_file_count = staged_statuses(&snapshot.status).count();
        let changes_file_count = changed_statuses(&snapshot.status).count();
        Self {
            version: snapshot.version,
            stale: snapshot.flags.stale,
            loading: snapshot.flags.loading,
            limited: snapshot.flags.limited,
            combined_tree: snapshot.diff_file_tree.clone(),
            staged_tree: snapshot.staged_diff_file_tree.clone(),
            changes_tree: snapshot.changes_diff_file_tree.clone(),
            staged_file_count,
            changes_file_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(path: &str, index_status: &str, worktree_status: &str) -> FileStatusDto {
        FileStatusDto {
            path: path.to_string(),
            index_status: index_status.to_string(),
            worktree_status: worktree_status.to_string(),
        }
    }

    fn node(path: &str) -> DiffTreeNodeDto {
        DiffTreeNodeDto {
            id: path.to_string(),
            name: path.to_string(),
            path: path.to_string(),
            node_type: "file".to_string(),
            status: Some("modified".to_string()),
            additions: Some(1),
            deletions: Some(0),
            children: Vec::new(),
        }
    }

    fn parts(limited: bool) -> RepositorySnapshotParts {
        RepositorySnapshotParts {
            status: Vec::new(),
            diff_stats: Vec::new(),
            branch_cards: Vec::new(),
            diff_file_tree: vec![node("combined.rs")],
            staged_diff_file_tree: vec![node("staged.rs")],
            changes_diff_file_tree: vec![node("changes.rs")],
            limited,
        }
    }

    #[test]
    fn head_diff_file_tree_dto_exposes_combined_tree_from_snapshot() {
        let snapshot = parts(false).into_snapshot(7);

        let dto = RepositoryHeadDiffFileTreeSnapshotDto::from_snapshot(&snapshot);

        assert_eq!(dto.version, 7);
        assert_eq!(dto.combined_tree.len(), 1);
        assert_eq!(dto.combined_tree[0].path, "combined.rs");
        assert_eq!(dto.staged_tree[0].path, "staged.rs");
        assert_eq!(dto.changes_tree[0].path, "changes.rs");
    }

    #[test]
    fn head_diff_file_tree_counts_staged_changes_and_ignored_boundaries() {
        let mut snapshot = parts(false).into_snapshot(3);
        snapshot.status = vec![
            status("staged-only.rs", "modified", "none"),
            status("changes-only.rs", "none", "modified"),
            status("both.rs", "new", "deleted"),
            status("ignored", "none", "ignored"),
            status("clean.rs", "none", "none"),
        ];

        let dto = RepositoryHeadDiffFileTreeSnapshotDto::from_snapshot(&snapshot);

        assert_eq!(dto.staged_file_count, 2);
        assert_eq!(dto.changes_file_count, 2);
    }

    #[test]
    fn limited_flag_is_carried_into_snapshot() {
        assert!(parts(true).into_snapshot(1).flags.limited);
        assert!(!parts(false).into_snapshot(1).flags.limited);
    }

    #[test]
    fn limited_flag_is_carried_into_all_snapshot_dtos_and_event() {
        let snapshot = parts(true).into_snapshot(9);

        assert!(RepositoryStatusSnapshotDto::from_snapshot(&snapshot).limited);
        assert!(RepositoryDiffStatsSnapshotDto::from_snapshot(&snapshot).limited);
        assert!(RepositoryBranchCardsSnapshotDto::from_snapshot(&snapshot).limited);
        assert!(RepositoryHeadDiffFileTreeSnapshotDto::from_snapshot(&snapshot).limited);
        assert!(
            RepositorySnapshotChangedEvent::from_snapshot("/repo".to_string(), &snapshot).limited
        );
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RepositorySnapshotChangedEvent {
    pub worktree_path: String,
    pub version: u64,
    pub stale: bool,
    pub loading: bool,
    pub limited: bool,
}

impl RepositorySnapshotChangedEvent {
    pub fn from_snapshot(worktree_path: String, snapshot: &RepositorySnapshot) -> Self {
        Self {
            worktree_path,
            version: snapshot.version,
            stale: snapshot.flags.stale,
            loading: snapshot.flags.loading,
            limited: snapshot.flags.limited,
        }
    }
}
