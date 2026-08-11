#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfigDocument {
    pub server: ServerConfig,
    pub telemetry: TelemetryConfig,
    pub app: AppSettings,
    pub workflow: WorkflowConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub token: String,
    pub tls: TlsConfig,
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
    pub performance_telemetry: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSettings {
    pub close_to_tray: bool,
    pub auto_launch: bool,
    pub start_minimized: bool,
    pub last_root_path: String,
    pub last_repo_paths: Vec<String>,
    pub external_editor: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowConfig {
    pub approval_auto_approve: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct NotionRepoConfig {
    pub api_token: String,
    pub database_id: String,
    pub property_mapping: NotionPropertyMapping,
}

impl std::fmt::Debug for NotionRepoConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotionRepoConfig")
            .field("api_token", &"[REDACTED]")
            .field("database_id", &self.database_id)
            .field("property_mapping", &self.property_mapping)
            .finish()
    }
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

#[cfg(test)]
mod value_objects_tests {
    use super::*;

    #[test]
    fn test_notion_config_debugでapi_tokenをマスクする() {
        let config = NotionRepoConfig {
            api_token: "ntn_secret_token".to_string(),
            database_id: "db-1".to_string(),
            property_mapping: NotionPropertyMapping::default(),
        };

        let output = format!("{config:?}");

        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("ntn_secret_token"));
    }
}
