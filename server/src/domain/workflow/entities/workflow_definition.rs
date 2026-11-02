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
        Self {
            file_stem: _file_stem,
            directory: _directory,
            source: _source,
            format: _format,
            content: _content,
        }
    }

    /// 解釈した内容から新しい利用者定義を作る。
    pub fn new(
        _directory: PathBuf,
        _content: WorkflowDefinitionContent,
    ) -> Result<Self, WorkflowError> {
        crate::domain::workflow::validation::validate_name(&_content.name)
            .map_err(|error| WorkflowError::validation(error.to_string()))?;
        Ok(Self {
            source: WorkflowSource::File {
                path: _directory.join(format!("{}.yml", _content.name)),
            },
            file_stem: _content.name.clone(),
            directory: _directory,
            format: WorkflowSourceFormat::Yaml,
            content: Ok(_content),
        })
    }

    /// 保管場所での識別名を返す。
    pub fn file_stem(&self) -> &str {
        &self.file_stem
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

    /// 取得元、形式、名前と保存先の重複から保存の可否を決める。
    pub fn validate_save(
        &self,
        _original: Option<&Self>,
        _existing: &[WorkflowSummary],
    ) -> Result<(), WorkflowError> {
        if self.source.is_builtin()
            || _original.is_some_and(|original| original.source.is_builtin())
        {
            return Err(WorkflowError::validation(
                "ビルトインワークフローは編集できません",
            ));
        }
        if _original.is_some_and(|original| original.format == WorkflowSourceFormat::Lua) {
            return Err(WorkflowError::validation(
                "Lua workflow は保存できません。外部エディタで編集してください",
            ));
        }
        let content = self.content().map_err(Clone::clone)?;
        crate::domain::workflow::validation::validate_name(&content.name)
            .map_err(|error| WorkflowError::validation(error.to_string()))?;
        if _original.is_none_or(|original| original.file_stem != self.file_stem)
            && _existing.iter().any(|summary| summary.name == content.name)
        {
            return Err(WorkflowError::validation(format!(
                "ワークフロー '{}' は既に存在します",
                content.name
            )));
        }
        Ok(())
    }

    /// 保存の成功後に不要になる元のファイルを決める。
    pub fn obsolete_path(&self, _original_name: Option<&str>) -> Option<PathBuf> {
        _original_name
            .filter(|name| *name != self.file_stem)
            .map(|name| self.directory.join(format!("{name}.yml")))
    }

    /// 取得元から削除の可否を決める。
    pub fn ensure_deletable(&self) -> Result<(), WorkflowError> {
        if self.source.is_builtin() {
            return Err(WorkflowError::validation(
                "ビルトインワークフローは削除できません",
            ));
        }
        Ok(())
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

    /// 定義の取得元、解釈結果と実行状況から一覧の項目を決める。
    pub fn summary(
        &self,
        _running: &[crate::domain::workflow::WorkflowExecutionSummary],
    ) -> WorkflowSummary {
        let (description, failure) = match &self.content {
            Ok(content) => (content.description.clone(), None),
            Err(error) => (
                if matches!(error, WorkflowError::Validation(message) if message.starts_with("workflow_diagnostics:"))
                {
                    "Invalid workflow definition".to_string()
                } else {
                    String::new()
                },
                Some(crate::domain::failure::WorkFailure::from_error(error)),
            ),
        };
        WorkflowSummary {
            name: self.file_stem.clone(),
            builtin: self.source.is_builtin(),
            source_format: self.format,
            is_running: _running
                .iter()
                .any(|running| running.workflow_name == self.file_stem),
            description,
            failure,
        }
    }

    /// builtin を拒否し、外部編集に使うフォルダとファイルを決める。
    pub fn editor_paths(&self) -> Result<[PathBuf; 2], WorkflowError> {
        Ok([self.directory.clone(), self.file_path()?.to_path_buf()])
    }

    /// 複製の可否を判断し、指定された名前の利用者定義を作る。
    pub fn duplicate(&self, _new_name: &str) -> Result<Self, WorkflowError> {
        if self.format == WorkflowSourceFormat::Lua {
            return Err(WorkflowError::validation(
                "Lua workflow は複製できません。外部エディタで編集してください",
            ));
        }
        let mut content = self.content().map_err(Clone::clone)?.clone();
        content.name = _new_name.to_string();
        Self::new(self.directory.clone(), content)
    }
}

#[cfg(test)]
#[path = "workflow_definition_test.rs"]
mod workflow_definition_tests;
