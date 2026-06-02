use serde::{Deserialize, Serialize};

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
