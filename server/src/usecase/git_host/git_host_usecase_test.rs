use crate::usecase::git_host::test_helpers::{
    sample_issue, sample_pr_status, FakeIssueCache, FakePrCache,
};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::domain::git_host::PrStatus;

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

#[async_trait::async_trait]

impl GitHostProvider for FakeProvider {
    async fn fetch_pr_status(&self, _repo_path: &str) -> Result<PrStatus, GitHostError> {
        self.pr_fetch_count.fetch_add(1, Ordering::SeqCst);
        self.pr_status.clone()
    }

    async fn list_issues(&self, _repo_path: &str) -> Result<Vec<IssueInfo>, GitHostError> {
        self.issue_fetch_count.fetch_add(1, Ordering::SeqCst);
        Ok(self.issues.clone())
    }
}

fn usecase_with(
    provider: Arc<FakeProvider>,
    pr_cache: Arc<FakePrCache>,
    issue_cache: Arc<FakeIssueCache>,
) -> GitHostUsecase {
    GitHostUsecase::new(provider, pr_cache, issue_cache)
}

#[tokio::test]
async fn test_pr読取_取得失敗を前の成功と区別し回復時に解除する() {
    // Given
    struct Provider(parking_lot::RwLock<bool>);
    #[async_trait::async_trait]
    impl GitHostProvider for Provider {
        async fn fetch_pr_status(&self, _: &str) -> Result<PrStatus, GitHostError> {
            if *self.0.read() {
                Err(GitHostError::External("denied".into()))
            } else {
                Ok(PrStatus::default())
            }
        }
        async fn list_issues(&self, _: &str) -> Result<Vec<IssueInfo>, GitHostError> {
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
    usecase.refresh_pr_status("/repo").await.unwrap();
    let initial = usecase.pr_status_result("/repo");
    *provider.0.write() = true;
    let failure = usecase.refresh_pr_status("/repo").await;
    let failed = usecase.pr_status_result("/repo");
    *provider.0.write() = false;
    usecase.refresh_pr_status("/repo").await.unwrap();
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

#[tokio::test]
async fn test_issue手動再取得_失敗を保持し成功時に解除する() {
    // Given
    struct Provider(parking_lot::RwLock<bool>);
    #[async_trait::async_trait]
    impl GitHostProvider for Provider {
        async fn fetch_pr_status(&self, _: &str) -> Result<PrStatus, GitHostError> {
            Ok(PrStatus::default())
        }
        async fn list_issues(&self, _: &str) -> Result<Vec<IssueInfo>, GitHostError> {
            if *self.0.read() {
                Err(GitHostError::External("offline".into()))
            } else {
                Ok(vec![sample_issue(1)])
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
    usecase.fetch_issues("/repo").await.unwrap();
    let initial = usecase.get_cached_issues("/repo").await;
    *provider.0.write() = true;
    let failure = usecase.fetch_issues("/repo").await;
    let failed = usecase.get_cached_issues("/repo").await;
    let read_failure = usecase.get_cached_issues("/repo").await;
    *provider.0.write() = false;
    usecase.fetch_issues("/repo").await.unwrap();
    let recovered = usecase.get_cached_issues("/repo").await;
    // Then
    assert_eq!(initial.value, Some(vec![sample_issue(1)]));
    assert!(initial.error.is_none());
    assert!(failure.is_err());
    assert_eq!(failed.value, initial.value);
    assert_eq!(
        failed.error,
        Some(crate::domain::failure::WorkFailure::from_error(
            &GitHostError::External("offline".into())
        ))
    );
    assert_eq!(read_failure.value, initial.value);
    assert_eq!(read_failure.error, failed.error);
    assert_eq!(recovered.value, initial.value);
    assert!(recovered.error.is_none());
}

#[tokio::test]
async fn forced_refresh_updates_caches_even_when_previous_values_are_fresh() {
    let fetched_pr = sample_pr_status();
    let fetched_issues = vec![sample_issue(2)];
    let provider = Arc::new(FakeProvider::new(
        fetched_pr.clone(),
        fetched_issues.clone(),
    ));
    let pr_cache = Arc::new(FakePrCache::with_lookup(Some(PrStatus::default())));
    let issue_cache = Arc::new(FakeIssueCache::with_lookup(Some(vec![sample_issue(1)])));
    let uc = usecase_with(provider.clone(), pr_cache.clone(), issue_cache.clone());
    assert_eq!(uc.refresh_pr_status("/repo").await, Ok(()));
    assert_eq!(uc.fetch_issues("/repo").await.unwrap(), fetched_issues);
    assert_eq!(pr_cache.stored_values(), vec![fetched_pr]);
    assert_eq!(issue_cache.stored_values(), vec![fetched_issues]);
    assert_eq!(provider.pr_fetch_count(), 1);
    assert_eq!(provider.issue_fetch_count(), 1);
}

#[tokio::test]
async fn forced_pr_refresh_failure_keeps_the_previous_cache() {
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
    let result = uc.refresh_pr_status("/repo").await;
    // Then
    assert_eq!(result, Err(GitHostError::External("offline".into())));
    assert!(pr_cache.stored_values().is_empty());
    assert_eq!(uc.pr_status_result("/repo").value, Some(previous));
}

#[tokio::test]
async fn provider_absent_fetches_empty_values() {
    let provider = Arc::new(FakeProvider::empty());
    let pr_cache = Arc::new(FakePrCache::default());
    let uc = usecase_with(
        provider,
        pr_cache.clone(),
        Arc::new(FakeIssueCache::default()),
    );

    assert_eq!(uc.refresh_pr_status("/repo").await, Ok(()));
    assert_eq!(pr_cache.stored_values(), vec![PrStatus::default()]);
    assert!(uc.fetch_issues("/repo").await.unwrap().is_empty());
}

#[tokio::test]
async fn test_pr状態の読み取り_最後に取れた値を返し取りに行かない() {
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
#[tokio::test]
async fn test_pr状態の読み取り_未取得なら取りに行かず未設定を返す() {
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

#[tokio::test]
async fn test_pr状態の取り直し_変わったときだけ保持してworkspacesの購読へ知らせる() {
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
        uc.refresh_pr_status("/repo").await.unwrap();
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

#[tokio::test]
async fn cached_issues_hit_does_not_fetch_provider() {
    let cached = vec![sample_issue(1)];
    let provider = Arc::new(FakeProvider::new(PrStatus::default(), Vec::new()));
    let uc = usecase_with(
        provider.clone(),
        Arc::new(FakePrCache::default()),
        Arc::new(FakeIssueCache::with_lookup(Some(cached.clone()))),
    );

    assert_eq!(uc.get_cached_issues("/repo").await.value, Some(cached));
    assert_eq!(provider.issue_fetch_count(), 0);
}

#[tokio::test]
async fn cached_issues_miss_fetches_and_stores() {
    let fetched = vec![sample_issue(2)];
    let provider = Arc::new(FakeProvider::new(PrStatus::default(), fetched.clone()));
    let issue_cache = Arc::new(FakeIssueCache::with_lookup(None));
    let uc = usecase_with(
        provider.clone(),
        Arc::new(FakePrCache::default()),
        issue_cache.clone(),
    );

    assert_eq!(
        uc.get_cached_issues("/repo").await.value,
        Some(fetched.clone())
    );
    assert_eq!(provider.issue_fetch_count(), 1);
    assert_eq!(issue_cache.stored_values(), vec![fetched]);
}

#[tokio::test]
async fn test_pr状態の取り直し_同じ失敗なら記録も通知もしない() {
    // Given
    let error = GitHostError::External("offline".into());
    let provider = Arc::new(FakeProvider {
        pr_status: Err(error.clone()),
        ..FakeProvider::empty()
    });
    let cache = Arc::new(FakePrCache::default());
    let publisher = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    let uc = usecase_with(provider, cache.clone(), Arc::new(FakeIssueCache::default()))
        .with_state_publisher(publisher);
    uc.refresh_pr_status("/repo").await.unwrap_err();
    crate::test_support::state_subscription::take_changes(&mut changes);
    // When
    let result = uc.refresh_pr_status("/repo").await;
    // Then
    assert_eq!(result, Err(error));
    assert_eq!(cache.records.load(Ordering::SeqCst), 1);
    assert!(crate::test_support::state_subscription::take_changes(&mut changes).is_empty());
}

#[tokio::test]
async fn test_pr状態の取り直し_違う失敗なら記録して通知する() {
    // Given
    let error = GitHostError::External("offline".into());
    let provider = Arc::new(FakeProvider {
        pr_status: Err(error.clone()),
        ..FakeProvider::empty()
    });
    let cache = Arc::new(FakePrCache::default());
    cache.record("/repo", Err(GitHostError::External("denied".into())));
    let publisher = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    let uc = usecase_with(provider, cache.clone(), Arc::new(FakeIssueCache::default()))
        .with_state_publisher(publisher);
    // When
    let result = uc.refresh_pr_status("/repo").await;
    // Then
    assert_eq!(result, Err(error));
    assert_eq!(cache.records.load(Ordering::SeqCst), 2);
    assert_eq!(
        crate::test_support::state_subscription::take_changes(&mut changes),
        vec![crate::usecase::state_subscription::StateChangeSource::WorkspaceList]
    );
}

#[tokio::test]
async fn test_issue絞り込み_キャッシュを変えず条件に一致するissueと読取失敗を返す() {
    // Given
    let mut issue = sample_issue(1);
    issue
        .labels
        .push(crate::domain::git_host::value_objects::issue::IssueLabel {
            name: "bug".into(),
            color: "fff".into(),
        });
    issue.milestone = Some(crate::domain::git_host::value_objects::issue::Milestone {
        title: "release".into(),
    });
    let cache = Arc::new(FakeIssueCache::with_lookup(Some(vec![
        issue,
        sample_issue(2),
    ])));
    cache.record("/repo", Err(GitHostError::External("offline".into())));
    let provider = Arc::new(FakeProvider::empty());
    let uc = usecase_with(
        provider.clone(),
        Arc::new(FakePrCache::default()),
        cache.clone(),
    );
    let filter = crate::domain::git_host::IssueFilter {
        labels: vec!["bug".into()],
        milestone: Some("release".into()),
    };
    // When
    let result = uc.get_filtered_issues("/repo", &filter).await;
    // Then
    assert_eq!(
        result
            .value
            .unwrap()
            .issues
            .iter()
            .map(|issue| issue.number)
            .collect::<Vec<_>>(),
        [1]
    );
    assert!(result.error.is_some());
    assert_eq!(cache.result("/repo").value.unwrap().len(), 2);
    assert_eq!(provider.issue_fetch_count(), 0);
}
