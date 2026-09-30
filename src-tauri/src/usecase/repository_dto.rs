//! repository ユースケースの read model（DTO）。
//!
//! Command / protocol 境界で JSON 化する読み取り結果をここに集約する。
//! domain entity と同形の単純な読み取りも、serde 依存を domain に戻さないため
//! 1:1 DTO 経由で返す。

use serde::{Deserialize, Serialize};

use crate::domain::repository::{Branch, FileDiffStat, FileStatus};

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
}

impl From<FileStatus> for FileStatusDto {
    fn from(status: FileStatus) -> Self {
        Self {
            path: status.path,
            index_status: status.index_status,
            worktree_status: status.worktree_status,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileDiffStatDto {
    pub path: String,
    pub index_additions: u32,
    pub index_deletions: u32,
    pub wt_additions: u32,
    pub wt_deletions: u32,
}

impl From<FileDiffStat> for FileDiffStatDto {
    fn from(stat: FileDiffStat) -> Self {
        Self {
            path: stat.path,
            index_additions: stat.index_additions,
            index_deletions: stat.index_deletions,
            wt_additions: stat.wt_additions,
            wt_deletions: stat.wt_deletions,
        }
    }
}

/// ワークツリー一覧の 1 エントリ（旧 `WorktreeEntry`）の read model。
///
/// worktree 識別情報（`domain::repository::Worktree`）に `dirty_count`（status 由来）と
/// `base_branch`（git_config 由来）を合成した表示・転送向けモデル。単一 Entity の 1:1 写像では
/// なく、複数集約を usecase が合成して組み立てる read model であり domain Entity ではない。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorktreeEntryDto {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_main: bool,
    pub is_locked: bool,
    pub dirty_count: u32,
    pub base_branch: Option<String>,
}
