pub(crate) mod hunk_service_tests {
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
            lines: lines.iter().map(|s| s.to_string()).collect(),
        }
    }

    // ── change groups（hardcoded hunk 入力） ──

    #[test]
    fn test_change_group算出_単一変更() {
        let h = hunk(0, 1, 2, 1, 3, &[" line1", " line2", "+line3"]);
        let groups = compute_change_groups(&[h]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group_index, 0);
        assert_eq!(groups[0].hunk_index, 0);
        assert!(groups[0].group_id.starts_with("g:"));
    }

    #[test]
    fn test_change_group算出_複数hunk() {
        let h0 = hunk(0, 1, 1, 1, 1, &["-a", "+A"]);
        let h1 = hunk(1, 5, 1, 5, 1, &["-b", "+B"]);
        let groups = compute_change_groups(&[h0, h1]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].group_index, 0);
        assert_eq!(groups[1].group_index, 1);
    }

    #[test]
    fn test_hunk_idは同一内容なら位置に依存しない() {
        let h0 = hunk(0, 1, 1, 1, 1, &["-a", "+A"]);
        let h1 = hunk(0, 20, 1, 30, 1, &["-a", "+A"]);

        let first = assign_hunk_ids(&[h0]);
        let second = assign_hunk_ids(&[h1]);

        assert_eq!(first[0].hunk_id, second[0].hunk_id);
    }

    #[test]
    fn test_group_idは同一内容なら位置に依存しない() {
        let h0 = hunk(0, 1, 1, 1, 1, &["-a", "+A"]);
        let h1 = hunk(0, 20, 1, 30, 1, &["-a", "+A"]);

        let first = compute_change_groups(&[h0]);
        let second = compute_change_groups(&[h1]);

        assert_eq!(first[0].group_id, second[0].group_id);
    }

    #[test]
    fn test_group_idは異なる内容なら異なる() {
        let h0 = hunk(0, 1, 1, 1, 1, &["-a", "+A"]);
        let h1 = hunk(0, 1, 1, 1, 1, &["-b", "+B"]);

        let first = compute_change_groups(&[h0]);
        let second = compute_change_groups(&[h1]);

        assert_ne!(first[0].group_id, second[0].group_id);
    }

    #[test]
    fn test_hunk_idは同一内容の複数出現をordinalで区別する() {
        let h0 = hunk(0, 1, 1, 1, 1, &["-a", "+A"]);
        let h1 = hunk(1, 10, 1, 10, 1, &["-a", "+A"]);

        let hunks = assign_hunk_ids(&[h0, h1]);

        assert_ne!(hunks[0].hunk_id, hunks[1].hunk_id);
        assert!(hunks[0].hunk_id.ends_with(":0"));
        assert!(hunks[1].hunk_id.ends_with(":1"));
    }

    #[test]
    fn test_stable_side_hunk_idは前方の同一hunkが消えても後続hunkで変わらない() {
        let original = "x\na\ny\nmid\nx\na\ny\n";
        let working = "x\nA\ny\nmid\nx\nA\ny\n";
        let staged_after_first = "x\nA\ny\nmid\nx\na\ny\n";
        let first = hunk(0, 1, 3, 1, 3, &[" x", "-a", "+A", " y"]);
        let second = hunk(1, 5, 3, 5, 3, &[" x", "-a", "+A", " y"]);
        let refreshed_second = hunk(0, 5, 3, 5, 3, &[" x", "-a", "+A", " y"]);

        let initial_hunks = assign_stable_hunk_ids_for_side(
            &[first, second],
            original,
            working,
            StableGroupIdSide::Modified,
        );
        let refreshed_hunks = assign_stable_hunk_ids_for_side(
            &[refreshed_second],
            staged_after_first,
            working,
            StableGroupIdSide::Modified,
        );

        assert_ne!(initial_hunks[0].hunk_id, initial_hunks[1].hunk_id);
        assert_eq!(initial_hunks[1].hunk_id, refreshed_hunks[0].hunk_id);
    }

    #[test]
    fn test_original_side_hunk_idはunstageで前方同一hunkが消えても後続hunkで変わらない() {
        let head = "x\na\ny\nmid\nx\na\ny\n";
        let staged = "x\nA\ny\nmid\nx\nA\ny\n";
        let staged_after_first_unstage = "x\na\ny\nmid\nx\nA\ny\n";
        let first = hunk(0, 1, 3, 1, 3, &[" x", "-a", "+A", " y"]);
        let second = hunk(1, 5, 3, 5, 3, &[" x", "-a", "+A", " y"]);
        let refreshed_second = hunk(0, 5, 3, 5, 3, &[" x", "-a", "+A", " y"]);

        let initial_hunks = assign_stable_hunk_ids_for_side(
            &[first, second],
            head,
            staged,
            StableGroupIdSide::Original,
        );
        let refreshed_hunks = assign_stable_hunk_ids_for_side(
            &[refreshed_second],
            head,
            staged_after_first_unstage,
            StableGroupIdSide::Original,
        );

        assert_ne!(initial_hunks[0].hunk_id, initial_hunks[1].hunk_id);
        assert_eq!(initial_hunks[1].hunk_id, refreshed_hunks[0].hunk_id);
    }

    #[test]
    fn test_group_idは隣接contextで同一内容の複数出現を区別する() {
        let h = hunk(
            0,
            1,
            7,
            1,
            7,
            &[" alpha", "-a", "+A", " beta", "-a", "+A", " gamma"],
        );

        let groups = compute_change_groups(&[h]);

        assert_eq!(groups.len(), 2);
        assert_ne!(groups[0].group_id, groups[1].group_id);
    }

    #[test]
    fn test_group_idは同一hunk内の同じ局所patternをordinalで区別する() {
        let h = hunk(
            0,
            1,
            8,
            1,
            8,
            &[" x", "-a", "+A", " y", " x", "-a", "+A", " y"],
        );

        let groups = compute_change_groups(&[h]);

        assert_eq!(groups.len(), 2);
        assert_ne!(groups[0].group_id, groups[1].group_id);
        assert!(groups[0].group_id.ends_with(":0"));
        assert!(groups[1].group_id.ends_with(":1"));
    }

    #[test]
    fn test_group_idは前方の同一内容groupが消えても後続groupで変わらない() {
        let initial = hunk(
            0,
            1,
            7,
            1,
            7,
            &[" alpha", "-a", "+A", " beta", "-a", "+A", " gamma"],
        );
        let refreshed = hunk(
            0,
            1,
            6,
            1,
            6,
            &[" alpha", " A", " beta", "-a", "+A", " gamma"],
        );

        let initial_groups = compute_change_groups(&[initial]);
        let refreshed_groups = compute_change_groups(&[refreshed]);

        assert_eq!(initial_groups.len(), 2);
        assert_eq!(refreshed_groups.len(), 1);
        assert_eq!(initial_groups[1].group_id, refreshed_groups[0].group_id);
    }

    #[test]
    fn test_stable_side_group_idは前方の同一局所patternが消えても後続groupで変わらない() {
        let original = "x\na\ny\nx\na\ny\n";
        let working = "x\nA\ny\nx\nA\ny\n";
        let staged_after_first = "x\nA\ny\nx\na\ny\n";
        let initial = hunk(
            0,
            1,
            6,
            1,
            6,
            &[" x", "-a", "+A", " y", " x", "-a", "+A", " y"],
        );
        let refreshed = hunk(0, 1, 6, 1, 6, &[" x", " A", " y", " x", "-a", "+A", " y"]);

        let initial_groups = compute_change_groups(std::slice::from_ref(&initial));
        let refreshed_groups = compute_change_groups(std::slice::from_ref(&refreshed));
        let initial_groups = assign_stable_group_ids_for_side(
            &[initial],
            &initial_groups,
            original,
            working,
            StableGroupIdSide::Modified,
        );
        let refreshed_groups = assign_stable_group_ids_for_side(
            &[refreshed],
            &refreshed_groups,
            staged_after_first,
            working,
            StableGroupIdSide::Modified,
        );

        assert_eq!(initial_groups.len(), 2);
        assert_eq!(refreshed_groups.len(), 1);
        assert_eq!(initial_groups[1].group_id, refreshed_groups[0].group_id);
    }

    #[test]
    fn test_original_side_group_idはunstageで前方同一patternが消えても後続groupで変わらない() {
        let head = "x\na\ny\nx\na\ny\n";
        let staged = "x\nA\ny\nx\nA\ny\n";
        let staged_after_first_unstage = "x\na\ny\nx\nA\ny\n";
        let initial = hunk(
            0,
            1,
            6,
            1,
            6,
            &[" x", "-a", "+A", " y", " x", "-a", "+A", " y"],
        );
        let refreshed = hunk(0, 1, 6, 1, 6, &[" x", " a", " y", " x", "-a", "+A", " y"]);

        let initial_groups = compute_change_groups(std::slice::from_ref(&initial));
        let refreshed_groups = compute_change_groups(std::slice::from_ref(&refreshed));
        let initial_groups = assign_stable_group_ids_for_side(
            &[initial],
            &initial_groups,
            head,
            staged,
            StableGroupIdSide::Original,
        );
        let refreshed_groups = assign_stable_group_ids_for_side(
            &[refreshed],
            &refreshed_groups,
            head,
            staged_after_first_unstage,
            StableGroupIdSide::Original,
        );

        assert_eq!(initial_groups.len(), 2);
        assert_eq!(refreshed_groups.len(), 1);
        assert_eq!(initial_groups[1].group_id, refreshed_groups[0].group_id);
    }

    // ── generate_group_patch ──

    #[test]
    fn test_group_patch生成_単一変更() {
        let h = hunk(
            0,
            1,
            3,
            1,
            3,
            &[" line1", "-original", "+modified", " line3"],
        );
        let group = ChangeGroup {
            group_index: 0,
            group_id: "g:test:0".to_string(),
            hunk_index: 0,
            new_start: 2,
            new_end: 2,
            line_offset_start: 1,
            line_offset_end: 2,
            is_staged: None,
        };

        let patch = generate_group_patch("src/file.ts", &h, &group);
        assert!(patch.contains("--- a/src/file.ts"));
        assert!(patch.contains("+++ b/src/file.ts"));
        assert!(patch.contains("-original"));
        assert!(patch.contains("+modified"));
        assert!(patch.ends_with('\n'));
    }

    // ── compute_hidden_ranges ──

    #[test]
    fn test_非表示範囲_hunk無し() {
        let result = compute_hidden_ranges(&[], 100, 3);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start_line, 1);
        assert_eq!(result[0].end_line, 100);
        assert_eq!(result[0].hidden_count, 100);
    }

    #[test]
    fn test_非表示範囲_総行数ゼロ() {
        let result = compute_hidden_ranges(&[], 0, 3);
        assert!(result.is_empty());
    }

    #[test]
    fn test_非表示範囲_先頭hunk() {
        let hunks = vec![hunk(0, 1, 2, 1, 3, &["-old", "+new1", "+new2", " ctx"])];
        let result = compute_hidden_ranges(&hunks, 20, 3);
        // Hunk covers lines 1-3, context extends to 6 → Hidden: 7-20
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start_line, 7);
        assert_eq!(result[0].end_line, 20);
    }

    #[test]
    fn test_非表示範囲_中間hunk() {
        let hunks = vec![hunk(0, 10, 2, 10, 2, &["-old", "+new"])];
        let result = compute_hidden_ranges(&hunks, 20, 3);
        // Hunk covers lines 10-11, context extends to 7-14 → Hidden: 1-6, 15-20
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].start_line, 1);
        assert_eq!(result[0].end_line, 6);
        assert_eq!(result[1].start_line, 15);
        assert_eq!(result[1].end_line, 20);
    }

    #[test]
    fn test_非表示範囲_隣接hunkがマージ() {
        let hunks = vec![
            hunk(0, 5, 1, 5, 1, &["-a", "+b"]),
            hunk(1, 8, 1, 8, 1, &["-c", "+d"]),
        ];
        // context=3: hunk0 visible 2-8, hunk1 visible 5-11 → merged 2-11
        let result = compute_hidden_ranges(&hunks, 20, 3);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].start_line, 1);
        assert_eq!(result[0].end_line, 1);
        assert_eq!(result[1].start_line, 12);
        assert_eq!(result[1].end_line, 20);
    }

    #[test]
    fn test_非表示範囲_全行変更() {
        let hunks = vec![hunk(
            0,
            1,
            5,
            1,
            5,
            &["-a", "-b", "-c", "-d", "-e", "+A", "+B", "+C", "+D", "+E"],
        )];
        let result = compute_hidden_ranges(&hunks, 5, 3);
        assert!(result.is_empty());
    }

    // ── compute_visible_ranges（private helper） ──

    #[test]
    fn test_可視範囲_hunk無し() {
        let result = compute_visible_ranges(&[], 100, 3);
        assert!(result.is_empty());
    }

    #[test]
    fn test_可視範囲_単一hunk() {
        let hunks = vec![hunk(0, 10, 2, 10, 2, &["-old", "+new"])];
        let result = compute_visible_ranges(&hunks, 20, 3);
        assert_eq!(result, vec![(7, 14)]);
    }

    #[test]
    fn test_可視範囲_削除のみhunk() {
        let hunks = vec![hunk(0, 5, 3, 5, 0, &["-a", "-b", "-c"])];
        let result = compute_visible_ranges(&hunks, 20, 3);
        assert_eq!(result, vec![(2, 8)]);
    }

    #[test]
    fn test_可視範囲_重複マージ() {
        let hunks = vec![
            hunk(0, 5, 1, 5, 1, &["-a", "+b"]),
            hunk(1, 8, 1, 8, 1, &["-c", "+d"]),
        ];
        let result = compute_visible_ranges(&hunks, 20, 3);
        assert_eq!(result, vec![(2, 11)]);
    }

    // ── compute_visible_markdown_blocks（hardcoded hunk 入力） ──

    #[test]
    fn test_可視ブロック_hunk無しは空() {
        let result = compute_visible_markdown_blocks(&[], "a\nb\n", "a\nb\n", 3);
        assert!(result.is_empty());
    }

    #[test]
    fn test_可視ブロック_単一変更() {
        let original = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let modified = "a\nb\nc\nd\nE\nf\ng\nh\ni\nj\n";
        // line 5 (e→E) の変更
        let hunks = vec![hunk(0, 5, 1, 5, 1, &["-e", "+E"])];
        let result = compute_visible_markdown_blocks(&hunks, original, modified, 2);
        assert_eq!(result.len(), 1);
        assert!(result[0].content.contains('E'));
        assert!(result[0].start_line <= 5);
        assert!(result[0].end_line >= 5);
        assert_eq!(result[0].deleted_content.as_deref(), Some("e"));
    }
}
