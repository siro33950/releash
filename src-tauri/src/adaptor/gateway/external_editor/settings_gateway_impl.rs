use std::sync::Arc;

use crate::config::AppConfig;
use crate::domain::external_editor::EditorSettingsGateway;

#[derive(Clone)]
pub struct EditorSettingsConfigGateway {
    config: Arc<AppConfig>,
}

impl EditorSettingsConfigGateway {
    pub fn new(config: Arc<AppConfig>) -> Self {
        Self { config }
    }
}

impl EditorSettingsGateway for EditorSettingsConfigGateway {
    fn selected_editor(&self) -> Result<String, String> {
        Ok(self.config.get_config()?.app.external_editor)
    }

    fn update_selected_editor(&self, editor: String) -> Result<(), String> {
        self.config.with_config_mut(|config| {
            config.app.external_editor = editor;
            Ok(())
        })
    }
}
