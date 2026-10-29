use std::sync::Arc;

use crate::domain::app_config::repository::AppConfigRepository;
use crate::usecase::app_config::error::UsecaseError;
use crate::usecase::app_config::query_service::{
    AppConfigQueryService, WorkflowConfigQueryService,
};
use crate::usecase::state_subscription::StateChangeSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowConfigInput {
    pub approval_auto_approve: bool,
}

pub struct AppConfigUsecase {
    config_mutation: Arc<parking_lot::Mutex<()>>,
    repository: Arc<dyn AppConfigRepository>,
    query: AppConfigQueryService,
    state_publisher: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
}

impl AppConfigUsecase {
    pub fn new(
        repository: Arc<dyn AppConfigRepository>,
        workflow_query: Arc<dyn WorkflowConfigQueryService>,
        terminal_query: Arc<dyn super::query_service::TerminalStartupCommandQueryService>,
        config_mutation: Arc<parking_lot::Mutex<()>>,
    ) -> Self {
        let query = AppConfigQueryService::new(repository.clone(), workflow_query, terminal_query);
        Self {
            config_mutation,
            repository,
            query,
            state_publisher: None,
        }
    }

    pub fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    pub(crate) fn desktop_settings(
        &self,
    ) -> Result<super::query_service::DesktopSettingsDto, UsecaseError> {
        self.query.desktop_settings()
    }

    pub fn get_workflow_config(
        &self,
    ) -> Result<super::query_service::WorkflowConfigDto, UsecaseError> {
        self.query.get_workflow_config()
    }

    pub fn terminal_startup_command(&self) -> Result<String, UsecaseError> {
        self.query.terminal_startup_command()
    }

    /// Terminal の起動コマンドの変更を保存する。
    pub fn update_terminal_startup_command(&self, command: String) -> Result<(), UsecaseError> {
        let guard = self.config_mutation.lock();
        let mut config = self.repository.load()?;
        config.update_terminal_startup_command(command)?;
        self.repository.save(config)?;
        drop(guard);

        if let Some(publisher) = &self.state_publisher {
            publisher.notify(StateChangeSource::AppConfig);
        }
        Ok(())
    }

    /// performance telemetry の有効・無効を保存する。
    pub fn update_performance_telemetry(&self, enabled: bool) -> Result<(), UsecaseError> {
        let guard = self.config_mutation.lock();
        let mut config = self.repository.load()?;
        config.update_performance_telemetry(enabled);
        self.repository.save(config)?;
        drop(guard);

        if let Some(publisher) = &self.state_publisher {
            publisher.notify(StateChangeSource::AppConfig);
        }
        Ok(())
    }

    /// 最小化状態で起動する設定の変更を保存する。
    pub fn update_app_settings(&self, start_minimized: bool) -> Result<(), UsecaseError> {
        let guard = self.config_mutation.lock();
        let mut config = self.repository.load()?;
        config.update_start_minimized(start_minimized);
        self.repository.save(config)?;
        drop(guard);

        if let Some(publisher) = &self.state_publisher {
            publisher.notify(StateChangeSource::AppConfig);
        }
        Ok(())
    }

    /// ログイン時に起動する設定の変更を保存する。
    pub fn update_login_item_preference(&self, requested: bool) -> Result<(), UsecaseError> {
        let guard = self.config_mutation.lock();
        let mut config = self.repository.load()?;
        config.update_auto_launch(requested);
        self.repository.save(config)?;
        drop(guard);

        if let Some(publisher) = &self.state_publisher {
            publisher.notify(StateChangeSource::AppConfig);
        }
        Ok(())
    }

    /// 承認の自動化の有効・無効を保存する。
    pub fn update_workflow_config(&self, input: WorkflowConfigInput) -> Result<(), UsecaseError> {
        let guard = self.config_mutation.lock();
        let mut config = self.repository.load()?;
        config.update_approval_auto_approve(input.approval_auto_approve);
        self.repository.save(config)?;
        drop(guard);

        if let Some(publisher) = &self.state_publisher {
            publisher.notify(StateChangeSource::AppConfig);
        }
        Ok(())
    }

    /// crash reporting の有効・無効を保存する。
    pub fn update_crash_reporting(&self, enabled: bool) -> Result<(), UsecaseError> {
        let guard = self.config_mutation.lock();
        let mut config = self.repository.load()?;
        config.update_crash_reporting(enabled);
        self.repository.save(config)?;
        drop(guard);

        if let Some(publisher) = &self.state_publisher {
            publisher.notify(StateChangeSource::AppConfig);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "usecase_test.rs"]
mod usecase_tests;
