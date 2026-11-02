use std::path::{Path, PathBuf};

/// サーバが管理する定義と Facet の共通フォルダ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDirectory {
    path: PathBuf,
}

impl WorkflowDirectory {
    /// 定義の保管場所を表す。
    pub fn new(_path: PathBuf) -> Self {
        Self { path: _path }
    }

    /// 定義の保管場所を返す。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// フォルダを外部エディタで開く対象を返す。
    pub fn editor_paths(&self) -> [PathBuf; 1] {
        [self.path.clone()]
    }
}

#[cfg(test)]
#[path = "directory_test.rs"]
mod directory_tests;
