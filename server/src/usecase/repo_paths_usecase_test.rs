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
        use crate::usecase::test_helpers::{usecase, FakeRepo};
        for condition in ["repository", "not-repository", "unreadable"] {
            // Given
            let fake = Arc::new(FakeRepo {
                no_main_repo_path: condition == "not-repository",
                fail_main_repo_path: condition == "unreadable",
                ..Default::default()
            });
            let (paths, mut changes) = usecase_with(Arc::new(FakeRepoPaths::default()));
            // When
            let result = paths.initialize_from_cwd(&usecase(fake.clone()));
            // Then
            assert_eq!(*fake.find_main_repo_path_calls.lock(), vec!["/cwd"]);
            if condition == "unreadable" {
                assert_eq!(
                    result.unwrap_err().to_string(),
                    "main repo path is unavailable"
                );
            } else {
                result.unwrap();
            }
            if condition == "repository" {
                assert_eq!(paths.get(), vec!["/main"]);
                assert_eq!(
                    changes.try_recv().unwrap(),
                    crate::usecase::state_subscription::StateChangeSource::Repositories
                );
            } else {
                assert!(paths.get().is_empty());
            }
            assert!(changes.try_recv().is_err());
        }
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
}
