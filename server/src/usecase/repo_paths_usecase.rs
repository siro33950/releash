//! repo_paths 責務のユースケース（リポジトリパス一覧の取得・追加・削除）。
//!
//! Repository パスの更新と購読への配信を順序付ける。
//! 「追加/削除が成功した時だけ変更を通知する」という業務手順を
//! 購読 Usecase 経由で実行し、controller は
//! ユースケースを呼ぶ薄い入口に徹する。

use std::sync::Arc;

use crate::domain::repository::RepoPathsRepository;

use super::repository_error::UsecaseError;

#[derive(Clone)]
pub struct RepoPathsUsecase {
    repo: Arc<dyn RepoPathsRepository>,
    subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
    mutation: Arc<parking_lot::Mutex<()>>,
}

impl RepoPathsUsecase {
    pub fn new(
        repo: Arc<dyn RepoPathsRepository>,
        subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        Self {
            repo,
            subscriptions,
            mutation: Arc::new(parking_lot::Mutex::new(())),
        }
    }

    pub(crate) fn initialize_from_cwd(
        &self,
        repository: &crate::usecase::repository_usecase::RepositoryUsecase,
    ) -> Result<(), UsecaseError> {
        if let Some(root) = repository.find_main_repo_path(&repository.get_cwd()?)? {
            self.add(&root)?;
        }
        Ok(())
    }

    pub fn get(&self) -> Vec<String> {
        self.repo.get()
    }

    /// 追加できた場合に `true`、既存・空文字で追加されなかった場合に `false`。
    /// 追加成功時のみ、更新後の一覧が配信されるまで待つ。
    pub fn add(&self, path: &str) -> Result<bool, UsecaseError> {
        let _mutation = self.mutation.lock();
        let added = self.repo.add(path)?;
        if added {
            self.notify_changed();
        }
        Ok(added)
    }

    /// 削除できた場合に `true`、存在せず削除されなかった場合に `false`。
    /// 削除成功時のみ、更新後の一覧が配信されるまで待つ。
    pub fn remove(&self, path: &str) -> Result<bool, UsecaseError> {
        let _mutation = self.mutation.lock();
        let removed = self.repo.remove(path)?;
        if removed {
            self.notify_changed();
        }
        Ok(removed)
    }

    fn notify_changed(&self) {
        self.subscriptions.notify_and_wait(
            crate::usecase::state_subscription::StateChangeSource::Repositories,
            &crate::usecase::state_subscription::SubscriptionTarget::RepositoryPaths,
        );
    }
}

#[cfg(test)]
#[path = "repo_paths_usecase_test.rs"]
mod repo_paths_usecase_tests;
