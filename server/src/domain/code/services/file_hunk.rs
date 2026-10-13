use std::collections::{BTreeMap, BTreeSet};

use crate::domain::code::{CodeError, Hunk};

pub fn ensure_hunk_comparison(base: crate::domain::code::ReviewBase) -> Result<(), CodeError> {
    if base != crate::domain::code::ReviewBase::Head {
        return Err(CodeError::Rule(
            "file hunk actions are unavailable for branch-base".into(),
        ));
    }
    Ok(())
}

pub fn select_hunk<'a>(hunks: Option<&'a [Hunk]>, id: &str) -> Result<&'a Hunk, CodeError> {
    hunks
        .ok_or_else(|| CodeError::Rule("file hunk requires text".into()))?
        .iter()
        .find(|hunk| hunk.hunk_id == id)
        .ok_or_else(|| CodeError::StaleReviewGroupTarget {
            group_id: id.into(),
        })
}

pub fn line_counts(hunks: &[Hunk]) -> (u32, u32) {
    let mut additions = 0;
    let mut deletions = 0;
    for line in hunks.iter().flat_map(|hunk| &hunk.lines) {
        match line.as_bytes().first() {
            Some(b'+') => additions += 1,
            Some(b'-') => deletions += 1,
            _ => {}
        }
    }
    (additions, deletions)
}

pub struct FilePresence {
    pub head: bool,
    pub index: bool,
    pub working: bool,
}

pub fn index_patch(
    path: &str,
    hunks: &[Hunk],
    index: &str,
    target: &str,
    presence: FilePresence,
    stage: bool,
) -> String {
    let mut patch = patch(path, hunks, index, target);
    if !presence.index && !patch.is_empty() {
        if let Some((_, rest)) = patch.split_once('\n') {
            patch = format!("--- /dev/null\n{rest}");
        }
    }
    if target.is_empty()
        && !patch.is_empty()
        && ((stage && !presence.working) || (!stage && !presence.head))
    {
        let mut lines = patch.lines();
        if let (Some(original), Some(_)) = (lines.next(), lines.next()) {
            patch = format!(
                "{original}\n+++ /dev/null\n{}\n",
                lines.collect::<Vec<_>>().join("\n")
            );
        }
    }
    patch
}

#[derive(Clone, PartialEq)]
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

fn edits(hunks: &[Hunk], modified: &str) -> Vec<Edit> {
    let lines: Vec<_> = modified.split_inclusive('\n').collect();
    edits_with_lines(hunks, &lines)
}

fn edits_with_lines(hunks: &[Hunk], lines: &[&str]) -> Vec<Edit> {
    let mut result = Vec::new();
    for hunk in hunks {
        let mut old = if hunk.old_lines == 0 {
            hunk.old_start
        } else {
            hunk.old_start.saturating_sub(1)
        } as usize;
        let mut new = if hunk.new_lines == 0 {
            hunk.new_start
        } else {
            hunk.new_start.saturating_sub(1)
        } as usize;
        let mut current: Option<(usize, usize)> = None;
        for line in &hunk.lines {
            match line.as_bytes().first() {
                Some(b'-') => {
                    current.get_or_insert((old, new));
                    old += 1;
                }
                Some(b'+') => {
                    current.get_or_insert((old, new));
                    new += 1;
                }
                Some(b' ') => {
                    if let Some((start, new_start)) = current.take() {
                        push_edits(&mut result, start, old, &lines[new_start..new]);
                    }
                    old += 1;
                    new += 1;
                }
                _ => {}
            }
        }
        if let Some((start, new_start)) = current {
            push_edits(&mut result, start, old, &lines[new_start..new]);
        }
    }
    result
}

fn push_edits(result: &mut Vec<Edit>, start: usize, end: usize, lines: &[&str]) {
    for position in start..end {
        result.push(Edit {
            start: position,
            end: position + 1,
            replacement: lines.get(position - start).copied().unwrap_or("").into(),
        });
    }
    let consumed = end - start;
    if lines.len() > consumed {
        result.push(Edit {
            start: end,
            end,
            replacement: lines[consumed..].concat(),
        });
    }
}

pub fn hunk_stage_states(
    index: Option<&str>,
    working: &str,
    staged_hunks: &[Hunk],
    hunks: &[Hunk],
) -> Vec<bool> {
    let Some(index) = index else {
        return vec![false; hunks.len()];
    };
    let staged: BTreeMap<_, _> = edits(staged_hunks, index)
        .into_iter()
        .map(|edit| ((edit.start, edit.end), edit.replacement))
        .collect();
    let working_lines: Vec<_> = working.split_inclusive('\n').collect();
    hunks
        .iter()
        .map(|hunk| {
            let selected = edits_with_lines(std::slice::from_ref(hunk), &working_lines);
            selected
                .iter()
                .all(|edit| staged.get(&(edit.start, edit.end)) == Some(&edit.replacement))
        })
        .collect()
}

pub fn index_after_hunk(
    head: &str,
    index: &str,
    working: &str,
    staged_hunks: &[Hunk],
    selected: &Hunk,
    stage: bool,
) -> Result<String, CodeError> {
    let selected = edits(std::slice::from_ref(selected), working);
    let mut retained = edits(staged_hunks, index);
    let targets: BTreeSet<_> = selected.iter().map(|edit| (edit.start, edit.end)).collect();
    retained.retain(|edit| !targets.contains(&(edit.start, edit.end)));
    if stage {
        retained.extend(selected);
    }
    retained.sort_by_key(|edit| (edit.start, edit.end));
    let lines: Vec<_> = head.split_inclusive('\n').collect();
    let mut cursor = 0;
    let mut result = String::new();
    for edit in retained {
        if edit.start < cursor || edit.end > lines.len() {
            return Err(CodeError::Rule("overlapping file hunk changes".into()));
        }
        result.push_str(&lines[cursor..edit.start].concat());
        result.push_str(&edit.replacement);
        cursor = edit.end;
    }
    result.push_str(&lines[cursor..].concat());
    Ok(result)
}

pub fn patch(path: &str, hunks: &[Hunk], original: &str, modified: &str) -> String {
    if hunks.is_empty() {
        return String::new();
    }
    let quoted = format!(
        "\"{}\"",
        path.replace('\\', "\\\\")
            .replace('\"', "\\\"")
            .replace('\n', "\\n")
            .replace('\t', "\\t")
    );
    let mut patch = format!(
        "--- \"a/{}\"\n+++ \"b/{}\"\n",
        &quoted[1..quoted.len() - 1],
        &quoted[1..quoted.len() - 1]
    );
    let old_count = original.lines().count();
    let new_count = modified.lines().count();
    for hunk in hunks {
        patch.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
        ));
        let mut old = hunk.old_start;
        let mut new = hunk.new_start;
        for line in &hunk.lines {
            patch.push_str(line);
            patch.push('\n');
            let prefix = line.as_bytes().first().copied();
            let missing_old = matches!(prefix, Some(b'-' | b' '))
                && old as usize == old_count
                && !original.ends_with('\n');
            let missing_new = matches!(prefix, Some(b'+' | b' '))
                && new as usize == new_count
                && !modified.ends_with('\n');
            if missing_old || missing_new {
                patch.push_str("\\ No newline at end of file\n");
            }
            if matches!(prefix, Some(b'-' | b' ')) {
                old += 1;
            }
            if matches!(prefix, Some(b'+' | b' ')) {
                new += 1;
            }
        }
    }
    patch
}

#[cfg(test)]
#[path = "file_hunk_test.rs"]
mod file_hunk_tests;
