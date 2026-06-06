/// 単一の diff hunk（変更の塊の集合とヘッダ情報）。
#[derive(Debug, Clone)]
pub struct Hunk {
    pub index: u32,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<String>,
}

/// hunk 内の連続した変更ブロック（Approve 単位）。
#[derive(Debug, Clone)]
pub struct ChangeGroup {
    pub group_index: u32,
    pub hunk_index: u32,
    pub new_start: u32,
    pub new_end: u32,
    pub line_offset_start: u32,
    pub line_offset_end: u32,
    pub is_staged: Option<bool>,
}
