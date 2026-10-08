pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn test_validate結果_未設定はプロパティ空で返る() {
        let result = NotionValidationResult::not_configured();

        assert_eq!(result.status, NotionConfigStatus::NotConfigured);
        assert!(result.properties.is_empty());
    }
}
