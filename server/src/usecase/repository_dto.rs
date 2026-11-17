//! repository ユースケースの read model（DTO）。
//!
//! Command / protocol 境界で JSON 化する読み取り結果をここに集約する。
//! domain entity と同形の単純な読み取りも、serde 依存を domain に戻さないため
//! 1:1 DTO 経由で返す。

use serde::{Deserialize, Serialize};

use crate::domain::git::{Branch, FileStatus};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchDto {
    pub name: String,
    pub is_remote: bool,
}

impl From<Branch> for BranchDto {
    fn from(branch: Branch) -> Self {
        Self {
            name: branch.name,
            is_remote: branch.is_remote,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileStatusDto {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
    /// conflict しているファイルでは、その種類。conflict していないファイルでは `None`。
    pub conflict: Option<ConflictKindDto>,
}

/// 走査結果を運ぶときの、conflict の種類。`ConflictKind` と 1 対 1 に対応する。
///
/// 運ぶときは、`both_modified`、`both_added`、`both_deleted`、`added_by_us`、
/// `added_by_them`、`deleted_by_us`、`deleted_by_them` のどれかの文字列になる。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKindDto {
    BothModified,
    BothAdded,
    BothDeleted,
    AddedByUs,
    AddedByThem,
    DeletedByUs,
    DeletedByThem,
}

impl From<crate::domain::git::ConflictKind> for ConflictKindDto {
    fn from(kind: crate::domain::git::ConflictKind) -> Self {
        match kind {
            crate::domain::git::ConflictKind::BothModified => Self::BothModified,
            crate::domain::git::ConflictKind::BothAdded => Self::BothAdded,
            crate::domain::git::ConflictKind::BothDeleted => Self::BothDeleted,
            crate::domain::git::ConflictKind::AddedByUs => Self::AddedByUs,
            crate::domain::git::ConflictKind::AddedByThem => Self::AddedByThem,
            crate::domain::git::ConflictKind::DeletedByUs => Self::DeletedByUs,
            crate::domain::git::ConflictKind::DeletedByThem => Self::DeletedByThem,
        }
    }
}

impl From<ConflictKindDto> for crate::domain::git::ConflictKind {
    fn from(kind: ConflictKindDto) -> Self {
        match kind {
            ConflictKindDto::BothModified => Self::BothModified,
            ConflictKindDto::BothAdded => Self::BothAdded,
            ConflictKindDto::BothDeleted => Self::BothDeleted,
            ConflictKindDto::AddedByUs => Self::AddedByUs,
            ConflictKindDto::AddedByThem => Self::AddedByThem,
            ConflictKindDto::DeletedByUs => Self::DeletedByUs,
            ConflictKindDto::DeletedByThem => Self::DeletedByThem,
        }
    }
}

impl From<FileStatus> for FileStatusDto {
    fn from(status: FileStatus) -> Self {
        Self {
            path: status.path,
            index_status: status.index_status,
            worktree_status: status.worktree_status,
            conflict: status.conflict.map(Into::into),
        }
    }
}

impl From<FileStatusDto> for FileStatus {
    /// 運んできた走査結果を、domain の走査結果に戻す。
    fn from(status: FileStatusDto) -> Self {
        Self {
            path: status.path,
            index_status: status.index_status,
            worktree_status: status.worktree_status,
            conflict: status.conflict.map(Into::into),
        }
    }
}

/// ワークツリー一覧の 1 エントリ（旧 `WorktreeEntry`）の read model。
///
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorktreeEntryDto {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_main: bool,
    pub is_locked: bool,
}

#[cfg(test)]
#[path = "repository_dto_test.rs"]
mod repository_dto_tests;
