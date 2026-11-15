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
    /// conflict しているファイルでは、`ConflictKind::as_str` の文字列。conflict していないファイルでは `None`。
    pub conflict: Option<String>,
}

impl From<FileStatus> for FileStatusDto {
    fn from(status: FileStatus) -> Self {
        Self {
            path: status.path,
            index_status: status.index_status,
            worktree_status: status.worktree_status,
            conflict: status.conflict.map(|kind| kind.as_str().to_owned()),
        }
    }
}

impl TryFrom<FileStatusDto> for FileStatus {
    type Error = crate::domain::git::GitError;

    /// 運んできた走査結果を、domain の走査結果に戻す。`conflict` は `ConflictKind::parse` で読む。
    fn try_from(status: FileStatusDto) -> Result<Self, Self::Error> {
        Ok(Self {
            path: status.path,
            index_status: status.index_status,
            worktree_status: status.worktree_status,
            conflict: status
                .conflict
                .as_deref()
                .map(crate::domain::git::ConflictKind::parse)
                .transpose()?,
        })
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
