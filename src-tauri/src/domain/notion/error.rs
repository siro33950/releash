#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotionError {
    RequestFailed(String),
    ApiError(String),
    ParseError(String),
}

impl std::fmt::Display for NotionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotionError::RequestFailed(msg) => write!(f, "リクエスト失敗: {msg}"),
            NotionError::ApiError(msg) => write!(f, "API エラー: {msg}"),
            NotionError::ParseError(msg) => write!(f, "パースエラー: {msg}"),
        }
    }
}

impl std::error::Error for NotionError {}

#[cfg(test)]
mod notion_error_tests {
    use super::*;

    #[test]
    fn test_notionエラー_display文字列を維持する() {
        let err = NotionError::RequestFailed("timeout".to_string());
        assert_eq!(err.to_string(), "リクエスト失敗: timeout");

        let err = NotionError::ApiError("HTTP 500".to_string());
        assert_eq!(err.to_string(), "API エラー: HTTP 500");

        let err = NotionError::ParseError("invalid json".to_string());
        assert_eq!(err.to_string(), "パースエラー: invalid json");
    }
}
