pub(crate) mod notion_usecase_error_tests {
    use super::super::*;

    #[test]
    fn test_notion_usecaseエラー_未設定メッセージを維持する() {
        assert_eq!(
            NotionUsecaseError::ConfigNotFound.to_string(),
            NOTION_CONFIG_NOT_FOUND
        );
    }

    #[test]
    fn test_notion_usecaseエラー_notionエラー文字列を維持する() {
        let error = NotionUsecaseError::from(NotionError::ApiError("HTTP 500".to_string()));

        assert_eq!(error.to_string(), "API エラー: HTTP 500");
    }
}
