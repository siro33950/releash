pub(crate) mod notion_error_tests {
    use super::super::*;

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
