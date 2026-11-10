use std::path::{Path, PathBuf};

use crate::domain::workflow::{FacetKey, FacetKind, WorkflowError, WorkflowSource};

/// 種類と key で識別され、本文と編集元を所有する Facet。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facet {
    kind: FacetKind,
    key: FacetKey,
    directory: PathBuf,
    source: WorkflowSource,
    content: String,
}

impl Facet {
    /// 取得した Facet の識別子、本文と編集元を復元する。
    pub fn restore(
        kind: FacetKind,
        key: FacetKey,
        directory: PathBuf,
        source: WorkflowSource,
        content: String,
    ) -> Self {
        Self {
            kind,
            key,
            directory,
            source,
            content,
        }
    }

    /// 取得元から利用者ファイルの場所を返し、組み込みの Facet を拒否する。
    pub fn file_path(&self) -> Result<&Path, WorkflowError> {
        match &self.source {
            WorkflowSource::Builtin => Err(WorkflowError::validation(
                "ビルトインファセットは外部エディタで開けません",
            )),
            WorkflowSource::File { path } => Ok(path),
        }
    }

    /// builtin を拒否し、外部編集に使うフォルダとファイルを決める。
    pub fn editor_paths(&self) -> Result<[PathBuf; 2], WorkflowError> {
        Ok([self.directory.clone(), self.file_path()?.to_path_buf()])
    }
}

#[cfg(test)]
#[path = "facet_test.rs"]
mod facet_tests;
