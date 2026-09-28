use std::sync::Arc;

use crate::domain::app_config::error::AppConfigError;
use crate::domain::app_config::repository::ConfigRepository;
use crate::domain::app_config::value_objects::{AppConfigDocument, WorkflowConfig};
use crate::usecase::app_config::error::UsecaseError;
use crate::usecase::app_config::query_service::AppConfigQueryService;

pub struct AppConfigUsecase {
    repository: Arc<dyn ConfigRepository>,
    query: AppConfigQueryService,
    state_publisher: Option<crate::usecase::state_subscription::StateSubscriptionOutputRef>,
}

impl AppConfigUsecase {
    pub fn new(repository: Arc<dyn ConfigRepository>) -> Self {
        let query = AppConfigQueryService::new(repository.clone());
        Self {
            repository,
            query,
            state_publisher: None,
        }
    }

    pub(crate) fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionOutputRef,
    ) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    pub(crate) fn desktop_settings(
        &self,
    ) -> Result<super::query_service::DesktopSettingsDto, UsecaseError> {
        self.query.desktop_settings()
    }

    pub fn get_workflow_config(&self) -> Result<WorkflowConfig, UsecaseError> {
        self.query.get_workflow_config()
    }

    fn update(
        &self,
        mutate: impl FnOnce(&mut AppConfigDocument) -> Result<(), AppConfigError> + Send + 'static,
    ) -> Result<(), UsecaseError> {
        self.repository.update(Box::new(mutate))?;
        if let Some(publisher) = &self.state_publisher {
            publisher.invalidate(crate::usecase::state_subscription::StateChangeSource::AppConfig);
        }
        Ok(())
    }

    pub fn update_performance_telemetry(&self, enabled: bool) -> Result<(), UsecaseError> {
        self.update(move |config| {
            config.telemetry.performance_telemetry = enabled;
            Ok(())
        })
    }

    pub fn update_app_settings(
        &self,
        close_to_tray: bool,
        start_minimized: bool,
    ) -> Result<(), UsecaseError> {
        self.update(move |config| {
            config.app.close_to_tray = close_to_tray;
            config.app.start_minimized = start_minimized;
            Ok(())
        })
    }

    pub fn update_login_item_preference(&self, requested: bool) -> Result<(), UsecaseError> {
        self.update(move |config| {
            config.app.auto_launch = requested;
            Ok(())
        })
    }

    pub fn update_workflow_config(&self, workflow: WorkflowConfig) -> Result<(), UsecaseError> {
        self.update(move |config| {
            config.workflow = workflow;
            Ok(())
        })
    }

    pub fn update_crash_reporting(&self, enabled: bool) -> Result<(), UsecaseError> {
        self.update(move |config| {
            config.telemetry.crash_reporting = enabled;
            Ok(())
        })
    }
}

#[cfg(test)]
#[path = "usecase_test.rs"]
mod usecase_tests;
