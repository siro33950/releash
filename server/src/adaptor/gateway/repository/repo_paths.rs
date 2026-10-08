//! repo_paths 責務の gateway 実装。
//!
//! リポジトリパス一覧のメモリ共有リスト（`SharedRepoPaths`）への読み書きと、
//! アプリ設定ファイル（`app.last_repo_paths`）への永続化を封じ込める。
//! 永続化先は app_config ドメインが所有する設定機構であり、本 gateway は
//! それを利用するのみ（所有しない）。

use std::sync::Arc;

use crate::domain::app_config::ConfigRepository;
use crate::domain::repository::{normalize_repo_path, RepoPathsRepository, RepositoryError};

/// 登録済みリポジトリパス一覧のメモリ共有リスト。
pub type SharedRepoPaths = Arc<parking_lot::RwLock<Vec<String>>>;

/// `RepoPathsRepository` の実装。
pub struct RepoPathsGateway {
    shared: SharedRepoPaths,
    app_config: Arc<dyn ConfigRepository>,
}

impl RepoPathsGateway {
    pub fn new(shared: SharedRepoPaths, app_config: Arc<dyn ConfigRepository>) -> Self {
        Self { shared, app_config }
    }

    fn save_paths(&self, paths: Vec<String>) -> Result<(), RepositoryError> {
        self.app_config
            .update(Box::new(move |config| {
                config.app.last_repo_paths = paths;
                Ok(())
            }))
            .map_err(|e| RepositoryError::External(e.to_string()))
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

        self.save_paths(new_paths.clone())?;

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

        self.save_paths(new_paths.clone())?;

        *paths = new_paths;
        Ok(true)
    }
}

#[cfg(feature = "test-support")]
impl RepoPathsGateway {
    pub fn test_app_config(&self) -> Arc<dyn ConfigRepository> {
        self.app_config.clone()
    }
}
