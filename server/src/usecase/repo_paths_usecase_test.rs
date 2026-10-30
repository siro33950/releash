mod repo_paths_usecase_tests {
    use super::super::*;
    use crate::domain::app_config::value_objects::{TelemetryConfig, WorkflowConfig};
    use crate::domain::app_config::{AppConfig, AppConfigError};
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
    }

    impl AppConfigRepository for FakeRepoPaths {
        fn load(&self) -> Result<AppConfig, AppConfigError> {
            Ok(AppConfig::new(
                false,
                false,
                self.paths.read().clone(),
                String::new(),
                String::new(),
                TelemetryConfig {
                    crash_reporting: false,
                    performance_telemetry: false,
                },
                WorkflowConfig {
                    approval_auto_approve: false,
                },
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
            ))
        }

        fn save(&self, config: AppConfig) -> Result<(), AppConfigError> {
            if self.fail {
                return Err(AppConfigError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Other,
                        message: "boom".into(),
                    },
                ));
            }
            *self.paths.write() = config.repo_paths().to_vec();
            Ok(())
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
        (
            RepoPathsUsecase::new(repo.clone(), repo, subscriptions, Default::default()),
            changes,
        )
    }

    #[test]
    fn test_起動時登録_cwdのrootを追加し非repositoryと読取失敗は一覧を変えない() {
        todo!()
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
            .deps()
            .start_subscription(
                "client",
                &SubscriptionTarget::RepositoryPaths,
                "paths",
                None,
            )
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
        let usecase = RepoPathsUsecase::new(repo.clone(), repo, subscriptions, Default::default());
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
            .deps()
            .start_subscription(
                "client",
                &SubscriptionTarget::RepositoryPaths,
                "paths",
                None,
            )
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
        let usecase = RepoPathsUsecase::new(repo.clone(), repo, subscriptions, Default::default());
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
