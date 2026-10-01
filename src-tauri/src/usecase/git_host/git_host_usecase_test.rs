use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::*;
use crate::domain::git_host::{PrInfo, PrStatus};

struct FakeProvider {
    pr_status: Result<PrStatus, GitHostError>,
    issues: Vec<IssueInfo>,
    pr_fetch_count: AtomicUsize,
    issue_fetch_count: AtomicUsize,
}

impl FakeProvider {
    fn new(pr_status: PrStatus, issues: Vec<IssueInfo>) -> Self {
        Self {
            pr_status: Ok(pr_status),
            issues,
            pr_fetch_count: AtomicUsize::new(0),
            issue_fetch_count: AtomicUsize::new(0),
        }
    }

    fn empty() -> Self {
        Self::new(PrStatus::default(), Vec::new())
    }

    fn pr_fetch_count(&self) -> usize {
        self.pr_fetch_count.load(Ordering::SeqCst)
    }

    fn issue_fetch_count(&self) -> usize {
        self.issue_fetch_count.load(Ordering::SeqCst)
    }
}

impl GitHostProvider for FakeProvider {
    fn fetch_pr_status(&self, _repo_path: &str) -> Result<PrStatus, GitHostError> {
        self.pr_fetch_count.fetch_add(1, Ordering::SeqCst);
        self.pr_status.clone()
    }

    fn list_issues(&self, _repo_path: &str) -> Result<Vec<IssueInfo>, GitHostError> {
        self.issue_fetch_count.fetch_add(1, Ordering::SeqCst);
        Ok(self.issues.clone())
    }
}

#[derive(Default)]
struct FakePrCache {
    lookup_value: Mutex<crate::domain::git_host::CachedResult<PrStatus>>,
    stored_values: Mutex<Vec<PrStatus>>,
}

impl FakePrCache {
    fn with_lookup(value: Option<PrStatus>) -> Self {
        Self {
            lookup_value: Mutex::new(crate::domain::git_host::CachedResult { value, error: None }),
            stored_values: Mutex::new(Vec::new()),
        }
    }

    fn stored_values(&self) -> Vec<PrStatus> {
        self.stored_values.lock().unwrap().clone()
    }
}

impl PrStatusCache for FakePrCache {
    fn result(&self, _: &str) -> crate::domain::git_host::CachedResult<PrStatus> {
        self.lookup_value.lock().unwrap().clone()
    }
    fn record(&self, _: &str, result: Result<PrStatus, GitHostError>) {
        if let Ok(value) = &result {
            self.stored_values.lock().unwrap().push(value.clone());
        }
        self.lookup_value.lock().unwrap().record(result);
    }
}

#[derive(Default)]
struct FakeIssueCache {
    lookup_value: Mutex<crate::domain::git_host::CachedResult<Vec<IssueInfo>>>,
    stored_values: Mutex<Vec<Vec<IssueInfo>>>,
}

impl FakeIssueCache {
    fn with_lookup(value: Option<Vec<IssueInfo>>) -> Self {
        Self {
            lookup_value: Mutex::new(crate::domain::git_host::CachedResult { value, error: None }),
            stored_values: Mutex::new(Vec::new()),
        }
    }

    fn stored_values(&self) -> Vec<Vec<IssueInfo>> {
        self.stored_values.lock().unwrap().clone()
    }
}

impl IssueCache for FakeIssueCache {
    fn result(&self, _: &str) -> crate::domain::git_host::CachedResult<Vec<IssueInfo>> {
        self.lookup_value.lock().unwrap().clone()
    }
    fn record(&self, _: &str, result: Result<Vec<IssueInfo>, GitHostError>) {
        if let Ok(value) = &result {
            self.stored_values.lock().unwrap().push(value.clone());
        }
        self.lookup_value.lock().unwrap().record(result);
    }
}

fn sample_pr_status() -> PrStatus {
    PrStatus {
        open_prs: HashMap::from([(
            "feat/test".to_string(),
            PrInfo {
                number: 42,
                url: "https://github.com/owner/repo/pull/42".to_string(),
            },
        )]),
        merged_branches: vec!["feat/done".to_string()],
    }
}

fn sample_issue(number: u64) -> IssueInfo {
    IssueInfo {
        number,
        title: "Test issue".to_string(),
        state: "OPEN".to_string(),
        url: format!("https://github.com/owner/repo/issues/{number}"),
        author: crate::domain::git_host::value_objects::issue::PrAuthor {
            login: "user".to_string(),
        },
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-02T00:00:00Z".to_string(),
        labels: Vec::new(),
        assignees: Vec::new(),
        body: String::new(),
        milestone: None,
    }
}

fn usecase_with(
    provider: Arc<FakeProvider>,
    pr_cache: Arc<FakePrCache>,
    issue_cache: Arc<FakeIssueCache>,
) -> GitHostUsecase {
    GitHostUsecase::new(provider, pr_cache, issue_cache)
}

#[test]
fn test_pr読取_取得失敗を前の成功と区別し回復時に解除する() {
    // Given
    struct Provider(parking_lot::RwLock<bool>);
    impl GitHostProvider for Provider {
        fn fetch_pr_status(&self, _: &str) -> Result<PrStatus, GitHostError> {
            if *self.0.read() {
                Err(GitHostError::External("denied".into()))
            } else {
                Ok(PrStatus::default())
            }
        }
        fn list_issues(&self, _: &str) -> Result<Vec<IssueInfo>, GitHostError> {
            Ok(vec![])
        }
    }
    let provider = Arc::new(Provider(parking_lot::RwLock::new(false)));
    let usecase = GitHostUsecase::new(
        provider.clone(),
        Arc::new(FakePrCache::default()),
        Arc::new(FakeIssueCache::default()),
    );
    // When
    usecase.refresh_pr_status("/repo").unwrap();
    let initial = usecase.pr_status_result("/repo");
    *provider.0.write() = true;
    let failure = usecase.refresh_pr_status("/repo");
    let failed = usecase.pr_status_result("/repo");
    *provider.0.write() = false;
    usecase.refresh_pr_status("/repo").unwrap();
    let recovered = usecase.pr_status_result("/repo");
    // Then
    assert_eq!(initial.value, Some(PrStatus::default()));
    assert!(initial.error.is_none());
    assert!(failure.is_err());
    assert_eq!(failed.value, initial.value);
    assert_eq!(
        failed.error.unwrap().kind,
        crate::domain::failure::Failure::Technical(
            crate::domain::failure::TechnicalFailureNature::Other
        )
    );
    assert_eq!(recovered.value, initial.value);
    assert!(recovered.error.is_none());
}

#[test]
fn test_issue手動再取得_失敗を保持し成功時に解除する() {
    // Given
    struct Provider(parking_lot::RwLock<bool>);
    impl GitHostProvider for Provider {
        fn fetch_pr_status(&self, _: &str) -> Result<PrStatus, GitHostError> {
            Ok(PrStatus::default())
        }
        fn list_issues(&self, _: &str) -> Result<Vec<IssueInfo>, GitHostError> {
            if *self.0.read() {
                Err(GitHostError::External("offline".into()))
            } else {
                Ok(vec![])
            }
        }
    }
    let provider = Arc::new(Provider(parking_lot::RwLock::new(false)));
    let usecase = GitHostUsecase::new(
        provider.clone(),
        Arc::new(FakePrCache::default()),
        Arc::new(FakeIssueCache::default()),
    );
    // When
    usecase.fetch_issues("/repo").unwrap();
    let initial = usecase.issue_cache.result("/repo");
    *provider.0.write() = true;
    let failure = usecase.fetch_issues("/repo");
    let failed = usecase.issue_cache.result("/repo");
    let read_failure = usecase.get_cached_issues("/repo");
    *provider.0.write() = false;
    usecase.fetch_issues("/repo").unwrap();
    let recovered = usecase.issue_cache.result("/repo");
    // Then
    assert_eq!(initial.value, Some(vec![]));
    assert!(initial.error.is_none());
    assert!(failure.is_err());
    assert_eq!(failed.value, initial.value);
    assert_eq!(failed.error, Some(GitHostError::External("offline".into())));
    assert!(matches!(read_failure, Err(GitHostError::External(message)) if message == "offline"));
    assert_eq!(recovered.value, initial.value);
    assert!(recovered.error.is_none());
}

#[test]
fn forced_refresh_updates_caches_even_when_previous_values_are_fresh() {
    let fetched_pr = sample_pr_status();
    let fetched_issues = vec![sample_issue(2)];
    let provider = Arc::new(FakeProvider::new(
        fetched_pr.clone(),
        fetched_issues.clone(),
    ));
    let pr_cache = Arc::new(FakePrCache::with_lookup(Some(PrStatus::default())));
    let issue_cache = Arc::new(FakeIssueCache::with_lookup(Some(vec![sample_issue(1)])));
    let uc = usecase_with(provider.clone(), pr_cache.clone(), issue_cache.clone());
    assert_eq!(uc.refresh_pr_status("/repo"), Ok(()));
    assert_eq!(uc.fetch_issues("/repo").unwrap(), fetched_issues);
    assert_eq!(pr_cache.stored_values(), vec![fetched_pr]);
    assert_eq!(issue_cache.stored_values(), vec![fetched_issues]);
    assert_eq!(provider.pr_fetch_count(), 1);
    assert_eq!(provider.issue_fetch_count(), 1);
}

#[test]
fn forced_pr_refresh_failure_keeps_the_previous_cache() {
    // Given
    let previous = sample_pr_status();
    let provider = Arc::new(FakeProvider {
        pr_status: Err(GitHostError::External("offline".into())),
        ..FakeProvider::empty()
    });
    let pr_cache = Arc::new(FakePrCache::with_lookup(Some(previous.clone())));
    let uc = usecase_with(
        provider,
        pr_cache.clone(),
        Arc::new(FakeIssueCache::default()),
    );
    // When
    let result = uc.refresh_pr_status("/repo");
    // Then
    assert_eq!(result, Err(GitHostError::External("offline".into())));
    assert!(pr_cache.stored_values().is_empty());
    assert_eq!(uc.pr_status_result("/repo").value, Some(previous));
}

#[test]
fn provider_absent_fetches_empty_values() {
    let provider = Arc::new(FakeProvider::empty());
    let pr_cache = Arc::new(FakePrCache::default());
    let uc = usecase_with(
        provider,
        pr_cache.clone(),
        Arc::new(FakeIssueCache::default()),
    );

    assert_eq!(uc.refresh_pr_status("/repo"), Ok(()));
    assert_eq!(pr_cache.stored_values(), vec![PrStatus::default()]);
    assert!(uc.fetch_issues("/repo").unwrap().is_empty());
}

#[test]
fn test_pr状態の読み取り_最後に取れた値を返し取りに行かない() {
    // Given
    let known = sample_pr_status();
    let provider = Arc::new(FakeProvider::new(PrStatus::default(), Vec::new()));
    let uc = usecase_with(
        provider.clone(),
        Arc::new(FakePrCache::with_lookup(Some(known.clone()))),
        Arc::new(FakeIssueCache::default()),
    );
    // When
    let result = uc.pr_status_result("/repo");
    // Then
    assert_eq!(result.value, Some(known));
    assert_eq!(provider.pr_fetch_count(), 0);
}
#[test]
fn test_pr状態の読み取り_未取得なら取りに行かず未設定を返す() {
    // Given
    let provider = Arc::new(FakeProvider::empty());
    let uc = usecase_with(
        provider.clone(),
        Arc::new(FakePrCache::default()),
        Arc::new(FakeIssueCache::default()),
    );
    // When
    let result = uc.pr_status_result("/repo");
    // Then
    assert_eq!(result.value, None);
    assert_eq!(provider.pr_fetch_count(), 0);
}

#[test]
fn test_pr状態の取り直し_変わったときだけ保持してworkspacesの購読へ知らせる() {
    use crate::usecase::state_subscription::StateChangeSource;
    // Given
    let fetched = sample_pr_status();
    let provider = Arc::new(FakeProvider::new(fetched.clone(), Vec::new()));
    for (known, changed) in [(None, true), (Some(fetched.clone()), false)] {
        let pr_cache = Arc::new(FakePrCache::with_lookup(known));
        let publisher = crate::test_support::state_subscription::test_subscriptions();
        let mut changes = crate::test_support::state_subscription::changes(&publisher);
        let uc = usecase_with(
            provider.clone(),
            pr_cache.clone(),
            Arc::new(FakeIssueCache::default()),
        )
        .with_state_publisher(publisher);
        // When
        uc.refresh_pr_status("/repo").unwrap();
        // Then
        assert_eq!(pr_cache.stored_values().len(), usize::from(changed));
        assert_eq!(
            crate::test_support::state_subscription::take_changes(&mut changes),
            if changed {
                vec![StateChangeSource::WorkspaceList]
            } else {
                Vec::new()
            }
        );
    }
}

#[test]
fn cached_issues_hit_does_not_fetch_provider() {
    let cached = vec![sample_issue(1)];
    let provider = Arc::new(FakeProvider::new(PrStatus::default(), Vec::new()));
    let uc = usecase_with(
        provider.clone(),
        Arc::new(FakePrCache::default()),
        Arc::new(FakeIssueCache::with_lookup(Some(cached.clone()))),
    );

    assert_eq!(uc.get_cached_issues("/repo").unwrap(), cached);
    assert_eq!(provider.issue_fetch_count(), 0);
}

#[test]
fn cached_issues_miss_fetches_and_stores() {
    let fetched = vec![sample_issue(2)];
    let provider = Arc::new(FakeProvider::new(PrStatus::default(), fetched.clone()));
    let issue_cache = Arc::new(FakeIssueCache::with_lookup(None));
    let uc = usecase_with(
        provider.clone(),
        Arc::new(FakePrCache::default()),
        issue_cache.clone(),
    );

    assert_eq!(uc.get_cached_issues("/repo").unwrap(), fetched);
    assert_eq!(provider.issue_fetch_count(), 1);
    assert_eq!(issue_cache.stored_values(), vec![fetched]);
}

#[test]
fn test_github検出_停止時に既定pr状態を保存しない() {
    // Given
    use crate::common::operation_context::{OperationContext, OperationStopped};
    let pr_cache = Arc::new(FakePrCache::default());
    let issue_cache = Arc::new(FakeIssueCache::default());
    let uc = GitHostUsecase::new(
        Arc::new(crate::adaptor::gateway::git_host::github::GitHubGitHostGateway::default()),
        pr_cache.clone(),
        issue_cache.clone(),
    );
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    let context = OperationContext::new(None, Arc::new(token));
    // When
    let (pr, issues) = crate::common::operation_context::sync_scope(context, || {
        (
            uc.refresh_pr_status("/missing"),
            uc.fetch_issues("/missing"),
        )
    });
    // Then
    assert!(
        matches!(pr, Err(GitHostError::Technical(error)) if error == OperationStopped::Cancelled.into())
    );
    assert!(
        matches!(issues, Err(GitHostError::Technical(error)) if error == OperationStopped::Cancelled.into())
    );
    assert!(pr_cache.stored_values().is_empty());
    assert!(issue_cache.stored_values().is_empty());
}
#[test]
fn test_github検出_期限切れ時に既定pr状態を保存しない() {
    // Given
    use crate::common::operation_context::{Deadline, OperationContext, OperationStopped};
    let pr_cache = Arc::new(FakePrCache::default());
    let issue_cache = Arc::new(FakeIssueCache::default());
    let uc = GitHostUsecase::new(
        Arc::new(crate::adaptor::gateway::git_host::github::GitHubGitHostGateway::default()),
        pr_cache.clone(),
        issue_cache.clone(),
    );
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    let context = OperationContext::new(
        Some(Deadline::new(std::time::Instant::now())),
        Arc::new(token),
    );
    // When
    let (pr, issues) = crate::common::operation_context::sync_scope(context, || {
        (
            uc.refresh_pr_status("/missing"),
            uc.fetch_issues("/missing"),
        )
    });
    // Then
    assert!(
        matches!(pr, Err(GitHostError::Technical(error)) if error == OperationStopped::Expired.into())
    );
    assert!(
        matches!(issues, Err(GitHostError::Technical(error)) if error == OperationStopped::Expired.into())
    );
    assert!(pr_cache.stored_values().is_empty());
    assert!(issue_cache.stored_values().is_empty());
}
