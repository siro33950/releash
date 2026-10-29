use std::collections::HashMap;

use serde::{Deserialize, Serialize};

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
    #[serde(skip)]
    pub provider_configuration_revisions:
        HashMap<crate::domain::provider_lifecycle::ProviderKind, u32>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSection {
    #[serde(default)]
    pub auto_launch: bool,
    #[serde(default)]
    pub start_minimized: bool,
    #[serde(default)]
    pub last_repo_paths: Vec<String>,
    #[serde(default)]
    pub external_editor: String,
    #[serde(default)]
    pub terminal_startup_command: String,
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

#[cfg(any(test, feature = "test-support"))]
pub fn config_to_domain(config: &ReleashConfig) -> crate::domain::app_config::AppConfig {
    try_config_to_domain(config).expect("provider executable must be valid")
}

pub(super) fn try_config_to_domain(
    config: &ReleashConfig,
) -> Result<crate::domain::app_config::AppConfig, crate::domain::app_config::AppConfigError> {
    use crate::domain::agent_session::aggregates::ProviderExecutable;
    use crate::domain::app_config::value_objects::{TelemetryConfig, WorkflowConfig};
    use crate::domain::app_config::{AppConfig, AppConfigError};
    use crate::domain::provider_lifecycle::ProviderKind;

    let mut executables = HashMap::new();
    for (provider, path) in [
        (ProviderKind::Claude, &config.agents.claude.cli_path),
        (ProviderKind::Codex, &config.agents.codex.cli_path),
    ] {
        if let Some(path) = path {
            let executable = ProviderExecutable::new(path.clone())
                .map_err(|_| AppConfigError::InvalidInput("Invalid provider executable".into()))?;
            executables.insert(provider, executable);
        }
    }
    Ok(AppConfig::new(
        config.app.auto_launch,
        config.app.start_minimized,
        config.app.last_repo_paths.clone(),
        config.app.external_editor.clone(),
        config.app.terminal_startup_command.clone(),
        TelemetryConfig {
            crash_reporting: config.telemetry.crash_reporting,
            performance_telemetry: config.telemetry.performance_telemetry,
        },
        WorkflowConfig {
            approval_auto_approve: config.workflow.approval_auto_approve,
        },
        executables,
        config.provider_configuration_revisions.clone(),
    ))
}

pub fn apply_domain_to_config(
    config: &mut ReleashConfig,
    domain: crate::domain::app_config::AppConfig,
) {
    use crate::domain::provider_lifecycle::ProviderKind;

    config.app = AppSection {
        auto_launch: domain.auto_launch(),
        start_minimized: domain.start_minimized(),
        last_repo_paths: domain.repo_paths().to_vec(),
        external_editor: domain.external_editor().to_owned(),
        terminal_startup_command: domain.terminal_startup_command().to_owned(),
    };
    config.telemetry = TelemetrySection {
        crash_reporting: domain.telemetry().crash_reporting,
        performance_telemetry: domain.telemetry().performance_telemetry,
    };
    config.workflow.approval_auto_approve = domain.workflow().approval_auto_approve;
    let executables = domain.provider_executables();
    config.agents.claude.cli_path = executables
        .get(&ProviderKind::Claude)
        .map(|executable| executable.as_str().to_owned());
    config.agents.codex.cli_path = executables
        .get(&ProviderKind::Codex)
        .map(|executable| executable.as_str().to_owned());
    config.provider_configuration_revisions = ProviderKind::supported()
        .iter()
        .map(|provider| (*provider, domain.provider_configuration_revision(*provider)))
        .filter(|(_, revision)| *revision != 0)
        .collect();
}

#[cfg(test)]
#[path = "config_models_test.rs"]
mod config_models_tests;
