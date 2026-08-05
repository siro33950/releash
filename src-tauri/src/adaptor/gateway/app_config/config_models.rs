use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::domain::app_config::value_objects as domain_vo;
use crate::domain::notification::DesktopNotifyMode as DomainDesktopNotifyMode;

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
    pub notion: HashMap<String, NotionRepoConfigModel>,
    #[serde(default)]
    pub app: AppSection,
    #[serde(default)]
    pub agents: AgentsSection,
    #[serde(default)]
    pub workflow: WorkflowSection,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NotionRepoConfigModel {
    pub api_token: String,
    pub database_id: String,
    #[serde(default)]
    pub property_mapping: NotionPropertyMappingModel,
}

impl std::fmt::Debug for NotionRepoConfigModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotionRepoConfigModel")
            .field("api_token", &"[REDACTED]")
            .field("database_id", &self.database_id)
            .field("property_mapping", &self.property_mapping)
            .finish()
    }
}

fn default_notion_title() -> String {
    "Name".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotionLabelPropertyModel {
    pub name: String,
    pub property_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotionPropertyMappingModel {
    #[serde(default = "default_notion_title")]
    pub title: String,
    #[serde(default, deserialize_with = "deserialize_notion_labels")]
    pub labels: Vec<NotionLabelPropertyModel>,
    #[serde(default)]
    pub branch_name: String,
    #[serde(default)]
    pub branch_prefix: String,
}

impl Default for NotionPropertyMappingModel {
    fn default() -> Self {
        Self {
            title: default_notion_title(),
            labels: Vec::new(),
            branch_name: String::new(),
            branch_prefix: String::new(),
        }
    }
}

fn deserialize_notion_labels<'de, D>(
    deserializer: D,
) -> Result<Vec<NotionLabelPropertyModel>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry {
        Full { name: String, property_type: String },
        Name(String),
    }

    let entries: Vec<Entry> = Vec::deserialize(deserializer)?;
    Ok(entries
        .into_iter()
        .map(|entry| match entry {
            Entry::Full {
                name,
                property_type,
            } => NotionLabelPropertyModel {
                name,
                property_type,
            },
            Entry::Name(name) => NotionLabelPropertyModel {
                name,
                property_type: "select".to_string(),
            },
        })
        .collect())
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
    pub cli_path: Option<String>,
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
        }
    }
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

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
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
#[path = "config_models_test.rs"]
mod config_models_tests;
