use std::sync::Arc;

use crate::domain::external_editor::{EditorLauncherGateway, EditorSettingsGateway};

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
}

#[cfg(test)]
#[path = "facet_test.rs"]
pub(crate) mod facet_tests;
