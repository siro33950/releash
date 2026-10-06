//! code 責務の Tauri コマンド引数として受理する入力メッセージ型。
//!
//! domain の値オブジェクトは serde 非依存のため、フロントから受け取る転送表現を本型で
//! 受理し、`into_domain()` で対応するドメイン値オブジェクトへ変換する。フィールド名・
//! camelCase は移行前と等価に保つ（フロントは変更しない）。

use serde::Deserialize;

use crate::domain::code::{DiffFileEntry, DiffSide, DiffTreeNode, Hunk};

/// `compute_hidden_ranges` のコマンド引数として受け取る hunk。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkInput {
    pub index: u32,
    #[serde(default)]
    pub hunk_id: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<String>,
}

impl HunkInput {
    pub fn into_domain(self) -> Hunk {
        Hunk {
            index: self.index,
            hunk_id: self.hunk_id,
            old_start: self.old_start,
            old_lines: self.old_lines,
            new_start: self.new_start,
            new_lines: self.new_lines,
            lines: self.lines,
        }
    }
}

/// `compute_markdown_diff_ranges` の対象 side。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkdownDiffSideInput {
    Modified,
    Original,
}

impl MarkdownDiffSideInput {
    pub fn into_usecase(self) -> DiffSide {
        match self {
            Self::Modified => DiffSide::Modified,
            Self::Original => DiffSide::Original,
        }
    }
}

/// `build_diff_file_tree` のコマンド引数として受け取るフラットなファイルエントリ。
/// フィールド名は snake_case（移行前と等価）。
#[derive(Debug, Clone, Deserialize)]
pub struct DiffFileEntryInput {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

impl DiffFileEntryInput {
    pub fn into_domain(self) -> DiffFileEntry {
        DiffFileEntry {
            path: self.path,
            status: self.status,
            additions: self.additions,
            deletions: self.deletions,
        }
    }
}

/// `get_file_navigation` のコマンド引数として受け取る diff ツリーノード（再帰）。
/// フィールド名は snake_case（移行前と等価）。
#[derive(Debug, Clone, Deserialize)]
pub struct DiffTreeNodeInput {
    pub id: String,
    pub name: String,
    pub path: String,
    pub node_type: String,
    pub status: Option<String>,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub children: Vec<DiffTreeNodeInput>,
}

impl DiffTreeNodeInput {
    pub fn into_domain(self) -> DiffTreeNode {
        DiffTreeNode {
            id: self.id,
            name: self.name,
            path: self.path,
            node_type: self.node_type,
            status: self.status,
            additions: self.additions,
            deletions: self.deletions,
            children: self.children.into_iter().map(Self::into_domain).collect(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewGroupActionInput {
    pub worktree_path: String,
    pub path: String,
    pub section: String,
    pub base: String,
    pub group_id: String,
}

#[cfg(test)]
#[path = "code_test.rs"]
mod code_tests;
