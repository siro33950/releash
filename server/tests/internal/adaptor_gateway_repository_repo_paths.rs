pub(crate) mod repo_paths_gateway_tests {

    use releashd::test_support::integration::repository::AppConfigRepository;
    use releashd::test_support::integration::repository::RepoPathsGateway;
    use releashd::test_support::integration::repository::RepoPathsRepository;
    use releashd::test_support::integration::repository::SharedRepoPaths;
    use releashd::test_support::integration::settings::AppConfig;
    use releashd::test_support::integration::settings::ReleashConfig;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_gateway(dir: &TempDir) -> RepoPathsGateway {
        let shared: SharedRepoPaths = Arc::new(parking_lot::RwLock::new(Vec::new()));
        let path = dir.path().join("releash.toml");
        let config = ReleashConfig::default();
        let app_config: Arc<dyn AppConfigRepository> = Arc::new(AppConfig::new(config, path));
        RepoPathsGateway::new(shared, app_config)
    }

    #[test]
    pub fn test_追加_設定へ永続化() {
        todo!()
    }

    #[test]
    pub fn test_削除_設定へ永続化() {
        todo!()
    }
}
