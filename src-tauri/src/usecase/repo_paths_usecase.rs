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
mod repo_paths_usecase_tests {
    use super::*;
    use crate::domain::repository::RepositoryError;
    use parking_lot::RwLock;

    /// ドメイン抽象のみに依存することを検証するための手書き fake。
    #[derive(Default)]
    struct FakeRepoPaths {
        paths: Arc<RwLock<Vec<String>>>,
        fail: bool,
    }

    impl RepoPathsRepository for FakeRepoPaths {
        fn get(&self) -> Vec<String> {
            self.paths.read().clone()
        }
        fn add(&self, path: &str) -> Result<bool, RepositoryError> {
            if self.fail {
                return Err(RepositoryError::External("boom".to_string()));
            }
            let mut p = self.paths.write();
            if p.iter().any(|x| x == path) {
                return Ok(false);
            }
            p.push(path.to_string());
            Ok(true)
        }
        fn remove(&self, path: &str) -> Result<bool, RepositoryError> {
            let mut p = self.paths.write();
            let before = p.len();
            p.retain(|x| x != path);
            Ok(p.len() != before)
        }
    }

    fn usecase_with(
        repo: Arc<FakeRepoPaths>,
    ) -> (
        RepoPathsUsecase,
        tokio::sync::broadcast::Receiver<crate::usecase::state_subscription::StateChangeSource>,
    ) {
        let subscriptions = crate::test_support::state_subscription::test_subscriptions();
        let changes = subscriptions.changes();
        (RepoPathsUsecase::new(repo, subscriptions), changes)
    }

    #[test]
    fn test_追加_取得_削除を委譲する() {
        let (uc, _) = usecase_with(Arc::new(FakeRepoPaths::default()));
        assert!(uc.add("/repo/a").unwrap());
        assert!(!uc.add("/repo/a").unwrap());
        assert_eq!(uc.get(), vec!["/repo/a".to_string()]);
        assert!(uc.remove("/repo/a").unwrap());
        assert!(uc.get().is_empty());
    }

    #[test]
    fn test_ドメインエラーをusecaseエラーへ変換する() {
        let (uc, mut changes) = usecase_with(Arc::new(FakeRepoPaths {
            fail: true,
            ..Default::default()
        }));
        assert_eq!(uc.add("/repo/a").unwrap_err().to_string(), "boom");
        assert!(changes.try_recv().is_err());
    }

    #[test]
    fn test_追加削除成功時のみ購読口へ通知する() {
        use crate::usecase::state_subscription::StateChangeSource;
        // Given
        let (uc, mut changes) = usecase_with(Arc::new(FakeRepoPaths::default()));
        // When
        assert!(uc.add("/repo/a").unwrap());
        assert!(!uc.add("/repo/a").unwrap());
        assert!(uc.remove("/repo/a").unwrap());
        assert!(!uc.remove("/repo/a").unwrap());
        // Then
        assert_eq!(changes.try_recv().unwrap(), StateChangeSource::Repositories);
        assert_eq!(changes.try_recv().unwrap(), StateChangeSource::Repositories);
        assert!(changes.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_連続した追加削除_各更新後の一覧を順に配信する() {
        use crate::test_support::state_subscription::{same, Event, StateSubscriptionEvent};
        use crate::usecase::state_subscription::{
            StateSubscriptionUsecase, StateValue, SubscriptionTarget,
        };
        use futures_util::StreamExt;

        // Given
        let subscriptions = StateSubscriptionUsecase::new(
            vec![],
            crate::test_support::state_subscription::read_driver(),
        );
        let repo = Arc::new(FakeRepoPaths {
            paths: subscriptions
                .test_repository_paths
                .as_ref()
                .unwrap()
                .clone(),
            fail: false,
        });
        let mut stream = Box::pin(subscriptions.open("client".into()).unwrap());
        assert!(matches!(
            stream.next().await,
            Some(StateSubscriptionEvent::Ready)
        ));
        subscriptions
            .start_subscription("client", &SubscriptionTarget::RepositoryPaths, None)
            .await
            .unwrap();
        assert!(matches!(
            stream.next().await,
            Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
        ));
        assert!(matches!(
            stream.next().await,
            Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
        ));

        // When
        let usecase = RepoPathsUsecase::new(repo, subscriptions);
        tokio::task::spawn_blocking(move || {
            assert!(usecase.add("/repo").unwrap());
            assert!(usecase.remove("/repo").unwrap());
        })
        .await
        .unwrap();

        // Then
        assert!(
            matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if same(&value, StateValue::RepositoryPaths(vec!["/repo".into()])))
        );
        assert!(
            matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if same(&value, StateValue::RepositoryPaths(vec![])))
        );
    }

    #[tokio::test]
    async fn test_並行に開始した追加削除_各更新後の一覧を順に配信する() {
        use crate::test_support::state_subscription::{same, Event, StateSubscriptionEvent};
        use crate::usecase::state_subscription::{
            StateSubscriptionUsecase, StateValue, SubscriptionTarget,
        };
        use futures_util::StreamExt;

        // Given
        let subscriptions = StateSubscriptionUsecase::new(
            vec!["/repo/a".into()],
            crate::test_support::state_subscription::read_driver(),
        );
        let repo = Arc::new(FakeRepoPaths {
            paths: subscriptions
                .test_repository_paths
                .as_ref()
                .unwrap()
                .clone(),
            fail: false,
        });
        let mut stream = Box::pin(subscriptions.open("client".into()).unwrap());
        assert!(matches!(
            stream.next().await,
            Some(StateSubscriptionEvent::Ready)
        ));
        subscriptions
            .start_subscription("client", &SubscriptionTarget::RepositoryPaths, None)
            .await
            .unwrap();
        assert!(matches!(
            stream.next().await,
            Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
        ));
        assert!(matches!(
            stream.next().await,
            Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
        ));
        let usecase = RepoPathsUsecase::new(repo, subscriptions);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let add = tokio::task::spawn_blocking({
            let usecase = usecase.clone();
            let barrier = barrier.clone();
            move || {
                barrier.wait();
                usecase.add("/repo/b").unwrap()
            }
        });
        let remove = tokio::task::spawn_blocking(move || {
            barrier.wait();
            usecase.remove("/repo/a").unwrap()
        });
        // When
        let (added, removed) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(add, remove)
        })
        .await
        .unwrap();
        assert!(added.unwrap());
        assert!(removed.unwrap());
        let first = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap();
        let second = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap();
        // Then
        assert!(
            matches!(first, StateSubscriptionEvent::Item(_, Event::Change(_, _, value))
            if same(&value, StateValue::RepositoryPaths(vec!["/repo/a".into(), "/repo/b".into()]))
                || same(&value, StateValue::RepositoryPaths(vec![])))
        );
        assert!(
            matches!(second, StateSubscriptionEvent::Item(_, Event::Change(_, _, value))
            if same(&value, StateValue::RepositoryPaths(vec!["/repo/b".into()])))
        );
    }
}
