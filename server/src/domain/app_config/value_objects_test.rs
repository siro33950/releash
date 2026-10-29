mod value_objects_tests {
    use super::super::*;

    #[test]
    fn test_notion_config_debugでapi_tokenをマスクする() {
        let config = NotionRepoConfig {
            api_token: "ntn_secret_token".to_string(),
            database_id: "db-1".to_string(),
            property_mapping: NotionPropertyMapping::default(),
        };

        let output = format!("{config:?}");

        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("ntn_secret_token"));
    }
    #[test]
    fn test_notion設定_tokenかdatabaseが空または空白だけなら未設定() {
        // Given
        let cases = [("", "db"), ("token", ""), (" \t", "db"), ("token", "\n ")];
        // When
        let results: Vec<_> = cases
            .into_iter()
            .map(|(token, database)| {
                NotionRepoConfig {
                    api_token: token.into(),
                    database_id: database.into(),
                    property_mapping: Default::default(),
                }
                .is_configured()
            })
            .collect();
        // Then
        assert_eq!(results, vec![false; cases.len()]);
    }
    #[test]
    fn test_notion設定_tokenとdatabaseが空白以外を持てば設定済み() {
        // Given
        let cases = [("token", "db"), (" token ", " db ")];
        // When
        let results: Vec<_> = cases
            .into_iter()
            .map(|(token, database)| {
                NotionRepoConfig {
                    api_token: token.into(),
                    database_id: database.into(),
                    property_mapping: Default::default(),
                }
                .is_configured()
            })
            .collect();
        // Then
        assert_eq!(results, vec![true; cases.len()]);
    }
}
