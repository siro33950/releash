use std::path::{Path, PathBuf};

use crate::domain::workflow::{
    WorkflowDefinitionContent, WorkflowError, WorkflowSource, WorkflowSourceFormat,
};

/// 編集元と解釈結果を所有する workflow 定義。
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowDefinition {
    directory: PathBuf,
    source: WorkflowSource,
    format: WorkflowSourceFormat,
    content: Result<WorkflowDefinitionContent, WorkflowError>,
}

impl WorkflowDefinition {
    /// 取得した定義の編集元と解釈結果を復元する。
    pub fn restore(
        directory: PathBuf,
        source: WorkflowSource,
        format: WorkflowSourceFormat,
        content: Result<WorkflowDefinitionContent, WorkflowError>,
    ) -> Self {
        Self {
            directory,
            source,
            format,
            content,
        }
    }

    /// 実際に取得した編集元を返す。
    pub fn source(&self) -> &WorkflowSource {
        &self.source
    }

    /// 定義の形式を返す。
    pub fn format(&self) -> WorkflowSourceFormat {
        self.format
    }

    /// 解釈した内容、または解釈できなかった理由を返す。
    pub fn content(&self) -> Result<&WorkflowDefinitionContent, &WorkflowError> {
        self.content.as_ref()
    }

    /// 取得元から利用者ファイルの場所を返し、組み込みの定義を拒否する。
    pub fn file_path(&self) -> Result<&Path, WorkflowError> {
        match &self.source {
            WorkflowSource::Builtin => Err(WorkflowError::validation(
                "ビルトインワークフローは外部エディタで開けません",
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
#[path = "workflow_definition_test.rs"]
mod workflow_definition_tests;
