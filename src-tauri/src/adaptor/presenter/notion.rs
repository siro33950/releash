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

impl From<notion_domain::NotionTaskPage> for NotionTaskPageView {
    fn from(page: notion_domain::NotionTaskPage) -> Self {
        Self {
            tasks: page.tasks.into_iter().map(Into::into).collect(),
            has_more: page.has_more,
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

impl From<crate::usecase::notion::usecase::NotionRepoConfigDto> for NotionRepoConfigView {
    fn from(config: crate::usecase::notion::usecase::NotionRepoConfigDto) -> Self {
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

impl From<crate::usecase::notion::usecase::NotionPropertyMappingDto> for PropertyMappingView {
    fn from(mapping: crate::usecase::notion::usecase::NotionPropertyMappingDto) -> Self {
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

impl From<crate::usecase::notion::usecase::NotionLabelPropertyDto> for LabelPropertyView {
    fn from(label: crate::usecase::notion::usecase::NotionLabelPropertyDto) -> Self {
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
#[path = "notion_test.rs"]
mod notion_tests;
