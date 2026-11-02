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
        _kind: FacetKind,
        _key: FacetKey,
        _directory: PathBuf,
        _source: WorkflowSource,
        _content: String,
    ) -> Self {
        Self {
            kind: _kind,
            key: _key,
            directory: _directory,
            source: _source,
            content: _content,
        }
    }

    /// Facet の種類を返す。
    pub fn kind(&self) -> FacetKind {
        self.kind
    }

    /// Facet の識別子を返す。
    pub fn key(&self) -> &FacetKey {
        &self.key
    }

    /// 実際に取得した編集元を返す。
    pub fn source(&self) -> &WorkflowSource {
        &self.source
    }

    /// Facet の本文を返す。
    pub fn content(&self) -> &str {
        &self.content
    }

    /// 取得元、本文の変数と新規保存の重複を検証して本文を変更する。
    pub fn update_content(
        &mut self,
        _content: &str,
        _is_new: bool,
        _existing_keys: &[String],
    ) -> Result<(), WorkflowError> {
        if self.source.is_builtin() {
            return Err(WorkflowError::validation(
                "ビルトインファセットは編集できません",
            ));
        }
        let undefined: Vec<_> =
            crate::domain::workflow::services::reference::extract_template_references(_content)
                .into_iter()
                .filter(|value| {
                    crate::domain::workflow::services::reference::split_reference(value).is_none()
                })
                .collect();
        if !undefined.is_empty() {
            return Err(WorkflowError::validation(format!(
                "未定義のテンプレート変数が含まれています: {}",
                undefined.join(", ")
            )));
        }
        if _is_new && _existing_keys.iter().any(|key| key == self.key.as_str()) {
            return Err(WorkflowError::validation(format!(
                "ファセット '{}' は既に存在します",
                self.key.as_str()
            )));
        }
        self.content = _content.to_string();
        Ok(())
    }

    /// 取得元から削除の可否を決める。
    pub fn ensure_deletable(&self) -> Result<(), WorkflowError> {
        if self.source.is_builtin() {
            return Err(WorkflowError::validation(
                "ビルトインファセットは削除できません",
            ));
        }
        Ok(())
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

    /// Facet の取得元と本文から一覧の項目を決める。
    pub fn summary(&self) -> crate::domain::workflow::FacetSummary {
        let description = self
            .content
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| line.strip_prefix("# ").unwrap_or(line).trim().to_string())
            .unwrap_or_default();
        crate::domain::workflow::FacetSummary {
            key: self.key.as_str().to_string(),
            kind: self.kind.canonical_name().to_string(),
            description,
            builtin: self.source.is_builtin(),
        }
    }

    /// builtin を拒否し、外部編集に使うフォルダとファイルを決める。
    pub fn editor_paths(&self) -> Result<[PathBuf; 2], WorkflowError> {
        Ok([self.directory.clone(), self.file_path()?.to_path_buf()])
    }

    /// 複製先の key と重複を検証し、利用者の Facet を作る。
    pub fn duplicate(
        &self,
        _new_key: &str,
        _existing_keys: &[String],
    ) -> Result<Self, WorkflowError> {
        let key = FacetKey::new(_new_key)?;
        if _existing_keys
            .iter()
            .any(|existing| existing == key.as_str())
        {
            return Err(WorkflowError::validation(format!(
                "ファセット '{_new_key}' は既に存在します"
            )));
        }
        Ok(Self {
            kind: self.kind,
            key,
            directory: self.directory.clone(),
            source: WorkflowSource::File {
                path: self
                    .directory
                    .join(self.kind.dir_name())
                    .join(format!("{_new_key}.md")),
            },
            content: self.content.clone(),
        })
    }
}

#[cfg(test)]
#[path = "facet_test.rs"]
mod facet_tests;
