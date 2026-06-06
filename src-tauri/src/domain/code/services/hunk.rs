//! hunk の区切り（change group）・patch 生成・range 算出の純粋ロジック。
//!
//! diff バッファの計算（git2 依存）は `DiffComputer`（gateway）が担い、本サービスは
//! 生成済みの `Hunk` を入力に純粋計算を行う。

use crate::domain::code::value_objects::{ChangeGroup, HiddenRange, Hunk, VisibleBlock};

/// hunk を連続する change group に分割する。
///
/// change group は hunk 内の連続した `+` / `-` 行のブロック。context 行（` `）が
/// group を区切る。
fn split_hunk_into_groups(hunk: &Hunk, start_group_index: u32) -> Vec<ChangeGroup> {
    let mut groups: Vec<ChangeGroup> = Vec::new();
    let mut modified_line = hunk.new_start;
    let mut in_group = false;
    let mut group_start_offset: u32 = 0;
    let mut group_new_start: u32 = 0;
    let mut last_plus_line: u32 = 0;
    let mut has_plus = false;

    for (i, line) in hunk.lines.iter().enumerate() {
        let prefix = line.as_bytes().first().copied().unwrap_or(b' ');

        if prefix == b'\\' {
            continue;
        }

        if prefix == b'+' || prefix == b'-' {
            if !in_group {
                in_group = true;
                group_start_offset = i as u32;
                group_new_start = modified_line;
                last_plus_line = 0;
                has_plus = false;
            }
            if prefix == b'+' {
                last_plus_line = modified_line;
                has_plus = true;
                modified_line += 1;
            }
        } else {
            if in_group {
                let new_end = if has_plus {
                    last_plus_line
                } else {
                    group_new_start.saturating_sub(1).max(1)
                };
                groups.push(ChangeGroup {
                    group_index: start_group_index + groups.len() as u32,
                    hunk_index: hunk.index,
                    new_start: if has_plus {
                        group_new_start
                    } else {
                        group_new_start.saturating_sub(1).max(1)
                    },
                    new_end,
                    line_offset_start: group_start_offset,
                    line_offset_end: i as u32 - 1,
                    is_staged: None,
                });
                in_group = false;
            }
            modified_line += 1;
        }
    }

    if in_group {
        let new_end = if has_plus {
            last_plus_line
        } else {
            group_new_start.saturating_sub(1).max(1)
        };
        groups.push(ChangeGroup {
            group_index: start_group_index + groups.len() as u32,
            hunk_index: hunk.index,
            new_start: if has_plus {
                group_new_start
            } else {
                group_new_start.saturating_sub(1).max(1)
            },
            new_end,
            line_offset_start: group_start_offset,
            line_offset_end: hunk.lines.len() as u32 - 1,
            is_staged: None,
        });
    }

    groups
}

/// hunk 群から change group 群を算出する。
pub fn compute_change_groups(hunks: &[Hunk]) -> Vec<ChangeGroup> {
    let mut groups: Vec<ChangeGroup> = Vec::new();
    for hunk in hunks {
        groups.extend(split_hunk_into_groups(hunk, groups.len() as u32));
    }
    groups
}

/// change group に属する行を hunk から取り出す。
#[allow(dead_code)]
fn extract_group_lines(group: &ChangeGroup, hunks: &[Hunk]) -> Vec<String> {
    let Some(hunk) = hunks.iter().find(|h| h.index == group.hunk_index) else {
        return Vec::new();
    };
    let start = group.line_offset_start as usize;
    let end = (group.line_offset_end as usize + 1).min(hunk.lines.len());
    hunk.lines[start..end].to_vec()
}

/// change group の開始に対応する旧ファイルの行位置を得る。
#[allow(dead_code)]
fn get_group_old_position(group: &ChangeGroup, hunks: &[Hunk]) -> i64 {
    let Some(hunk) = hunks.iter().find(|h| h.index == group.hunk_index) else {
        return -1;
    };
    let mut old_line = hunk.old_start as i64;
    for i in 0..group.line_offset_start as usize {
        if let Some(line) = hunk.lines.get(i) {
            let prefix = line.as_bytes().first().copied().unwrap_or(b' ');
            if prefix == b'-' || prefix == b' ' {
                old_line += 1;
            }
        }
    }
    old_line
}

/// staged 側の diff group と比較して change group の staged 状態を付与する。
#[allow(dead_code)]
pub fn mark_staged_groups(
    groups: &[ChangeGroup],
    staged_groups: &[ChangeGroup],
    hunks: &[Hunk],
    staged_hunks: &[Hunk],
) -> Vec<ChangeGroup> {
    let mut staged_keys = std::collections::HashSet::new();
    for sg in staged_groups {
        let lines = extract_group_lines(sg, staged_hunks);
        let pos = get_group_old_position(sg, staged_hunks);
        staged_keys.insert(format!("{pos}:{}", lines.join("\n")));
    }

    groups
        .iter()
        .map(|g| {
            let lines = extract_group_lines(g, hunks);
            let pos = get_group_old_position(g, hunks);
            let key = format!("{pos}:{}", lines.join("\n"));
            ChangeGroup {
                is_staged: Some(staged_keys.contains(&key)),
                ..g.clone()
            }
        })
        .collect()
}

/// hunk 内の単一 change group に対する unified-diff patch を生成する。
///
/// group 範囲外の行は変換される:
/// - `+` 行は破棄（この group の一部ではない）
/// - `-` 行は context（` `）行に変換
pub fn generate_group_patch(file_path: &str, hunk: &Hunk, group: &ChangeGroup) -> String {
    let mut result_lines: Vec<String> = Vec::new();

    for (i, line) in hunk.lines.iter().enumerate() {
        let prefix = line.as_bytes().first().copied().unwrap_or(b' ');
        let i = i as u32;

        if i >= group.line_offset_start && i <= group.line_offset_end {
            result_lines.push(line.clone());
        } else if prefix == b'-' {
            // Convert non-group deletion to context
            result_lines.push(format!(" {}", &line[1..]));
        } else if prefix == b'+' {
            // Drop non-group additions
        } else {
            result_lines.push(line.clone());
        }
    }

    let mut old_lines: u32 = 0;
    let mut new_lines: u32 = 0;
    for line in &result_lines {
        let p = line.as_bytes().first().copied().unwrap_or(b' ');
        match p {
            b' ' => {
                old_lines += 1;
                new_lines += 1;
            }
            b'-' => old_lines += 1,
            b'+' => new_lines += 1,
            _ => {}
        }
    }

    let mut output = Vec::new();
    output.push(format!("--- a/{file_path}"));
    output.push(format!("+++ b/{file_path}"));
    output.push(format!(
        "@@ -{},{old_lines} +{},{new_lines} @@",
        hunk.old_start, hunk.old_start
    ));
    output.extend(result_lines);

    format!("{}\n", output.join("\n"))
}

/// 選択した hunk 群から unified-diff patch を生成する。
#[allow(dead_code)]
pub fn generate_patch(file_path: &str, hunks: &[Hunk], selected_indices: &[u32]) -> String {
    let selected: Vec<&Hunk> = hunks
        .iter()
        .filter(|h| selected_indices.contains(&h.index))
        .collect();
    if selected.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    lines.push(format!("--- a/{file_path}"));
    lines.push(format!("+++ b/{file_path}"));

    for hunk in selected {
        lines.push(format!(
            "@@ -{},{} +{},{} @@",
            hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
        ));
        lines.extend(hunk.lines.iter().cloned());
    }

    format!("{}\n", lines.join("\n"))
}

/// 各 hunk に context 行を付与して可視範囲を算出し、隣接範囲をマージする。
fn compute_visible_ranges(hunks: &[Hunk], total_lines: u32, context_lines: u32) -> Vec<(u32, u32)> {
    let mut visible_ranges: Vec<(u32, u32)> = Vec::new();
    for hunk in hunks {
        let hunk_start = hunk.new_start;
        let hunk_end = hunk.new_start + hunk.new_lines.max(1) - 1;

        let visible_start = if hunk_start > context_lines {
            hunk_start - context_lines
        } else {
            1
        };
        let visible_end = (hunk_end + context_lines).min(total_lines);
        visible_ranges.push((visible_start, visible_end));
    }

    visible_ranges.sort_by_key(|r| r.0);
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for range in &visible_ranges {
        if let Some(last) = merged.last_mut() {
            if range.0 <= last.1 + 1 {
                last.1 = last.1.max(range.1);
            } else {
                merged.push(*range);
            }
        } else {
            merged.push(*range);
        }
    }
    merged
}

/// 変更を含まない（折り畳む）行範囲を算出する。各 hunk は `context_lines` の
/// context で囲まれる。
pub fn compute_hidden_ranges(
    hunks: &[Hunk],
    total_lines: u32,
    context_lines: u32,
) -> Vec<HiddenRange> {
    if total_lines == 0 {
        return Vec::new();
    }

    let merged = compute_visible_ranges(hunks, total_lines, context_lines);

    // Compute hidden ranges (gaps between visible ranges)
    let mut hidden: Vec<HiddenRange> = Vec::new();
    let mut current = 1u32;

    for (vis_start, vis_end) in &merged {
        if current < *vis_start {
            let count = vis_start - current;
            hidden.push(HiddenRange {
                start_line: current,
                end_line: vis_start - 1,
                hidden_count: count,
            });
        }
        current = vis_end + 1;
    }

    // Trailing hidden range after last visible block
    if current <= total_lines {
        let count = total_lines - current + 1;
        hidden.push(HiddenRange {
            start_line: current,
            end_line: total_lines,
            hidden_count: count,
        });
    }

    hidden
}

/// 生成済み hunk から Markdown diff-only 表示の可視ブロックを算出する。
///
/// `original` / `modified` の行も参照して削除内容（`deleted_content`）を併記する。
pub fn compute_visible_markdown_blocks(
    hunks: &[Hunk],
    original: &str,
    modified: &str,
    context_lines: u32,
) -> Vec<VisibleBlock> {
    let mod_lines: Vec<&str> = modified.lines().collect();
    let orig_lines: Vec<&str> = original.lines().collect();
    let total_lines = mod_lines.len() as u32;

    if hunks.is_empty() {
        return Vec::new();
    }

    let merged = compute_visible_ranges(hunks, total_lines, context_lines);

    // Build visible blocks from merged ranges, including deleted content
    merged
        .iter()
        .map(|(start, end)| {
            let s = (*start as usize).saturating_sub(1);
            let e = (*end as usize).min(mod_lines.len());
            let content = mod_lines[s..e].join("\n");

            // Collect deleted lines from hunks that overlap this visible range
            let mut deleted: Vec<&str> = Vec::new();
            for hunk in hunks {
                let hunk_mod_start = hunk.new_start;
                let hunk_mod_end = hunk.new_start + hunk.new_lines.max(1) - 1;
                // Check if hunk overlaps with this visible range
                if hunk_mod_end >= *start && hunk_mod_start <= *end {
                    // Extract deleted lines (lines starting with '-') from original
                    let mut orig_line = hunk.old_start as usize;
                    for line in &hunk.lines {
                        let prefix = line.as_bytes().first().copied().unwrap_or(b' ');
                        if prefix == b'-' {
                            if orig_line >= 1 && orig_line <= orig_lines.len() {
                                deleted.push(orig_lines[orig_line - 1]);
                            }
                            orig_line += 1;
                        } else if prefix == b' ' {
                            orig_line += 1;
                        }
                    }
                }
            }

            let deleted_content = if deleted.is_empty() {
                None
            } else {
                Some(deleted.join("\n"))
            };

            VisibleBlock {
                start_line: *start,
                end_line: *end,
                content,
                deleted_content,
            }
        })
        .collect()
}

#[cfg(test)]
mod hunk_service_tests {
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

    // ── mark_staged_groups（hardcoded hunk 入力） ──

    #[test]
    fn test_staged判定_キー一致でstaged() {
        let h = hunk(0, 1, 3, 1, 3, &[" a", "-b", "+B", " c"]);
        let hunks = std::slice::from_ref(&h);
        let groups = compute_change_groups(hunks);
        // staged 側に同一 group が存在 → is_staged: true
        let result = mark_staged_groups(&groups, &groups, hunks, hunks);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].is_staged, Some(true));
    }

    #[test]
    fn test_staged判定_staged無しでfalse() {
        let h = hunk(0, 1, 3, 1, 3, &[" a", "-b", "+B", " c"]);
        let hunks = std::slice::from_ref(&h);
        let groups = compute_change_groups(hunks);
        let result = mark_staged_groups(&groups, &[], hunks, &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].is_staged, Some(false));
    }

    #[test]
    fn test_staged判定_空入力は空() {
        let result = mark_staged_groups(&[], &[], &[], &[]);
        assert!(result.is_empty());
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

    // ── generate_patch ──

    #[test]
    fn test_patch生成_単一hunk選択() {
        let hunks = vec![
            hunk(
                0,
                1,
                3,
                1,
                3,
                &[" line1", "-original", "+modified", " line3"],
            ),
            hunk(
                1,
                8,
                3,
                8,
                4,
                &[" line8", "-old9", "+new9", "+new10", " line11"],
            ),
        ];

        let patch = generate_patch("src/file.ts", &hunks, &[0]);
        assert!(patch.contains("--- a/src/file.ts"));
        assert!(patch.contains("+++ b/src/file.ts"));
        assert!(patch.contains("@@ -1,3 +1,3 @@"));
        assert!(patch.contains("+modified"));
        assert!(!patch.contains("+new9"));
    }

    #[test]
    fn test_patch生成_複数hunk選択() {
        let hunks = vec![
            hunk(
                0,
                1,
                3,
                1,
                3,
                &[" line1", "-original", "+modified", " line3"],
            ),
            hunk(
                1,
                8,
                3,
                8,
                4,
                &[" line8", "-old9", "+new9", "+new10", " line11"],
            ),
        ];

        let patch = generate_patch("src/file.ts", &hunks, &[0, 1]);
        assert!(patch.contains("@@ -1,3 +1,3 @@"));
        assert!(patch.contains("@@ -8,3 +8,4 @@"));
    }

    #[test]
    fn test_patch生成_空選択() {
        let hunks = vec![hunk(0, 1, 1, 1, 1, &["-a", "+b"])];
        assert_eq!(generate_patch("f.ts", &hunks, &[]), "");
    }

    #[test]
    fn test_patch生成_無効インデックス() {
        let hunks = vec![hunk(0, 1, 1, 1, 1, &["-a", "+b"])];
        assert_eq!(generate_patch("f.ts", &hunks, &[99]), "");
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
