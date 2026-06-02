use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeListRequest {}

/// worktree 一覧の 1 エントリ（repository ローカル情報のみ）。
///
/// PR ステータス（別ドメイン git_host 由来）はここには含めず、worktree 一覧の返却後に
/// [`WorktreePrStatusSync`] で後追い配信する（一覧表示と PR 表示の 2 段階化）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeEntryMsg {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_main: bool,
    pub is_locked: bool,
    pub dirty_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeListResponse {
    pub worktrees: Vec<WorktreeEntryMsg>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeSelectRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeSelectResponse {
    pub success: bool,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
