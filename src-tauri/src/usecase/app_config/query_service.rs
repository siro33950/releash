use std::sync::Arc;

use crate::domain::app_config::repository::ConfigRepository;
use crate::domain::app_config::value_objects::WorkflowConfig;
use crate::usecase::app_config::error::UsecaseError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesktopSettingsDto {
    pub close_to_tray: bool,
    pub start_minimized: bool,
    pub crash_reporting: bool,
    pub performance_telemetry: bool,
    pub auto_launch: bool,
}

pub struct AppConfigQueryService {
    repository: Arc<dyn ConfigRepository>,
}

impl AppConfigQueryService {
    pub fn new(repository: Arc<dyn ConfigRepository>) -> Self {
        Self { repository }
    }

    pub(crate) fn desktop_settings(&self) -> Result<DesktopSettingsDto, UsecaseError> {
        let config = self.repository.load()?;
        Ok(DesktopSettingsDto {
            close_to_tray: config.app.close_to_tray,
            start_minimized: config.app.start_minimized,
            crash_reporting: config.telemetry.crash_reporting,
            performance_telemetry: config.telemetry.performance_telemetry,
            auto_launch: config.app.auto_launch,
        })
    }

    pub fn get_workflow_config(&self) -> Result<WorkflowConfig, UsecaseError> {
        Ok(self.repository.load()?.workflow)
    }
}
