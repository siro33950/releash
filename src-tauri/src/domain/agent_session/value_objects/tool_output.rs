#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutputRef {
    pub id: String,
    pub byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutputSummary {
    pub line_count: u64,
    pub byte_size: u64,
    pub is_error: bool,
    pub truncated: bool,
}
