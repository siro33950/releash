pub(crate) mod markdown_diff_service_tests {
    use super::super::*;

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
