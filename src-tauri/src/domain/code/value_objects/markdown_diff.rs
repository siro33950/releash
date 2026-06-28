/// Markdown gutter diff の対象 side。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSide {
    Modified,
    Original,
}

/// Markdown gutter diff range。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRange {
    pub start_line: u32,
    pub end_line: u32,
    pub kind: DiffRangeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffRangeKind {
    Added,
    Modified,
    Deleted,
}

/// Markdown split diff row。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitRow {
    pub left: Option<String>,
    pub right: Option<String>,
    pub kind: SplitRowKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitRowKind {
    Unchanged,
    Added,
    Removed,
    Modified,
}

/// Markdown inline diff chunk。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineChunk {
    pub content: String,
    pub kind: InlineChunkKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineChunkKind {
    Unchanged,
    Added,
    Removed,
}
