/// diff-only 表示で折り畳む（非表示にする）行範囲。
#[derive(Debug, Clone)]
pub struct HiddenRange {
    pub start_line: u32,
    pub end_line: u32,
    pub hidden_count: u32,
}

/// Markdown diff-only 表示で可視にする行ブロック（削除内容を併記）。
#[derive(Debug, Clone)]
pub struct VisibleBlock {
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
    pub deleted_content: Option<String>,
}
