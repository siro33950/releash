//! code ユースケースの read model（DTO）。
//!
//! フロントへ返す転送表現を所有する層。Query 経路（QueryService）が読み取り要求ごとに
//! 本 DTO を直接組み立てる。serialize 表現（フィールド名・camelCase・省略）は移行前の
//! 各型と等価に保つ。
//!
//! branch diff のサマリは git2 の diff 結果を denormalize した表示・転送向けモデルで
//! あり domain Entity ではない。Query 経路（[`BranchDiffQuery`](super::code_query_service::BranchDiffQuery)）の
//! gateway 実装がデータソース（git2）から直接組み立てる。

use serde::{Deserialize, Serialize};

use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffStatsDto {
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedFileDto {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub binary: bool,
    pub stats: DiffStatsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchDiffSummaryDto {
    pub base_branch: String,
    pub changed_files: Vec<ChangedFileDto>,
    pub stats: DiffStatsDto,
}

// ── hunk / patch / range（Query が domain サービスの算出結果を詰め替えて返す） ──

/// 単一の diff hunk の転送表現。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HunkDto {
    pub index: u32,
    pub hunk_id: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<String>,
}

/// hunk 内の変更ブロック（Approve 単位）の転送表現。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangeGroupDto {
    pub group_index: u32,
    pub group_id: String,
    pub hunk_index: u32,
    pub new_start: u32,
    pub new_end: u32,
    pub line_offset_start: u32,
    pub line_offset_end: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_staged: Option<bool>,
}

/// diff hunk 計算結果の転送表現。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunksResultDto {
    pub hunks: Vec<HunkDto>,
    pub change_groups: Vec<ChangeGroupDto>,
}

/// diff-only 表示で折り畳む行範囲の転送表現。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenRangeDto {
    pub start_line: u32,
    pub end_line: u32,
    pub hidden_count: u32,
}

/// Markdown diff-only 表示で可視にする行ブロックの転送表現。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleBlockDto {
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_content: Option<String>,
}

/// Markdown gutter diff range の転送表現。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffRangeDto {
    pub start_line: u32,
    pub end_line: u32,
    #[serde(rename = "type")]
    pub kind: DiffRangeKindDto,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DiffRangeKindDto {
    Added,
    Modified,
    Deleted,
}

/// Markdown split diff row の転送表現。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SplitRowDto {
    pub left: Option<String>,
    pub right: Option<String>,
    #[serde(rename = "type")]
    pub kind: SplitRowKindDto,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SplitRowKindDto {
    Unchanged,
    Added,
    Removed,
    Modified,
}

/// Markdown inline diff chunk の転送表現。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InlineChunkDto {
    pub content: String,
    #[serde(rename = "type")]
    pub kind: InlineChunkKindDto,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InlineChunkKindDto {
    Unchanged,
    Added,
    Removed,
}

// ── diff_tree（フィールド名は snake_case のまま＝移行前と等価） ──

/// diff ファイルツリーのノードの転送表現。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffTreeNodeDto {
    pub id: String,
    pub name: String,
    pub path: String,
    pub node_type: String,
    pub status: Option<String>,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub children: Vec<DiffTreeNodeDto>,
}

/// ファイルナビゲーション情報の転送表現。
#[derive(Debug, Clone, Serialize)]
pub struct FileNavigationResultDto {
    pub current_index: usize,
    pub total: usize,
    pub prev_file: Option<String>,
    pub next_file: Option<String>,
}

/// review ファイル一覧 read model。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshotDto {
    pub version: u64,
    pub stale: bool,
    pub loading: bool,
    pub base: String,
    pub files: Vec<ReviewFileEntryDto>,
    pub staged_files: Vec<FileStatusDto>,
    pub changed_files: Vec<FileStatusDto>,
    pub diff_stats: Vec<FileDiffStatDto>,
    pub tree: Vec<DiffTreeNodeDto>,
    pub staged_tree: Vec<DiffTreeNodeDto>,
    pub changes_tree: Vec<DiffTreeNodeDto>,
    pub staged_file_count: usize,
    pub changes_file_count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFileEntryDto {
    pub file_id: String,
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ReviewFileViewDto {
    TextDiff(ReviewTextDiffDto),
    Image(ReviewImageDto),
    Binary(ReviewBinaryDto),
    Fallback(ReviewFallbackDto),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewTextDiffDto {
    pub version: u64,
    pub stale: bool,
    pub file_id: String,
    pub path: String,
    pub original: String,
    pub modified: String,
    pub source: ReviewTextSource,
    pub hunks: Vec<HunkDto>,
    pub change_groups: Vec<ChangeGroupDto>,
    pub limited: bool,
    pub total_lines: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReviewTextSource {
    Diff,
    Added,
    Deleted,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewImageDto {
    pub version: u64,
    pub stale: bool,
    pub file_id: String,
    pub path: String,
    pub original_url: Option<String>,
    pub modified_url: Option<String>,
    pub mime: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewBinaryDto {
    pub version: u64,
    pub stale: bool,
    pub file_id: String,
    pub path: String,
    pub original_size: Option<u64>,
    pub modified_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFallbackDto {
    pub version: u64,
    pub stale: bool,
    pub file_id: String,
    pub path: String,
    pub reason: ReviewLimitReasonDto,
    pub total_lines: Option<u32>,
    pub size_bytes: Option<u64>,
    pub hunk_count: Option<u32>,
    pub limited: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReviewLimitReasonDto {
    FileSize,
    LineCount,
    HunkCount,
    Tokenization,
}

#[cfg(test)]
#[path = "code_dto_test.rs"]
mod code_dto_tests;
