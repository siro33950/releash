/// telemetry の有効・無効を表す設定値。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryConfig {
    pub crash_reporting: bool,
    pub performance_telemetry: bool,
}

/// workflow の承認方針を表す設定値。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowConfig {
    pub approval_auto_approve: bool,
}

/// Notion の接続情報と property の対応を表す設定値。
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
#[path = "value_objects_test.rs"]
mod value_objects_tests;
