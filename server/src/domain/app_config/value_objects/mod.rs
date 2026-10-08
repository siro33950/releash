#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfigDocument {
    pub telemetry: TelemetryConfig,
    pub app: AppSettings,
    pub workflow: WorkflowConfig,
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

impl NotionRepoConfig {
    pub fn is_configured(&self) -> bool {
        !self.api_token.trim().is_empty() && !self.database_id.trim().is_empty()
    }
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
#[path = "mod_test.rs"]
mod mod_tests;
