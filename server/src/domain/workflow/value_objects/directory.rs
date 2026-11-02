use std::path::{Path, PathBuf};

/// サーバが管理する定義と Facet の共通フォルダ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDirectory {
    path: PathBuf,
}

impl WorkflowDirectory {
    /// 定義の保管場所を表す。
    pub fn new(_path: PathBuf) -> Self {
        todo!()
    }

    /// 定義の保管場所を返す。
    pub fn path(&self) -> &Path {
        todo!()
    }

    /// フォルダを外部エディタで開く対象を返す。
    pub fn editor_paths(&self) -> [PathBuf; 1] {
        todo!()
    }
}
