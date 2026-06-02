//! repo_paths 責務の gateway 実装。
//!
//! リポジトリパス一覧のメモリ共有リスト（`SharedRepoPaths`）への読み書きと、
//! アプリ設定ファイル（`app.last_repo_paths`）への永続化を封じ込める。
//! 永続化先は app_config ドメインが所有する設定機構であり、本 gateway は
//! それを利用するのみ（所有しない）。

use std::sync::Arc;

use crate::config::AppConfig;
use crate::domain::repository::{normalize_repo_path, RepoPathsRepository, RepositoryError};

/// 登録済みリポジトリパス一覧のメモリ共有リスト。
pub type SharedRepoPaths = Arc<parking_lot::RwLock<Vec<String>>>;

/// `RepoPathsRepository` の実装。
pub struct RepoPathsGateway {
    shared: SharedRepoPaths,
    app_config: Arc<AppConfig>,
}

impl RepoPathsGateway {
    pub fn new(shared: SharedRepoPaths, app_config: Arc<AppConfig>) -> Self {
        Self { shared, app_config }
    }
}

impl RepoPathsRepository for RepoPathsGateway {
    fn get(&self) -> Vec<String> {
        self.shared.read().clone()
    }

    fn add(&self, path: &str) -> Result<bool, RepositoryError> {
        let normalized = normalize_repo_path(path);
        if normalized.is_empty() {
            return Ok(false);
        }

        let mut paths = self.shared.write();
        if paths.iter().any(|p| p == &normalized) {
            return Ok(false);
        }

        let mut new_paths = paths.clone();
        new_paths.push(normalized);

        self.app_config
            .with_config_mut(|config| {
                config.app.last_repo_paths = new_paths.clone();
                Ok(())
            })
            .map_err(RepositoryError::External)?;

        *paths = new_paths;
        Ok(true)
    }

    fn remove(&self, path: &str) -> Result<bool, RepositoryError> {
        let normalized = normalize_repo_path(path);

        let mut paths = self.shared.write();
        let new_paths: Vec<String> = paths
            .iter()
            .filter(|p| *p != &normalized)
            .cloned()
            .collect();
        if new_paths.len() == paths.len() {
            return Ok(false);
        }

        self.app_config
            .with_config_mut(|config| {
                config.app.last_repo_paths = new_paths.clone();
                Ok(())
            })
            .map_err(RepositoryError::External)?;

        *paths = new_paths;
        Ok(true)
    }
}

#[cfg(test)]
mod repo_paths_gateway_tests {
    use super::*;
    use crate::config::{AppConfig, ReleashConfig};
    use tempfile::TempDir;

    fn make_gateway(dir: &TempDir) -> RepoPathsGateway {
        let shared: SharedRepoPaths = Arc::new(parking_lot::RwLock::new(Vec::new()));
        let path = dir.path().join("releash.toml");
        let config = ReleashConfig::default();
        let app_config = Arc::new(AppConfig::new(config, path));
        RepoPathsGateway::new(shared, app_config)
    }

    #[test]
    fn test_追加_新規パス() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        let added = gw.add("/repo/a").unwrap();
        assert!(added);
        assert_eq!(gw.get(), vec!["/repo/a"]);
    }

    #[test]
    fn test_追加_重複はfalse() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        gw.add("/repo/a").unwrap();
        let added = gw.add("/repo/a").unwrap();
        assert!(!added);
        assert_eq!(gw.get().len(), 1);
    }

    #[test]
    fn test_追加_正規化してから重複判定() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        gw.add("/repo/a/").unwrap();
        let added = gw.add("/repo/a").unwrap();
        assert!(!added);
    }

    #[test]
    fn test_追加_空パスはfalse() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        let added = gw.add("").unwrap();
        assert!(!added);
        assert!(gw.get().is_empty());
    }

    #[test]
    fn test_削除_既存() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        gw.add("/repo/a").unwrap();
        let removed = gw.remove("/repo/a").unwrap();
        assert!(removed);
        assert!(gw.get().is_empty());
    }

    #[test]
    fn test_削除_未存在はfalse() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        let removed = gw.remove("/repo/a").unwrap();
        assert!(!removed);
    }

    #[test]
    fn test_追加_設定へ永続化() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        gw.add("/repo/a").unwrap();
        gw.add("/repo/b").unwrap();

        let cfg = gw.app_config.get_config().unwrap();
        assert_eq!(cfg.app.last_repo_paths, vec!["/repo/a", "/repo/b"]);
    }

    #[test]
    fn test_削除_設定へ永続化() {
        let dir = TempDir::new().unwrap();
        let gw = make_gateway(&dir);

        gw.add("/repo/a").unwrap();
        gw.add("/repo/b").unwrap();
        gw.remove("/repo/a").unwrap();

        let cfg = gw.app_config.get_config().unwrap();
        assert_eq!(cfg.app.last_repo_paths, vec!["/repo/b"]);
    }
}
