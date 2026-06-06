//! 2 つのテキストバッファの差分計算（git2 依存）の gateway 実装。
//!
//! `git2::Patch::from_buffers` による unified diff を `Hunk` 列へ変換する部分のみを
//! 担う。hunk 区切り（change group）・range 算出・patch 生成といった後段の純粋ロジックは
//! ドメインサービス（`domain::code::services::hunk`）が担う。

use std::path::Path;

use crate::domain::code::{DiffComputer, Hunk};

/// 2 つのバッファを diff して hunk 列を生成する。
/// diff 計算に失敗した場合は空の hunk 列を返す（移行前の挙動と等価）。
pub(crate) fn diff_buffers(original: &str, modified: &str, file_path: Option<&str>) -> Vec<Hunk> {
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

    hunks
}

/// `DiffComputer` の git2 実装。
pub struct DiffComputerGateway;

impl DiffComputer for DiffComputerGateway {
    fn diff_buffers(&self, original: &str, modified: &str, file_path: Option<&str>) -> Vec<Hunk> {
        diff_buffers(original, modified, file_path)
    }
}

#[cfg(test)]
mod diff_compute_gateway_tests {
    use super::*;
    use crate::domain::code::services::hunk::{
        compute_change_groups, compute_hidden_ranges, compute_visible_markdown_blocks,
        mark_staged_groups,
    };

    // ── diff_buffers（git2 diff） ──

    #[test]
    fn test_diff_同一内容は空() {
        let hunks = diff_buffers("hello\nworld\n", "hello\nworld\n", None);
        assert!(hunks.is_empty());
        assert!(compute_change_groups(&hunks).is_empty());
    }

    #[test]
    fn test_diff_追加行検出() {
        let hunks = diff_buffers("line1\nline2\n", "line1\nline2\nline3\n", None);
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].lines.iter().any(|l| l == "+line3"));
    }

    #[test]
    fn test_diff_削除行検出() {
        let hunks = diff_buffers("line1\nline2\nline3\n", "line1\nline3\n", None);
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].lines.iter().any(|l| l == "-line2"));
    }

    #[test]
    fn test_diff_変更行検出() {
        let hunks = diff_buffers("line1\noriginal\nline3\n", "line1\nmodified\nline3\n", None);
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].lines.iter().any(|l| l == "-original"));
        assert!(hunks[0].lines.iter().any(|l| l == "+modified"));
    }

    #[test]
    fn test_diff_複数hunk検出() {
        let lines = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let modified = lines.replace("b\n", "B\n").replace("r\n", "R\n");
        let hunks = diff_buffers(lines, &modified, None);
        assert_eq!(hunks.len(), 2);
    }

    #[test]
    fn test_diff_空original() {
        let hunks = diff_buffers("", "new content\n", None);
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].lines.iter().any(|l| l == "+new content"));
    }

    #[test]
    fn test_diff_空modified() {
        let hunks = diff_buffers("content\n", "", None);
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].lines.iter().any(|l| l == "-content"));
    }

    #[test]
    fn test_diff_インデックスは連番() {
        let lines = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let modified = lines.replace("b\n", "B\n").replace("r\n", "R\n");
        let hunks = diff_buffers(lines, &modified, None);
        assert_eq!(hunks[0].index, 0);
        assert_eq!(hunks[1].index, 1);
    }

    #[test]
    fn test_diff_hunkは位置情報を持つ() {
        let hunks = diff_buffers("line1\nline2\n", "line1\nline2\nline3\n", None);
        assert!(hunks[0].old_start > 0);
        assert!(hunks[0].new_start > 0);
    }

    // ── diff_buffers + change groups（旧 compute_diff_hunks 相当） ──

    #[test]
    fn test_change_group_単一変更() {
        let hunks = diff_buffers("line1\nline2\n", "line1\nline2\nline3\n", None);
        let groups = compute_change_groups(&hunks);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group_index, 0);
        assert_eq!(groups[0].hunk_index, 0);
    }

    #[test]
    fn test_change_group_複数hunk() {
        let lines = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let modified = lines.replace("b\n", "B\n").replace("r\n", "R\n");
        let hunks = diff_buffers(lines, &modified, None);
        let groups = compute_change_groups(&hunks);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].group_index, 0);
        assert_eq!(groups[1].group_index, 1);
    }

    // ── mark_staged_groups（実 diff から hunk を生成） ──

    #[test]
    fn test_staged判定_部分staged() {
        let head = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let working = head.replace("b\n", "B\n").replace("r\n", "R\n");
        let staged = head.replace("b\n", "B\n");

        let wt_hunks = diff_buffers(head, &working, None);
        let wt_cg = compute_change_groups(&wt_hunks);
        let st_hunks = diff_buffers(head, &staged, None);
        let st_cg = compute_change_groups(&st_hunks);

        let result = mark_staged_groups(&wt_cg, &st_cg, &wt_hunks, &st_hunks);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].is_staged, Some(true));
        assert_eq!(result[1].is_staged, Some(false));
    }

    #[test]
    fn test_staged判定_全staged() {
        let head = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let working = head.replace("b\n", "B\n");

        let wt_hunks = diff_buffers(head, &working, None);
        let wt_cg = compute_change_groups(&wt_hunks);
        let st_hunks = diff_buffers(head, &working, None);
        let st_cg = compute_change_groups(&st_hunks);

        let result = mark_staged_groups(&wt_cg, &st_cg, &wt_hunks, &st_hunks);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].is_staged, Some(true));
    }

    #[test]
    fn test_staged判定_全unstaged() {
        let head = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let working = head.replace("b\n", "B\n");

        let wt_hunks = diff_buffers(head, &working, None);
        let wt_cg = compute_change_groups(&wt_hunks);
        let st_hunks = diff_buffers(head, head, None);
        let st_cg = compute_change_groups(&st_hunks);

        let result = mark_staged_groups(&wt_cg, &st_cg, &wt_hunks, &st_hunks);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].is_staged, Some(false));
    }

    // ── hidden ranges from content（diff_buffers + 純粋算出） ──

    #[test]
    fn test_非表示範囲_内容から_変更なし() {
        let text = "line1\nline2\nline3\n";
        let hunks = diff_buffers(text, text, None);
        let result = compute_hidden_ranges(&hunks, text.lines().count() as u32, 3);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start_line, 1);
    }

    #[test]
    fn test_非表示範囲_内容から_変更あり() {
        let lines: Vec<String> = (1..=30).map(|i| format!("line{i}")).collect();
        let original = lines.join("\n");
        let mut modified_lines = lines.clone();
        modified_lines[14] = "CHANGED15".to_string();
        let modified = modified_lines.join("\n");
        let hunks = diff_buffers(&original, &modified, None);
        let result = compute_hidden_ranges(&hunks, modified.lines().count() as u32, 3);
        assert!(
            !result.is_empty(),
            "expected hidden ranges for a 30-line file with change at line 15"
        );
    }

    // ── visible markdown blocks（diff_buffers + 純粋算出） ──

    #[test]
    fn test_可視ブロック_変更なし() {
        let text = "line1\nline2\nline3\n";
        let hunks = diff_buffers(text, text, None);
        let result = compute_visible_markdown_blocks(&hunks, text, text, 3);
        assert!(result.is_empty());
    }

    #[test]
    fn test_可視ブロック_単一変更() {
        let original = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let modified = "a\nb\nc\nd\nE\nf\ng\nh\ni\nj\n";
        let hunks = diff_buffers(original, modified, None);
        let result = compute_visible_markdown_blocks(&hunks, original, modified, 2);
        assert_eq!(result.len(), 1);
        assert!(result[0].content.contains('E'));
        assert!(result[0].start_line <= 5);
        assert!(result[0].end_line >= 5);
    }

    #[test]
    fn test_可視ブロック_複数変更() {
        let original = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
        let modified = original.replace("b\n", "B\n").replace("s\n", "S\n");
        let hunks = diff_buffers(original, &modified, None);
        let result = compute_visible_markdown_blocks(&hunks, original, &modified, 2);
        assert_eq!(result.len(), 2);
    }
}
