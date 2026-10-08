use releashd::test_support::integration::platform::Fetched;
use releashd::test_support::integration::platform::GitHostError;
use releashd::test_support::integration::platform::GitHostProvider;
use releashd::test_support::integration::platform::GitHostUsecase;
use releashd::test_support::integration::platform::IssueInfo;
use releashd::test_support::integration::platform::PrInfo;
use releashd::test_support::integration::platform::PrStatus;
use releashd::test_support::integration::platform::RepoPathsUsecase;
use releashd::test_support::integration::platform::RepositoryStateService;
use releashd::test_support::integration::repository::Worktree;
use releashd::test_support::integration::workspace::WorkspaceList;
use releashd::test_support::integration::workspace::WorkspaceListUsecase;
use releashd::test_support::integration::workspace::WorkspaceListWorktree;
use std::sync::Arc;

use crate::usecase_repository_state_test_helpers_runtime::CanonicalWorktreePathNormalizer;
use releashd::test_support::integration::platform::NoopRepositoryStateWatcher;
use releashd::test_support::integration::platform::TestRepositoryStateWorkerRuntime;
use releashd::test_support::integration::repository::RepoPathsRepository;
use releashd::test_support::integration::repository::RepositoryError;
use releashd::test_support::integration::subscriptions::StateChangeSource;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Default)]
struct Paths(parking_lot::RwLock<Vec<String>>);

impl RepoPathsRepository for Paths {
    fn get(&self) -> Vec<String> {
        self.0.read().clone()
    }
    fn add(&self, path: &str) -> Result<bool, RepositoryError> {
        self.0.write().push(path.to_string());
        Ok(true)
    }
    fn remove(&self, path: &str) -> Result<bool, RepositoryError> {
        self.0.write().retain(|registered| registered != path);
        Ok(true)
    }
}

/// 取得を `release` まで止められる PR の取得元。
struct PullRequests {
    status: PrStatus,
    release: Option<Arc<parking_lot::Mutex<std::sync::mpsc::Receiver<()>>>>,
}

#[async_trait::async_trait]
impl GitHostProvider for PullRequests {
    async fn fetch_pr_status(&self, _: &str) -> Result<PrStatus, GitHostError> {
        if let Some(release) = &self.release {
            let release = release.clone();
            releashd::test_support::integration::platform::spawn_blocking(move || {
                release.lock().recv_timeout(Duration::from_secs(5))
            })
            .await
            .unwrap()
            .map_err(|error| GitHostError::External(error.to_string()))?;
        }
        Ok(self.status.clone())
    }
    async fn list_issues(&self, _: &str) -> Result<Vec<IssueInfo>, GitHostError> {
        Ok(Vec::new())
    }
}

struct Fixture {
    usecase: WorkspaceListUsecase,
    repository_state: Arc<RepositoryStateService>,
    repositories: Arc<RepoPathsUsecase>,
    subscriptions: releashd::test_support::integration::subscriptions::StateSubscriptionUsecase,
    repo: git2::Repository,
    path: String,
    main_branch: String,
    worktrees: tempfile::TempDir,
    _repo_dir: tempfile::TempDir,
    _data_dir: tempfile::TempDir,
}

impl Fixture {
    fn new(pull_requests: PullRequests) -> Self {
        let (repo_dir, repo) = crate::test_support_git::create_test_repo();
        crate::test_support_git::create_initial_commit(&repo);
        let path = repo_dir
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let main_branch = repo.head().unwrap().shorthand().unwrap().to_string();
        let data_dir = tempfile::tempdir().unwrap();
        let subscriptions =
            releashd::test_support::integration::subscriptions::test_subscriptions();
        let repository =
            Arc::new(releashd::test_support::integration::platform::build_repository_usecase());
        let repositories = Arc::new(RepoPathsUsecase::new(
            Arc::new(Paths(parking_lot::RwLock::new(vec![path.clone()]))),
            subscriptions.clone(),
        ));
        let repository_state = Arc::new(RepositoryStateService::new(
            Arc::new(
                releashd::test_support::integration::repository::RepositoryStateRepositoryGateway::new(
                    repository.clone(),
                ),
            ),
            Arc::new(
                releashd::test_support::integration::repository::DefaultRepositoryScanner::new(
                    repository.clone(),
                    Arc::new(releashd::test_support::integration::platform::build_code_usecase()),
                ),
            ),
            subscriptions.clone(),
            Arc::new(NoopRepositoryStateWatcher),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(CanonicalWorktreePathNormalizer),
            releashd::test_support::integration::subscriptions::repository_driver(),
        ));
        let workflows_dir = data_dir.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        let workflow = Arc::new(
            releashd::test_support::integration::platform::build_workflow_usecase(
                data_dir.path().join("data"),
                Some(workflows_dir),
            ),
        );
        let git_host = Arc::new(
            GitHostUsecase::new(
                Arc::new(pull_requests),
                Arc::new(releashd::test_support::integration::platform::LatestPrStatuses::default()),
                Arc::new(releashd::test_support::integration::platform::InMemoryTtlCache::<
                    Vec<IssueInfo>,
                >::new(
                    releashd::test_support::integration::platform::CacheTtl::EXTERNAL_INFORMATION
                )),
            )
            .with_state_publisher(subscriptions.clone()),
        );
        Self {
            usecase: WorkspaceListUsecase::new(
                repositories.clone(),
                repository,
                repository_state.clone(),
                workflow,
                git_host,
            ),
            repository_state,
            repositories,
            subscriptions,
            repo,
            path,
            main_branch,
            worktrees: tempfile::tempdir().unwrap(),
            _repo_dir: repo_dir,
            _data_dir: data_dir,
        }
    }

    fn add_worktree(&self, name: &str) -> String {
        let path = self.worktrees.path().join(name);
        self.repo.worktree(name, &path, None).unwrap();
        path.canonicalize().unwrap().to_string_lossy().into_owned()
    }

    /// 一覧が求める対象を監視し、条件を満たす一覧が読めるまで待つ。
    async fn watch_until(&self, ready: impl Fn(&[WorkspaceListWorktree]) -> bool) -> WorkspaceList {
        let mut watched = std::collections::HashSet::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                for path in self.usecase.watch_paths().0 {
                    if watched.insert(path.clone()) {
                        self.repository_state.start_git_dir_watching(&path).unwrap();
                    }
                }
                let list = self.usecase.read().await.unwrap();
                if list.repositories[0]
                    .worktrees
                    .value
                    .as_deref()
                    .is_some_and(&ready)
                {
                    return list;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("workspace list did not reach the expected state")
    }
}

fn rows(list: &WorkspaceList) -> &[WorkspaceListWorktree] {
    list.repositories[0].worktrees.value.as_deref().unwrap()
}

#[tokio::test]
pub async fn test_一覧の読み取り_監視前は取得中を返す() {
    // Given
    let fixture = Fixture::new(PullRequests {
        status: PrStatus::default(),
        release: None,
    });
    std::fs::write(
        std::path::Path::new(&fixture.path).join("untracked.txt"),
        "new",
    )
    .unwrap();
    fixture.add_worktree("feature");

    // When
    let before = fixture.usecase.read().await.unwrap();

    // Then
    assert_eq!(before.repositories.len(), 1);
    assert_eq!(before.repositories[0].path, fixture.path);
    assert_eq!(before.repositories[0].worktrees, Fetched::default());
}

#[tokio::test]
pub async fn test_一覧の読み取り_走査後にworktreeと変更の数と実行木を並べる() {
    // Given
    let fixture = Fixture::new(PullRequests {
        status: PrStatus::default(),
        release: None,
    });
    std::fs::write(
        std::path::Path::new(&fixture.path).join("untracked.txt"),
        "new",
    )
    .unwrap();
    let feature = fixture.add_worktree("feature");

    // When
    let list = fixture
        .watch_until(|rows| {
            rows.len() == 2
                && rows[0].dirty_count.value == Some(1)
                && rows[1].dirty_count.value == Some(0)
        })
        .await;

    // Then
    let rows = rows(&list);
    assert!(list.repositories[0].worktrees.error.is_none());
    assert!(rows[0].worktree.is_main);
    assert_eq!(rows[0].worktree.branch, fixture.main_branch);
    assert_eq!(rows[0].worktree.path, fixture.path);
    assert_eq!(rows[1].worktree.branch, "feature");
    assert_eq!(rows[1].worktree.path, feature);
    assert_eq!(rows[1].dirty_count.value, Some(0));
    for row in rows {
        assert!(!row.deleting);
        assert!(row.pull_request.is_none());
        assert!(row.tree.error.is_none());
        assert!(row.tree.value.as_ref().unwrap().nodes().is_empty());
    }
}

#[tokio::test]
pub async fn test_手動更新_走査済みの値を保持しpr取得完了まで待つ() {
    // Given
    let (release, blocked) = std::sync::mpsc::channel();
    let fixture = Fixture::new(PullRequests {
        status: PrStatus {
            open_prs: HashMap::from([(
                "feature".into(),
                PrInfo {
                    number: 42,
                    url: "https://example.test/pull/42".into(),
                },
            )]),
            merged_branches: Vec::new(),
        },
        release: Some(Arc::new(parking_lot::Mutex::new(blocked))),
    });
    fixture.watch_until(|rows| rows.len() == 1).await;
    fixture.add_worktree("feature");

    // When
    let refreshing = tokio::spawn({
        let usecase = fixture.usecase.clone();
        async move { usecase.refresh().await }
    });
    fixture.watch_until(|rows| rows.len() == 2).await;
    assert!(!refreshing.is_finished());
    let scanned = fixture.usecase.read().await.unwrap();

    release.send(()).unwrap();
    refreshing.await.unwrap();
    // Then
    assert_eq!(rows(&scanned).len(), 2);
    assert_eq!(rows(&scanned)[1].worktree.branch, "feature");
    assert!(rows(&scanned)[1].pull_request.is_none());
}

#[tokio::test]
pub async fn test_手動更新_pr取得後に前の一覧へprを反映する() {
    // Given
    let (release, blocked) = std::sync::mpsc::channel();
    let fixture = Fixture::new(PullRequests {
        status: PrStatus {
            open_prs: HashMap::from([(
                "feature".into(),
                PrInfo {
                    number: 42,
                    url: "https://example.test/pull/42".into(),
                },
            )]),
            merged_branches: Vec::new(),
        },
        release: Some(Arc::new(parking_lot::Mutex::new(blocked))),
    });
    fixture.watch_until(|rows| rows.len() == 1).await;
    fixture.add_worktree("feature");
    let mut changes =
        releashd::test_support::integration::subscriptions::changes(&fixture.subscriptions);

    let refreshing = tokio::spawn({
        let usecase = fixture.usecase.clone();
        async move { usecase.refresh().await }
    });
    fixture.watch_until(|rows| rows.len() == 2).await;
    assert!(!refreshing.is_finished());

    // When
    release.send(()).unwrap();
    refreshing.await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while changes.recv().await.unwrap() != StateChangeSource::WorkspaceList {}
    })
    .await
    .expect("pull request change must be notified");

    let list = fixture.usecase.read().await.unwrap();
    // Then
    assert_eq!(
        rows(&list)[1].pull_request,
        Some(PrInfo {
            number: 42,
            url: "https://example.test/pull/42".into(),
        })
    );
}

#[tokio::test]
pub async fn test_repositoryの削除_一覧と監視の対象から外れる() {
    // Given
    let fixture = Fixture::new(PullRequests {
        status: PrStatus::default(),
        release: None,
    });
    fixture.watch_until(|rows| rows.len() == 1).await;

    // When
    fixture.repositories.remove(&fixture.path).unwrap();

    // Then
    assert!(fixture
        .usecase
        .read()
        .await
        .unwrap()
        .repositories
        .is_empty());
    assert!(fixture.usecase.watch_paths().0.is_empty());
}

#[tokio::test]
pub async fn test_一覧の読取_収集処理の失敗を空の一覧に変えない() {
    // Given
    struct PanickingPaths;
    impl RepoPathsRepository for PanickingPaths {
        fn get(&self) -> Vec<String> {
            panic!("repository collection failed")
        }
        fn add(&self, _: &str) -> Result<bool, RepositoryError> {
            unreachable!()
        }
        fn remove(&self, _: &str) -> Result<bool, RepositoryError> {
            unreachable!()
        }
    }
    let mut fixture = Fixture::new(PullRequests {
        status: PrStatus::default(),
        release: None,
    });
    fixture
        .usecase
        .test_replace_repositories(Arc::new(RepoPathsUsecase::new(
            Arc::new(PanickingPaths),
            fixture.subscriptions.clone(),
        )));
    // When
    let error = fixture.usecase.read().await.unwrap_err();
    // Then
    assert_eq!(
        error.nature,
        releashd::test_support::integration::platform::TechnicalFailureNature::Other
    );
    assert!(error.message.contains("repository collection failed"));
}

#[tokio::test]
pub async fn test_watch_paths_一覧とrootの読み取り失敗をrepositoryに対応させる() {
    use releashd::test_support::integration::platform::RepositoryScanner;
    use releashd::test_support::integration::platform::RepositorySnapshotParts;
    use releashd::test_support::integration::platform::RepositoryStateError;
    use releashd::test_support::integration::platform::RepositoryStateRepository;
    struct Root(bool);
    impl RepositoryStateRepository for Root {
        fn main_repo_path(&self, path: &str) -> Result<String, RepositoryStateError> {
            if self.0 {
                Err(RepositoryStateError::Watcher("root failed".into()))
            } else {
                Ok(path.into())
            }
        }
    }
    struct Scan;
    #[async_trait::async_trait]
    impl RepositoryScanner for Scan {
        fn scan(&self, _: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
            Ok(RepositorySnapshotParts {
                status: vec![],
                diff_stats: vec![],
                dirty_count: 0,
                diff_file_tree: vec![],
                staged_diff_file_tree: vec![],
                changes_diff_file_tree: vec![],
            })
        }
        async fn scan_async(
            &self,
            path: &str,
        ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
            self.scan(path)
        }
        fn scan_worktrees(&self, _: &str) -> Result<Vec<Worktree>, RepositoryStateError> {
            Err(RepositoryStateError::Watcher("list failed".into()))
        }
        fn prune_stale_branch_bases(&self, _: &str) -> Result<(), RepositoryStateError> {
            Ok(())
        }
    }
    for root_failed in [false, true] {
        let mut fixture = Fixture::new(PullRequests {
            status: PrStatus::default(),
            release: None,
        });
        let state = Arc::new(RepositoryStateService::new(
            Arc::new(Root(root_failed)),
            Arc::new(Scan),
            fixture.subscriptions.clone(),
            Arc::new(NoopRepositoryStateWatcher),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(CanonicalWorktreePathNormalizer),
            releashd::test_support::integration::subscriptions::repository_driver(),
        ));
        fixture.usecase.test_replace_repository_state(state.clone());
        if !root_failed {
            state.start_git_dir_watching(&fixture.path).unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                while state.worktrees(&fixture.path).error.is_none() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        let (paths, failures) = fixture.usecase.watch_paths();
        assert!(paths.contains(&fixture.path));
        assert!(!failures.is_empty());
        assert!(failures.iter().all(|(path, _)| path == &fixture.path));
        assert!(failures
            .iter()
            .any(|(_, failure)| failure.message.contains(if root_failed {
                "root failed"
            } else {
                "list failed"
            })));
    }
}
