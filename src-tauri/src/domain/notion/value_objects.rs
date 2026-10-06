use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotionTaskQuery {
    pub title_filter: String,
    pub label_filters: HashMap<String, Vec<String>>,
    pub cursor: Option<String>,
    pub page_size: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotionLabelOption {
    pub property_name: String,
    pub property_type: String,
    pub options: Vec<String>,
    pub option_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotionTaskPage {
    pub tasks: Vec<NotionTask>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotionTask {
    pub id: String,
    pub title: String,
    pub url: String,
    pub labels: HashMap<String, Vec<String>>,
    pub branch_name: String,
    pub created_at: String,
    pub last_edited_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotionValidationResult {
    pub status: NotionConfigStatus,
    pub properties: Vec<NotionPropertyInfo>,
}

impl NotionValidationResult {
    pub fn not_configured() -> Self {
        Self {
            status: NotionConfigStatus::NotConfigured,
            properties: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotionPropertyInfo {
    pub name: String,
    pub property_type: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotionConfigStatus {
    NotConfigured,
    Configured,
    InvalidToken,
    InvalidDatabase,
    NetworkError,
}

#[cfg(test)]
#[path = "value_objects_test.rs"]
mod value_objects_tests;
