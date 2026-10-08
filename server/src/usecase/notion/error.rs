use crate::domain::app_config::AppConfigError;
use crate::domain::notion::NotionError;

pub(crate) const NOTION_CONFIG_NOT_FOUND: &str = "Notion設定が見つかりません";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotionUsecaseError {
    ConfigNotFound,
    AppConfig(AppConfigError),
    Notion(NotionError),
}

impl std::fmt::Display for NotionUsecaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfigNotFound => f.write_str(NOTION_CONFIG_NOT_FOUND),
            Self::AppConfig(error) => write!(f, "{error}"),
            Self::Notion(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for NotionUsecaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ConfigNotFound => None,
            Self::AppConfig(error) => Some(error),
            Self::Notion(error) => Some(error),
        }
    }
}

impl From<AppConfigError> for NotionUsecaseError {
    fn from(value: AppConfigError) -> Self {
        Self::AppConfig(value)
    }
}

impl From<NotionError> for NotionUsecaseError {
    fn from(value: NotionError) -> Self {
        Self::Notion(value)
    }
}

#[cfg(test)]
#[path = "error_test.rs"]
pub(crate) mod error_tests;
