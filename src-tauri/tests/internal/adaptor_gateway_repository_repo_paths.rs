pub(crate) mod repo_paths_gateway_tests {

    use releash_lib::test_support::integration::repository::ConfigRepository;
    use releash_lib::test_support::integration::repository::RepoPathsGateway;
    use releash_lib::test_support::integration::repository::RepoPathsRepository;
    use releash_lib::test_support::integration::repository::SharedRepoPaths;
    use releash_lib::test_support::integration::settings::AppConfig;
    use releash_lib::test_support::integration::settings::ReleashConfig;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_gateway(dir: &TempDir) -> RepoPathsGateway {
        let shared: SharedRepoPaths = Arc::new(parking_lot::RwLock::new(Vec::new()));
        let path = dir.path().join("releash.toml");
        let config = ReleashConfig::default();
        let app_config: Arc<dyn ConfigRepository> = Arc::new(AppConfig::new(config, path));
        RepoPathsGateway::new(shared, app_config)
    }

    #[test]
    pub fn test_追加_新規パス() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        let added = gw.add("/repo/a").unwrap();
        assert!(added);
        assert_eq!(gw.get(), vec!["/repo/a"]);
    }

    #[test]
    pub fn test_追加_重複はfalse() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        gw.add("/repo/a").unwrap();
        let added = gw.add("/repo/a").unwrap();
        assert!(!added);
        assert_eq!(gw.get().len(), 1);
    }

    #[test]
    pub fn test_追加_正規化してから重複判定() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        gw.add("/repo/a/").unwrap();
        let added = gw.add("/repo/a").unwrap();
        assert!(!added);
    }

    #[test]
    pub fn test_追加_空パスはfalse() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        let added = gw.add("").unwrap();
        assert!(!added);
        assert!(gw.get().is_empty());
    }

    #[test]
    pub fn test_削除_既存() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        gw.add("/repo/a").unwrap();
        let removed = gw.remove("/repo/a").unwrap();
        assert!(removed);
        assert!(gw.get().is_empty());
    }

    #[test]
    pub fn test_削除_未存在はfalse() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        let removed = gw.remove("/repo/a").unwrap();
        assert!(!removed);
    }

    #[test]
    pub fn test_追加_設定へ永続化() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        gw.add("/repo/a").unwrap();
        gw.add("/repo/b").unwrap();

        let cfg = gw.test_app_config().load().unwrap();
        assert_eq!(cfg.app.last_repo_paths, vec!["/repo/a", "/repo/b"]);
    }

    #[test]
    pub fn test_削除_設定へ永続化() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        gw.add("/repo/a").unwrap();
        gw.add("/repo/b").unwrap();
        gw.remove("/repo/a").unwrap();

        let cfg = gw.test_app_config().load().unwrap();
        assert_eq!(cfg.app.last_repo_paths, vec!["/repo/b"]);
    }
}
