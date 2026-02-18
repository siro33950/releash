use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct NotionRepoConfig {
    pub api_token: String,
    pub database_id: String,
    #[serde(default)]
    pub property_mapping: PropertyMapping,
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

fn default_title() -> String {
    "Name".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LabelProperty {
    pub name: String,
    pub property_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyMapping {
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default, deserialize_with = "deserialize_labels")]
    pub labels: Vec<LabelProperty>,
    #[serde(default)]
    pub branch_name: String,
}

impl Default for PropertyMapping {
    fn default() -> Self {
        Self {
            title: default_title(),
            labels: vec![],
            branch_name: String::new(),
        }
    }
}

fn deserialize_labels<'de, D>(deserializer: D) -> Result<Vec<LabelProperty>, D::Error>
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
        .map(|e| match e {
            Entry::Full {
                name,
                property_type,
            } => LabelProperty {
                name,
                property_type,
            },
            Entry::Name(name) => LabelProperty {
                name,
                property_type: "select".to_string(),
            },
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotionTaskQuery {
    pub title_filter: String,
    pub label_filters: HashMap<String, String>,
    pub cursor: Option<String>,
    pub page_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotionLabelOption {
    pub property_name: String,
    pub property_type: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotionTaskPage {
    pub tasks: Vec<NotionTask>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotionTask {
    pub id: String,
    pub title: String,
    pub url: String,
    pub labels: HashMap<String, Vec<String>>,
    pub branch_name: String,
    pub created_at: String,
    pub last_edited_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotionValidationResult {
    pub status: NotionConfigStatus,
    pub properties: Vec<NotionPropertyInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotionPropertyInfo {
    pub name: String,
    pub property_type: String,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotionConfigStatus {
    NotConfigured,
    Configured,
    InvalidToken,
    InvalidDatabase,
}

#[derive(Debug, Clone)]
pub enum NotionError {
    RequestFailed(String),
    ApiError(String),
    ParseError(String),
    #[allow(dead_code)]
    PropertyNotFound(String),
}

impl std::fmt::Display for NotionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotionError::RequestFailed(msg) => write!(f, "リクエスト失敗: {msg}"),
            NotionError::ApiError(msg) => write!(f, "API エラー: {msg}"),
            NotionError::ParseError(msg) => write!(f, "パースエラー: {msg}"),
            NotionError::PropertyNotFound(name) => {
                write!(f, "プロパティが見つかりません: {name}")
            }
        }
    }
}

impl std::error::Error for NotionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_mapping_default() {
        let mapping = PropertyMapping::default();
        assert_eq!(mapping.title, "Name");
        assert!(mapping.labels.is_empty());
        assert!(mapping.branch_name.is_empty());
    }

    #[test]
    fn notion_task_roundtrip() {
        let mut labels = HashMap::new();
        labels.insert("Status".to_string(), vec!["In Progress".to_string()]);
        labels.insert("Tags".to_string(), vec!["bug".to_string()]);

        let task = NotionTask {
            id: "page-id-123".to_string(),
            title: "テストタスク".to_string(),
            url: "https://notion.so/page-id-123".to_string(),
            labels,
            branch_name: "feat/test-task".to_string(),
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            last_edited_at: "2026-01-02T00:00:00.000Z".to_string(),
        };

        let json = serde_json::to_string(&task).unwrap();
        let deserialized: NotionTask = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, task.id);
        assert_eq!(deserialized.title, task.title);
        assert_eq!(deserialized.labels.len(), 2);
    }

    #[test]
    fn notion_repo_config_roundtrip_toml() {
        let config = NotionRepoConfig {
            api_token: "ntn_test_token".to_string(),
            database_id: "db-id-456".to_string(),
            property_mapping: PropertyMapping {
                title: "Task Name".to_string(),
                labels: vec![
                    LabelProperty {
                        name: "Tags".to_string(),
                        property_type: "multi_select".to_string(),
                    },
                    LabelProperty {
                        name: "Status".to_string(),
                        property_type: "status".to_string(),
                    },
                ],
                branch_name: "Branch".to_string(),
            },
        };

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let deserialized: NotionRepoConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(deserialized.api_token, config.api_token);
        assert_eq!(deserialized.database_id, config.database_id);
        assert_eq!(deserialized.property_mapping.title, "Task Name");
        assert_eq!(deserialized.property_mapping.labels.len(), 2);
        assert_eq!(deserialized.property_mapping.labels[0].name, "Tags");
        assert_eq!(
            deserialized.property_mapping.labels[0].property_type,
            "multi_select"
        );
    }

    #[test]
    fn notion_repo_config_default_mapping() {
        let toml_str = r#"
api_token = "ntn_test"
database_id = "db-123"
"#;
        let config: NotionRepoConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.property_mapping.title, "Name");
        assert!(config.property_mapping.labels.is_empty());
    }

    #[test]
    fn labels_backward_compat_string_array() {
        let toml_str = r#"
api_token = "ntn_test"
database_id = "db-123"

[property_mapping]
title = "Name"
labels = ["Status", "Tags"]
"#;
        let config: NotionRepoConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.property_mapping.labels.len(), 2);
        assert_eq!(config.property_mapping.labels[0].name, "Status");
        assert_eq!(config.property_mapping.labels[0].property_type, "select");
        assert_eq!(config.property_mapping.labels[1].name, "Tags");
    }

    #[test]
    fn labels_new_format() {
        let toml_str = r#"
api_token = "ntn_test"
database_id = "db-123"

[property_mapping]
title = "Name"

[[property_mapping.labels]]
name = "Status"
property_type = "status"

[[property_mapping.labels]]
name = "Tags"
property_type = "multi_select"
"#;
        let config: NotionRepoConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.property_mapping.labels.len(), 2);
        assert_eq!(config.property_mapping.labels[0].name, "Status");
        assert_eq!(config.property_mapping.labels[0].property_type, "status");
        assert_eq!(config.property_mapping.labels[1].name, "Tags");
        assert_eq!(
            config.property_mapping.labels[1].property_type,
            "multi_select"
        );
    }

    #[test]
    fn notion_config_status_serializes_snake_case() {
        let status = serde_json::to_string(&NotionConfigStatus::InvalidToken).unwrap();
        assert_eq!(status, r#""invalid_token""#);

        let status = serde_json::to_string(&NotionConfigStatus::NotConfigured).unwrap();
        assert_eq!(status, r#""not_configured""#);
    }

    #[test]
    fn notion_error_display() {
        let err = NotionError::RequestFailed("timeout".to_string());
        assert!(err.to_string().contains("timeout"));

        let err = NotionError::PropertyNotFound("Name".to_string());
        assert!(err.to_string().contains("Name"));
    }

    #[test]
    fn notion_task_query_roundtrip() {
        let mut label_filters = HashMap::new();
        label_filters.insert("Status".to_string(), "Todo".to_string());

        let query = NotionTaskQuery {
            title_filter: "test".to_string(),
            label_filters,
            cursor: Some("cursor-abc".to_string()),
            page_size: Some(20),
        };

        let json = serde_json::to_string(&query).unwrap();
        let deserialized: NotionTaskQuery = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.title_filter, "test");
        assert_eq!(deserialized.label_filters.get("Status").unwrap(), "Todo");
        assert_eq!(deserialized.cursor.unwrap(), "cursor-abc");
        assert_eq!(deserialized.page_size.unwrap(), 20);
    }

    #[test]
    fn notion_label_option_roundtrip() {
        let opt = NotionLabelOption {
            property_name: "Status".to_string(),
            property_type: "status".to_string(),
            options: vec!["Todo".to_string(), "In Progress".to_string()],
        };

        let json = serde_json::to_string(&opt).unwrap();
        let deserialized: NotionLabelOption = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.property_name, "Status");
        assert_eq!(deserialized.options.len(), 2);
    }
}
