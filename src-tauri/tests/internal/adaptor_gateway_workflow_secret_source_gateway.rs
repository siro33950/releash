pub(crate) mod tests {

    use releash_lib::test_support::integration::repository::ConfigSecretRepository;
    use releash_lib::test_support::integration::settings::AppConfig;
    use releash_lib::test_support::integration::settings::ReleashConfig;
    use releash_lib::test_support::integration::workflow::SecretSourceGateway;
    use releash_lib::test_support::integration::workflow::WorkflowSecretSourceConfigGateway;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    pub fn collects_and_normalizes_configured_secret_values() {
        let tmp = TempDir::new().unwrap();
        let mut config = ReleashConfig::default();
        config.notion.insert(
            "/repo".to_string(),
            releash_lib::test_support::integration::settings::NotionRepoConfigModel {
                api_token: "token-12345678".to_string(),
                database_id: "db".to_string(),
                property_mapping: Default::default(),
            },
        );
        let app_config: Arc<dyn ConfigSecretRepository> =
            Arc::new(AppConfig::new(config, tmp.path().join("config.toml")));

        let secrets = WorkflowSecretSourceConfigGateway::new(app_config)
            .configured_secret_values()
            .unwrap();

        assert!(secrets.contains(&"token-12345678".to_string()));
        assert_eq!(
            secrets
                .iter()
                .filter(|secret| secret.as_str() == "token-12345678")
                .count(),
            1
        );
    }
}
