use std::path::Path;

use super::types::{ChangeGroup, DiffHunksResult, HiddenRange, Hunk, VisibleBlock};

/// Compute diff hunks and change groups from two text buffers.
///
/// Uses `git2::Patch::from_buffers` for the underlying diff computation,
/// then converts the result into `Hunk` / `ChangeGroup` structures
/// that the frontend can consume directly.
pub fn compute_diff_hunks(
    original: &str,
    modified: &str,
    file_path: Option<&str>,
) -> DiffHunksResult {
    let path = Path::new(file_path.unwrap_or("file"));
    let mut hunks: Vec<Hunk> = Vec::new();

    let patch = git2::Patch::from_buffers(
        original.as_bytes(),
        Some(path),
        modified.as_bytes(),
        Some(path),
        None,
    );

    if let Ok(patch) = patch {
        let num_hunks = patch.num_hunks();
        for hunk_idx in 0..num_hunks {
            let Ok((hdr, _)) = patch.hunk(hunk_idx) else {
                continue;
            };
            let num_lines = patch.num_lines_in_hunk(hunk_idx).unwrap_or(0);
            let mut lines: Vec<String> = Vec::new();

            for line_idx in 0..num_lines {
                let Ok(line) = patch.line_in_hunk(hunk_idx, line_idx) else {
                    continue;
                };
                let content = std::str::from_utf8(line.content()).unwrap_or("");
                let content = content.strip_suffix('\n').unwrap_or(content);
                let prefix = match line.origin() {
                    '+' => "+",
                    '-' => "-",
                    ' ' => " ",
                    '\\' => "\\",
                    _ => continue,
                };
                lines.push(format!("{prefix}{content}"));
            }

            hunks.push(Hunk {
                index: hunk_idx as u32,
                old_start: hdr.old_start(),
                old_lines: hdr.old_lines(),
                new_start: hdr.new_start(),
                new_lines: hdr.new_lines(),
                lines,
            });
        }
    }

    let change_groups = compute_change_groups(&hunks);

    DiffHunksResult {
        hunks,
        change_groups,
    }
}

/// Split hunks into contiguous change groups.
///
/// A change group is a contiguous block of `+` and/or `-` lines within a hunk.
/// Context lines (` `) separate groups.
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

/// Compute change groups from a list of hunks.
pub fn compute_change_groups(hunks: &[Hunk]) -> Vec<ChangeGroup> {
    let mut groups: Vec<ChangeGroup> = Vec::new();
    for hunk in hunks {
        groups.extend(split_hunk_into_groups(hunk, groups.len() as u32));
    }
    groups
}

/// Extract the lines belonging to a change group from its hunk.
#[allow(dead_code)]
fn extract_group_lines(group: &ChangeGroup, hunks: &[Hunk]) -> Vec<String> {
    let Some(hunk) = hunks.iter().find(|h| h.index == group.hunk_index) else {
        return Vec::new();
    };
    let start = group.line_offset_start as usize;
    let end = (group.line_offset_end as usize + 1).min(hunk.lines.len());
    hunk.lines[start..end].to_vec()
}

/// Get the old-file line position corresponding to the start of a change group.
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

/// Mark change groups as staged by comparing with staged diff groups.
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

/// Generate a unified-diff patch for a single change group within a hunk.
///
/// Lines outside the group range are converted:
/// - `+` lines are dropped (not part of this group)
/// - `-` lines become context (` `) lines
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

/// Generate a unified-diff patch from selected hunks.
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

/// Compute hidden (foldable) line ranges for diff-only mode in the Gutter viewer.
///
/// Given a list of hunks and the total line count of the modified file,
/// Compute merged visible line ranges around each hunk with context.
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

/// returns the ranges of lines that should be hidden (folded) because they
/// contain no changes. Each hunk is surrounded by `context_lines` of visible
/// context.
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

/// Compute hidden ranges directly from content strings.
///
/// Internally diffs `original` against `modified` and then computes hidden ranges.
pub fn compute_hidden_ranges_from_content(
    original: &str,
    modified: &str,
    context_lines: u32,
) -> Vec<HiddenRange> {
    let hunks_result = compute_diff_hunks(original, modified, None);
    let total_lines = modified.lines().count() as u32;
    compute_hidden_ranges(&hunks_result.hunks, total_lines, context_lines)
}

/// Compute visible blocks for Markdown diff-only mode.
///
/// Diffs `original` against `modified` and returns only the blocks
/// (with surrounding context lines) that contain changes.
pub fn compute_visible_markdown_blocks(
    original: &str,
    modified: &str,
    context_lines: u32,
) -> Vec<VisibleBlock> {
    let hunks_result = compute_diff_hunks(original, modified, None);
    let mod_lines: Vec<&str> = modified.lines().collect();
    let orig_lines: Vec<&str> = original.lines().collect();
    let total_lines = mod_lines.len() as u32;

    if hunks_result.hunks.is_empty() {
        return Vec::new();
    }

    let merged = compute_visible_ranges(&hunks_result.hunks, total_lines, context_lines);

    // Build visible blocks from merged ranges, including deleted content
    merged
        .iter()
        .map(|(start, end)| {
            let s = (*start as usize).saturating_sub(1);
            let e = (*end as usize).min(mod_lines.len());
            let content = mod_lines[s..e].join("\n");

            // Collect deleted lines from hunks that overlap this visible range
            let mut deleted: Vec<&str> = Vec::new();
            for hunk in &hunks_result.hunks {
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

/// Convert an absolute file path to a repository-relative path.
pub fn get_relative_path(root_path: &str, file_path: &str) -> Option<String> {
    let prefix = format!("{root_path}/");
    if file_path.starts_with(&prefix) {
        Some(file_path[prefix.len()..].to_string())
    } else {
        Some(file_path.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── compute_diff_hunks ──

    #[test]
    fn identical_content_returns_empty() {
        let result = compute_diff_hunks("hello\nworld\n", "hello\nworld\n", None);
        assert!(result.hunks.is_empty());
        assert!(result.change_groups.is_empty());
    }

    #[test]
    fn detect_added_lines() {
        let result = compute_diff_hunks("line1\nline2\n", "line1\nline2\nline3\n", None);
        assert_eq!(result.hunks.len(), 1);
        assert!(result.hunks[0].lines.iter().any(|l| l == "+line3"));
    }

    #[test]
    fn detect_removed_lines() {
        let result = compute_diff_hunks("line1\nline2\nline3\n", "line1\nline3\n", None);
        assert_eq!(result.hunks.len(), 1);
        assert!(result.hunks[0].lines.iter().any(|l| l == "-line2"));
    }

    #[test]
    fn detect_modified_lines() {
        let result =
            compute_diff_hunks("line1\noriginal\nline3\n", "line1\nmodified\nline3\n", None);
        assert_eq!(result.hunks.len(), 1);
        assert!(result.hunks[0].lines.iter().any(|l| l == "-original"));
        assert!(result.hunks[0].lines.iter().any(|l| l == "+modified"));
    }

    #[test]
    fn detect_multiple_hunks() {
        let lines = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let modified = lines.replace("b\n", "B\n").replace("r\n", "R\n");
        let result = compute_diff_hunks(lines, &modified, None);
        assert_eq!(result.hunks.len(), 2);
    }

    #[test]
    fn empty_original() {
        let result = compute_diff_hunks("", "new content\n", None);
        assert_eq!(result.hunks.len(), 1);
        assert!(result.hunks[0].lines.iter().any(|l| l == "+new content"));
    }

    #[test]
    fn empty_modified() {
        let result = compute_diff_hunks("content\n", "", None);
        assert_eq!(result.hunks.len(), 1);
        assert!(result.hunks[0].lines.iter().any(|l| l == "-content"));
    }

    #[test]
    fn sequential_indices() {
        let lines = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let modified = lines.replace("b\n", "B\n").replace("r\n", "R\n");
        let result = compute_diff_hunks(lines, &modified, None);
        assert_eq!(result.hunks[0].index, 0);
        assert_eq!(result.hunks[1].index, 1);
    }

    #[test]
    fn hunks_have_position_info() {
        let result = compute_diff_hunks("line1\nline2\n", "line1\nline2\nline3\n", None);
        assert!(result.hunks[0].old_start > 0);
        assert!(result.hunks[0].new_start > 0);
    }

    // ── change groups ──

    #[test]
    fn change_groups_from_single_change() {
        let result = compute_diff_hunks("line1\nline2\n", "line1\nline2\nline3\n", None);
        assert_eq!(result.change_groups.len(), 1);
        assert_eq!(result.change_groups[0].group_index, 0);
        assert_eq!(result.change_groups[0].hunk_index, 0);
    }

    #[test]
    fn change_groups_from_multiple_hunks() {
        let lines = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let modified = lines.replace("b\n", "B\n").replace("r\n", "R\n");
        let result = compute_diff_hunks(lines, &modified, None);
        assert_eq!(result.change_groups.len(), 2);
        assert_eq!(result.change_groups[0].group_index, 0);
        assert_eq!(result.change_groups[1].group_index, 1);
    }

    // ── mark_staged_groups ──

    #[test]
    fn mark_staged_partial() {
        let head = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let working = head.replace("b\n", "B\n").replace("r\n", "R\n");
        let staged = head.replace("b\n", "B\n");

        let wt = compute_diff_hunks(head, &working, None);
        let st = compute_diff_hunks(head, &staged, None);

        let result = mark_staged_groups(&wt.change_groups, &st.change_groups, &wt.hunks, &st.hunks);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].is_staged, Some(true));
        assert_eq!(result[1].is_staged, Some(false));
    }

    #[test]
    fn mark_staged_all() {
        let head = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let working = head.replace("b\n", "B\n");
        let staged = &working;

        let wt = compute_diff_hunks(head, &working, None);
        let st = compute_diff_hunks(head, staged, None);

        let result = mark_staged_groups(&wt.change_groups, &st.change_groups, &wt.hunks, &st.hunks);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].is_staged, Some(true));
    }

    #[test]
    fn mark_staged_none() {
        let head = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let working = head.replace("b\n", "B\n");

        let wt = compute_diff_hunks(head, &working, None);
        let st = compute_diff_hunks(head, head, None);

        let result = mark_staged_groups(&wt.change_groups, &st.change_groups, &wt.hunks, &st.hunks);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].is_staged, Some(false));
    }

    #[test]
    fn mark_staged_empty() {
        let result = mark_staged_groups(&[], &[], &[], &[]);
        assert!(result.is_empty());
    }

    // ── generate_group_patch ──

    #[test]
    fn group_patch_single_change() {
        let hunk = Hunk {
            index: 0,
            old_start: 1,
            old_lines: 3,
            new_start: 1,
            new_lines: 3,
            lines: vec![
                " line1".to_string(),
                "-original".to_string(),
                "+modified".to_string(),
                " line3".to_string(),
            ],
        };
        let group = ChangeGroup {
            group_index: 0,
            hunk_index: 0,
            new_start: 2,
            new_end: 2,
            line_offset_start: 1,
            line_offset_end: 2,
            is_staged: None,
        };

        let patch = generate_group_patch("src/file.ts", &hunk, &group);
        assert!(patch.contains("--- a/src/file.ts"));
        assert!(patch.contains("+++ b/src/file.ts"));
        assert!(patch.contains("-original"));
        assert!(patch.contains("+modified"));
        assert!(patch.ends_with('\n'));
    }

    // ── generate_patch ──

    #[test]
    fn patch_single_hunk() {
        let hunks = vec![
            Hunk {
                index: 0,
                old_start: 1,
                old_lines: 3,
                new_start: 1,
                new_lines: 3,
                lines: vec![
                    " line1".to_string(),
                    "-original".to_string(),
                    "+modified".to_string(),
                    " line3".to_string(),
                ],
            },
            Hunk {
                index: 1,
                old_start: 8,
                old_lines: 3,
                new_start: 8,
                new_lines: 4,
                lines: vec![
                    " line8".to_string(),
                    "-old9".to_string(),
                    "+new9".to_string(),
                    "+new10".to_string(),
                    " line11".to_string(),
                ],
            },
        ];

        let patch = generate_patch("src/file.ts", &hunks, &[0]);
        assert!(patch.contains("--- a/src/file.ts"));
        assert!(patch.contains("+++ b/src/file.ts"));
        assert!(patch.contains("@@ -1,3 +1,3 @@"));
        assert!(patch.contains("+modified"));
        assert!(!patch.contains("+new9"));
    }

    #[test]
    fn patch_multiple_hunks() {
        let hunks = vec![
            Hunk {
                index: 0,
                old_start: 1,
                old_lines: 3,
                new_start: 1,
                new_lines: 3,
                lines: vec![
                    " line1".to_string(),
                    "-original".to_string(),
                    "+modified".to_string(),
                    " line3".to_string(),
                ],
            },
            Hunk {
                index: 1,
                old_start: 8,
                old_lines: 3,
                new_start: 8,
                new_lines: 4,
                lines: vec![
                    " line8".to_string(),
                    "-old9".to_string(),
                    "+new9".to_string(),
                    "+new10".to_string(),
                    " line11".to_string(),
                ],
            },
        ];

        let patch = generate_patch("src/file.ts", &hunks, &[0, 1]);
        assert!(patch.contains("@@ -1,3 +1,3 @@"));
        assert!(patch.contains("@@ -8,3 +8,4 @@"));
    }

    #[test]
    fn patch_empty_selection() {
        let hunks = vec![Hunk {
            index: 0,
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 1,
            lines: vec!["-a".to_string(), "+b".to_string()],
        }];
        assert_eq!(generate_patch("f.ts", &hunks, &[]), "");
    }

    #[test]
    fn patch_invalid_index() {
        let hunks = vec![Hunk {
            index: 0,
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 1,
            lines: vec!["-a".to_string(), "+b".to_string()],
        }];
        assert_eq!(generate_patch("f.ts", &hunks, &[99]), "");
    }

    // ── get_relative_path ──

    #[test]
    fn relative_path_strips_prefix() {
        let result = get_relative_path("/Users/foo/project", "/Users/foo/project/src/index.ts");
        assert_eq!(result, Some("src/index.ts".to_string()));
    }

    #[test]
    fn relative_path_no_prefix() {
        let result = get_relative_path("/Users/foo/project", "other/path.ts");
        assert_eq!(result, Some("other/path.ts".to_string()));
    }

    // ── compute_hidden_ranges ──

    #[test]
    fn hidden_ranges_no_hunks() {
        let result = compute_hidden_ranges(&[], 100, 3);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start_line, 1);
        assert_eq!(result[0].end_line, 100);
        assert_eq!(result[0].hidden_count, 100);
    }

    #[test]
    fn hidden_ranges_zero_total_lines() {
        let result = compute_hidden_ranges(&[], 0, 3);
        assert!(result.is_empty());
    }

    #[test]
    fn hidden_ranges_single_hunk_at_start() {
        let hunks = vec![Hunk {
            index: 0,
            old_start: 1,
            old_lines: 2,
            new_start: 1,
            new_lines: 3,
            lines: vec![
                "-old".to_string(),
                "+new1".to_string(),
                "+new2".to_string(),
                " ctx".to_string(),
            ],
        }];
        let result = compute_hidden_ranges(&hunks, 20, 3);
        // Hunk covers lines 1-3, context extends to 6
        // Hidden: 7-20
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start_line, 7);
        assert_eq!(result[0].end_line, 20);
    }

    #[test]
    fn hidden_ranges_single_hunk_in_middle() {
        let hunks = vec![Hunk {
            index: 0,
            old_start: 10,
            old_lines: 2,
            new_start: 10,
            new_lines: 2,
            lines: vec!["-old".to_string(), "+new".to_string()],
        }];
        let result = compute_hidden_ranges(&hunks, 20, 3);
        // Hunk covers lines 10-11, context extends to 7-14
        // Hidden: 1-6, 15-20
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].start_line, 1);
        assert_eq!(result[0].end_line, 6);
        assert_eq!(result[1].start_line, 15);
        assert_eq!(result[1].end_line, 20);
    }

    #[test]
    fn hidden_ranges_adjacent_hunks_merge() {
        let hunks = vec![
            Hunk {
                index: 0,
                old_start: 5,
                old_lines: 1,
                new_start: 5,
                new_lines: 1,
                lines: vec!["-a".to_string(), "+b".to_string()],
            },
            Hunk {
                index: 1,
                old_start: 8,
                old_lines: 1,
                new_start: 8,
                new_lines: 1,
                lines: vec!["-c".to_string(), "+d".to_string()],
            },
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
    fn hidden_ranges_all_lines_changed() {
        let hunks = vec![Hunk {
            index: 0,
            old_start: 1,
            old_lines: 5,
            new_start: 1,
            new_lines: 5,
            lines: vec![
                "-a".to_string(),
                "-b".to_string(),
                "-c".to_string(),
                "-d".to_string(),
                "-e".to_string(),
                "+A".to_string(),
                "+B".to_string(),
                "+C".to_string(),
                "+D".to_string(),
                "+E".to_string(),
            ],
        }];
        let result = compute_hidden_ranges(&hunks, 5, 3);
        assert!(result.is_empty());
    }

    // ── compute_visible_ranges (shared helper) ──

    #[test]
    fn visible_ranges_empty_hunks() {
        let result = compute_visible_ranges(&[], 100, 3);
        assert!(result.is_empty());
    }

    #[test]
    fn visible_ranges_single_hunk() {
        let hunks = vec![Hunk {
            index: 0,
            old_start: 10,
            old_lines: 2,
            new_start: 10,
            new_lines: 2,
            lines: vec!["-old".to_string(), "+new".to_string()],
        }];
        let result = compute_visible_ranges(&hunks, 20, 3);
        assert_eq!(result, vec![(7, 14)]);
    }

    #[test]
    fn visible_ranges_delete_only_hunk() {
        let hunks = vec![Hunk {
            index: 0,
            old_start: 5,
            old_lines: 3,
            new_start: 5,
            new_lines: 0,
            lines: vec!["-a".to_string(), "-b".to_string(), "-c".to_string()],
        }];
        let result = compute_visible_ranges(&hunks, 20, 3);
        assert_eq!(result, vec![(2, 8)]);
    }

    #[test]
    fn visible_ranges_overlapping_merge() {
        let hunks = vec![
            Hunk {
                index: 0,
                old_start: 5,
                old_lines: 1,
                new_start: 5,
                new_lines: 1,
                lines: vec!["-a".to_string(), "+b".to_string()],
            },
            Hunk {
                index: 1,
                old_start: 8,
                old_lines: 1,
                new_start: 8,
                new_lines: 1,
                lines: vec!["-c".to_string(), "+d".to_string()],
            },
        ];
        let result = compute_visible_ranges(&hunks, 20, 3);
        assert_eq!(result, vec![(2, 11)]);
    }

    // ── compute_hidden_ranges_from_content ──

    #[test]
    fn hidden_ranges_from_content_no_changes() {
        let text = "line1\nline2\nline3\n";
        let result = compute_hidden_ranges_from_content(text, text, 3);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start_line, 1);
    }

    #[test]
    fn hidden_ranges_from_content_with_change() {
        // Use a large file so hidden ranges exist outside the hunk + context
        let lines: Vec<String> = (1..=30).map(|i| format!("line{i}")).collect();
        let original = lines.join("\n");
        let mut modified_lines = lines.clone();
        modified_lines[14] = "CHANGED15".to_string(); // Change line 15
        let modified = modified_lines.join("\n");
        let result = compute_hidden_ranges_from_content(&original, &modified, 3);
        // With 30 lines and a change around line 15, there should be hidden ranges
        // before and after the visible context around the hunk
        assert!(
            !result.is_empty(),
            "expected hidden ranges for a 30-line file with change at line 15"
        );
    }

    // ── compute_visible_markdown_blocks ──

    #[test]
    fn visible_blocks_no_changes() {
        let text = "line1\nline2\nline3\n";
        let result = compute_visible_markdown_blocks(text, text, 3);
        assert!(result.is_empty());
    }

    #[test]
    fn visible_blocks_single_change() {
        let original = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let modified = "a\nb\nc\nd\nE\nf\ng\nh\ni\nj\n";
        let result = compute_visible_markdown_blocks(original, modified, 2);
        assert_eq!(result.len(), 1);
        assert!(result[0].content.contains("E"));
        assert!(result[0].start_line <= 5);
        assert!(result[0].end_line >= 5);
    }

    #[test]
    fn visible_blocks_multiple_changes() {
        let original = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let modified = original.replace("b\n", "B\n").replace("s\n", "S\n");
        let result = compute_visible_markdown_blocks(original, &modified, 2);
        assert_eq!(result.len(), 2);
    }
}
