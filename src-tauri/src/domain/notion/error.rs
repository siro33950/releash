#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotionError {
    Technical(crate::domain::failure::TechnicalFailure),
    RequestFailed(String),
    ApiError(String),
    ParseError(String),
}

impl std::fmt::Display for NotionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Technical(error) => std::fmt::Display::fmt(error, f),
            NotionError::RequestFailed(msg) => write!(f, "リクエスト失敗: {msg}"),
            NotionError::ApiError(msg) => write!(f, "API エラー: {msg}"),
            NotionError::ParseError(msg) => write!(f, "パースエラー: {msg}"),
        }
    }
}

impl std::error::Error for NotionError {}

#[cfg(test)]
#[path = "error_test.rs"]
pub(crate) mod error_tests;
