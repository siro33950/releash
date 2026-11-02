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
        content: &str,
        is_new: bool,
        existing_keys: &[String],
    ) -> Result<(), WorkflowError> {
        if self.source.is_builtin() {
            return Err(WorkflowError::validation(
                "ビルトインファセットは編集できません",
            ));
        }
        let undefined: Vec<_> =
            crate::domain::workflow::services::reference::extract_template_references(content)
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
        if is_new && existing_keys.iter().any(|key| key == self.key.as_str()) {
            return Err(WorkflowError::validation(format!(
                "ファセット '{}' は既に存在します",
                self.key.as_str()
            )));
        }
        self.content = content.to_string();
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

    /// builtin を拒否し、外部編集に使うフォルダとファイルを決める。
    pub fn editor_paths(&self) -> Result<[PathBuf; 2], WorkflowError> {
        Ok([self.directory.clone(), self.file_path()?.to_path_buf()])
    }

    /// 複製先の key と重複を検証し、利用者の Facet を作る。
    pub fn duplicate(
        &self,
        new_key: &str,
        existing_keys: &[String],
    ) -> Result<Self, WorkflowError> {
        let key = FacetKey::new(new_key)?;
        if existing_keys
            .iter()
            .any(|existing| existing == key.as_str())
        {
            return Err(WorkflowError::validation(format!(
                "ファセット '{new_key}' は既に存在します"
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
                    .join(format!("{new_key}.md")),
            },
            content: self.content.clone(),
        })
    }
}

#[cfg(test)]
#[path = "facet_test.rs"]
mod facet_tests;
