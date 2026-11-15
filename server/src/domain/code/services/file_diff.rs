use crate::domain::code::value_objects::DiffLine;
use crate::domain::code::Hunk;

/// hunk と変更後の全文から、ファイル全体の差分を行の並びにする。
///
/// hunk に含まれない行は、変更前と変更後の両方にある行として並べる。
pub fn lines(hunks: &[Hunk], modified: &str) -> Vec<DiffLine> {
    let mut unchanged = modified.lines();
    let mut result = Vec::new();
    let (mut old, mut new) = (1, 1);
    for hunk in hunks {
        let first = if hunk.new_lines == 0 {
            hunk.new_start + 1
        } else {
            hunk.new_start
        };
        while new < first {
            let Some(text) = unchanged.next() else { break };
            result.push(DiffLine::Context {
                old,
                new,
                text: text.into(),
            });
            old += 1;
            new += 1;
        }
        for line in &hunk.lines {
            let Some(text) = line.get(1..) else { continue };
            let text = text.strip_suffix('\r').unwrap_or(text).to_string();
            match line.as_bytes()[0] {
                b' ' => {
                    unchanged.next();
                    result.push(DiffLine::Context { old, new, text });
                    old += 1;
                    new += 1;
                }
                b'+' => {
                    unchanged.next();
                    result.push(DiffLine::Addition { new, text });
                    new += 1;
                }
                b'-' => {
                    result.push(DiffLine::Deletion { old, text });
                    old += 1;
                }
                _ => {}
            }
        }
    }
    for text in unchanged {
        result.push(DiffLine::Context {
            old,
            new,
            text: text.into(),
        });
        old += 1;
        new += 1;
    }
    result
}

#[cfg(test)]
#[path = "file_diff_test.rs"]
mod file_diff_tests;
