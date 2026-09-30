use super::*;
use crate::domain::git_host::{GitHostError, GitHostProvider, IssueInfo, PrInfo, PrStatus};
use crate::domain::repository::{RepoPathsRepository, RepositoryError};
use crate::usecase::repository_state::runtime::tests_support::{
    CanonicalWorktreePathNormalizer, TestRepositoryStateWorkerRuntime,
};
use crate::usecase::repository_state::worktree::NoopRepositoryStateWatcher;
use crate::usecase::state_subscription::StateChangeSource;
use std::collections::HashMap;
use std::time::Duration;

fn worktree(path: &str, branch: &str, is_merged: bool) -> Worktree {
    Worktree {
        name: branch.to_string(),
        path: path.to_string(),
        branch: branch.to_string(),
        is_main: false,
        is_locked: false,
        is_merged,
    }
}

fn values(path: &str, branch: &str, is_merged: bool) -> WorktreeValues {
    WorktreeValues {
        worktree: worktree(path, branch, is_merged),
        deleting: false,
        dirty_count: 0,
    }
}

fn failed_tree(message: &str) -> Fetched<WorkspaceTree> {
    Fetched {
        value: None,
        error: Some(message.to_string()),
    }
}

#[test]
fn test_一覧の合成_prの状態でpr情報とmerge済みを決める() {
    // Given
    let repositories = vec![RepositoryValues {
        path: "/a".into(),
        worktrees: Fetched::ready(vec![
            values("/a", "open", false),
            values("/a/merged", "merged", false),
            values("/a/git", "git-merged", true),
        ]),
        pull_requests: Some(PrStatus {
            open_prs: HashMap::from([(
                "open".into(),
                PrInfo {
                    number: 42,
                    url: "https://example.test/pull/42".into(),
                },
            )]),
            merged_branches: vec!["open".into(), "merged".into()],
        }),
    }];

    // When
    let list = compose(repositories, vec![Fetched::default(); 3]);

    // Then
    let rows = list.repositories[0].worktrees.value.as_ref().unwrap();
    assert_eq!(
        rows[0].pull_request,
        Some(PrInfo {
            number: 42,
            url: "https://example.test/pull/42".into(),
        })
    );
    assert!(!rows[0].merged);
    assert!(rows[1].merged);
    assert_eq!(rows[1].pull_request, None);
    assert!(rows[2].merged);
}

#[test]
fn test_一覧の合成_prが未取得ならworktreeのmerge済みをそのまま使う() {
    // Given
    let repositories = vec![RepositoryValues {
        path: "/a".into(),
        worktrees: Fetched::ready(vec![
            values("/a", "main", false),
            values("/a/git", "git-merged", true),
        ]),
        pull_requests: None,
    }];

    // When
    let list = compose(repositories, vec![Fetched::default(); 2]);

    // Then
    let rows = list.repositories[0].worktrees.value.as_ref().unwrap();
    assert!(!rows[0].merged);
    assert!(rows[1].merged);
    assert!(rows.iter().all(|row| row.pull_request.is_none()));
}

#[test]
fn test_一覧の合成_読めているworktreeの並び順に実行木を割り当て失敗と削除中を保つ() {
    // Given
    let mut deleting = values("/a/two", "two", false);
    deleting.deleting = true;
    deleting.dirty_count = 3;
    let repositories = vec![
        RepositoryValues {
            path: "/a".into(),
            worktrees: Fetched {
                value: Some(vec![values("/a", "one", false), deleting]),
                error: Some("scan failed".into()),
            },
            pull_requests: None,
        },
        RepositoryValues {
            path: "/b".into(),
            worktrees: Fetched {
                value: None,
                error: Some("not a repository".into()),
            },
            pull_requests: None,
        },
        RepositoryValues {
            path: "/c".into(),
            worktrees: Fetched::ready(vec![values("/c", "three", false)]),
            pull_requests: None,
        },
    ];

    // When
    let list = compose(
        repositories,
        vec![failed_tree("one"), failed_tree("two"), failed_tree("three")],
    );

    // Then
    let first = &list.repositories[0];
    assert_eq!(first.worktrees.error.as_deref(), Some("scan failed"));
    let rows = first.worktrees.value.as_ref().unwrap();
    assert_eq!(rows[0].tree, failed_tree("one"));
    assert_eq!(rows[1].tree, failed_tree("two"));
    assert!(rows[1].deleting);
    assert_eq!(rows[1].dirty_count, 3);
    assert_eq!(
        list.repositories[1].worktrees,
        Fetched {
            value: None,
            error: Some("not a repository".into()),
        }
    );
    let rows = list.repositories[2].worktrees.value.as_ref().unwrap();
    assert_eq!(rows[0].tree, failed_tree("three"));
}

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
    release: Option<parking_lot::Mutex<std::sync::mpsc::Receiver<()>>>,
}

impl GitHostProvider for PullRequests {
    fn fetch_pr_status(&self, _: &str) -> Result<PrStatus, GitHostError> {
        if let Some(release) = &self.release {
            release
                .lock()
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| GitHostError::External(error.to_string()))?;
        }
        Ok(self.status.clone())
    }
    fn list_issues(&self, _: &str) -> Result<Vec<IssueInfo>, GitHostError> {
        Ok(Vec::new())
    }
}

struct Fixture {
    usecase: WorkspaceListUsecase,
    repository_state: Arc<RepositoryStateService>,
    repositories: Arc<RepoPathsUsecase>,
    subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
    repo: git2::Repository,
    path: String,
    main_branch: String,
    worktrees: tempfile::TempDir,
    _repo_dir: tempfile::TempDir,
    _data_dir: tempfile::TempDir,
}

impl Fixture {
    fn new(pull_requests: PullRequests) -> Self {
        use crate::adaptor::controller::wiring;
        let (repo_dir, repo) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&repo);
        let path = repo_dir
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let main_branch = repo.head().unwrap().shorthand().unwrap().to_string();
        let data_dir = tempfile::tempdir().unwrap();
        let subscriptions = crate::test_support::state_subscription::test_subscriptions();
        let repository = Arc::new(wiring::build_repository_usecase());
        let repositories = Arc::new(RepoPathsUsecase::new(
            Arc::new(Paths(parking_lot::RwLock::new(vec![path.clone()]))),
            subscriptions.clone(),
        ));
        let repository_state = Arc::new(RepositoryStateService::new(
            Arc::new(
                crate::adaptor::gateway::repository::state::RepositoryStateRepositoryGateway::new(
                    repository.clone(),
                ),
            ),
            Arc::new(
                crate::adaptor::gateway::repository::scanner::DefaultRepositoryScanner::new(
                    repository.clone(),
                    Arc::new(wiring::build_code_usecase()),
                ),
            ),
            subscriptions.clone(),
            Arc::new(NoopRepositoryStateWatcher),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(CanonicalWorktreePathNormalizer),
        ));
        let workflows_dir = data_dir.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        let workflow = Arc::new(wiring::build_workflow_usecase(
            data_dir.path().join("data"),
            Some(workflows_dir),
        ));
        let git_host = Arc::new(
            GitHostUsecase::new(
                Arc::new(pull_requests),
                Arc::new(crate::adaptor::gateway::git_host::LatestPrStatuses::default()),
                Arc::new(crate::adaptor::gateway::git_host::InMemoryTtlCache::<
                    Vec<IssueInfo>,
                >::new(
                    crate::domain::git_host::CacheTtl::EXTERNAL_INFORMATION
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
                for path in self.usecase.watch_paths() {
                    if watched.insert(path.clone()) {
                        self.repository_state.start_git_dir_watching(&path).unwrap();
                    }
                }
                let list = self.usecase.read().await;
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
async fn test_一覧の読み取り_監視前は取得中で走査後にworktreeと変更の数と実行木を並べる() {
    // Given: main に未コミットの変更が 1 件あり、linked worktree が 1 つある Repository
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

    // When: まだ何も監視していない
    let before = fixture.usecase.read().await;

    // Then: 取得中であり、項目なしではない
    assert_eq!(before.repositories.len(), 1);
    assert_eq!(before.repositories[0].path, fixture.path);
    assert_eq!(before.repositories[0].worktrees, Fetched::default());

    // When: 監視が始まり、走査が終わる
    let list = fixture
        .watch_until(|rows| rows.len() == 2 && rows[0].dirty_count == 1)
        .await;

    // Then
    let rows = rows(&list);
    assert!(list.repositories[0].worktrees.error.is_none());
    assert!(rows[0].worktree.is_main);
    assert_eq!(rows[0].worktree.branch, fixture.main_branch);
    assert_eq!(rows[0].worktree.path, fixture.path);
    assert_eq!(rows[1].worktree.branch, "feature");
    assert_eq!(rows[1].worktree.path, feature);
    assert_eq!(rows[1].dirty_count, 0);
    for row in rows {
        assert!(!row.deleting);
        assert!(row.pull_request.is_none());
        assert!(row.tree.error.is_none());
        assert!(row.tree.value.as_ref().unwrap().nodes().is_empty());
    }
}

#[tokio::test]
async fn test_手動更新_走査をやり直して終わりまで待ちprの取得は待たない() {
    // Given: PR の取得が終わらない Repository
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
        release: Some(parking_lot::Mutex::new(blocked)),
    });
    fixture.watch_until(|rows| rows.len() == 1).await;
    fixture.add_worktree("feature");
    let mut changes = crate::test_support::state_subscription::changes(&fixture.subscriptions);

    // When
    tokio::time::timeout(Duration::from_secs(5), fixture.usecase.refresh())
        .await
        .expect("refresh must not wait for pull requests");
    let scanned = fixture.usecase.read().await;

    // Then: 走査の結果は読め、PR はまだ無い
    assert_eq!(rows(&scanned).len(), 2);
    assert_eq!(rows(&scanned)[1].worktree.branch, "feature");
    assert!(rows(&scanned)[1].pull_request.is_none());

    // When: PR が取れる
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while changes.recv().await.unwrap() != StateChangeSource::WorkspaceList {}
    })
    .await
    .expect("pull request change must be notified");

    // Then
    let list = fixture.usecase.read().await;
    assert_eq!(
        rows(&list)[1].pull_request,
        Some(PrInfo {
            number: 42,
            url: "https://example.test/pull/42".into(),
        })
    );
}

#[tokio::test]
async fn test_repositoryの削除_一覧と監視の対象から外れる() {
    // Given
    let fixture = Fixture::new(PullRequests {
        status: PrStatus::default(),
        release: None,
    });
    fixture.watch_until(|rows| rows.len() == 1).await;

    // When
    fixture.repositories.remove(&fixture.path).unwrap();

    // Then
    assert!(fixture.usecase.read().await.repositories.is_empty());
    assert!(fixture.usecase.watch_paths().is_empty());
}
