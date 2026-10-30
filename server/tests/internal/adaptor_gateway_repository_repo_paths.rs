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
        // Given
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);
        let mut config = gw.load().unwrap();
        config.add_repo_path("/repo/a");
        config.add_repo_path("/repo/b");

        // When
        gw.save(config).unwrap();

        // Then
        assert_eq!(gw.get(), vec!["/repo/a", "/repo/b"]);
        assert_eq!(gw.test_app_config().load().unwrap().repo_paths(), gw.get());
        let saved = releashd::test_support::integration::settings::load_or_create_config(
            &dir.path().join("releash.toml"),
        )
        .unwrap();
        assert_eq!(saved.app.last_repo_paths, gw.get());
    }

    #[test]
    pub fn test_削除_設定へ永続化() {
        // Given
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);
        let mut config = gw.load().unwrap();
        config.add_repo_path("/repo/a");
        config.add_repo_path("/repo/b");
        gw.save(config).unwrap();
        let mut config = gw.load().unwrap();
        config.remove_repo_path("/repo/a");

        // When
        gw.save(config).unwrap();

        // Then
        assert_eq!(gw.get(), vec!["/repo/b"]);
        assert_eq!(gw.test_app_config().load().unwrap().repo_paths(), gw.get());
        let saved = releashd::test_support::integration::settings::load_or_create_config(
            &dir.path().join("releash.toml"),
        )
        .unwrap();
        assert_eq!(saved.app.last_repo_paths, gw.get());
    }

    #[test]
    pub fn test_保存失敗_共有パスと設定を保持する() {
        // Given
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);
        let mut config = gw.load().unwrap();
        config.add_repo_path("/repo/a");
        gw.save(config).unwrap();
        let path = dir.path().join("releash.toml");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        // When
        let mut config = gw.load().unwrap();
        config.remove_repo_path("/repo/a");
        let result = gw.save(config);

        // Then
        assert!(result.is_err());
        assert_eq!(gw.get(), vec!["/repo/a"]);
        assert_eq!(gw.test_app_config().load().unwrap().repo_paths(), gw.get());
    }
}
