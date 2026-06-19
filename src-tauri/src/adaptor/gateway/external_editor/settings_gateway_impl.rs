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
    fn selected_editor(&self) -> Result<String, String> {
        Ok(self
            .config
            .load()
            .map_err(|e| e.to_string())?
            .app
            .external_editor)
    }

    fn update_selected_editor(&self, editor: String) -> Result<(), String> {
        let mut config = self.config.load().map_err(|e| e.to_string())?;
        config.app.external_editor = editor;
        self.config.save(config).map_err(|e| e.to_string())
    }
}
