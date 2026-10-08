use crate::domain::external_editor::EditorError;
use std::sync::Arc;

use crate::domain::app_config::ConfigRepository;
use crate::domain::external_editor::EditorSettingsGateway;

#[derive(Clone)]
pub struct EditorSettingsConfigGateway {
    config: Arc<dyn ConfigRepository>,
}

impl EditorSettingsConfigGateway {
    pub fn new(config: Arc<dyn ConfigRepository>) -> Self {
        Self { config }
    }
}

impl EditorSettingsGateway for EditorSettingsConfigGateway {
    fn selected_editor(&self) -> Result<String, EditorError> {
        Ok(self
            .config
            .load()
            .map_err(EditorError::Settings)?
            .app
            .external_editor)
    }

    fn update_selected_editor(&self, editor: String) -> Result<(), EditorError> {
        let mut config = self.config.load().map_err(EditorError::Settings)?;
        config.app.external_editor = editor;
        self.config.save(config).map_err(EditorError::Settings)
    }
}
