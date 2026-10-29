use crate::domain::external_editor::EditorError;
use std::sync::Arc;

use crate::domain::app_config::AppConfigRepository;
use crate::domain::external_editor::EditorSettingsGateway;

#[derive(Clone)]
pub struct EditorSettingsConfigGateway {
    config: Arc<dyn AppConfigRepository>,
}

impl EditorSettingsConfigGateway {
    pub fn new(config: Arc<dyn AppConfigRepository>) -> Self {
        Self { config }
    }
}

impl EditorSettingsGateway for EditorSettingsConfigGateway {
    fn selected_editor(&self) -> Result<String, EditorError> {
        Ok(self
            .config
            .load()
            .map_err(EditorError::Settings)?
            .external_editor()
            .to_owned())
    }
}
