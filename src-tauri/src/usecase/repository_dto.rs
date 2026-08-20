//! repository ユースケースの read model（DTO）。
//!
//! Command / protocol 境界で JSON 化する読み取り結果をここに集約する。
//! domain entity と同形の単純な読み取りも、serde 依存を domain に戻さないため
//! 1:1 DTO 経由で返す。

use serde::{Deserialize, Serialize};

use crate::domain::repository::{Branch, Commit, FileDiffStat, FileStatus};

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
pub struct CommitDto {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: i64,
}

impl From<Commit> for CommitDto {
    fn from(commit: Commit) -> Self {
        Self {
            hash: commit.hash,
            short_hash: commit.short_hash,
            message: commit.message,
            author_name: commit.author_name,
            author_email: commit.author_email,
            timestamp: commit.timestamp,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeEntryDto {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_main: bool,
    pub is_locked: bool,
    pub dirty_count: u32,
    pub base_branch: Option<String>,
    pub management_kind: String,
}

/// ブランチカード（旧 `WorktreeBranch`）の read model。
///
/// 単一の読み取りクエリ結果を denormalize しただけの表示・転送向けモデルであり
/// domain Entity ではない。Query 経路（[`BranchCardQuery`](super::repository_query_service::BranchCardQuery)）の
/// gateway 実装がデータソース（git2）から直接組み立てる。PR 情報（別ドメイン git_host 由来）は
/// この repository read model には含めず、呼び出し側が別途取得・合成する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchCardDto {
    pub name: String,
    pub is_main_worktree: bool,
    pub worktree_path: Option<String>,
    pub dirty_count: usize,
    pub is_merged: bool,
    pub ahead: usize,
    pub behind: usize,
    pub has_upstream: bool,
    pub base_ahead: usize,
    pub management_kind: Option<String>,
}

/// 管理 UI の worktree 表示先。分類は backend が確定し、client は
/// 返された一覧をそのまま描画する。
#[derive(Debug, Clone, Default, Serialize)]
pub struct WorktreeDisplayGroupsDto {
    /// 通常一覧に出す worktree card。
    pub working_areas: Vec<BranchCardDto>,
    /// 掃除候補として提示する worktree card。
    pub cleanup_candidates: Vec<BranchCardDto>,
}
