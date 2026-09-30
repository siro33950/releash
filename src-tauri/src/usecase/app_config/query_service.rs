use std::sync::Arc;

use crate::domain::app_config::repository::ConfigRepository;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkflowConfigDto {
    pub approval_auto_approve: bool,
}

pub(crate) trait WorkflowConfigQueryService: Send + Sync {
    fn get_workflow_config(
        &self,
    ) -> Result<WorkflowConfigDto, crate::domain::app_config::AppConfigError>;
}

pub struct AppConfigQueryService {
    repository: Arc<dyn ConfigRepository>,
    workflow: Arc<dyn WorkflowConfigQueryService>,
}

impl AppConfigQueryService {
    pub(crate) fn new(
        repository: Arc<dyn ConfigRepository>,
        workflow: Arc<dyn WorkflowConfigQueryService>,
    ) -> Self {
        Self {
            repository,
            workflow,
        }
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

    pub fn get_workflow_config(&self) -> Result<WorkflowConfigDto, UsecaseError> {
        Ok(self.workflow.get_workflow_config()?)
    }
}
