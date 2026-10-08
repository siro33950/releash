//! 2 つのテキストバッファの差分計算（git2 依存）の gateway 実装。
//!
//! `git2::Patch::from_buffers` による unified diff を `Hunk` 列へ変換する部分のみを
//! 担う。hunk 区切り（change group）・range 算出・patch 生成といった後段の純粋ロジックは
//! ドメインサービス（`domain::code::services::hunk`）が担う。

use crate::adaptor::gateway::shared::git_operation;
use std::path::Path;

use crate::domain::code::{CodeError, DiffComputer, Hunk};

/// 2 つのバッファを diff して hunk 列を生成する。
/// git2 の通常エラーは従来どおり空の hunk 列とし、期限切れ・取消はエラーを返す。
pub fn diff_buffers(
    original: &str,
    modified: &str,
    file_path: Option<&str>,
) -> Result<Vec<Hunk>, CodeError> {
    let path = Path::new(file_path.unwrap_or("file"));
    let mut hunks: Vec<Hunk> = Vec::new();

    let patch = git_operation::optional(git_operation::run(|| {
        git2::Patch::from_buffers(
            original.as_bytes(),
            Some(path),
            modified.as_bytes(),
            Some(path),
            None,
        )
    }))?;

    if let Some(patch) = patch {
        let num_hunks = patch.num_hunks();
        for hunk_idx in 0..num_hunks {
            let Some((hdr, _)) =
                git_operation::optional(git_operation::run(|| patch.hunk(hunk_idx)))?
            else {
                continue;
            };
            let num_lines =
                git_operation::optional(git_operation::run(|| patch.num_lines_in_hunk(hunk_idx)))?
                    .unwrap_or(0);
            let mut lines: Vec<String> = Vec::new();

            for line_idx in 0..num_lines {
                let Some(line) = git_operation::optional(git_operation::run(|| {
                    patch.line_in_hunk(hunk_idx, line_idx)
                }))?
                else {
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
                hunk_id: String::new(),
                old_start: hdr.old_start(),
                old_lines: hdr.old_lines(),
                new_start: hdr.new_start(),
                new_lines: hdr.new_lines(),
                lines,
            });
        }
    }

    Ok(hunks)
}

/// `DiffComputer` の git2 実装。
pub struct DiffComputerGateway;

impl DiffComputer for DiffComputerGateway {
    fn diff_buffers(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
    ) -> Result<Vec<Hunk>, CodeError> {
        diff_buffers(original, modified, file_path)
    }
}

#[cfg(test)]
#[path = "diff_compute_test.rs"]
mod diff_compute_tests;
