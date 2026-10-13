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

#[test]
fn test_terminal起動設定_nulを含む入力で既存設定を変えない() {
    // Given
    let mut app = super::AppSettings {
        close_to_tray: false,
        auto_launch: false,
        start_minimized: false,
        last_root_path: String::new(),
        last_repo_paths: vec![],
        external_editor: String::new(),
        terminal_startup_command: "echo ready".into(),
    };
    // When
    let result = app.update_terminal_startup_command("bad\0command".into());
    // Then
    assert!(result.is_err());
    assert_eq!(app.terminal_startup_command, "echo ready");
}
