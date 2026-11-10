use super::*;
use crate::domain::app_config::{AppConfigError, RepositoryConfig};
use crate::domain::git::GitError;
use crate::usecase::test_helpers::FakeRepo;
use parking_lot::Mutex;
use tokio::sync::broadcast::{error::TryRecvError, Receiver};

struct Config {
    saved: Mutex<RepositoryConfig>,
    calls: Mutex<Vec<(&'static str, bool)>>,
    failure: Option<&'static str>,
    mutation: Arc<Mutex<()>>,
    git: Arc<FakeRepo>,
    publisher: StateSubscriptionUsecase,
    changes: Mutex<Receiver<StateChangeSource>>,
}

impl Config {
    fn new(value: RepositoryConfig) -> Self {
        let publisher = crate::test_support::state_subscription::test_subscriptions();
        let changes = Mutex::new(publisher.changes());
        Self {
            saved: Mutex::new(value),
            calls: Mutex::new(Vec::new()),
            failure: None,
            mutation: Default::default(),
            git: Arc::new(FakeRepo::default()),
            publisher,
            changes,
        }
    }

    fn usecase(self: &Arc<Self>) -> RepositoryConfigUsecase {
        RepositoryConfigUsecase::new(self.clone(), self.git.clone(), self.mutation.clone())
            .with_state_publisher(self.publisher.clone())
    }
}

impl RepositoryConfigRepository for Config {
    fn load(&self, repo_path: &str) -> Result<RepositoryConfig, AppConfigError> {
        assert_eq!(repo_path, self.saved.lock().repo_path());
        assert!(self.git.removed_branch_base_overrides.lock().is_empty());
        assert_eq!(self.changes.lock().try_recv(), Err(TryRecvError::Empty));
        self.calls.lock().push(("load", self.mutation.is_locked()));
        if self.failure == Some("load") {
            return Err(AppConfigError::InvalidInput("load failed".into()));
        }
        Ok(self.saved.lock().clone())
    }

    fn save(&self, value: RepositoryConfig) -> Result<(), AppConfigError> {
        assert_eq!(self.changes.lock().try_recv(), Err(TryRecvError::Empty));
        self.calls.lock().push(("save", self.mutation.is_locked()));
        if self.failure == Some("save") {
            return Err(AppConfigError::InvalidInput("save failed".into()));
        }
        *self.saved.lock() = value;
        Ok(())
    }
}

fn value(base: Option<&str>) -> RepositoryConfig {
    RepositoryConfig::new(
        "/repo".into(),
        base.map(str::to_owned),
        [("feature".into(), "release".into())].into(),
        Some("main".into()),
        Some(crate::domain::app_config::value_objects::NotionRepoConfig {
            api_token: "test-token".into(),
            database_id: "database".into(),
            property_mapping: Default::default(),
        }),
    )
}

#[test]
fn test_ベース設定保存_排他中に読み変更して保存し解除後に通知する() {
    for base in [Some("release"), None] {
        // Given
        let before = value(Some("develop"));
        let config = Arc::new(Config::new(before.clone()));
        // When
        config.usecase().set_releash_base("/repo", base).unwrap();
        // Then
        assert_eq!(*config.saved.lock(), value(base));
        assert_eq!(*config.calls.lock(), vec![("load", true), ("save", true)]);
        assert!(config.mutation.try_lock().is_some());
        assert_eq!(
            config.changes.lock().try_recv().unwrap(),
            StateChangeSource::Repository(vec!["/repo".into()])
        );
        assert_eq!(config.changes.lock().try_recv(), Err(TryRecvError::Empty));
    }
}

#[test]
fn test_ベース設定保存_読み取りまたは保存に失敗したら保存値を保ち通知しない() {
    for failure in ["load", "save"] {
        // Given
        let before = value(Some("develop"));
        let config = Arc::new(Config {
            failure: Some(failure),
            ..Config::new(before.clone())
        });
        // When
        let result = config.usecase().set_releash_base("/repo", Some("next"));
        // Then
        assert!(
            matches!(result, Err(UsecaseError::AppConfig(AppConfigError::InvalidInput(message))) if message == format!("{failure} failed"))
        );
        assert_eq!(*config.saved.lock(), before);
        let mut expected = vec![("load", true)];
        if failure == "save" {
            expected.push(("save", true));
        }
        assert_eq!(*config.calls.lock(), expected);
        assert!(config.mutation.try_lock().is_some());
        assert_eq!(config.changes.lock().try_recv(), Err(TryRecvError::Empty));
    }
}

#[test]
fn test_ベース設定整理_entityを読み現存しないブランチの個別指定だけ削除する() {
    for (existing, removed) in [
        (vec!["alive".into(), "other".into()], vec!["stale".into()]),
        (vec!["alive".into(), "stale".into()], Vec::new()),
    ] {
        // Given
        let config = Arc::new(Config::new(RepositoryConfig::new(
            "/repo".into(),
            Some("develop".into()),
            [
                ("alive".into(), "main".into()),
                ("stale".into(), "release".into()),
            ]
            .into(),
            Some("main".into()),
            None,
        )));
        // When
        config
            .usecase()
            .prune_stale_branch_bases("/repo", &existing)
            .unwrap();
        // Then
        assert_eq!(*config.calls.lock(), vec![("load", false)]);
        assert_eq!(
            *config.git.removed_branch_base_overrides.lock(),
            vec![("/repo".into(), removed.clone())]
        );
        assert_eq!(
            *config.git.set_branch_base_override_calls.lock(),
            removed
                .into_iter()
                .map(|branch| (branch, None))
                .collect::<Vec<_>>()
        );
        assert_eq!(config.changes.lock().try_recv(), Err(TryRecvError::Empty));
    }
}

#[test]
fn test_ベース設定整理_削除失敗を伝播し通知しない() {
    // Given
    let config = Arc::new(Config {
        git: Arc::new(FakeRepo {
            fail_cleanup: true,
            ..Default::default()
        }),
        ..Config::new(value(Some("develop")))
    });
    // When
    let result = config.usecase().prune_stale_branch_bases("/repo", &[]);
    // Then
    assert!(
        matches!(result, Err(UsecaseError::Repository(GitError::External(message))) if message == "cleanup failed")
    );
    assert_eq!(*config.calls.lock(), vec![("load", false)]);
    assert_eq!(
        *config.git.removed_branch_base_overrides.lock(),
        vec![("/repo".into(), vec!["feature".into()])]
    );
    assert_eq!(config.changes.lock().try_recv(), Err(TryRecvError::Empty));
}

#[test]
fn test_ベース設定テスト入口_保存した指定を再読取できる() {
    // Given
    let usecase = crate::usecase::test_helpers::repository_config_usecase();
    // When
    usecase.set_releash_base("/repo", Some("main")).unwrap();
    // Then
    assert_eq!(
        usecase.get_releash_base("/repo").unwrap().as_deref(),
        Some("main")
    );
    assert_eq!(usecase.get_releash_base("/other").unwrap(), None);
    // When
    usecase.set_releash_base("/repo", None).unwrap();
    // Then
    assert_eq!(usecase.get_releash_base("/repo").unwrap(), None);
}
