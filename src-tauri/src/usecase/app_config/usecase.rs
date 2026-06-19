use std::sync::Arc;

use crate::domain::app_config::repository::ConfigRepository;
use crate::domain::app_config::services::generate_token;
use crate::domain::app_config::value_objects::{AppSettings, RemoteConfig, WorkflowConfig};
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
        let mut config = self.repository.load()?;
        config.telemetry_enabled = enabled;
        self.repository.save(config)?;
        Ok(())
    }

    pub fn update_server_port(&self, port: u16) -> Result<(), UsecaseError> {
        validate_port("server_port", port)?;
        let mut config = self.repository.load()?;
        config.server.port = port;
        self.repository.save(config)?;
        Ok(())
    }

    pub fn regenerate_token(&self) -> Result<String, UsecaseError> {
        let mut config = self.repository.load()?;
        config.server.token = generate_token();
        let token = config.server.token.clone();
        self.repository.save(config)?;
        Ok(token)
    }

    pub fn update_mcp_config(&self, port: u16, token: String) -> Result<(), UsecaseError> {
        let token = token.trim().to_string();
        validate_port("mcp_port", port)?;
        if token.is_empty() {
            return Err(UsecaseError::InvalidInput(
                "mcp_token must not be empty".to_string(),
            ));
        }
        let mut config = self.repository.load()?;
        config.server.mcp_port = port;
        config.server.mcp_token = token;
        self.repository.save(config)?;
        Ok(())
    }

    pub fn regenerate_mcp_token(&self) -> Result<String, UsecaseError> {
        let mut config = self.repository.load()?;
        config.server.mcp_token = generate_token();
        let token = config.server.mcp_token.clone();
        self.repository.save(config)?;
        Ok(token)
    }

    pub fn update_app_settings(&self, app: AppSettings) -> Result<(), UsecaseError> {
        let mut config = self.repository.load()?;
        config.app.close_to_tray = app.close_to_tray;
        config.app.auto_launch = app.auto_launch;
        config.app.start_minimized = app.start_minimized;
        config.app.agent_shortcuts = app.agent_shortcuts;
        self.repository.save(config)?;
        Ok(())
    }

    pub fn update_last_server_context(
        &self,
        last_root_path: String,
        last_bind_ip: String,
    ) -> Result<(), UsecaseError> {
        let mut config = self.repository.load()?;
        if !last_root_path.is_empty() && config.app.last_repo_paths.is_empty() {
            config.app.last_repo_paths = vec![last_root_path.clone()];
        }
        config.app.last_root_path = last_root_path;
        config.app.last_bind_ip = last_bind_ip;
        self.repository.save(config)?;
        Ok(())
    }

    pub fn update_remote_config(&self, remote: RemoteConfig) -> Result<(), UsecaseError> {
        let mut config = self.repository.load()?;
        config.remote = remote;
        self.repository.save(config)?;
        Ok(())
    }

    pub fn update_workflow_config(&self, workflow: WorkflowConfig) -> Result<(), UsecaseError> {
        let mut config = self.repository.load()?;
        config.workflow = workflow;
        self.repository.save(config)?;
        Ok(())
    }

    pub fn update_crash_reporting(&self, enabled: bool) -> Result<(), UsecaseError> {
        let mut config = self.repository.load()?;
        config.telemetry.crash_reporting = enabled;
        self.repository.save(config)?;
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
    use crate::domain::app_config::repository::ConfigRepository;
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
    }

    fn test_config() -> AppConfigDocument {
        AppConfigDocument {
            telemetry_enabled: true,
            server: ServerConfig {
                bind: "127.0.0.1".to_string(),
                port: 9700,
                hook_port: 19700,
                token: "server-token".to_string(),
                mcp_port: 19801,
                mcp_token: "mcp-token".to_string(),
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
            remote: RemoteConfig {
                auto_start: false,
                auto_start_on_lan: false,
            },
            app: AppSettings {
                close_to_tray: true,
                auto_launch: false,
                start_minimized: false,
                last_root_path: String::new(),
                last_repo_paths: vec![],
                last_bind_ip: String::new(),
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

    #[test]
    fn test_mcp設定更新_空トークンはエラー() {
        // Given
        let repo = Arc::new(FakeConfigRepository::new());
        let usecase = AppConfigUsecase::new(repo);

        // When
        let result = usecase.update_mcp_config(19801, " ".to_string());

        // Then
        assert!(matches!(result, Err(UsecaseError::InvalidInput(_))));
    }

    #[test]
    fn test_mcpトークン再生成_旧値と異なる値を保存する() {
        // Given
        let repo = Arc::new(FakeConfigRepository::new());
        let usecase = AppConfigUsecase::new(repo);
        let old = usecase.query().get_mcp_config().unwrap().token;

        // When
        let new = usecase.regenerate_mcp_token().unwrap();

        // Then
        assert_ne!(new, old);
        assert_eq!(usecase.query().get_mcp_config().unwrap().token, new);
    }
}
