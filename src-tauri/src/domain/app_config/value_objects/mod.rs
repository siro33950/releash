use std::collections::HashMap;

pub use crate::domain::notification::NotifyConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfigDocument {
    pub telemetry_enabled: bool,
    pub server: ServerConfig,
    pub telemetry: TelemetryConfig,
    pub app: AppSettings,
    pub workflow: WorkflowConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub hook_port: u16,
    pub token: String,
    pub tls: TlsConfig,
    pub notify: NotifyConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsConfig {
    pub enabled: bool,
    pub cert: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryConfig {
    pub crash_reporting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSettings {
    pub close_to_tray: bool,
    pub auto_launch: bool,
    pub start_minimized: bool,
    pub last_root_path: String,
    pub last_repo_paths: Vec<String>,
    pub external_editor: String,
    pub agent_shortcuts: AgentShortcutConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentShortcutConfig {
    pub overrides: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowConfig {
    pub approval_auto_approve: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotionRepoConfig {
    pub api_token: String,
    pub database_id: String,
    pub property_mapping: NotionPropertyMapping,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotionPropertyMapping {
    pub title: String,
    pub labels: Vec<NotionLabelProperty>,
    pub branch_name: String,
    pub branch_prefix: String,
}

impl Default for NotionPropertyMapping {
    fn default() -> Self {
        Self {
            title: "Name".to_string(),
            labels: Vec::new(),
            branch_name: String::new(),
            branch_prefix: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotionLabelProperty {
    pub name: String,
    pub property_type: String,
}
