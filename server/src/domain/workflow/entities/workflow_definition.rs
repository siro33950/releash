use std::path::{Path, PathBuf};

use crate::domain::workflow::{
    WorkflowDefinitionContent, WorkflowError, WorkflowSource, WorkflowSourceFormat, WorkflowSummary,
};

/// 編集元と解釈結果を所有する workflow 定義。
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowDefinition {
    file_stem: String,
    directory: PathBuf,
    source: WorkflowSource,
    format: WorkflowSourceFormat,
    content: Result<WorkflowDefinitionContent, WorkflowError>,
}

impl WorkflowDefinition {
    /// 取得した定義の編集元と解釈結果を復元する。
    pub fn restore(
        _file_stem: String,
        _directory: PathBuf,
        _source: WorkflowSource,
        _format: WorkflowSourceFormat,
        _content: Result<WorkflowDefinitionContent, WorkflowError>,
    ) -> Self {
        todo!()
    }

    /// 解釈した内容から新しい利用者定義を作る。
    pub fn new(
        _directory: PathBuf,
        _content: WorkflowDefinitionContent,
    ) -> Result<Self, WorkflowError> {
        todo!()
    }

    /// 保管場所での識別名を返す。
    pub fn file_stem(&self) -> &str {
        todo!()
    }

    /// 実際に取得した編集元を返す。
    pub fn source(&self) -> &WorkflowSource {
        todo!()
    }

    /// 定義の形式を返す。
    pub fn format(&self) -> WorkflowSourceFormat {
        todo!()
    }

    /// 解釈した内容、または解釈できなかった理由を返す。
    pub fn content(&self) -> Result<&WorkflowDefinitionContent, &WorkflowError> {
        todo!()
    }

    /// 取得元、形式、名前と保存先の重複から保存の可否を決める。
    pub fn validate_save(
        &self,
        _original: Option<&Self>,
        _existing: &[WorkflowSummary],
    ) -> Result<(), WorkflowError> {
        todo!()
    }

    /// 保存の成功後に不要になる元のファイルを決める。
    pub fn obsolete_path(&self, _original_name: Option<&str>) -> Option<PathBuf> {
        todo!()
    }

    /// 取得元から削除の可否を決める。
    pub fn ensure_deletable(&self) -> Result<(), WorkflowError> {
        todo!()
    }

    /// 取得元から利用者ファイルの場所を返し、組み込みの定義を拒否する。
    pub fn file_path(&self) -> Result<&Path, WorkflowError> {
        todo!()
    }

    /// 定義の取得元、解釈結果と実行状況から一覧の項目を決める。
    pub fn summary(
        &self,
        _running: &[crate::domain::workflow::WorkflowExecutionSummary],
    ) -> WorkflowSummary {
        todo!()
    }

    /// builtin を拒否し、外部編集に使うフォルダとファイルを決める。
    pub fn editor_paths(&self) -> Result<[PathBuf; 2], WorkflowError> {
        todo!()
    }

    /// 複製の可否を判断し、指定された名前の利用者定義を作る。
    pub fn duplicate(&self, _new_name: &str) -> Result<Self, WorkflowError> {
        todo!()
    }
}
