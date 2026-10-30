use std::sync::Arc;

use crate::domain::app_config::RepositoryConfigRepository;
use crate::domain::git::GitService;
use crate::usecase::repository_error::UsecaseError;
use crate::usecase::state_subscription::{StateChangeSource, StateSubscriptionUsecase};

/// repository ごとのベース設定の読み取りと変更を受け付ける。
#[derive(Clone)]
pub struct RepositoryConfigUsecase {
    repository_config: Arc<dyn RepositoryConfigRepository>,
    git: Arc<dyn GitService>,
    config_mutation: Arc<parking_lot::Mutex<()>>,
    state_publisher: Option<StateSubscriptionUsecase>,
}

impl RepositoryConfigUsecase {
    /// 設定の保存先、ブランチ個別の指定の保存先と変更時の排他を受け取る。
    pub fn new(
        repository_config: Arc<dyn RepositoryConfigRepository>,
        git: Arc<dyn GitService>,
        config_mutation: Arc<parking_lot::Mutex<()>>,
    ) -> Self {
        Self {
            repository_config,
            git,
            config_mutation,
            state_publisher: None,
        }
    }

    /// 設定の変更を購読へ知らせる配信先を受け取る。
    pub fn with_state_publisher(mut self, publisher: StateSubscriptionUsecase) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    /// repository の設定変更を購読へ知らせる。
    fn notify_repository_changed(&self, path: &str) {
        if let Some(publisher) = &self.state_publisher {
            publisher.notify(StateChangeSource::Repository(vec![path.into()]));
        }
    }

    /// repository に保存されたベースブランチの指定を返す。
    pub fn get_releash_base(&self, repo_path: &str) -> Result<Option<String>, UsecaseError> {
        let config = self.repository_config.load(repo_path)?;
        Ok(config.base_branch().map(str::to_owned))
    }

    /// 保存された設定から、指定したブランチのベースを返す。
    pub fn get_branch_base(
        &self,
        repo_path: &str,
        branch_name: &str,
    ) -> Result<Option<String>, UsecaseError> {
        let config = self.repository_config.load(repo_path)?;
        Ok(config.resolve_base_branch(branch_name).map(str::to_owned))
    }

    /// repository の base branch の変更を保存する。
    pub fn set_releash_base(
        &self,
        repo_path: &str,
        base: Option<&str>,
    ) -> Result<(), UsecaseError> {
        let guard = self.config_mutation.lock();
        let mut config = self.repository_config.load(repo_path)?;
        config.set_base_branch(base);
        self.repository_config.save(config)?;
        drop(guard);
        self.notify_repository_changed(repo_path);
        Ok(())
    }

    /// ブランチ個別のベース指定を保存する。None は指定を削除する。
    pub fn set_branch_base_override(
        &self,
        repo_path: &str,
        branch_name: &str,
        base: Option<&str>,
    ) -> Result<(), UsecaseError> {
        self.git
            .set_branch_base_override(repo_path, branch_name, base)?;
        self.notify_repository_changed(repo_path);
        Ok(())
    }

    /// 現存しないブランチに残った個別のベース指定を削除する。
    pub fn prune_stale_branch_bases(
        &self,
        repo_path: &str,
        existing_branches: &[String],
    ) -> Result<(), UsecaseError> {
        let config = self.repository_config.load(repo_path)?;
        let stale_branches = config.stale_branch_overrides(existing_branches);
        self.git
            .remove_branch_base_overrides(repo_path, &stale_branches)?;
        Ok(())
    }
}
