use std::path::PathBuf;

use crate::domain::code::services::directory::worktree_path_is_visible;
use crate::domain::external_editor::EditorError;

/// 外部エディタで開く対象。worktree のディレクトリか、worktree のディレクトリとその中のファイルである。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorTarget {
    /// worktree のディレクトリの絶対 path。
    worktree: PathBuf,
    /// worktree からの相対 path。
    file: Option<PathBuf>,
}

impl EditorTarget {
    /// worktree のディレクトリを対象にする。path が空なら `InvalidInput` で失敗する。
    pub fn worktree(worktree: &str) -> Result<Self, EditorError> {
        if worktree.is_empty() {
            return Err(EditorError::InvalidInput("worktree path is empty".into()));
        }
        Ok(Self {
            worktree: worktree.into(),
            file: None,
        })
    }

    /// worktree のディレクトリと、その中のファイルを対象にする。
    ///
    /// worktree の path が空なら `InvalidInput` で失敗する。
    /// `file` が worktree の外か `.git` を指すときも `InvalidInput` で失敗する。
    /// この判定は `domain::code::services::directory::worktree_path_is_visible` に従う。
    pub fn file(worktree: &str, file: &str) -> Result<Self, EditorError> {
        if !worktree_path_is_visible(file) {
            return Err(EditorError::InvalidInput(format!(
                "file is outside the worktree: {file}"
            )));
        }
        Ok(Self {
            file: Some(file.into()),
            ..Self::worktree(worktree)?
        })
    }

    /// 外部エディタへ一度に渡す path を決める。worktree のディレクトリを先に、ファイルの絶対 path を後にする。
    pub fn editor_paths(&self) -> Vec<PathBuf> {
        let file = self.file.iter().map(|file| self.worktree.join(file));
        std::iter::once(self.worktree.clone()).chain(file).collect()
    }

    /// 開けなかったときの文面で対象を呼ぶ語を決める。ディレクトリだけなら「フォルダ」、ファイルがあれば「ファイル」である。
    pub fn label(&self) -> &'static str {
        if self.file.is_some() {
            "ファイル"
        } else {
            "フォルダ"
        }
    }
}

#[cfg(test)]
#[path = "editor_target_test.rs"]
mod editor_target_tests;
