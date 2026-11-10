//! code ユースケースの read model（DTO）。
//!
//! フロントへ返す転送表現を所有する層。Query 経路（QueryService）が読み取り要求ごとに
//! 本 DTO を直接組み立てる。serialize 表現（フィールド名・camelCase・省略）は移行前の
//! 各型と等価に保つ。
//!
//! branch diff のサマリは git2 の diff 結果を denormalize した表示・転送向けモデルで
//! あり domain Entity ではない。Query 経路（[`BranchDiffQuery`](super::code_query_service::BranchDiffQuery)）の
//! gateway 実装がデータソース（git2）から直接組み立てる。

// ── hunk / patch / range（Query が domain サービスの算出結果を詰め替えて返す） ──

/// 単一の diff hunk の転送表現。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HunkDto {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<String>,
}

/// diff hunk 計算結果の転送表現。
#[derive(Debug, Clone)]
pub struct DiffHunksResultDto {
    pub hunks: Vec<HunkDto>,
}

// ── diff_tree（フィールド名は snake_case のまま＝移行前と等価） ──

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewFileViewDto {
    TextDiff(ReviewTextDiffDto),
    Image(ReviewImageDto),
    Binary(ReviewBinaryDto),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTextDiffDto {
    pub path: String,
    pub original: String,
    pub modified: String,
    pub source: ReviewTextSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewTextSource {
    Diff,
    Added,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewImageDto {
    pub path: String,
    pub original_url: Option<String>,
    pub modified_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewBinaryDto {
    pub path: String,
    pub original_size: Option<u64>,
    pub modified_size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileTabViewDto {
    pub view: ReviewFileViewDto,
    pub initial_mode: crate::domain::code::value_objects::review::FileMode,
    pub patch: String,
}
