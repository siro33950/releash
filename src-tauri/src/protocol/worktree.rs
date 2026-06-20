use serde::{Deserialize, Serialize};

/// worktree 1 件分の PR ステータス。worktree のパスで対象を識別する
/// （branch 名マッチングは Rust 側で済ませて配信する）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreePrEntry {
    pub path: String,
    pub pr_number: u64,
    pub pr_url: String,
}

/// worktree 一覧に対する PR ステータスの後追い配信。
///
/// `entries` には PR が存在する worktree のみを含める。フロントは `path` 一致で
/// 該当行に PR バッジを反映する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreePrStatusSync {
    pub entries: Vec<WorktreePrEntry>,
}
