use std::collections::HashMap;

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReleashConfig {
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodexAgentSection {
    pub cli_path: Option<String>,
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
    pub terminal_startup_command: String,
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
            terminal_startup_command: String::new(),
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

pub fn config_to_domain(config: &ReleashConfig) -> crate::domain::app_config::AppConfig {
    todo!()
}

pub fn apply_domain_to_config(
    config: &mut ReleashConfig,
    domain: crate::domain::app_config::AppConfig,
) {
    todo!()
}

#[cfg(test)]
#[path = "config_models_test.rs"]
mod config_models_tests;
