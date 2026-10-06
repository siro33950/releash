mod secret_masker_tests {
    use super::super::*;

    #[test]
    fn test_secret_masker_既知パターンと設定値をredactする() {
        let masked = mask_sensitive_text(
            "password=secret123 ghp_abcdefghijklmnopqrstuvwxyz1234567890 custom-token",
            &["custom-token".to_string()],
        );
        assert!(!masked.contains("secret123"));
        assert!(!masked.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
        assert!(!masked.contains("custom-token"));
    }

    #[test]
    fn test_secret_masker_json文字列だけを再帰的にredactする() {
        let mut value = serde_json::json!({
            "nested": ["token=abc123456", {"x": "custom-secret"}],
            "number": 1
        });
        mask_json_strings(&mut value, &["custom-secret".to_string()]);
        let text = serde_json::to_string(&value).unwrap();
        assert!(!text.contains("abc123456"));
        assert!(!text.contains("custom-secret"));
    }

    #[test]
    fn test_secret_masker_contract名に依存せずartifactをredactする() {
        let value = serde_json::json!({
            "nested": ["configured-secret", {"message": "token=abc123456"}]
        });

        let masked = mask_sensitive_artifact(
            "unrelated-contract",
            value,
            &["configured-secret".to_string()],
        );
        let text = serde_json::to_string(&masked).unwrap();

        assert!(!text.contains("configured-secret"));
        assert!(!text.contains("abc123456"));
        assert!(text.contains("[REDACTED]"));
    }

    #[test]
    fn test_env_secret_values_名前と長さで抽出する() {
        let values = collect_secret_values_from_env_vars(vec![
            ("MY_TOKEN".to_string(), "SECRET_VALUE".to_string()),
            ("PATH".to_string(), "/bin:/usr/bin".to_string()),
            ("API_KEY".to_string(), "short".to_string()),
        ]);
        assert_eq!(values, vec!["SECRET_VALUE".to_string()]);
    }
}
