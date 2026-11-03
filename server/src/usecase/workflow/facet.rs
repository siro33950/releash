use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::external_editor::{EditorLauncherGateway, EditorSettingsGateway};

use crate::domain::workflow::services::template_preview;
use crate::domain::workflow::{FacetKind, FacetRepository, WorkflowError};

#[derive(Clone)]
/// Facet の変更と外部編集を扱う。
pub struct WorkflowFacetUsecase {
    facets: Arc<dyn FacetRepository>,
    editor_settings: Arc<dyn EditorSettingsGateway>,
    editor_launcher: Arc<dyn EditorLauncherGateway>,
}

impl WorkflowFacetUsecase {
    /// Facet の取得、保存と外部編集の依存先を受け取る。
    pub fn new(
        facets: Arc<dyn FacetRepository>,
        editor_settings: Arc<dyn EditorSettingsGateway>,
        editor_launcher: Arc<dyn EditorLauncherGateway>,
    ) -> Self {
        Self {
            facets,
            editor_settings,
            editor_launcher,
        }
    }

    /// Facet に本文の更新可否を問い、保存する。
    pub fn save_facet(
        &self,
        kind: FacetKind,
        key: &str,
        content: &str,
        is_new: bool,
    ) -> Result<(), WorkflowError> {
        let mut facet = self.facets.get_or_new(kind, key)?;
        let existing_keys = self.facets.list(kind)?;
        facet.update_content(content, is_new, &existing_keys)?;
        self.facets.save(facet, is_new)
    }

    /// Facet に削除の可否を問い、削除する。
    pub fn delete_facet(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
        let facet = self.facets.get(kind, key)?;
        facet.ensure_deletable()?;
        self.facets.delete(&facet)
    }

    /// Facet に複製の可否を問い、複製を保存する。
    pub fn duplicate_facet(
        &self,
        kind: FacetKind,
        source_key: &str,
        new_key: &str,
    ) -> Result<(), WorkflowError> {
        let facet = self.facets.get(kind, source_key)?;
        let existing_keys = self.facets.list(kind)?;
        let duplicate = facet.duplicate(new_key, &existing_keys)?;
        self.facets.save(duplicate, true)
    }

    /// Facet を選択中の外部エディタで開く。
    pub fn open_in_editor(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
        let facet = self.facets.get(kind, key)?;
        let paths = facet.editor_paths()?;
        let editor = self
            .editor_settings
            .selected_editor()
            .map_err(WorkflowError::Editor)?;
        self.editor_launcher
            .open_paths(&paths, &editor, "ファセット")
            .map_err(WorkflowError::Editor)
    }

    pub fn render_facet_preview(
        &self,
        content: &str,
        sample_values: &HashMap<String, String>,
    ) -> String {
        template_preview::render_template_variables(content, sample_values)
    }
}

#[cfg(test)]
#[path = "facet_test.rs"]
pub(crate) mod facet_tests;
