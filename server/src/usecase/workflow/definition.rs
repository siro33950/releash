use std::sync::Arc;

use crate::domain::external_editor::{EditorLauncherGateway, EditorSettingsGateway};
use crate::domain::workflow::WorkflowDirectoryService;

use crate::domain::workflow::{WorkflowDefinition, WorkflowDefinitionRepository, WorkflowError};
use crate::domain::workflow::{WorkflowDefinitionSourceService, WorkflowSourceSaveError};

#[derive(Clone)]
/// 定義の変更と外部編集を扱う。
pub struct WorkflowDefinitionUsecase {
    definitions: Arc<dyn WorkflowDefinitionRepository>,
    definition_sources: Arc<dyn WorkflowDefinitionSourceService>,
    editor_settings: Arc<dyn EditorSettingsGateway>,
    editor_launcher: Arc<dyn EditorLauncherGateway>,
    directory_service: Arc<dyn WorkflowDirectoryService>,
}

impl WorkflowDefinitionUsecase {
    /// 定義の取得、保存と外部編集の依存先を受け取る。
    pub fn new(
        definitions: Arc<dyn WorkflowDefinitionRepository>,
        definition_sources: Arc<dyn WorkflowDefinitionSourceService>,
        editor_settings: Arc<dyn EditorSettingsGateway>,
        editor_launcher: Arc<dyn EditorLauncherGateway>,
        directory_service: Arc<dyn WorkflowDirectoryService>,
    ) -> Self {
        Self {
            definitions,
            definition_sources,
            editor_settings,
            editor_launcher,
            directory_service,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    /// 入力された定義を検証し、保存する。
    pub fn save_workflow_source(
        &self,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        let definition = self
            .definition_sources
            .parse(source)
            .map_err(source_save_error_to_workflow_error)?;
        let original = self.definitions.find(original_name)?;
        let existing = self.definitions.list(&[])?;
        definition.validate_save(original.as_ref(), &existing)?;
        self.definitions
            .save_source(&definition, source, original_name)?;
        Ok(definition)
    }

    /// 入力された定義を検証して保存し、拒否理由を診断として返す。
    pub fn save_workflow_source_with_diagnostics(
        &self,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowSourceSaveError> {
        let definition = self.definition_sources.parse(source)?;
        let original = self
            .definitions
            .find(original_name)
            .map_err(WorkflowSourceSaveError::Workflow)?;
        let existing = self
            .definitions
            .list(&[])
            .map_err(WorkflowSourceSaveError::Workflow)?;
        definition
            .validate_save(original.as_ref(), &existing)
            .map_err(WorkflowSourceSaveError::Workflow)?;
        self.definitions
            .save_source(&definition, source, original_name)
            .map_err(WorkflowSourceSaveError::Workflow)?;
        Ok(definition)
    }

    /// 定義に削除の可否を問い、削除する。
    pub fn delete_workflow(&self, name: &str) -> Result<(), WorkflowError> {
        let definition = self.definitions.get(name)?;
        definition.ensure_deletable()?;
        self.definitions.delete(&definition)
    }

    /// 指定された定義を複製して保存する。
    pub fn duplicate_workflow(
        &self,
        source_name: &str,
        new_name: &str,
    ) -> Result<(), WorkflowError> {
        let workflow = self.definitions.get(source_name)?;
        let duplicate = workflow.duplicate(new_name)?;
        let existing = self.definitions.list(&[])?;
        duplicate.validate_save(None, &existing)?;
        self.definitions.save(duplicate, None)
    }

    /// 定義を選択中の外部エディタで開く。
    pub fn open_in_editor(&self, name: &str) -> Result<(), WorkflowError> {
        let workflow = self.definitions.get(name)?;
        let paths = workflow.editor_paths()?;
        let editor = self
            .editor_settings
            .selected_editor()
            .map_err(WorkflowError::Editor)?;
        self.editor_launcher
            .open_paths(&paths, &editor, "ワークフロー")
            .map_err(WorkflowError::Editor)
    }

    /// 定義フォルダを用意し、選択中の外部エディタで開く。
    pub fn open_directory_in_editor(&self) -> Result<(), WorkflowError> {
        let directory = self.directory_service.prepare()?;
        let paths = directory.editor_paths();
        let editor = self
            .editor_settings
            .selected_editor()
            .map_err(WorkflowError::Editor)?;
        self.editor_launcher
            .open_paths(&paths, &editor, "フォルダ")
            .map_err(WorkflowError::Editor)
    }
}

#[cfg(test)]
#[path = "definition_test.rs"]
mod definition_tests;

#[cfg(any(test, feature = "test-support"))]
fn source_save_error_to_workflow_error(_error: WorkflowSourceSaveError) -> WorkflowError {
    todo!()
}
