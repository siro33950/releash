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
mod markdown_diff_service_tests {
    use super::*;

    fn hunk(
        index: u32,
        old_start: u32,
        old_lines: u32,
        new_start: u32,
        new_lines: u32,
        lines: &[&str],
    ) -> Hunk {
        Hunk {
            index,
            hunk_id: String::new(),
            old_start,
            old_lines,
            new_start,
            new_lines,
            lines: lines.iter().map(|line| line.to_string()).collect(),
        }
    }

    fn diff_block(
        kind: DiffBlockKind,
        left: Option<&str>,
        right: Option<&str>,
        line_count: u32,
    ) -> DiffBlock {
        DiffBlock {
            kind,
            left: left.map(str::to_string),
            right: right.map(str::to_string),
            line_count,
        }
    }

    #[test]
    fn diff_blockは同一内容をunchangedとして返す() {
        let text = "line1\nline2\nline3\n";
        assert_eq!(
            compute_diff_blocks(&[], text, text),
            vec![DiffBlock {
                kind: DiffBlockKind::Unchanged,
                left: Some(text.to_string()),
                right: Some(text.to_string()),
                line_count: 3,
            }]
        );
    }

    #[test]
    fn diff_blockは追加行をaddedとして返す() {
        let original = "line1\nline2\n";
        let modified = "line1\nline2\nline3\n";
        let hunks = [hunk(0, 2, 1, 2, 2, &[" line2", "+line3"])];

        assert_eq!(
            compute_diff_blocks(&hunks, original, modified),
            vec![
                DiffBlock {
                    kind: DiffBlockKind::Unchanged,
                    left: Some("line1\nline2\n".to_string()),
                    right: Some("line1\nline2\n".to_string()),
                    line_count: 2,
                },
                DiffBlock {
                    kind: DiffBlockKind::Added,
                    left: None,
                    right: Some("line3\n".to_string()),
                    line_count: 1,
                },
            ]
        );
    }

    #[test]
    fn diff_blockは削除行をremovedとして返す() {
        let original = "line1\nremoved\nline3\n";
        let modified = "line1\nline3\n";
        let hunks = [hunk(0, 1, 3, 1, 2, &[" line1", "-removed", " line3"])];

        assert_eq!(
            compute_diff_blocks(&hunks, original, modified),
            vec![
                DiffBlock {
                    kind: DiffBlockKind::Unchanged,
                    left: Some("line1\n".to_string()),
                    right: Some("line1\n".to_string()),
                    line_count: 1,
                },
                DiffBlock {
                    kind: DiffBlockKind::Removed,
                    left: Some("removed\n".to_string()),
                    right: None,
                    line_count: 1,
                },
                DiffBlock {
                    kind: DiffBlockKind::Unchanged,
                    left: Some("line3\n".to_string()),
                    right: Some("line3\n".to_string()),
                    line_count: 1,
                },
            ]
        );
    }

    #[test]
    fn diff_blockは隣接する削除追加を順序保持する() {
        let original = "line1\nold line\nline3\n";
        let modified = "line1\nnew line\nline3\n";
        let hunks = [hunk(
            0,
            1,
            3,
            1,
            3,
            &[" line1", "-old line", "+new line", " line3"],
        )];

        assert_eq!(
            compute_diff_blocks(&hunks, original, modified),
            vec![
                DiffBlock {
                    kind: DiffBlockKind::Unchanged,
                    left: Some("line1\n".to_string()),
                    right: Some("line1\n".to_string()),
                    line_count: 1,
                },
                DiffBlock {
                    kind: DiffBlockKind::Removed,
                    left: Some("old line\n".to_string()),
                    right: None,
                    line_count: 1,
                },
                DiffBlock {
                    kind: DiffBlockKind::Added,
                    left: None,
                    right: Some("new line\n".to_string()),
                    line_count: 1,
                },
                DiffBlock {
                    kind: DiffBlockKind::Unchanged,
                    left: Some("line3\n".to_string()),
                    right: Some("line3\n".to_string()),
                    line_count: 1,
                },
            ]
        );
    }

    #[test]
    fn diff_blockは複数行の追加削除と空入力を扱う() {
        let added_hunk = [hunk(0, 1, 0, 1, 2, &["+line1", "+line2"])];
        assert_eq!(
            compute_diff_blocks(&added_hunk, "", "line1\nline2\n"),
            vec![DiffBlock {
                kind: DiffBlockKind::Added,
                left: None,
                right: Some("line1\nline2\n".to_string()),
                line_count: 2,
            }]
        );

        let removed_hunk = [hunk(0, 1, 2, 1, 0, &["-line1", "-line2"])];
        assert_eq!(
            compute_diff_blocks(&removed_hunk, "line1\nline2\n", ""),
            vec![DiffBlock {
                kind: DiffBlockKind::Removed,
                left: Some("line1\nline2\n".to_string()),
                right: None,
                line_count: 2,
            }]
        );
    }

    #[test]
    fn diff_blockは境界条件の空行を保持する() {
        let original = "a\nb\n\nc\nd\n";
        let modified = "a\nB\nc\nd\n\n";
        let hunks = [hunk(
            0,
            1,
            5,
            1,
            5,
            &[" a", "-b", "-", "+B", " c", " d", "+"],
        )];

        let blocks = compute_diff_blocks(&hunks, original, modified);
        assert_eq!(blocks[1].left.as_deref(), Some("b\n\n"));
        assert_eq!(blocks[2].right.as_deref(), Some("B\n"));
        assert_eq!(blocks[4].right.as_deref(), Some("\n"));
    }

    #[test]
    fn diff_blockはmarkdown_source_line_mapping用の材料を返す() {
        let original = "# Title\n\nold paragraph\n\n- keep\n";
        let modified = "# Title\n\nnew paragraph\n\n- keep\n- added\n";
        let hunks = [hunk(
            0,
            1,
            5,
            1,
            6,
            &[
                " # Title",
                " ",
                "-old paragraph",
                "+new paragraph",
                " ",
                " - keep",
                "+- added",
            ],
        )];

        let blocks = compute_diff_blocks(&hunks, original, modified);
        assert_eq!(blocks[0].line_count, 2);
        assert_eq!(blocks[1].left.as_deref(), Some("old paragraph\n"));
        assert_eq!(blocks[2].right.as_deref(), Some("new paragraph\n"));
        assert_eq!(blocks[4].right.as_deref(), Some("- added\n"));
    }

    #[test]
    fn diff_rangeはblockからmodified_addedを導出する() {
        let blocks = vec![
            diff_block(
                DiffBlockKind::Unchanged,
                Some("line1\n"),
                Some("line1\n"),
                1,
            ),
            diff_block(DiffBlockKind::Removed, Some("old\n"), None, 1),
            diff_block(DiffBlockKind::Added, None, Some("new\n"), 1),
            diff_block(DiffBlockKind::Added, None, Some("added\n"), 1),
        ];

        assert_eq!(
            markdown_diff_ranges_from_blocks(&blocks, DiffSide::Modified),
            vec![
                DiffRange {
                    start_line: 2,
                    end_line: 2,
                    kind: DiffRangeKind::Modified,
                },
                DiffRange {
                    start_line: 3,
                    end_line: 3,
                    kind: DiffRangeKind::Added,
                },
            ]
        );
        assert_eq!(
            markdown_diff_ranges_from_blocks(&blocks, DiffSide::Original),
            vec![DiffRange {
                start_line: 2,
                end_line: 2,
                kind: DiffRangeKind::Modified,
            }]
        );
    }

    #[test]
    fn diff_rangeはoriginal側の単独removedをdeletedとして導出する() {
        let blocks = vec![
            diff_block(
                DiffBlockKind::Unchanged,
                Some("line1\n"),
                Some("line1\n"),
                1,
            ),
            diff_block(DiffBlockKind::Removed, Some("old1\nold2\n"), None, 2),
            diff_block(
                DiffBlockKind::Unchanged,
                Some("line4\n"),
                Some("line2\n"),
                1,
            ),
        ];

        assert_eq!(
            markdown_diff_ranges_from_blocks(&blocks, DiffSide::Original),
            vec![DiffRange {
                start_line: 2,
                end_line: 3,
                kind: DiffRangeKind::Deleted,
            }]
        );
        assert_eq!(
            markdown_diff_ranges_from_blocks(&blocks, DiffSide::Modified),
            Vec::new()
        );
    }

    #[test]
    fn split_rowはblockから導出する() {
        let blocks = vec![
            diff_block(DiffBlockKind::Unchanged, Some("same\n"), Some("same\n"), 1),
            diff_block(DiffBlockKind::Removed, Some("old\n"), None, 1),
            diff_block(DiffBlockKind::Added, None, Some("new\n"), 1),
            diff_block(DiffBlockKind::Removed, Some("removed\n"), None, 1),
        ];

        assert_eq!(
            markdown_split_rows_from_blocks(&blocks),
            vec![
                SplitRow {
                    left: Some("same\n".to_string()),
                    right: Some("same\n".to_string()),
                    kind: SplitRowKind::Unchanged,
                },
                SplitRow {
                    left: Some("old\n".to_string()),
                    right: Some("new\n".to_string()),
                    kind: SplitRowKind::Modified,
                },
                SplitRow {
                    left: Some("removed\n".to_string()),
                    right: None,
                    kind: SplitRowKind::Removed,
                },
            ]
        );
    }

    #[test]
    fn split_rowは単独addedを右側だけのaddedとして導出する() {
        let blocks = vec![diff_block(DiffBlockKind::Added, None, Some("new\n"), 1)];

        assert_eq!(
            markdown_split_rows_from_blocks(&blocks),
            vec![SplitRow {
                left: None,
                right: Some("new\n".to_string()),
                kind: SplitRowKind::Added,
            }]
        );
    }

    #[test]
    fn inline_chunkはblockから導出する() {
        let blocks = vec![
            diff_block(DiffBlockKind::Unchanged, Some("same\n"), Some("same\n"), 1),
            diff_block(DiffBlockKind::Removed, Some("old\n"), None, 1),
            diff_block(DiffBlockKind::Added, None, Some("new\n"), 1),
        ];

        assert_eq!(
            markdown_inline_chunks_from_blocks(&blocks),
            vec![
                InlineChunk {
                    content: "same\n".to_string(),
                    kind: InlineChunkKind::Unchanged,
                },
                InlineChunk {
                    content: "old\n".to_string(),
                    kind: InlineChunkKind::Removed,
                },
                InlineChunk {
                    content: "new\n".to_string(),
                    kind: InlineChunkKind::Added,
                },
            ]
        );
    }

    #[test]
    fn read_model導出はunchangedのみの入力を差分なしとして扱う() {
        let blocks = vec![diff_block(
            DiffBlockKind::Unchanged,
            Some("same\n"),
            Some("same\n"),
            1,
        )];

        assert_eq!(
            markdown_diff_ranges_from_blocks(&blocks, DiffSide::Modified),
            Vec::new()
        );
        assert_eq!(
            markdown_diff_ranges_from_blocks(&blocks, DiffSide::Original),
            Vec::new()
        );
        assert_eq!(
            markdown_split_rows_from_blocks(&blocks),
            vec![SplitRow {
                left: Some("same\n".to_string()),
                right: Some("same\n".to_string()),
                kind: SplitRowKind::Unchanged,
            }]
        );
        assert_eq!(
            markdown_inline_chunks_from_blocks(&blocks),
            vec![InlineChunk {
                content: "same\n".to_string(),
                kind: InlineChunkKind::Unchanged,
            }]
        );
    }

    #[test]
    fn diff_rangeは末尾空行追加をmodified側addedとして導出する() {
        let original = "a\n";
        let modified = "a\n\n";
        let hunks = [hunk(0, 1, 1, 1, 2, &[" a", "+"])];
        let blocks = compute_diff_blocks(&hunks, original, modified);

        assert_eq!(
            markdown_diff_ranges_from_blocks(&blocks, DiffSide::Modified),
            vec![DiffRange {
                start_line: 2,
                end_line: 2,
                kind: DiffRangeKind::Added,
            }]
        );
    }

    #[test]
    fn diff_rangeは中間空行削除をoriginal側deletedとして導出する() {
        let original = "a\n\nb\n";
        let modified = "a\nb\n";
        let hunks = [hunk(0, 1, 3, 1, 2, &[" a", "-", " b"])];
        let blocks = compute_diff_blocks(&hunks, original, modified);

        assert_eq!(
            markdown_diff_ranges_from_blocks(&blocks, DiffSide::Original),
            vec![DiffRange {
                start_line: 2,
                end_line: 2,
                kind: DiffRangeKind::Deleted,
            }]
        );
    }

    #[test]
    fn diff_rangeは複数独立変更の後続行番号を保持する() {
        let blocks = vec![
            diff_block(DiffBlockKind::Unchanged, Some("a\n"), Some("a\n"), 1),
            diff_block(DiffBlockKind::Removed, Some("old1\n"), None, 1),
            diff_block(DiffBlockKind::Added, None, Some("new1\nextra\n"), 2),
            diff_block(DiffBlockKind::Unchanged, Some("b\n"), Some("b\n"), 1),
            diff_block(DiffBlockKind::Removed, Some("old2\n"), None, 1),
            diff_block(DiffBlockKind::Added, None, Some("new2\n"), 1),
            diff_block(DiffBlockKind::Unchanged, Some("c\n"), Some("c\n"), 1),
        ];

        assert_eq!(
            markdown_diff_ranges_from_blocks(&blocks, DiffSide::Modified),
            vec![
                DiffRange {
                    start_line: 2,
                    end_line: 3,
                    kind: DiffRangeKind::Modified,
                },
                DiffRange {
                    start_line: 5,
                    end_line: 5,
                    kind: DiffRangeKind::Modified,
                },
            ]
        );
        assert_eq!(
            markdown_diff_ranges_from_blocks(&blocks, DiffSide::Original),
            vec![
                DiffRange {
                    start_line: 2,
                    end_line: 2,
                    kind: DiffRangeKind::Modified,
                },
                DiffRange {
                    start_line: 4,
                    end_line: 4,
                    kind: DiffRangeKind::Modified,
                },
            ]
        );
    }
}
