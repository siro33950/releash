//! Markdown diff block の純粋計算。
//!
//! diff バッファの計算（git2 依存）は gateway が担い、本サービスは生成済みの
//! `Hunk` と original / modified 全文から変更ブロック列を導出する。

use crate::domain::code::value_objects::{
    DiffRange, DiffRangeKind, DiffSide, Hunk, InlineChunk, InlineChunkKind, SplitRow, SplitRowKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffBlockKind {
    Unchanged,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffBlock {
    pub kind: DiffBlockKind,
    pub left: Option<String>,
    pub right: Option<String>,
    pub line_count: u32,
}

fn split_lines_preserve_endings(input: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut start = 0;

    for (index, ch) in input.char_indices() {
        if ch == '\n' {
            let end = index + ch.len_utf8();
            lines.push(input[start..end].to_string());
            start = end;
        }
    }

    if start < input.len() {
        lines.push(input[start..].to_string());
    }

    lines
}

fn one_based_to_index(line: u32) -> usize {
    line.saturating_sub(1) as usize
}

fn hunk_line_content(line: &str) -> String {
    line.get(1..).unwrap_or("").to_string()
}

fn push_block(
    blocks: &mut Vec<DiffBlock>,
    kind: DiffBlockKind,
    left: Option<String>,
    right: Option<String>,
) {
    let has_content = left.as_deref().is_some_and(|value| !value.is_empty())
        || right.as_deref().is_some_and(|value| !value.is_empty());
    if !has_content {
        return;
    }

    if let Some(last) = blocks.last_mut().filter(|last| last.kind == kind) {
        if let Some(value) = left {
            last.left.get_or_insert_with(String::new).push_str(&value);
        }
        if let Some(value) = right {
            last.right.get_or_insert_with(String::new).push_str(&value);
        }
        last.line_count += 1;
        return;
    }

    blocks.push(DiffBlock {
        kind,
        left,
        right,
        line_count: 1,
    });
}

fn push_gap(
    blocks: &mut Vec<DiffBlock>,
    original_lines: &[String],
    modified_lines: &[String],
    old_index: &mut usize,
    new_index: &mut usize,
    target_old_index: usize,
    target_new_index: usize,
) {
    while *old_index < target_old_index && *new_index < target_new_index {
        let left = original_lines.get(*old_index).cloned().unwrap_or_default();
        let right = modified_lines.get(*new_index).cloned().unwrap_or_default();
        push_block(blocks, DiffBlockKind::Unchanged, Some(left), Some(right));
        *old_index += 1;
        *new_index += 1;
    }

    while *old_index < target_old_index {
        let left = original_lines.get(*old_index).cloned().unwrap_or_default();
        push_block(blocks, DiffBlockKind::Removed, Some(left), None);
        *old_index += 1;
    }

    while *new_index < target_new_index {
        let right = modified_lines.get(*new_index).cloned().unwrap_or_default();
        push_block(blocks, DiffBlockKind::Added, None, Some(right));
        *new_index += 1;
    }
}

/// hunk と全文から、左右の変更内容を保持した汎用 diff block 列を算出する。
pub fn compute_diff_blocks(hunks: &[Hunk], original: &str, modified: &str) -> Vec<DiffBlock> {
    let original_lines = split_lines_preserve_endings(original);
    let modified_lines = split_lines_preserve_endings(modified);
    let mut blocks = Vec::new();
    let mut old_index = 0usize;
    let mut new_index = 0usize;

    let mut sorted_hunks = hunks.to_vec();
    sorted_hunks.sort_by_key(|hunk| (hunk.old_start, hunk.new_start, hunk.index));

    for hunk in &sorted_hunks {
        push_gap(
            &mut blocks,
            &original_lines,
            &modified_lines,
            &mut old_index,
            &mut new_index,
            one_based_to_index(hunk.old_start),
            one_based_to_index(hunk.new_start),
        );

        for line in &hunk.lines {
            let prefix = line.as_bytes().first().copied().unwrap_or(b' ');
            match prefix {
                b' ' => {
                    let fallback = hunk_line_content(line);
                    let left = original_lines
                        .get(old_index)
                        .cloned()
                        .unwrap_or_else(|| fallback.clone());
                    let right = modified_lines
                        .get(new_index)
                        .cloned()
                        .unwrap_or_else(|| fallback.clone());
                    push_block(
                        &mut blocks,
                        DiffBlockKind::Unchanged,
                        Some(left),
                        Some(right),
                    );
                    old_index += 1;
                    new_index += 1;
                }
                b'-' => {
                    let left = original_lines
                        .get(old_index)
                        .cloned()
                        .unwrap_or_else(|| hunk_line_content(line));
                    push_block(&mut blocks, DiffBlockKind::Removed, Some(left), None);
                    old_index += 1;
                }
                b'+' => {
                    let right = modified_lines
                        .get(new_index)
                        .cloned()
                        .unwrap_or_else(|| hunk_line_content(line));
                    push_block(&mut blocks, DiffBlockKind::Added, None, Some(right));
                    new_index += 1;
                }
                b'\\' => {}
                _ => {
                    let fallback = line.to_string();
                    let left = original_lines
                        .get(old_index)
                        .cloned()
                        .unwrap_or_else(|| fallback.clone());
                    let right = modified_lines
                        .get(new_index)
                        .cloned()
                        .unwrap_or_else(|| fallback.clone());
                    push_block(
                        &mut blocks,
                        DiffBlockKind::Unchanged,
                        Some(left),
                        Some(right),
                    );
                    old_index += 1;
                    new_index += 1;
                }
            }
        }
    }

    push_gap(
        &mut blocks,
        &original_lines,
        &modified_lines,
        &mut old_index,
        &mut new_index,
        original_lines.len(),
        modified_lines.len(),
    );

    blocks
}

/// diff block 列から、指定 side の Markdown gutter range を導出する。
pub fn markdown_diff_ranges_from_blocks(blocks: &[DiffBlock], side: DiffSide) -> Vec<DiffRange> {
    let mut ranges = Vec::new();
    let mut line = 1u32;
    let mut index = 0usize;

    while index < blocks.len() {
        let block = &blocks[index];
        match (side, block.kind) {
            (_, DiffBlockKind::Unchanged) => {
                line += block.line_count;
                index += 1;
            }
            (DiffSide::Modified, DiffBlockKind::Removed) => {
                if let Some(next) = blocks
                    .get(index + 1)
                    .filter(|next| next.kind == DiffBlockKind::Added && next.line_count > 0)
                {
                    ranges.push(DiffRange {
                        start_line: line,
                        end_line: line + next.line_count - 1,
                        kind: DiffRangeKind::Modified,
                    });
                    line += next.line_count;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            (DiffSide::Modified, DiffBlockKind::Added) => {
                ranges.push(DiffRange {
                    start_line: line,
                    end_line: line + block.line_count - 1,
                    kind: DiffRangeKind::Added,
                });
                line += block.line_count;
                index += 1;
            }
            (DiffSide::Original, DiffBlockKind::Removed) => {
                if blocks
                    .get(index + 1)
                    .is_some_and(|next| next.kind == DiffBlockKind::Added)
                {
                    ranges.push(DiffRange {
                        start_line: line,
                        end_line: line + block.line_count - 1,
                        kind: DiffRangeKind::Modified,
                    });
                    line += block.line_count;
                    index += 2;
                } else {
                    ranges.push(DiffRange {
                        start_line: line,
                        end_line: line + block.line_count - 1,
                        kind: DiffRangeKind::Deleted,
                    });
                    line += block.line_count;
                    index += 1;
                }
            }
            (DiffSide::Original, DiffBlockKind::Added) => {
                index += 1;
            }
        }
    }

    ranges
}

/// diff block 列から Markdown split row を導出する。
pub fn markdown_split_rows_from_blocks(blocks: &[DiffBlock]) -> Vec<SplitRow> {
    let mut rows = Vec::new();
    let mut index = 0usize;

    while index < blocks.len() {
        let block = &blocks[index];
        match block.kind {
            DiffBlockKind::Unchanged => {
                rows.push(SplitRow {
                    left: block.left.clone(),
                    right: block.right.clone(),
                    kind: SplitRowKind::Unchanged,
                });
                index += 1;
            }
            DiffBlockKind::Removed => {
                if let Some(next) = blocks
                    .get(index + 1)
                    .filter(|next| next.kind == DiffBlockKind::Added)
                {
                    rows.push(SplitRow {
                        left: block.left.clone(),
                        right: next.right.clone(),
                        kind: SplitRowKind::Modified,
                    });
                    index += 2;
                } else {
                    rows.push(SplitRow {
                        left: block.left.clone(),
                        right: None,
                        kind: SplitRowKind::Removed,
                    });
                    index += 1;
                }
            }
            DiffBlockKind::Added => {
                rows.push(SplitRow {
                    left: None,
                    right: block.right.clone(),
                    kind: SplitRowKind::Added,
                });
                index += 1;
            }
        }
    }

    rows
}

/// diff block 列から Markdown inline chunk を導出する。
pub fn markdown_inline_chunks_from_blocks(blocks: &[DiffBlock]) -> Vec<InlineChunk> {
    blocks
        .iter()
        .filter_map(|block| match block.kind {
            DiffBlockKind::Unchanged => block.right.clone().map(|content| InlineChunk {
                content,
                kind: InlineChunkKind::Unchanged,
            }),
            DiffBlockKind::Added => block.right.clone().map(|content| InlineChunk {
                content,
                kind: InlineChunkKind::Added,
            }),
            DiffBlockKind::Removed => block.left.clone().map(|content| InlineChunk {
                content,
                kind: InlineChunkKind::Removed,
            }),
        })
        .collect()
}

#[cfg(test)]
#[path = "markdown_diff_test.rs"]
mod markdown_diff_tests;
