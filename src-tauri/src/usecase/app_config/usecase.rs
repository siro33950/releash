use std::sync::Arc;

use crate::domain::app_config::repository::ConfigRepository;
use crate::domain::app_config::services::generate_token;
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

    pub fn query(&self) -> &AppConfigQueryService {
        &self.query
    }

    pub fn update_telemetry_enabled(&self, enabled: bool) -> Result<(), UsecaseError> {
        self.repository.update(Box::new(move |config| {
            config.telemetry_enabled = enabled;
            Ok(())
        }))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn update_server_port(&self, port: u16) -> Result<(), UsecaseError> {
        validate_port("server_port", port)?;
        self.repository.update(Box::new(move |config| {
            config.server.port = port;
            Ok(())
        }))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn regenerate_token(&self) -> Result<String, UsecaseError> {
        let token = generate_token();
        let next_token = token.clone();
        self.repository.update(Box::new(move |config| {
            config.server.token = next_token;
            Ok(())
        }))?;
        Ok(token)
    }

    pub fn update_app_settings(&self, app: AppSettings) -> Result<(), UsecaseError> {
        self.repository.update(Box::new(move |config| {
            config.app.close_to_tray = app.close_to_tray;
            config.app.auto_launch = app.auto_launch;
            config.app.start_minimized = app.start_minimized;
            config.app.agent_shortcuts = app.agent_shortcuts;
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

fn validate_port(name: &str, port: u16) -> Result<(), UsecaseError> {
    if port == 0 {
        return Err(UsecaseError::InvalidInput(format!(
            "{name} must be between 1 and 65535"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod app_config_usecase_tests {
    use std::sync::{Arc, Mutex};

    use crate::domain::app_config::error::AppConfigError;
    use crate::domain::app_config::repository::{ConfigRepository, ConfigUpdate};
    use crate::domain::app_config::value_objects::*;
    use crate::domain::notification::DesktopNotifyMode;

    use super::*;

    struct FakeConfigRepository {
        config: Mutex<AppConfigDocument>,
    }

    impl FakeConfigRepository {
        fn new() -> Self {
            Self {
                config: Mutex::new(test_config()),
            }
        }
    }

    impl ConfigRepository for FakeConfigRepository {
        fn load(&self) -> Result<AppConfigDocument, AppConfigError> {
            Ok(self.config.lock().unwrap().clone())
        }

        fn save(&self, config: AppConfigDocument) -> Result<(), AppConfigError> {
            *self.config.lock().unwrap() = config;
            Ok(())
        }

        fn update(&self, f: ConfigUpdate) -> Result<(), AppConfigError> {
            let mut config = self.config.lock().unwrap();
            f(&mut config)
        }
    }

    fn test_config() -> AppConfigDocument {
        AppConfigDocument {
            telemetry_enabled: true,
            server: ServerConfig {
                bind: "127.0.0.1".to_string(),
                port: 9700,
                hook_port: 19700,
                token: "server-token".to_string(),
                tls: TlsConfig {
                    enabled: false,
                    cert: String::new(),
                    key: String::new(),
                },
                notify: NotifyConfig {
                    webhook_url: String::new(),
                    on_running: false,
                    on_done: true,
                    on_error: true,
                    on_waiting: true,
                    desktop_mode: DesktopNotifyMode::Always,
                    inactive_timeout_minutes: 2,
                },
            },
            telemetry: TelemetryConfig {
                crash_reporting: true,
            },
            app: AppSettings {
                close_to_tray: true,
                auto_launch: false,
                start_minimized: false,
                last_root_path: String::new(),
                last_repo_paths: vec![],
                external_editor: String::new(),
                agent_shortcuts: AgentShortcutConfig::default(),
            },
            workflow: WorkflowConfig {
                approval_auto_approve: false,
            },
        }
    }

    #[test]
    fn test_サーバポート更新_以降の参照で新値になる() {
        // Given
        let repo = Arc::new(FakeConfigRepository::new());
        let usecase = AppConfigUsecase::new(repo);

        // When
        usecase.update_server_port(18080).unwrap();

        // Then
        assert_eq!(usecase.query().get_server_config().unwrap().port, 18080);
    }
}
