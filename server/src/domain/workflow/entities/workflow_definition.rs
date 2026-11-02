use std::path::{Path, PathBuf};

use crate::domain::workflow::{
    WorkflowDefinitionContent, WorkflowError, WorkflowSource, WorkflowSourceFormat,
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
        file_stem: String,
        directory: PathBuf,
        source: WorkflowSource,
        format: WorkflowSourceFormat,
        content: Result<WorkflowDefinitionContent, WorkflowError>,
    ) -> Self {
        Self {
            file_stem,
            directory,
            source,
            format,
            content,
        }
    }

    /// 解釈した内容から新しい利用者定義を作る。
    pub fn new(
        directory: PathBuf,
        content: WorkflowDefinitionContent,
    ) -> Result<Self, WorkflowError> {
        crate::domain::workflow::validation::validate_name(&content.name)
            .map_err(|error| WorkflowError::validation(error.to_string()))?;
        Ok(Self {
            source: WorkflowSource::File {
                path: directory.join(format!("{}.yml", content.name)),
            },
            file_stem: content.name.clone(),
            directory,
            format: WorkflowSourceFormat::Yaml,
            content: Ok(content),
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
        _existing_names: &[String],
    ) -> Result<(), WorkflowError> {
        todo!()
    }

    /// 保存の成功後に不要になる元のファイルを決める。
    pub fn obsolete_path(&self, original_name: Option<&str>) -> Option<PathBuf> {
        original_name
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

    /// builtin を拒否し、外部編集に使うフォルダとファイルを決める。
    pub fn editor_paths(&self) -> Result<[PathBuf; 2], WorkflowError> {
        Ok([self.directory.clone(), self.file_path()?.to_path_buf()])
    }

    /// 複製の可否を判断し、指定された名前の利用者定義を作る。
    pub fn duplicate(&self, new_name: &str) -> Result<Self, WorkflowError> {
        if self.format == WorkflowSourceFormat::Lua {
            return Err(WorkflowError::validation(
                "Lua workflow は複製できません。外部エディタで編集してください",
            ));
        }
        let mut content = self.content().map_err(Clone::clone)?.clone();
        content.name = new_name.to_string();
        Self::new(self.directory.clone(), content)
    }
}

#[cfg(test)]
#[path = "workflow_definition_test.rs"]
mod workflow_definition_tests;
