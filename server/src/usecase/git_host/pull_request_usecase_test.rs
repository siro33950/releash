use super::*;
use crate::domain::failure::{BusinessFailure, TechnicalFailureNature};
use crate::domain::git::{Branch, RepositoryStatusScan, Worktree};
use crate::domain::git_host::*;
use crate::usecase::git_host::test_helpers::{FakeIssueCache, FakePrCache};
use std::sync::Mutex;
#[derive(Default)]
struct Git {
    origin: Mutex<Option<String>>,
    main_results: Mutex<std::collections::VecDeque<Result<String, GitError>>>,
    current_results: Mutex<std::collections::VecDeque<Result<String, GitError>>>,
}
impl GitService for Git {
    fn list_branches(&self, _repo_path: &str) -> Result<Vec<Branch>, GitError> {
        Ok(vec![
            Branch::remote("origin/topic"),
            Branch::remote("origin/main"),
        ])
    }
    fn default_branch(&self, _repo_path: &str) -> Result<Option<String>, GitError> {
        Ok(Some("main".into()))
    }
    fn current(&self, _repo_path: &str) -> Result<String, GitError> {
        self.current_results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok("topic".into()))
    }
    fn create_branch(&self, _repo_path: &str, _branch_name: &str) -> Result<(), GitError> {
        panic!("unused")
    }
    fn status_scan(&self, _repo_path: &str) -> Result<RepositoryStatusScan, GitError> {
        panic!("unused")
    }
    fn origin_url(&self, _repo_path: &str) -> Result<Option<String>, GitError> {
        Ok(Some(
            self.origin
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| "https://github.com/a/b".into()),
        ))
    }
    fn main_repo_path(&self, _any_path: &str) -> Result<String, GitError> {
        self.main_results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok("/repo".into()))
    }
    fn find_main_repo_path(&self, _any_path: &str) -> Result<Option<String>, GitError> {
        panic!("unused")
    }
    fn list_worktrees(&self, _repo_path: &str) -> Result<Vec<Worktree>, GitError> {
        panic!("unused")
    }
    fn create_worktree(
        &self,
        _repo_path: &str,
        _worktree_path: &str,
        _branch: &str,
        _create_branch: bool,
        _base_branch: Option<&str>,
    ) -> Result<Worktree, GitError> {
        panic!("unused")
    }
    fn validate_removal(
        &self,
        _repo_path: &str,
        _worktree_path: &str,
        _force: bool,
    ) -> Result<String, GitError> {
        panic!("unused")
    }
    fn remove(
        &self,
        _repo_path: &str,
        _worktree_path: &str,
        _force: bool,
    ) -> Result<Option<String>, GitError> {
        panic!("unused")
    }
    fn set_branch_base_override(
        &self,
        _repo_path: &str,
        _branch_name: &str,
        _base: Option<&str>,
    ) -> Result<(), GitError> {
        panic!("unused")
    }
    fn remove_branch_base_overrides(
        &self,
        _repo_path: &str,
        _branches: &[String],
    ) -> Result<(), GitError> {
        panic!("unused")
    }
    fn resolve_current_base_branch(&self, _path_hint: &str) -> Result<Option<String>, GitError> {
        panic!("unused")
    }
    fn resolve_base_commit_oid(
        &self,
        _path_hint: &str,
        _base_name: &str,
    ) -> Result<Option<String>, GitError> {
        panic!("unused")
    }
}
#[derive(Default)]
struct Cache(Mutex<CachedResult<PullRequest>>, Mutex<Vec<String>>);
impl PullRequestCache for Cache {
    fn hold(&self, path: &str) {
        self.1.lock().unwrap().push(path.into());
    }
    fn forget(&self, _: &str) {
        *self.0.lock().unwrap() = CachedResult::default();
    }
    fn result(&self, _: &str) -> CachedResult<PullRequest> {
        self.0.lock().unwrap().clone()
    }
    fn record(&self, _: &str, value: Result<PullRequest, GitHostError>) {
        self.0.lock().unwrap().record(value);
    }
}
struct Host {
    calls: Mutex<Vec<String>>,
    status: Mutex<Result<PrStatus, GitHostError>>,
    detail: Mutex<Result<PullRequest, GitHostError>>,
    log: Mutex<Result<String, GitHostError>>,
    operation_error: Mutex<Option<GitHostError>>,
}
impl Default for Host {
    fn default() -> Self {
        Self {
            operation_error: Mutex::new(None),
            calls: Mutex::new(Vec::new()),
            status: Mutex::new(Ok(PrStatus {
                open_prs: vec![("topic".into(), pr().info)],
                completed_prs: vec![],
            })),
            detail: Mutex::new(Ok(pr())),
            log: Mutex::new(Err(GitHostError::External("offline".into()))),
        }
    }
}
#[async_trait::async_trait]
impl GitHostProvider for Host {
    async fn fetch_pr_status(&self, _: &str) -> Result<PrStatus, GitHostError> {
        self.calls.lock().unwrap().push("list".into());
        self.status.lock().unwrap().clone()
    }
    async fn list_issues(&self, _: &str) -> Result<Vec<IssueInfo>, GitHostError> {
        panic!("unused")
    }
    async fn fetch_pull_request(&self, _: &str, number: u64) -> Result<PullRequest, GitHostError> {
        assert_eq!(number, 42);
        self.calls.lock().unwrap().push("detail".into());
        self.detail.lock().unwrap().clone()
    }
    async fn create_pull_request(
        &self,
        _: &str,
        head: &str,
        new: &NewPullRequest,
    ) -> Result<(), GitHostError> {
        assert_eq!(
            (head, new.title(), new.body(), new.base(), new.draft()),
            ("topic", "title", "body", "main", true)
        );
        self.calls.lock().unwrap().push("create".into());
        self.operation_error
            .lock()
            .unwrap()
            .clone()
            .map_or(Ok(()), Err)
    }
    async fn post_pull_request_comment(
        &self,
        _: &str,
        number: u64,
        text: &PrCommentText,
    ) -> Result<(), GitHostError> {
        assert_eq!((number, text.as_str()), (42, "comment"));
        self.calls.lock().unwrap().push("comment".into());
        self.operation_error
            .lock()
            .unwrap()
            .clone()
            .map_or(Ok(()), Err)
    }
    async fn fetch_check_log(&self, path: &str, job: ActionsJobId) -> Result<String, GitHostError> {
        assert_eq!((path, job), ("/repo", ActionsJobId(7)));
        self.calls.lock().unwrap().push("log".into());
        self.log.lock().unwrap().clone()
    }
}
fn pr() -> PullRequest {
    PullRequest {
        info: PrInfo {
            number: 42,
            url: "url".into(),
            state: PrState::Open,
            draft: false,
        },
        title: "title".into(),
        body: "body".into(),
        checks: Checks::default(),
        conversation: PrConversation::default(),
    }
}
#[tokio::test]
async fn test_pr操作_作成と投稿の後に取得し空入力は外部へ送らない() {
    // Given
    let host = Arc::new(Host::default());
    let git_host = Arc::new(GitHostUsecase::new(
        host.clone(),
        Arc::new(FakePrCache::default()),
        Arc::new(FakeIssueCache::default()),
    ));
    let usecase = PullRequestUsecase::new(
        Arc::new(Git::default()),
        git_host,
        host.clone(),
        Arc::new(Cache::default()),
    );
    // When
    usecase
        .create(
            "/worktree",
            "title".into(),
            "body".into(),
            "main".into(),
            true,
        )
        .await
        .unwrap();
    usecase
        .post_comment("/worktree", 42, "comment".into())
        .await
        .unwrap();
    // Then
    assert_eq!(
        *host.calls.lock().unwrap(),
        ["create", "list", "detail", "comment", "list", "detail"]
    );
    assert!(matches!(
        usecase.read("/worktree").unwrap().value,
        Some(BranchPullRequest::Present(_))
    ));
    assert_eq!(
        usecase
            .create("/worktree", "".into(), "".into(), "main".into(), false)
            .await
            .unwrap_err(),
        WorkFailure {
            kind: Failure::Business(BusinessFailure::Other),
            message: PullRequestCreationError::EmptyTitle.to_string()
        }
    );
    assert_eq!(
        usecase
            .post_comment("/worktree", 42, "".into())
            .await
            .unwrap_err(),
        WorkFailure {
            kind: Failure::Business(BusinessFailure::Other),
            message: PrCommentError::Empty.to_string()
        }
    );
    assert_eq!(host.calls.lock().unwrap().len(), 6);
}

fn reader(host: Arc<Host>, status: Arc<FakePrCache>, details: Arc<Cache>) -> PullRequestUsecase {
    PullRequestUsecase::new(
        Arc::new(Git::default()),
        Arc::new(GitHostUsecase::new(
            host.clone(),
            status,
            Arc::new(FakeIssueCache::default()),
        )),
        host,
        details,
    )
}
#[test]
fn test_pr読み取り_保持値と失敗を返し外部取得しない() {
    // Given
    let host = Arc::new(Host::default());
    let status = Arc::new(FakePrCache::with_lookup(Some(
        host.status.lock().unwrap().clone().unwrap(),
    )));
    let details = Arc::new(Cache::default());
    let error = GitHostError::External("detail unavailable".into());
    details.record("/worktree", Ok(pr()));
    details.record("/worktree", Err(error.clone()));
    let usecase = reader(host.clone(), status, details);
    // When
    let fetched = usecase.read("/worktree").unwrap();
    // Then
    assert_eq!(fetched.value, Some(BranchPullRequest::Present(pr())));
    assert_eq!(fetched.error, Some(WorkFailure::from_error(&error)));
    assert!(host.calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn test_pr更新_一覧の後に内容を取得し内容取得失敗を読み取りへ残す() {
    // Given
    let host = Arc::new(Host::default());
    let usecase = reader(
        host.clone(),
        Arc::new(FakePrCache::default()),
        Arc::new(Cache::default()),
    );
    // When
    usecase.refresh("/worktree").await.unwrap();
    // Then
    assert_eq!(*host.calls.lock().unwrap(), ["list", "detail"]);
    assert_eq!(
        usecase.read("/worktree").unwrap().value,
        Some(BranchPullRequest::Present(pr()))
    );
    // Given / When
    let error = GitHostError::External("detail unavailable".into());
    *host.detail.lock().unwrap() = Err(error.clone());
    usecase.refresh("/worktree").await.unwrap();
    // Then
    let fetched = usecase.read("/worktree").unwrap();
    assert_eq!(fetched.value, Some(BranchPullRequest::Present(pr())));
    assert_eq!(fetched.error, Some(WorkFailure::from_error(&error)));
}
#[tokio::test]
async fn test_pr更新_対象なしと一覧取得失敗では内容を取りに行かない() {
    // Given
    for status in [
        Ok(PrStatus::default()),
        Err(GitHostError::External("list unavailable".into())),
    ] {
        let host = Arc::new(Host::default());
        *host.status.lock().unwrap() = status.clone();
        let usecase = reader(
            host.clone(),
            Arc::new(FakePrCache::default()),
            Arc::new(Cache::default()),
        );
        // When
        usecase.refresh("/worktree").await.unwrap();
        let fetched = usecase.read("/worktree").unwrap();
        // Then
        assert_eq!(*host.calls.lock().unwrap(), ["list"]);
        match status {
            Ok(_) => assert_eq!(fetched.value, Some(BranchPullRequest::Absent)),
            Err(error) => {
                assert_eq!(fetched.value, None);
                assert_eq!(fetched.error, Some(WorkFailure::from_error(&error)));
            }
        }
    }
}
#[test]
fn test_pr作成候補_gitのブランチと既定ブランチを返す() {
    // Given
    let host = Arc::new(Host::default());
    let usecase = reader(
        host.clone(),
        Arc::new(FakePrCache::default()),
        Arc::new(Cache::default()),
    );
    // When
    let bases = usecase.bases("/worktree").unwrap();
    // Then
    assert_eq!(bases.branches, ["main", "topic"]);
    assert_eq!(bases.default_branch.as_deref(), Some("main"));
    assert!(host.calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn test_checkログ_mainのpathとjobを渡して成功と失敗を返す() {
    // Given
    let host = Arc::new(Host::default());
    let usecase = reader(
        host.clone(),
        Arc::new(FakePrCache::default()),
        Arc::new(Cache::default()),
    );
    for result in [
        Ok("job output".into()),
        Err(GitHostError::External("log unavailable".into())),
    ] {
        *host.log.lock().unwrap() = result.clone();
        // When
        let fetched = usecase
            .check_log("/worktree", ActionsJobId(7))
            .await
            .unwrap();
        // Then
        match result {
            Ok(log) => {
                assert_eq!(fetched.value, Some(log));
                assert_eq!(fetched.error, None);
            }
            Err(error) => {
                assert_eq!(fetched.value, None);
                assert_eq!(fetched.error, Some(WorkFailure::from_error(&error)));
            }
        }
    }
    assert_eq!(*host.calls.lock().unwrap(), ["log", "log"]);
}

#[tokio::test]
async fn test_pr操作_成功後のgit再取得失敗でも成功を返して購読へ知らせる() {
    // Given
    for create in [true, false] {
        for fail_main in [true, false] {
            let git = Arc::new(Git::default());
            let error = GitError::External("worktree unavailable".into());
            if fail_main {
                git.main_results
                    .lock()
                    .unwrap()
                    .extend([Ok("/repo".into()), Err(error.clone())]);
            } else {
                if create {
                    git.current_results
                        .lock()
                        .unwrap()
                        .push_back(Ok("topic".into()));
                }
                git.current_results
                    .lock()
                    .unwrap()
                    .push_back(Err(error.clone()));
            }
            let host = Arc::new(Host::default());
            let publisher = crate::test_support::state_subscription::test_subscriptions();
            let mut changes = crate::test_support::state_subscription::changes(&publisher);
            let usecase = PullRequestUsecase::new(
                git.clone(),
                Arc::new(GitHostUsecase::new(
                    host.clone(),
                    Arc::new(FakePrCache::default()),
                    Arc::new(FakeIssueCache::default()),
                )),
                host.clone(),
                Arc::new(Cache::default()),
            )
            .with_state_publisher(publisher);
            // When
            if create {
                usecase
                    .create(
                        "/worktree",
                        "title".into(),
                        "body".into(),
                        "main".into(),
                        true,
                    )
                    .await
                    .unwrap();
            } else {
                usecase
                    .post_comment("/worktree", 42, "comment".into())
                    .await
                    .unwrap();
            }
            // Then
            assert_eq!(
                *host.calls.lock().unwrap(),
                [if create { "create" } else { "comment" }]
            );
            assert_eq!(
                crate::test_support::state_subscription::take_changes(&mut changes),
                vec![StateChangeSource::PullRequest("/worktree".into())]
            );
            assert!(git.main_results.lock().unwrap().is_empty());
            assert!(git.current_results.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn test_checkログ_repository失敗ではproviderを呼ばない() {
    // Given
    let git = Arc::new(Git::default());
    let error = GitError::External("missing repository".into());
    git.main_results
        .lock()
        .unwrap()
        .push_back(Err(error.clone()));
    let host = Arc::new(Host::default());
    let mut usecase = reader(
        host.clone(),
        Arc::new(FakePrCache::default()),
        Arc::new(Cache::default()),
    );
    usecase.git = git;
    // When
    let result = usecase.check_log("/worktree", ActionsJobId(7)).await;
    // Then
    assert_eq!(result.unwrap_err(), error);
    assert!(host.calls.lock().unwrap().is_empty());
}

#[test]
fn test_pr解放_保持していた内容と失敗を捨てる() {
    // Given
    let host = Arc::new(Host::default());
    let status = Arc::new(FakePrCache::with_lookup(Some(
        host.status.lock().unwrap().clone().unwrap(),
    )));
    let cache = Arc::new(Cache::default());
    cache.record("/worktree", Ok(pr()));
    cache.record("/worktree", Err(GitHostError::External("offline".into())));
    let usecase = reader(host.clone(), status, cache);
    assert!(usecase.read("/worktree").unwrap().value.is_some());
    // When
    usecase.release("/worktree");
    // Then
    let value = usecase.read("/worktree").unwrap();
    assert!(value.value.is_none());
    assert!(value.error.is_none());
    assert!(host.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_pr作成_認可拒否では外部操作も購読通知もしない() {
    for github in [false, true] {
        // Given
        let git = Arc::new(Git::default());
        let expected = if github {
            git.current_results
                .lock()
                .unwrap()
                .push_back(Ok("unpushed".into()));
            PullRequestCreationError::HeadNotOnOrigin {
                branch: "unpushed".into(),
            }
        } else {
            *git.origin.lock().unwrap() = Some("https://example.com/a/b".into());
            PullRequestCreationError::NotGitHubRepository
        };
        let host = Arc::new(Host::default());
        let publisher = crate::test_support::state_subscription::test_subscriptions();
        let mut changes = crate::test_support::state_subscription::changes(&publisher);
        let mut usecase = reader(
            host.clone(),
            Arc::new(FakePrCache::default()),
            Arc::new(Cache::default()),
        )
        .with_state_publisher(publisher);
        usecase.git = git;
        // When
        let result = usecase
            .create(
                "/worktree",
                "title".into(),
                "body".into(),
                "main".into(),
                true,
            )
            .await;
        // Then
        assert_eq!(
            result.unwrap_err(),
            WorkFailure {
                kind: Failure::Business(BusinessFailure::Other),
                message: expected.to_string(),
            }
        );
        assert!(host.calls.lock().unwrap().is_empty());
        assert!(crate::test_support::state_subscription::take_changes(&mut changes).is_empty());
    }
}

#[tokio::test]
async fn test_pr操作_provider失敗では再取得も購読通知もしない() {
    for create in [true, false] {
        // Given
        let host = Arc::new(Host::default());
        let error = GitHostError::External("denied".into());
        *host.operation_error.lock().unwrap() = Some(error.clone());
        let publisher = crate::test_support::state_subscription::test_subscriptions();
        let mut changes = crate::test_support::state_subscription::changes(&publisher);
        let usecase = reader(
            host.clone(),
            Arc::new(FakePrCache::default()),
            Arc::new(Cache::default()),
        )
        .with_state_publisher(publisher);
        // When
        let result = if create {
            usecase
                .create(
                    "/worktree",
                    "title".into(),
                    "body".into(),
                    "main".into(),
                    true,
                )
                .await
        } else {
            usecase
                .post_comment("/worktree", 42, "comment".into())
                .await
        };
        // Then
        assert_eq!(
            result.unwrap_err(),
            WorkFailure {
                kind: Failure::Technical(TechnicalFailureNature::Other),
                message: error.to_string(),
            }
        );
        assert_eq!(
            *host.calls.lock().unwrap(),
            [if create { "create" } else { "comment" }]
        );
        assert!(crate::test_support::state_subscription::take_changes(&mut changes).is_empty());
    }
}

#[test]
fn test_pr購読開始_worktreeの内容を保持し始める() {
    // Given
    let cache = Arc::new(Cache::default());
    let usecase = reader(
        Arc::new(Host::default()),
        Arc::new(FakePrCache::default()),
        cache.clone(),
    );
    // When
    usecase.acquire("/worktree");
    // Then
    assert_eq!(*cache.1.lock().unwrap(), ["/worktree"]);
}

#[tokio::test]
async fn test_pr操作_git失敗の区別と理由を返し外部操作しない() {
    use crate::common::operation_context::OperationStopped;
    for create in [true, false] {
        for (error, kind) in [
            (
                GitError::Rule("rule".into()),
                Failure::Business(BusinessFailure::Other),
            ),
            (
                GitError::External("io".into()),
                Failure::Technical(TechnicalFailureNature::Other),
            ),
            (
                GitError::Technical(OperationStopped::Cancelled.into()),
                Failure::Technical(TechnicalFailureNature::Cancelled),
            ),
            (
                GitError::Technical(OperationStopped::Expired.into()),
                Failure::Technical(TechnicalFailureNature::TimedOut),
            ),
        ] {
            // Given
            let git = Arc::new(Git::default());
            git.main_results
                .lock()
                .unwrap()
                .push_back(Err(error.clone()));
            let host = Arc::new(Host::default());
            let mut usecase = reader(
                host.clone(),
                Arc::new(FakePrCache::default()),
                Arc::new(Cache::default()),
            );
            usecase.git = git;
            // When
            let result = if create {
                usecase
                    .create(
                        "/worktree",
                        "title".into(),
                        "body".into(),
                        "main".into(),
                        true,
                    )
                    .await
            } else {
                usecase
                    .post_comment("/worktree", 42, "comment".into())
                    .await
            };
            // Then
            assert_eq!(
                result.unwrap_err(),
                WorkFailure {
                    kind,
                    message: error.to_string()
                }
            );
            assert!(host.calls.lock().unwrap().is_empty());
        }
    }
}
