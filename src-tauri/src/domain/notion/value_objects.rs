use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotionTaskQuery {
    pub title_filter: String,
    pub label_filters: HashMap<String, Vec<String>>,
    pub cursor: Option<String>,
    pub page_size: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotionLabelOption {
    pub property_name: String,
    pub property_type: String,
    pub options: Vec<String>,
    pub option_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotionTaskPage {
    pub tasks: Vec<NotionTask>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotionTask {
    pub id: String,
    pub title: String,
    pub url: String,
    pub labels: HashMap<String, Vec<String>>,
    pub branch_name: String,
    pub created_at: String,
    pub last_edited_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotionValidationResult {
    pub status: NotionConfigStatus,
    pub properties: Vec<NotionPropertyInfo>,
}

impl NotionValidationResult {
    pub(crate) fn not_configured() -> Self {
        Self {
            status: NotionConfigStatus::NotConfigured,
            properties: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotionPropertyInfo {
    pub name: String,
    pub property_type: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotionConfigStatus {
    NotConfigured,
    Configured,
    InvalidToken,
    InvalidDatabase,
    NetworkError,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate結果_未設定はプロパティ空で返る() {
        let result = NotionValidationResult::not_configured();

        assert_eq!(result.status, NotionConfigStatus::NotConfigured);
        assert!(result.properties.is_empty());
    }
}
