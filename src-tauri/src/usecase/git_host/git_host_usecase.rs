use std::sync::Arc;

use crate::domain::git_host::{
    GitHostError, GitHostProvider, IssueCache, IssueInfo, PrStatus, PrStatusCache,
};

#[derive(Clone)]
pub struct GitHostUsecase {
    state_publisher: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
    provider: Arc<dyn GitHostProvider>,
    pr_cache: Arc<dyn PrStatusCache>,
    issue_cache: Arc<dyn IssueCache>,
}

impl GitHostUsecase {
    pub(crate) fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    pub fn new(
        provider: Arc<dyn GitHostProvider>,
        pr_cache: Arc<dyn PrStatusCache>,
        issue_cache: Arc<dyn IssueCache>,
    ) -> Self {
        Self {
            state_publisher: None,
            provider,
            pr_cache,
            issue_cache,
        }
    }

    /// 最後に取れた PR の状態。取りに行かない。
    pub fn known_pr_status(&self, repo_path: &str) -> Option<PrStatus> {
        self.pr_cache.lookup(repo_path)
    }

    /// PR の状態を取りに行って保持する。変わったときは Workspaces の購読へ知らせる。
    pub fn refresh_pr_status(&self, repo_path: &str) -> Result<(), GitHostError> {
        let value = self.provider.fetch_pr_status(repo_path)?;
        if self.pr_cache.lookup(repo_path).as_ref() == Some(&value) {
            return Ok(());
        }
        self.pr_cache.store(repo_path, value);
        if let Some(publisher) = &self.state_publisher {
            publisher.notify(crate::usecase::state_subscription::StateChangeSource::WorkspaceList);
        }
        Ok(())
    }

    pub fn fetch_issues(&self, repo_path: &str) -> Result<Vec<IssueInfo>, GitHostError> {
        let value = self.provider.list_issues(repo_path)?;
        self.issue_cache.store(repo_path, value.clone());
        if let Some(publisher) = &self.state_publisher {
            publisher.notify(
                crate::usecase::state_subscription::StateChangeSource::Issues(repo_path.into()),
            );
        }
        Ok(value)
    }

    pub fn get_cached_issues(&self, repo_path: &str) -> Result<Vec<IssueInfo>, GitHostError> {
        if let Some(issues) = self.issue_cache.lookup(repo_path) {
            return Ok(issues);
        }

        let issues = self.provider.list_issues(repo_path)?;
        self.issue_cache.store(repo_path, issues.clone());
        Ok(issues)
    }
}

#[cfg(test)]
mod tests {
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
        lookup_value: Mutex<Option<PrStatus>>,
        stored_values: Mutex<Vec<PrStatus>>,
    }

    impl FakePrCache {
        fn with_lookup(value: Option<PrStatus>) -> Self {
            Self {
                lookup_value: Mutex::new(value),
                stored_values: Mutex::new(Vec::new()),
            }
        }

        fn stored_values(&self) -> Vec<PrStatus> {
            self.stored_values.lock().unwrap().clone()
        }
    }

    impl PrStatusCache for FakePrCache {
        fn lookup(&self, _repo_path: &str) -> Option<PrStatus> {
            self.lookup_value.lock().unwrap().clone()
        }

        fn store(&self, _repo_path: &str, value: PrStatus) {
            self.stored_values.lock().unwrap().push(value);
        }
    }

    #[derive(Default)]
    struct FakeIssueCache {
        lookup_value: Mutex<Option<Vec<IssueInfo>>>,
        stored_values: Mutex<Vec<Vec<IssueInfo>>>,
    }

    impl FakeIssueCache {
        fn with_lookup(value: Option<Vec<IssueInfo>>) -> Self {
            Self {
                lookup_value: Mutex::new(value),
                stored_values: Mutex::new(Vec::new()),
            }
        }

        fn stored_values(&self) -> Vec<Vec<IssueInfo>> {
            self.stored_values.lock().unwrap().clone()
        }
    }

    impl IssueCache for FakeIssueCache {
        fn lookup(&self, _repo_path: &str) -> Option<Vec<IssueInfo>> {
            self.lookup_value.lock().unwrap().clone()
        }

        fn store(&self, _repo_path: &str, value: Vec<IssueInfo>) {
            self.stored_values.lock().unwrap().push(value);
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
        assert_eq!(
            uc.refresh_pr_status("/repo"),
            Err(GitHostError::External("offline".into()))
        );
        assert!(pr_cache.stored_values().is_empty());
        assert_eq!(uc.known_pr_status("/repo"), Some(previous));
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
        for cached in [Some(known), None] {
            let uc = usecase_with(
                provider.clone(),
                Arc::new(FakePrCache::with_lookup(cached.clone())),
                Arc::new(FakeIssueCache::default()),
            );
            // When / Then
            assert_eq!(uc.known_pr_status("/repo"), cached);
        }
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
}

#[cfg(test)]
#[test]
fn test_github検出_停止時に既定pr状態を保存しない() {
    use crate::common::operation_context::{Deadline, OperationContext, OperationStopped};
    struct NoStore;
    impl PrStatusCache for NoStore {
        fn lookup(&self, _: &str) -> Option<PrStatus> {
            None
        }
        fn store(&self, _: &str, _: PrStatus) {
            panic!("stopped status must not be cached")
        }
    }
    impl IssueCache for NoStore {
        fn lookup(&self, _: &str) -> Option<Vec<IssueInfo>> {
            None
        }
        fn store(&self, _: &str, _: Vec<IssueInfo>) {
            panic!("stopped issues must not be cached")
        }
    }
    let uc = GitHostUsecase::new(
        Arc::new(crate::adaptor::gateway::git_host::github::GitHubGitHostGateway::default()),
        Arc::new(NoStore),
        Arc::new(NoStore),
    );
    for expire in [false, true] {
        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();
        let context = OperationContext::new(
            expire.then(|| Deadline::new(std::time::Instant::now())),
            Arc::new(token),
        );
        crate::common::operation_context::sync_scope(context, || {
            let expected = if expire {
                OperationStopped::Expired
            } else {
                OperationStopped::Cancelled
            };
            assert!(
                matches!(uc.refresh_pr_status("/missing"), Err(GitHostError::Technical(error)) if error == expected.into())
            );
            assert!(
                matches!(uc.fetch_issues("/missing"), Err(GitHostError::Technical(error)) if error == expected.into())
            );
        });
    }
}
