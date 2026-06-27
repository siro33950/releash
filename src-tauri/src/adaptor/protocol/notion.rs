use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::domain::app_config::value_objects as app_config_vo;
use crate::domain::notion as notion_domain;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct NotionRepoConfigView {
    pub api_token: String,
    pub database_id: String,
    #[serde(default)]
    pub property_mapping: PropertyMappingView,
}

impl std::fmt::Debug for NotionRepoConfigView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotionRepoConfigView")
            .field("api_token", &"[REDACTED]")
            .field("database_id", &self.database_id)
            .field("property_mapping", &self.property_mapping)
            .finish()
    }
}

fn default_title() -> String {
    "Name".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct LabelPropertyView {
    pub name: String,
    pub property_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PropertyMappingView {
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default)]
    pub labels: Vec<LabelPropertyView>,
    #[serde(default)]
    pub branch_name: String,
    #[serde(default)]
    pub branch_prefix: String,
}

impl Default for PropertyMappingView {
    fn default() -> Self {
        Self {
            title: default_title(),
            labels: Vec::new(),
            branch_name: String::new(),
            branch_prefix: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotionTaskQueryInput {
    pub title_filter: String,
    pub label_filters: HashMap<String, Vec<String>>,
    pub cursor: Option<String>,
    pub page_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotionLabelOptionView {
    pub property_name: String,
    pub property_type: String,
    pub options: Vec<String>,
    #[serde(default)]
    pub option_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotionTaskPageView {
    pub tasks: Vec<NotionTaskView>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotionTaskView {
    pub id: String,
    pub title: String,
    pub url: String,
    pub labels: HashMap<String, Vec<String>>,
    pub branch_name: String,
    pub created_at: String,
    pub last_edited_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotionValidationResultView {
    pub status: NotionConfigStatusView,
    pub properties: Vec<NotionPropertyInfoView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotionPropertyInfoView {
    pub name: String,
    pub property_type: String,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotionConfigStatusView {
    NotConfigured,
    Configured,
    InvalidToken,
    InvalidDatabase,
    NetworkError,
}

impl From<NotionTaskQueryInput> for notion_domain::NotionTaskQuery {
    fn from(query: NotionTaskQueryInput) -> Self {
        Self {
            title_filter: query.title_filter,
            label_filters: query.label_filters,
            cursor: query.cursor,
            page_size: query.page_size,
        }
    }
}

impl From<notion_domain::NotionTaskPage> for NotionTaskPageView {
    fn from(page: notion_domain::NotionTaskPage) -> Self {
        Self {
            tasks: page.tasks.into_iter().map(Into::into).collect(),
            has_more: page.has_more,
            next_cursor: page.next_cursor,
        }
    }
}

impl From<notion_domain::NotionTask> for NotionTaskView {
    fn from(task: notion_domain::NotionTask) -> Self {
        Self {
            id: task.id,
            title: task.title,
            url: task.url,
            labels: task.labels,
            branch_name: task.branch_name,
            created_at: task.created_at,
            last_edited_at: task.last_edited_at,
        }
    }
}

impl From<notion_domain::NotionLabelOption> for NotionLabelOptionView {
    fn from(option: notion_domain::NotionLabelOption) -> Self {
        Self {
            property_name: option.property_name,
            property_type: option.property_type,
            options: option.options,
            option_ids: option.option_ids,
        }
    }
}

impl From<notion_domain::NotionValidationResult> for NotionValidationResultView {
    fn from(result: notion_domain::NotionValidationResult) -> Self {
        Self {
            status: result.status.into(),
            properties: result.properties.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<notion_domain::NotionConfigStatus> for NotionConfigStatusView {
    fn from(status: notion_domain::NotionConfigStatus) -> Self {
        match status {
            notion_domain::NotionConfigStatus::NotConfigured => Self::NotConfigured,
            notion_domain::NotionConfigStatus::Configured => Self::Configured,
            notion_domain::NotionConfigStatus::InvalidToken => Self::InvalidToken,
            notion_domain::NotionConfigStatus::InvalidDatabase => Self::InvalidDatabase,
            notion_domain::NotionConfigStatus::NetworkError => Self::NetworkError,
        }
    }
}

impl From<notion_domain::NotionPropertyInfo> for NotionPropertyInfoView {
    fn from(property: notion_domain::NotionPropertyInfo) -> Self {
        Self {
            name: property.name,
            property_type: property.property_type,
            options: property.options,
        }
    }
}

impl From<app_config_vo::NotionRepoConfig> for NotionRepoConfigView {
    fn from(config: app_config_vo::NotionRepoConfig) -> Self {
        Self {
            api_token: config.api_token,
            database_id: config.database_id,
            property_mapping: config.property_mapping.into(),
        }
    }
}

impl From<NotionRepoConfigView> for app_config_vo::NotionRepoConfig {
    fn from(config: NotionRepoConfigView) -> Self {
        Self {
            api_token: config.api_token,
            database_id: config.database_id,
            property_mapping: config.property_mapping.into(),
        }
    }
}

impl From<app_config_vo::NotionPropertyMapping> for PropertyMappingView {
    fn from(mapping: app_config_vo::NotionPropertyMapping) -> Self {
        Self {
            title: mapping.title,
            labels: mapping.labels.into_iter().map(Into::into).collect(),
            branch_name: mapping.branch_name,
            branch_prefix: mapping.branch_prefix,
        }
    }
}

impl From<PropertyMappingView> for app_config_vo::NotionPropertyMapping {
    fn from(mapping: PropertyMappingView) -> Self {
        Self {
            title: mapping.title,
            labels: mapping.labels.into_iter().map(Into::into).collect(),
            branch_name: mapping.branch_name,
            branch_prefix: mapping.branch_prefix,
        }
    }
}

impl From<app_config_vo::NotionLabelProperty> for LabelPropertyView {
    fn from(label: app_config_vo::NotionLabelProperty) -> Self {
        Self {
            name: label.name,
            property_type: label.property_type,
        }
    }
}

impl From<LabelPropertyView> for app_config_vo::NotionLabelProperty {
    fn from(label: LabelPropertyView) -> Self {
        Self {
            name: label.name,
            property_type: label.property_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notion_status_view_snake_caseでserializeされる() {
        let status = serde_json::to_string(&NotionConfigStatusView::InvalidToken).unwrap();
        assert_eq!(status, r#""invalid_token""#);

        let status = serde_json::to_string(&NotionConfigStatusView::NotConfigured).unwrap();
        assert_eq!(status, r#""not_configured""#);
    }

    #[test]
    fn test_notion_repo_config_view_debugでtokenをマスクする() {
        let config = NotionRepoConfigView {
            api_token: "ntn_secret_token".to_string(),
            database_id: "db-1".to_string(),
            property_mapping: PropertyMappingView::default(),
        };

        let output = format!("{config:?}");
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("ntn_secret_token"));
    }

    #[test]
    fn test_property_mapping_view_省略値を維持する() {
        let mapping: PropertyMappingView = toml::from_str("").unwrap();

        assert_eq!(mapping.title, "Name");
        assert!(mapping.labels.is_empty());
        assert!(mapping.branch_name.is_empty());
        assert!(mapping.branch_prefix.is_empty());
    }

    #[test]
    fn test_property_mapping_view_構造化labels配列を読む() {
        let json = r#"{
            "title": "Task",
            "labels": [
                { "name": "Status", "property_type": "status" },
                { "name": "Tags", "property_type": "multi_select" }
            ],
            "branch_name": "Branch",
            "branch_prefix": "feat/"
        }"#;

        let mapping: PropertyMappingView = serde_json::from_str(json).unwrap();

        assert_eq!(mapping.title, "Task");
        assert_eq!(mapping.labels.len(), 2);
        assert_eq!(mapping.labels[0].name, "Status");
        assert_eq!(mapping.labels[0].property_type, "status");
        assert_eq!(mapping.labels[1].name, "Tags");
        assert_eq!(mapping.labels[1].property_type, "multi_select");
        assert_eq!(mapping.branch_name, "Branch");
        assert_eq!(mapping.branch_prefix, "feat/");
    }

    #[test]
    fn test_property_mapping_view_labels文字列配列は受け付けない() {
        let json = r#"{ "labels": ["Status", "Tags"] }"#;

        assert!(serde_json::from_str::<PropertyMappingView>(json).is_err());
    }

    #[test]
    fn test_notion_task_query_input_json_roundtripする() {
        let mut label_filters = HashMap::new();
        label_filters.insert("Status".to_string(), vec!["Todo".to_string()]);
        let query = NotionTaskQueryInput {
            title_filter: "test".to_string(),
            label_filters,
            cursor: Some("cursor-abc".to_string()),
            page_size: Some(20),
        };

        let json = serde_json::to_string(&query).unwrap();
        let deserialized: NotionTaskQueryInput = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.title_filter, "test");
        assert_eq!(
            deserialized.label_filters.get("Status").unwrap(),
            &vec!["Todo".to_string()]
        );
        assert_eq!(deserialized.cursor.as_deref(), Some("cursor-abc"));
        assert_eq!(deserialized.page_size, Some(20));
    }

    #[test]
    fn test_notion_label_option_view_option_ids省略時は空になる() {
        let json = r#"{
            "property_name": "Status",
            "property_type": "status",
            "options": ["Todo", "Done"]
        }"#;
        let deserialized: NotionLabelOptionView = serde_json::from_str(json).unwrap();

        assert_eq!(deserialized.property_name, "Status");
        assert!(deserialized.option_ids.is_empty());
    }
}
