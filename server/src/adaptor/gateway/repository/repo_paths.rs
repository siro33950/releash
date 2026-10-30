//! repo_paths 責務の gateway 実装。
//!
//! リポジトリパス一覧のメモリ共有リスト（`SharedRepoPaths`）への読み書きと、
//! アプリ設定ファイル（`app.last_repo_paths`）への永続化を封じ込める。
//! 永続化先は app_config ドメインが所有する設定機構であり、本 gateway は
//! それを利用するのみ（所有しない）。

use std::sync::Arc;

use crate::domain::app_config::{AppConfig, AppConfigError, AppConfigRepository};
use crate::domain::git::RepoPathsRepository;

/// 登録済みリポジトリパス一覧のメモリ共有リスト。
pub type SharedRepoPaths = Arc<parking_lot::RwLock<Vec<String>>>;

/// `RepoPathsRepository` の実装。
pub struct RepoPathsGateway {
    shared: SharedRepoPaths,
    app_config: Arc<dyn AppConfigRepository>,
}

impl RepoPathsGateway {
    pub fn new(shared: SharedRepoPaths, app_config: Arc<dyn AppConfigRepository>) -> Self {
        Self { shared, app_config }
    }
}

impl RepoPathsRepository for RepoPathsGateway {
    fn get(&self) -> Vec<String> {
        self.shared.read().clone()
    }
}

impl AppConfigRepository for RepoPathsGateway {
    fn load(&self) -> Result<AppConfig, AppConfigError> {
        self.app_config.load()
    }

    fn save(&self, config: AppConfig) -> Result<(), AppConfigError> {
        let mut paths = self.shared.write();
        let new_paths = config.repo_paths().to_vec();
        self.app_config.save(config)?;
        *paths = new_paths;
        Ok(())
    }
}

#[cfg(feature = "test-support")]
impl RepoPathsGateway {
    pub fn test_app_config(&self) -> Arc<dyn AppConfigRepository> {
        self.app_config.clone()
    }
}
