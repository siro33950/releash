use std::sync::Arc;

use crate::domain::external_editor::{EditorLauncherGateway, EditorSettingsGateway};
use crate::domain::workflow::WorkflowDirectoryService;

use crate::domain::workflow::{WorkflowDefinitionRepository, WorkflowError};

#[derive(Clone)]
/// 定義の変更と外部編集を扱う。
pub struct WorkflowDefinitionUsecase {
    definitions: Arc<dyn WorkflowDefinitionRepository>,
    editor_settings: Arc<dyn EditorSettingsGateway>,
    editor_launcher: Arc<dyn EditorLauncherGateway>,
    directory_service: Arc<dyn WorkflowDirectoryService>,
}

impl WorkflowDefinitionUsecase {
    /// 定義の取得、保存と外部編集の依存先を受け取る。
    pub fn new(
        definitions: Arc<dyn WorkflowDefinitionRepository>,
        editor_settings: Arc<dyn EditorSettingsGateway>,
        editor_launcher: Arc<dyn EditorLauncherGateway>,
        directory_service: Arc<dyn WorkflowDirectoryService>,
    ) -> Self {
        Self {
            definitions,
            editor_settings,
            editor_launcher,
            directory_service,
        }
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
