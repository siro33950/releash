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
        todo!()
    }

    /// Facet の種類を返す。
    pub fn kind(&self) -> FacetKind {
        todo!()
    }

    /// Facet の識別子を返す。
    pub fn key(&self) -> &FacetKey {
        todo!()
    }

    /// 実際に取得した編集元を返す。
    pub fn source(&self) -> &WorkflowSource {
        todo!()
    }

    /// Facet の本文を返す。
    pub fn content(&self) -> &str {
        todo!()
    }

    /// 取得元、本文の変数と新規保存の重複を検証して本文を変更する。
    pub fn update_content(
        &mut self,
        _content: &str,
        _is_new: bool,
        _existing_keys: &[String],
    ) -> Result<(), WorkflowError> {
        todo!()
    }

    /// 取得元から削除の可否を決める。
    pub fn ensure_deletable(&self) -> Result<(), WorkflowError> {
        todo!()
    }

    /// 取得元から利用者ファイルの場所を返し、組み込みの Facet を拒否する。
    pub fn file_path(&self) -> Result<&Path, WorkflowError> {
        todo!()
    }

    /// Facet の取得元と本文から一覧の項目を決める。
    pub fn summary(&self) -> crate::domain::workflow::FacetSummary {
        todo!()
    }

    /// builtin を拒否し、外部編集に使うフォルダとファイルを決める。
    pub fn editor_paths(&self) -> Result<[PathBuf; 2], WorkflowError> {
        todo!()
    }

    /// 複製先の key と重複を検証し、利用者の Facet を作る。
    pub fn duplicate(
        &self,
        _new_key: &str,
        _existing_keys: &[String],
    ) -> Result<Self, WorkflowError> {
        todo!()
    }
}
