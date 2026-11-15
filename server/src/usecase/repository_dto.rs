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
    pub conflict: Option<crate::domain::git::ConflictKind>,
}

impl From<FileStatus> for FileStatusDto {
    fn from(status: FileStatus) -> Self {
        Self {
            path: status.path,
            index_status: status.index_status,
            worktree_status: status.worktree_status,
            conflict: status.conflict,
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
            conflict: status.conflict,
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
