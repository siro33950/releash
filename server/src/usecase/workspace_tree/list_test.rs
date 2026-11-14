use super::*;
use crate::domain::git_host::{PrInfo, PrStatus};

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
        dirty_count: Fetched::ready(0),
    }
}

fn failed_tree(message: &str) -> Fetched<WorkspaceTree> {
    Fetched {
        value: None,
        error: Some(failure(message)),
    }
}

fn failure(message: &str) -> crate::domain::failure::WorkFailure {
    crate::domain::failure::WorkFailure {
        kind: crate::domain::failure::Failure::Technical(
            crate::domain::failure::TechnicalFailureNature::Other,
        ),
        message: message.into(),
    }
}

#[test]
fn test_一覧の合成_prの状態でpr情報とmerge済みを決める() {
    // Given
    let repositories = vec![RepositoryValues {
        path: "/a".into(),
        name: "repository".into(),
        worktrees: Fetched::ready(vec![
            values("/a", "open", false),
            values("/a/merged", "merged", false),
            values("/a/git", "git-merged", true),
        ]),
        pull_requests: Fetched::ready(PrStatus {
            open_prs: Vec::from([(
                "open".into(),
                PrInfo {
                    number: 42,
                    url: "https://example.test/pull/42".into(),
                    state: crate::domain::git_host::PrState::Open,
                    draft: false,
                },
            )]),
            completed_prs: vec!["open".into(), "merged".into()]
                .into_iter()
                .map(|name: String| {
                    (
                        name,
                        PrInfo {
                            number: 1,
                            url: "merged-url".into(),
                            state: crate::domain::git_host::PrState::Merged,
                            draft: false,
                        },
                    )
                })
                .collect(),
        }),
    }];

    // When
    let list = compose(repositories, vec![Fetched::default(); 3]);

    // Then
    let rows = list.repositories[0].worktrees.value.as_ref().unwrap();
    assert_eq!(
        rows[0].state_pull_request,
        Some(PrInfo {
            number: 42,
            url: "https://example.test/pull/42".into(),
            state: crate::domain::git_host::PrState::Open,
            draft: false,
        })
    );
    assert!(!rows[0].merged);
    assert!(rows[1].merged);
    assert_eq!(
        rows[1].state_pull_request.as_ref().unwrap().state,
        crate::domain::git_host::PrState::Merged
    );
    assert!(rows[2].merged);
}

#[test]
fn test_一覧の合成_prが未取得ならworktreeのmerge済みをそのまま使う() {
    // Given
    let repositories = vec![RepositoryValues {
        path: "/a".into(),
        name: "repository".into(),
        worktrees: Fetched::ready(vec![
            values("/a", "main", false),
            values("/a/git", "git-merged", true),
        ]),
        pull_requests: Fetched::default(),
    }];

    // When
    let list = compose(repositories, vec![Fetched::default(); 2]);

    // Then
    let rows = list.repositories[0].worktrees.value.as_ref().unwrap();
    assert!(!rows[0].merged);
    assert!(rows[1].merged);
    assert!(rows.iter().all(|row| row.state_pull_request.is_none()));
}

#[test]
fn test_一覧の合成_読めているworktreeの並び順に実行木を割り当て失敗と削除中を保つ() {
    // Given
    let mut deleting = values("/a/two", "two", false);
    deleting.deleting = true;
    deleting.dirty_count = Fetched::ready(3);
    let repositories = vec![
        RepositoryValues {
            path: "/a".into(),
            name: "owner/a".into(),
            worktrees: Fetched {
                value: Some(vec![values("/a", "one", false), deleting]),
                error: Some(failure("scan failed")),
            },
            pull_requests: Fetched::default(),
        },
        RepositoryValues {
            path: "/b".into(),
            name: "b".into(),
            worktrees: Fetched {
                value: None,
                error: Some(failure("not a repository")),
            },
            pull_requests: Fetched::default(),
        },
        RepositoryValues {
            path: "/c".into(),
            name: "c".into(),
            worktrees: Fetched::ready(vec![values("/c", "three", false)]),
            pull_requests: Fetched::default(),
        },
    ];

    // When
    let list = compose(
        repositories,
        vec![failed_tree("one"), failed_tree("two"), failed_tree("three")],
    );

    // Then
    assert_eq!(
        list.repositories
            .iter()
            .map(|repository| (repository.path.as_str(), repository.name.as_str()))
            .collect::<Vec<_>>(),
        [("/a", "owner/a"), ("/b", "b"), ("/c", "c")]
    );
    let first = &list.repositories[0];
    assert_eq!(
        first
            .worktrees
            .error
            .as_ref()
            .map(|failure| failure.message.as_str()),
        Some("scan failed")
    );
    let rows = first.worktrees.value.as_ref().unwrap();
    assert_eq!(rows[0].tree, failed_tree("one"));
    assert_eq!(rows[1].tree, failed_tree("two"));
    assert!(rows[1].deleting);
    assert_eq!(rows[1].dirty_count, Fetched::ready(3));
    assert_eq!(
        list.repositories[1].worktrees,
        Fetched {
            value: None,
            error: Some(failure("not a repository")),
        }
    );
    let rows = list.repositories[2].worktrees.value.as_ref().unwrap();
    assert_eq!(rows[0].tree, failed_tree("three"));
}

#[test]
fn test_一覧の合成_未コミット数とprの読取失敗を未設定と区別する() {
    // Given
    let mut row = values("/repo", "main", false);
    row.dirty_count = Fetched {
        value: None,
        error: Some(failure("scan failed")),
    };
    let repositories = vec![RepositoryValues {
        path: "/repo".into(),
        name: "repository".into(),
        worktrees: Fetched::ready(vec![row]),
        pull_requests: Fetched {
            value: None,
            error: Some(failure("PR failed")),
        },
    }];
    // When
    let list = compose(repositories, vec![Fetched::default()]);
    // Then
    let row = &list.repositories[0].worktrees.value.as_ref().unwrap()[0];
    assert_eq!(row.dirty_count.value, None);
    assert_eq!(
        row.dirty_count
            .error
            .as_ref()
            .map(|failure| failure.message.as_str()),
        Some("scan failed")
    );
    assert_eq!(
        row.pull_request_error
            .as_ref()
            .map(|failure| failure.message.as_str()),
        Some("PR failed")
    );
}

#[test]
fn test_一覧の合成_完了済みprとopenのprを別々に保持する() {
    // Given
    for state in [
        crate::domain::git_host::PrState::Merged,
        crate::domain::git_host::PrState::Closed,
    ] {
        let completed = PrInfo {
            number: 42,
            url: "https://example.test/42".into(),
            state,
            draft: false,
        };
        let list = compose(
            vec![RepositoryValues {
                path: "/repo".into(),
                name: "repository".into(),
                worktrees: Fetched::ready(vec![values("/repo", "feature", false)]),
                pull_requests: Fetched::ready(PrStatus {
                    open_prs: Default::default(),
                    completed_prs: [("feature".into(), completed.clone())].into(),
                }),
            }],
            vec![Fetched::default()],
        );
        // Then
        let row = &list.repositories[0].worktrees.value.as_ref().unwrap()[0];
        assert_eq!(row.state_pull_request, Some(completed));
    }
}

struct RegisteredPaths(Vec<String>);

impl crate::domain::git::RepoPathsRepository for RegisteredPaths {
    fn get(&self) -> Vec<String> {
        self.0.clone()
    }
}

impl crate::domain::app_config::AppConfigRepository for RegisteredPaths {
    fn load(
        &self,
    ) -> Result<crate::domain::app_config::AppConfig, crate::domain::app_config::AppConfigError>
    {
        panic!("listing repositories must not load configuration")
    }

    fn save(
        &self,
        _: crate::domain::app_config::AppConfig,
    ) -> Result<(), crate::domain::app_config::AppConfigError> {
        panic!("listing repositories must not save configuration")
    }
}

struct UnusedGitHostProvider;

#[async_trait::async_trait]
impl crate::domain::git_host::GitHostProvider for UnusedGitHostProvider {
    async fn fetch_pr_status(
        &self,
        _: &str,
    ) -> Result<PrStatus, crate::domain::git_host::GitHostError> {
        panic!("listing repositories must not fetch pull requests")
    }

    async fn list_issues(
        &self,
        _: &str,
    ) -> Result<Vec<crate::domain::git_host::IssueInfo>, crate::domain::git_host::GitHostError>
    {
        panic!("listing repositories must not fetch issues")
    }
}

fn repository_list(
    paths: &[&str],
    git: Arc<crate::usecase::test_helpers::FakeRepo>,
) -> WorkspaceListUsecase {
    use crate::usecase::git_host::test_helpers::{FakeIssueCache, FakePrCache};
    use crate::usecase::repository_state::test_helpers::{counting_service, CountingScanner};
    use crate::usecase::repository_state::worktree::NoopRepositoryStateWatcher;

    let paths = Arc::new(RegisteredPaths(
        paths.iter().map(|path| (*path).into()).collect(),
    ));
    WorkspaceListUsecase::new(
        Arc::new(RepoPathsUsecase::new(
            paths.clone(),
            paths,
            crate::test_support::state_subscription::test_subscriptions(),
            Default::default(),
        )),
        Arc::new(crate::usecase::test_helpers::usecase(git)),
        Arc::new(counting_service(
            Arc::new(CountingScanner::default()),
            Arc::new(NoopRepositoryStateWatcher),
        )),
        Arc::new(crate::usecase::workflow::mod_tests::empty_usecase()),
        Arc::new(GitHostUsecase::new(
            Arc::new(UnusedGitHostProvider),
            Arc::new(FakePrCache::default()),
            Arc::new(FakeIssueCache::default()),
        )),
    )
}

#[tokio::test]
async fn test_repository一覧_登録順にoriginを読み一覧全体で決めた名前を対応させる() {
    // Given
    let paths = ["/z-local", "/a-local", "/middle"];
    let git = Arc::new(crate::usecase::test_helpers::FakeRepo {
        origins: [
            (
                "/z-local".into(),
                Ok(Some("git@github.com:first/shared.git".into())),
            ),
            (
                "/a-local".into(),
                Ok(Some("https://github.com/second/unique.git".into())),
            ),
            (
                "/middle".into(),
                Ok(Some("https://github.com/third/shared.git".into())),
            ),
        ]
        .into(),
        ..Default::default()
    });
    let usecase = repository_list(&paths, git.clone());
    // When
    let repositories = usecase.read().await.unwrap().repositories;
    // Then
    assert_eq!(*git.origin_url_calls.lock(), paths);
    assert_eq!(
        repositories
            .iter()
            .map(|repository| (repository.path.as_str(), repository.name.as_str()))
            .collect::<Vec<_>>(),
        [
            ("/z-local", "first/shared"),
            ("/a-local", "unique"),
            ("/middle", "third/shared"),
        ]
    );
}

#[tokio::test]
async fn test_repository一覧_originがなければフォルダ名にして後続も返す() {
    // Given
    let paths = ["/z-local", "/work/shared", "/a-local", "/work/articles"];
    let git = Arc::new(crate::usecase::test_helpers::FakeRepo {
        origins: [
            (
                "/z-local".into(),
                Ok(Some("https://github.com/first/shared.git".into())),
            ),
            ("/work/shared".into(), Ok(None)),
            (
                "/a-local".into(),
                Ok(Some("git@github.com:second/unique.git".into())),
            ),
            ("/work/articles".into(), Ok(None)),
        ]
        .into(),
        ..Default::default()
    });
    let usecase = repository_list(&paths, git.clone());
    // When
    let repositories = usecase.read().await.unwrap().repositories;
    // Then
    assert_eq!(*git.origin_url_calls.lock(), paths);
    assert_eq!(
        repositories
            .iter()
            .map(|repository| (repository.path.as_str(), repository.name.as_str()))
            .collect::<Vec<_>>(),
        [
            ("/z-local", "first/shared"),
            ("/work/shared", "shared"),
            ("/a-local", "unique"),
            ("/work/articles", "articles"),
        ]
    );
}

#[tokio::test]
async fn test_repository一覧_origin読取失敗をそのまま返す() {
    // Given
    let git = Arc::new(crate::usecase::test_helpers::FakeRepo {
        origins: [
            ("/work/first".into(), Ok(None)),
            (
                "/work/failed".into(),
                Err(GitError::External("origin read failed".into())),
            ),
        ]
        .into(),
        ..Default::default()
    });
    let usecase = repository_list(&["/work/first", "/work/failed"], git);
    // When
    let result = usecase.read().await;
    // Then
    assert!(matches!(
        result,
        Err(UsecaseError::Repository(GitError::External(message)))
            if message == "origin read failed"
    ));
}

#[tokio::test]
async fn test_一覧の読取_収集処理の失敗を空の一覧に変えない() {
    // Given
    struct PanickingPaths;
    impl crate::domain::git::RepoPathsRepository for PanickingPaths {
        fn get(&self) -> Vec<String> {
            panic!("repository collection failed")
        }
    }
    impl crate::domain::app_config::AppConfigRepository for PanickingPaths {
        fn load(
            &self,
        ) -> Result<crate::domain::app_config::AppConfig, crate::domain::app_config::AppConfigError>
        {
            unreachable!()
        }
        fn save(
            &self,
            _: crate::domain::app_config::AppConfig,
        ) -> Result<(), crate::domain::app_config::AppConfigError> {
            unreachable!()
        }
    }
    let mut usecase = repository_list(&[], Arc::default());
    usecase.repositories = Arc::new(RepoPathsUsecase::new(
        Arc::new(PanickingPaths),
        Arc::new(PanickingPaths),
        crate::test_support::state_subscription::test_subscriptions(),
        Default::default(),
    ));
    // When
    let error = usecase.read().await.unwrap_err();
    // Then
    let UsecaseError::Repository(GitError::Technical(error)) = error else {
        panic!("expected repository technical failure: {error:?}");
    };
    assert_eq!(
        error.nature,
        crate::domain::failure::TechnicalFailureNature::Other
    );
    assert!(error.message.contains("repository collection failed"));
}
