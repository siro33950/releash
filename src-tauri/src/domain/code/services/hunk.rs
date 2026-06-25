//! hunk の区切り（change group）・patch 生成・range 算出の純粋ロジック。
//!
//! diff バッファの計算（git2 依存）は `DiffComputer`（gateway）が担い、本サービスは
//! 生成済みの `Hunk` を入力に純粋計算を行う。

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use crate::domain::code::value_objects::{ChangeGroup, HiddenRange, Hunk, VisibleBlock};
use sha2::{Digest, Sha256};

const GROUP_CONTEXT_RADIUS: usize = 1;

/// ReviewBlobSide は review blob URL の content-source 選択軸であるのに対し、
/// StableGroupIdSide は hunk の old/new 行射影軸を表すため別型として扱う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StableGroupIdSide {
    Original,
    Modified,
}

#[derive(Debug, Clone)]
struct SideLine {
    offset: usize,
    line_number: usize,
    content: String,
    is_context: bool,
}

fn stable_lines_hash<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    let mut hasher = Sha256::new();
    for line in lines {
        hasher.update((line.len() as u64).to_be_bytes());
        hasher.update(b":");
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        write!(&mut hex, "{byte:02x}").expect("writing sha256 digest hex to string cannot fail");
    }
    hex
}

fn hunk_content_hash(hunk: &Hunk) -> String {
    stable_lines_hash(hunk.lines.iter().map(String::as_str))
}

fn group_identity_hash(hunk: &Hunk, group: &ChangeGroup) -> String {
    let start = group.line_offset_start as usize;
    let end = (group.line_offset_end as usize + 1).min(hunk.lines.len());
    let mut identity_lines = Vec::new();

    // design.md A1: 境界 context は純削除/純挿入 group を stable side へ anchorable にするため identity に含める。
    let mut before_context: Vec<&str> = hunk.lines[..start]
        .iter()
        .rev()
        .filter_map(|line| {
            line.as_bytes()
                .first()
                .copied()
                .filter(|prefix| *prefix == b' ')
                .map(|_| line.as_str())
        })
        .take(GROUP_CONTEXT_RADIUS)
        .collect();
    before_context.reverse();
    for line in before_context {
        identity_lines.push(format!("before:{line}"));
    }

    identity_lines.push("group:".to_string());
    identity_lines.extend(
        hunk.lines[start..end]
            .iter()
            .map(|line| format!("change:{line}")),
    );

    for line in hunk.lines[end..]
        .iter()
        .filter_map(|line| {
            line.as_bytes()
                .first()
                .copied()
                .filter(|prefix| *prefix == b' ')
                .map(|_| line.as_str())
        })
        .take(GROUP_CONTEXT_RADIUS)
    {
        identity_lines.push(format!("after:{line}"));
    }

    stable_lines_hash(identity_lines.iter().map(String::as_str))
}

fn line_content(line: &str) -> &str {
    line.get(1..).unwrap_or("")
}

fn hunk_side_lines(hunk: &Hunk, side: StableGroupIdSide) -> Vec<SideLine> {
    let mut old_line = hunk.old_start as usize;
    let mut new_line = hunk.new_start as usize;
    let mut lines = Vec::new();

    for (offset, line) in hunk.lines.iter().enumerate() {
        let prefix = line.as_bytes().first().copied().unwrap_or(b' ');
        match prefix {
            b' ' => {
                let line_number = match side {
                    StableGroupIdSide::Original => old_line,
                    StableGroupIdSide::Modified => new_line,
                };
                lines.push(SideLine {
                    offset,
                    line_number,
                    content: line_content(line).to_string(),
                    is_context: true,
                });
                old_line += 1;
                new_line += 1;
            }
            b'-' => {
                if side == StableGroupIdSide::Original {
                    lines.push(SideLine {
                        offset,
                        line_number: old_line,
                        content: line_content(line).to_string(),
                        is_context: false,
                    });
                }
                old_line += 1;
            }
            b'+' => {
                if side == StableGroupIdSide::Modified {
                    lines.push(SideLine {
                        offset,
                        line_number: new_line,
                        content: line_content(line).to_string(),
                        is_context: false,
                    });
                }
                new_line += 1;
            }
            b'\\' => {}
            _ => {
                let line_number = match side {
                    StableGroupIdSide::Original => old_line,
                    StableGroupIdSide::Modified => new_line,
                };
                lines.push(SideLine {
                    offset,
                    line_number,
                    content: line.to_string(),
                    is_context: true,
                });
                old_line += 1;
                new_line += 1;
            }
        }
    }

    lines
}

fn group_side_identity(
    hunk: &Hunk,
    group: &ChangeGroup,
    side: StableGroupIdSide,
) -> Option<(Vec<String>, usize)> {
    let start = group.line_offset_start as usize;
    let end = group.line_offset_end as usize;
    let side_lines = hunk_side_lines(hunk, side);
    let mut identity_lines = Vec::new();

    let mut before_context: Vec<SideLine> = side_lines
        .iter()
        .rev()
        .filter(|line| line.offset < start && line.is_context)
        .take(GROUP_CONTEXT_RADIUS)
        .cloned()
        .collect();
    before_context.reverse();
    identity_lines.extend(before_context);

    identity_lines.extend(
        side_lines
            .iter()
            .filter(|line| line.offset >= start && line.offset <= end && !line.is_context)
            .cloned(),
    );

    identity_lines.extend(
        side_lines
            .iter()
            .filter(|line| line.offset > end && line.is_context)
            .take(GROUP_CONTEXT_RADIUS)
            .cloned(),
    );

    let first_line = identity_lines.first()?.line_number.saturating_sub(1);
    Some((
        identity_lines
            .into_iter()
            .map(|line| line.content)
            .collect(),
        first_line,
    ))
}

fn side_occurrence(content: &str, identity_lines: &[String], expected_start: usize) -> Option<u32> {
    if identity_lines.is_empty() {
        return None;
    }

    let file_lines: Vec<&str> = content.lines().collect();
    if identity_lines.len() > file_lines.len() {
        return None;
    }

    let mut occurrence = 0;
    for start in 0..=file_lines.len() - identity_lines.len() {
        if file_lines[start..start + identity_lines.len()]
            .iter()
            .zip(identity_lines)
            .all(|(actual, expected)| *actual == expected)
        {
            if start == expected_start {
                return Some(occurrence);
            }
            occurrence += 1;
        }
    }

    None
}

fn stable_side_occurrence(
    hunk: &Hunk,
    group: &ChangeGroup,
    original: &str,
    modified: &str,
    side: StableGroupIdSide,
) -> Option<u32> {
    let (identity_lines, expected_start) = group_side_identity(hunk, group, side)?;
    let content = match side {
        StableGroupIdSide::Original => original,
        StableGroupIdSide::Modified => modified,
    };
    side_occurrence(content, &identity_lines, expected_start)
}

fn hunk_side_identity(hunk: &Hunk, side: StableGroupIdSide) -> Option<(Vec<String>, usize)> {
    let side_lines = hunk_side_lines(hunk, side);
    let first_line = side_lines.first()?.line_number.saturating_sub(1);
    Some((
        side_lines.into_iter().map(|line| line.content).collect(),
        first_line,
    ))
}

fn stable_side_hunk_occurrence(
    hunk: &Hunk,
    original: &str,
    modified: &str,
    side: StableGroupIdSide,
) -> Option<u32> {
    let (identity_lines, expected_start) = hunk_side_identity(hunk, side)?;
    let content = match side {
        StableGroupIdSide::Original => original,
        StableGroupIdSide::Modified => modified,
    };
    side_occurrence(content, &identity_lines, expected_start)
}

fn assign_stable_ids_for_side<T, Candidate, MakeId, AssignId>(
    items: &[T],
    mut candidate: Candidate,
    mut make_id: MakeId,
    mut assign_id: AssignId,
) -> Vec<T>
where
    T: Clone,
    Candidate: FnMut(&T) -> Option<(String, Option<u32>)>,
    MakeId: FnMut(&T, u32) -> String,
    AssignId: FnMut(&T, String) -> T,
{
    let mut fallback_occurrences: HashMap<String, u32> = HashMap::new();
    let mut used_ids: HashSet<String> = HashSet::new();

    items
        .iter()
        .map(|item| {
            let Some((hash, stable_occurrence)) = candidate(item) else {
                return item.clone();
            };
            let mut occurrence = stable_occurrence.unwrap_or_else(|| {
                let occurrence = fallback_occurrences.entry(hash.clone()).or_insert(0);
                let current = *occurrence;
                *occurrence += 1;
                current
            });
            let mut id = make_id(item, occurrence);
            while used_ids.contains(&id) {
                occurrence += 1;
                id = make_id(item, occurrence);
            }
            used_ids.insert(id.clone());
            assign_id(item, id)
        })
        .collect()
}

/// hunk の内容由来 stable id を算出する。
pub fn compute_hunk_id(hunk: &Hunk, occurrence: u32) -> String {
    format!("h:{}:{occurrence}", hunk_content_hash(hunk))
}

/// change group の内容由来 stable id を算出する。
pub fn compute_group_id(hunk: &Hunk, group: &ChangeGroup, occurrence: u32) -> String {
    format!("g:{}:{occurrence}", group_identity_hash(hunk, group))
}

/// review 操作で変化しない side の全文に対する出現順で group id を再付与する。
pub fn assign_stable_group_ids_for_side(
    hunks: &[Hunk],
    groups: &[ChangeGroup],
    original: &str,
    modified: &str,
    side: StableGroupIdSide,
) -> Vec<ChangeGroup> {
    assign_stable_ids_for_side(
        groups,
        |group| {
            let hunk = hunks.iter().find(|hunk| hunk.index == group.hunk_index)?;
            let hash = group_identity_hash(hunk, group);
            let occurrence = stable_side_occurrence(hunk, group, original, modified, side);
            Some((hash, occurrence))
        },
        |group, occurrence| {
            let hunk = hunks
                .iter()
                .find(|hunk| hunk.index == group.hunk_index)
                .expect("hunk resolved while building stable group id candidate");
            compute_group_id(hunk, group, occurrence)
        },
        |group, group_id| ChangeGroup {
            group_id,
            ..group.clone()
        },
    )
}

/// review 操作で変化しない side の全文に対する出現順で hunk id を再付与する。
pub fn assign_stable_hunk_ids_for_side(
    hunks: &[Hunk],
    original: &str,
    modified: &str,
    side: StableGroupIdSide,
) -> Vec<Hunk> {
    assign_stable_ids_for_side(
        hunks,
        |hunk| {
            let hash = hunk_content_hash(hunk);
            let occurrence = stable_side_hunk_occurrence(hunk, original, modified, side);
            Some((hash, occurrence))
        },
        compute_hunk_id,
        |hunk, hunk_id| Hunk {
            hunk_id,
            ..hunk.clone()
        },
    )
}

/// hunk 群へ内容由来 stable id を付与する。
pub fn assign_hunk_ids(hunks: &[Hunk]) -> Vec<Hunk> {
    let mut occurrences: HashMap<String, u32> = HashMap::new();
    hunks
        .iter()
        .map(|hunk| {
            let hash = hunk_content_hash(hunk);
            let occurrence = occurrences.entry(hash).or_insert(0);
            let hunk_id = compute_hunk_id(hunk, *occurrence);
            *occurrence += 1;
            Hunk {
                hunk_id,
                ..hunk.clone()
            }
        })
        .collect()
}

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
                    group_id: String::new(),
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
            group_id: String::new(),
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
    let mut occurrences: HashMap<String, u32> = HashMap::new();
    for hunk in hunks {
        let hunk_groups = split_hunk_into_groups(hunk, groups.len() as u32)
            .into_iter()
            .map(|group| {
                let hash = group_identity_hash(hunk, &group);
                let occurrence = occurrences.entry(hash).or_insert(0);
                let group_id = compute_group_id(hunk, &group, *occurrence);
                *occurrence += 1;
                ChangeGroup { group_id, ..group }
            });
        groups.extend(hunk_groups);
    }
    groups
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
        hunk.old_start, hunk.new_start
    ));
    output.extend(result_lines);

    format!("{}\n", output.join("\n"))
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
