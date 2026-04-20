use serde::{Deserialize, Serialize};

#[derive(Serialize, Debug, Clone)]
pub struct BranchInfo {
    pub name: String,
    pub is_remote: bool,
}

#[derive(Serialize)]
pub struct GitFileStatus {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
}

#[derive(Serialize, Debug, Clone)]
pub struct StatusFileStat {
    pub path: String,
    pub index_additions: u32,
    pub index_deletions: u32,
    pub wt_additions: u32,
    pub wt_deletions: u32,
}

#[derive(Serialize)]
pub struct CommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: i64,
}

#[derive(Serialize, Debug, Clone)]
pub struct WorktreeEntry {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_main: bool,
    pub is_locked: bool,
    pub dirty_count: u32,
    pub base_branch: Option<String>,
}

// ── Hunk / ChangeGroup (diff calculation) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub index: u32,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeGroup {
    pub group_index: u32,
    pub hunk_index: u32,
    pub new_start: u32,
    pub new_end: u32,
    pub line_offset_start: u32,
    pub line_offset_end: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_staged: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunksResult {
    pub hunks: Vec<Hunk>,
    pub change_groups: Vec<ChangeGroup>,
}

// ── HiddenRange / VisibleBlock (diff-only mode) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenRange {
    pub start_line: u32,
    pub end_line: u32,
    pub hidden_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleBlock {
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_content: Option<String>,
}

#[derive(Serialize)]
pub struct WorktreeBranch {
    pub name: String,
    pub is_main_worktree: bool,
    pub worktree_path: Option<String>,
    pub dirty_count: usize,
    pub is_merged: bool,
    pub has_pr: bool,
    pub pr_number: Option<u64>,
    pub pr_url: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub has_upstream: bool,
    pub base_ahead: usize,
}
