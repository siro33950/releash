use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::domain::app_config::value_objects as domain_vo;
use crate::domain::notification::DesktopNotifyMode as DomainDesktopNotifyMode;
use crate::notion::types::NotionRepoConfig;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReleashConfig {
    #[serde(default)]
    pub server: ServerSection,
    #[serde(default)]
    pub telemetry: TelemetrySection,
    #[serde(default)]
    pub notion: HashMap<String, NotionRepoConfig>,
    #[serde(default)]
    pub app: AppSection,
    #[serde(default)]
    pub agents: AgentsSection,
    #[serde(default)]
    pub workflow: WorkflowSection,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentsSection {
    pub default: Option<String>,
    #[serde(default)]
    pub claude: ClaudeAgentSection,
    #[serde(default)]
    pub codex: CodexAgentSection,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowSection {
    #[serde(default)]
    pub approval_auto_approve: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeAgentSection {
    /// registry の `fixed_models()` が優先されるため通常未使用（互換用に残す）。
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodexAgentSection {
    pub cli_path: Option<String>,
    /// registry の `fixed_models()` が優先されるため通常未使用（互換用に残す）。
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSection {
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    #[serde(default)]
    pub auto_launch: bool,
    #[serde(default)]
    pub start_minimized: bool,
    #[serde(default)]
    pub last_root_path: String,
    #[serde(default)]
    pub last_repo_paths: Vec<String>,
    #[serde(default)]
    pub external_editor: String,
    #[serde(default)]
    pub agent_shortcuts: AgentShortcutSection,
}

impl Default for AppSection {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            auto_launch: false,
            start_minimized: false,
            last_root_path: String::new(),
            last_repo_paths: Vec::new(),
            external_editor: String::new(),
            agent_shortcuts: AgentShortcutSection::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentShortcutSection {
    #[serde(default)]
    pub overrides: HashMap<String, String>,
}

fn default_crash_reporting() -> bool {
    true
}

fn default_performance_telemetry() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelemetrySection {
    #[serde(default = "default_crash_reporting")]
    pub crash_reporting: bool,
    #[serde(default = "default_performance_telemetry")]
    pub performance_telemetry: bool,
}

impl Default for TelemetrySection {
    fn default() -> Self {
        Self {
            crash_reporting: true,
            performance_telemetry: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerSection {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_hook_port")]
    pub hook_port: u16,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub tls: TlsSection,
    #[serde(default)]
    pub notify: NotifySection,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesktopNotifyMode {
    #[default]
    Always,
    WhenInactive,
}

fn default_inactive_timeout() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotifySection {
    #[serde(default)]
    pub webhook_url: String,
    #[serde(default)]
    pub on_running: bool,
    #[serde(default = "default_true")]
    pub on_done: bool,
    #[serde(default = "default_true")]
    pub on_error: bool,
    #[serde(default = "default_true")]
    pub on_waiting: bool,
    #[serde(default)]
    pub desktop_mode: DesktopNotifyMode,
    #[serde(default = "default_inactive_timeout")]
    pub inactive_timeout_minutes: u32,
}

impl Default for NotifySection {
    fn default() -> Self {
        Self {
            webhook_url: String::new(),
            on_running: false,
            on_done: true,
            on_error: true,
            on_waiting: true,
            desktop_mode: DesktopNotifyMode::default(),
            inactive_timeout_minutes: default_inactive_timeout(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TlsSection {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub cert: String,
    #[serde(default)]
    pub key: String,
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    9700
}

fn default_hook_port() -> u16 {
    19700
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
            hook_port: default_hook_port(),
            token: String::new(),
            tls: TlsSection::default(),
            notify: NotifySection::default(),
        }
    }
}

pub fn config_to_domain(config: &ReleashConfig) -> domain_vo::AppConfigDocument {
    domain_vo::AppConfigDocument {
        server: server_to_domain(&config.server),
        telemetry: telemetry_to_domain(&config.telemetry),
        app: app_to_domain(&config.app),
        workflow: workflow_to_domain(&config.workflow),
    }
}

pub fn apply_domain_to_config(config: &mut ReleashConfig, domain: domain_vo::AppConfigDocument) {
    config.server = server_to_model(domain.server);
    config.telemetry = telemetry_to_model(domain.telemetry);
    config.app = app_to_model(domain.app);
    config.workflow = workflow_to_model(domain.workflow);
}

pub fn server_to_domain(server: &ServerSection) -> domain_vo::ServerConfig {
    domain_vo::ServerConfig {
        bind: server.bind.clone(),
        port: server.port,
        hook_port: server.hook_port,
        token: server.token.clone(),
        tls: domain_vo::TlsConfig {
            enabled: server.tls.enabled,
            cert: server.tls.cert.clone(),
            key: server.tls.key.clone(),
        },
        notify: notify_to_domain(&server.notify),
    }
}

pub fn server_to_model(server: domain_vo::ServerConfig) -> ServerSection {
    ServerSection {
        bind: server.bind,
        port: server.port,
        hook_port: server.hook_port,
        token: server.token,
        tls: TlsSection {
            enabled: server.tls.enabled,
            cert: server.tls.cert,
            key: server.tls.key,
        },
        notify: notify_to_model(server.notify),
    }
}

pub fn notify_to_domain(notify: &NotifySection) -> domain_vo::NotifyConfig {
    domain_vo::NotifyConfig {
        webhook_url: notify.webhook_url.clone(),
        on_running: notify.on_running,
        on_done: notify.on_done,
        on_error: notify.on_error,
        on_waiting: notify.on_waiting,
        desktop_mode: match notify.desktop_mode {
            DesktopNotifyMode::Always => DomainDesktopNotifyMode::Always,
            DesktopNotifyMode::WhenInactive => DomainDesktopNotifyMode::WhenInactive,
        },
        inactive_timeout_minutes: notify.inactive_timeout_minutes,
    }
}

pub fn notify_to_model(notify: domain_vo::NotifyConfig) -> NotifySection {
    NotifySection {
        webhook_url: notify.webhook_url,
        on_running: notify.on_running,
        on_done: notify.on_done,
        on_error: notify.on_error,
        on_waiting: notify.on_waiting,
        desktop_mode: match notify.desktop_mode {
            DomainDesktopNotifyMode::Always => DesktopNotifyMode::Always,
            DomainDesktopNotifyMode::WhenInactive => DesktopNotifyMode::WhenInactive,
        },
        inactive_timeout_minutes: notify.inactive_timeout_minutes,
    }
}

pub fn telemetry_to_domain(telemetry: &TelemetrySection) -> domain_vo::TelemetryConfig {
    domain_vo::TelemetryConfig {
        crash_reporting: telemetry.crash_reporting,
        performance_telemetry: telemetry.performance_telemetry,
    }
}

pub fn telemetry_to_model(telemetry: domain_vo::TelemetryConfig) -> TelemetrySection {
    TelemetrySection {
        crash_reporting: telemetry.crash_reporting,
        performance_telemetry: telemetry.performance_telemetry,
    }
}

pub fn app_to_domain(app: &AppSection) -> domain_vo::AppSettings {
    domain_vo::AppSettings {
        close_to_tray: app.close_to_tray,
        auto_launch: app.auto_launch,
        start_minimized: app.start_minimized,
        last_root_path: app.last_root_path.clone(),
        last_repo_paths: app.last_repo_paths.clone(),
        external_editor: app.external_editor.clone(),
        agent_shortcuts: domain_vo::AgentShortcutConfig {
            overrides: app.agent_shortcuts.overrides.clone(),
        },
    }
}

pub fn app_to_model(app: domain_vo::AppSettings) -> AppSection {
    AppSection {
        close_to_tray: app.close_to_tray,
        auto_launch: app.auto_launch,
        start_minimized: app.start_minimized,
        last_root_path: app.last_root_path,
        last_repo_paths: app.last_repo_paths,
        external_editor: app.external_editor,
        agent_shortcuts: AgentShortcutSection {
            overrides: app.agent_shortcuts.overrides,
        },
    }
}

pub fn workflow_to_domain(workflow: &WorkflowSection) -> domain_vo::WorkflowConfig {
    domain_vo::WorkflowConfig {
        approval_auto_approve: workflow.approval_auto_approve,
    }
}

pub fn workflow_to_model(workflow: domain_vo::WorkflowConfig) -> WorkflowSection {
    WorkflowSection {
        approval_auto_approve: workflow.approval_auto_approve,
    }
}

#[cfg(test)]
mod config_models_tests {
    use super::*;

    fn assert_domain_roundtrip(config: ReleashConfig) {
        // Given
        let domain = config_to_domain(&config);
        let mut roundtripped = config.clone();

        // When
        apply_domain_to_config(&mut roundtripped, domain);

        // Then
        assert_eq!(roundtripped.server, config.server);
        assert_eq!(roundtripped.telemetry, config.telemetry);
        assert_eq!(roundtripped.app, config.app);
        assert_eq!(roundtripped.workflow, config.workflow);
        assert_eq!(roundtripped.notion.len(), config.notion.len());
        assert_eq!(roundtripped.agents.default, config.agents.default);
        assert_eq!(
            roundtripped.agents.claude.models,
            config.agents.claude.models
        );
        assert_eq!(
            roundtripped.agents.codex.cli_path,
            config.agents.codex.cli_path
        );
        assert_eq!(roundtripped.agents.codex.models, config.agents.codex.models);
    }

    #[test]
    fn test_config_model変換_既定値がdomain往復で同値になる() {
        // Given / When / Then
        assert_domain_roundtrip(ReleashConfig::default());
    }

    #[test]
    fn test_config_model変換_変更済み値がdomain往復で同値になる() {
        // Given
        let config = ReleashConfig {
            server: ServerSection {
                bind: "0.0.0.0".to_string(),
                port: 18080,
                hook_port: 28080,
                token: "server-token".to_string(),
                tls: TlsSection {
                    enabled: true,
                    cert: "/tmp/cert.pem".to_string(),
                    key: "/tmp/key.pem".to_string(),
                },
                notify: NotifySection {
                    webhook_url: "https://example.test/hook".to_string(),
                    on_running: true,
                    on_done: false,
                    desktop_mode: DesktopNotifyMode::WhenInactive,
                    inactive_timeout_minutes: 9,
                    ..NotifySection::default()
                },
            },
            telemetry: TelemetrySection {
                crash_reporting: false,
                performance_telemetry: false,
            },
            app: AppSection {
                close_to_tray: false,
                auto_launch: true,
                start_minimized: true,
                last_root_path: "/repo".to_string(),
                last_repo_paths: vec!["/repo".to_string(), "/repo2".to_string()],
                external_editor: "cursor".to_string(),
                agent_shortcuts: AgentShortcutSection {
                    overrides: std::collections::HashMap::from([(
                        "send".to_string(),
                        "Ctrl+Enter".to_string(),
                    )]),
                },
            },
            agents: AgentsSection {
                default: Some("codex".to_string()),
                claude: ClaudeAgentSection {
                    models: vec!["claude-model".to_string()],
                },
                codex: CodexAgentSection {
                    cli_path: Some("/opt/bin/codex".to_string()),
                    models: vec!["codex-model".to_string()],
                },
            },
            workflow: WorkflowSection {
                approval_auto_approve: true,
            },
            ..ReleashConfig::default()
        };

        // When / Then
        assert_domain_roundtrip(config);
    }
}
