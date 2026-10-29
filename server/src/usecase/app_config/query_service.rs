use std::sync::Arc;

use crate::domain::app_config::repository::AppConfigRepository;
use crate::usecase::app_config::error::UsecaseError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSettingsDto {
    pub close_to_tray: bool,
    pub start_minimized: bool,
    pub crash_reporting: bool,
    pub performance_telemetry: bool,
    pub auto_launch: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowConfigDto {
    pub approval_auto_approve: bool,
}

pub trait WorkflowConfigQueryService: Send + Sync {
    fn get_workflow_config(
        &self,
    ) -> Result<WorkflowConfigDto, crate::domain::app_config::AppConfigError>;
}

pub trait TerminalStartupCommandQueryService: Send + Sync {
    fn terminal_startup_command(&self)
        -> Result<String, crate::domain::app_config::AppConfigError>;
}

pub struct AppConfigQueryService {
    repository: Arc<dyn AppConfigRepository>,
    workflow: Arc<dyn WorkflowConfigQueryService>,
    terminal: Arc<dyn TerminalStartupCommandQueryService>,
}

impl AppConfigQueryService {
    pub(crate) fn new(
        repository: Arc<dyn AppConfigRepository>,
        workflow: Arc<dyn WorkflowConfigQueryService>,
        terminal: Arc<dyn TerminalStartupCommandQueryService>,
    ) -> Self {
        Self {
            repository,
            workflow,
            terminal,
        }
    }

    pub(crate) fn desktop_settings(&self) -> Result<DesktopSettingsDto, UsecaseError> {
        todo!()
    }

    pub fn terminal_startup_command(&self) -> Result<String, UsecaseError> {
        Ok(self.terminal.terminal_startup_command()?)
    }

    pub fn get_workflow_config(&self) -> Result<WorkflowConfigDto, UsecaseError> {
        Ok(self.workflow.get_workflow_config()?)
    }
}
