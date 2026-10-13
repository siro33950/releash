use crate::domain::code::Hunk;

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
