use serde::{Deserialize, Serialize};

use crate::usecase::code_dto::DiffTreeNodeDto;
use crate::usecase::repository_dto::{
    BranchCardDto, FileDiffStatDto, FileStatusDto, WorktreeDisplayGroupsDto,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotFlags {
    pub stale: bool,
    pub loading: bool,
}

impl SnapshotFlags {
    pub fn loading() -> Self {
        Self {
            stale: false,
            loading: true,
        }
    }

    pub fn ready() -> Self {
        Self {
            stale: false,
            loading: false,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositorySnapshotParts {
    pub status: Vec<FileStatusDto>,
    pub diff_stats: Vec<FileDiffStatDto>,
    pub branch_cards: Vec<BranchCardDto>,
    pub diff_file_tree: Vec<DiffTreeNodeDto>,
    pub staged_diff_file_tree: Vec<DiffTreeNodeDto>,
    pub changes_diff_file_tree: Vec<DiffTreeNodeDto>,
}

impl RepositorySnapshotParts {
    pub fn into_snapshot(self, version: u64) -> RepositorySnapshot {
        RepositorySnapshot {
            version,
            flags: SnapshotFlags::ready(),
            status: self.status,
            diff_stats: self.diff_stats,
            branch_cards: self.branch_cards,
            diff_file_tree: self.diff_file_tree,
            staged_diff_file_tree: self.staged_diff_file_tree,
            changes_diff_file_tree: self.changes_diff_file_tree,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RepositoryBranchCardsSnapshotDto {
    pub version: u64,
    pub stale: bool,
    pub loading: bool,
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
            branches: snapshot.branch_cards.clone(),
            worktree_display_groups: WorktreeDisplayGroupsDto::default(),
        }
    }
}

#[cfg(test)]
#[path = "snapshot_test.rs"]
mod snapshot_tests;
