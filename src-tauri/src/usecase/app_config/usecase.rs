use std::sync::Arc;

use crate::domain::app_config::repository::ConfigRepository;
use crate::domain::app_config::value_objects::{AppSettings, WorkflowConfig};
use crate::usecase::app_config::error::UsecaseError;
use crate::usecase::app_config::query_service::AppConfigQueryService;

pub struct AppConfigUsecase {
    repository: Arc<dyn ConfigRepository>,
    query: AppConfigQueryService,
}

impl AppConfigUsecase {
    pub fn new(repository: Arc<dyn ConfigRepository>) -> Self {
        let query = AppConfigQueryService::new(repository.clone());
        Self { repository, query }
    }

    pub(crate) fn desktop_settings(
        &self,
    ) -> Result<super::query_service::DesktopSettingsDto, UsecaseError> {
        self.query.desktop_settings()
    }

    pub fn get_app_settings(&self) -> Result<AppSettings, UsecaseError> {
        self.query.get_app_settings()
    }
    pub fn get_workflow_config(&self) -> Result<WorkflowConfig, UsecaseError> {
        self.query.get_workflow_config()
    }
    pub fn get_crash_reporting_enabled(&self) -> Result<bool, UsecaseError> {
        self.query.get_crash_reporting_enabled()
    }
    pub fn get_performance_telemetry_enabled(&self) -> Result<bool, UsecaseError> {
        self.query.get_performance_telemetry_enabled()
    }

    pub fn update_performance_telemetry(&self, enabled: bool) -> Result<(), UsecaseError> {
        self.repository.update(Box::new(move |config| {
            config.telemetry.performance_telemetry = enabled;
            Ok(())
        }))?;
        Ok(())
    }

    pub fn update_app_settings(&self, app: AppSettings) -> Result<(), UsecaseError> {
        self.repository.update(Box::new(move |config| {
            config.app.close_to_tray = app.close_to_tray;
            config.app.auto_launch = app.auto_launch;
            config.app.start_minimized = app.start_minimized;
            Ok(())
        }))?;
        Ok(())
    }

    pub fn update_workflow_config(&self, workflow: WorkflowConfig) -> Result<(), UsecaseError> {
        self.repository.update(Box::new(move |config| {
            config.workflow = workflow;
            Ok(())
        }))?;
        Ok(())
    }

    pub fn update_crash_reporting(&self, enabled: bool) -> Result<(), UsecaseError> {
        self.repository.update(Box::new(move |config| {
            config.telemetry.crash_reporting = enabled;
            Ok(())
        }))?;
        Ok(())
    }
}
