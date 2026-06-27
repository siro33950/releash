use std::sync::Arc;

use crate::domain::git_host::{
    GitHostProvider, IssueCache, IssueInfo, PrStatus, PrStatusCache, ProviderStatus,
};

#[derive(Clone)]
pub struct GitHostUsecase {
    provider: Arc<dyn GitHostProvider>,
    pr_cache: Arc<dyn PrStatusCache>,
    issue_cache: Arc<dyn IssueCache>,
}

impl GitHostUsecase {
    pub fn new(
        provider: Arc<dyn GitHostProvider>,
        pr_cache: Arc<dyn PrStatusCache>,
        issue_cache: Arc<dyn IssueCache>,
    ) -> Self {
        Self {
            provider,
            pr_cache,
            issue_cache,
        }
    }

    pub fn check_provider_status(&self, repo_path: &str) -> ProviderStatus {
        self.provider.provider_status(repo_path)
    }

    pub fn fetch_pr_status(&self, repo_path: &str) -> PrStatus {
        self.provider.fetch_pr_status(repo_path)
    }

    pub fn get_cached_pr_status(&self, repo_path: &str) -> PrStatus {
        if let Some(status) = self.pr_cache.lookup(repo_path) {
            return status;
        }

        let status = self.provider.fetch_pr_status(repo_path);
        self.pr_cache.store(repo_path, status.clone());
        status
    }

    pub fn fetch_issues(&self, repo_path: &str) -> Vec<IssueInfo> {
        self.provider.list_issues(repo_path)
    }

    pub fn get_cached_issues(&self, repo_path: &str) -> Vec<IssueInfo> {
        if let Some(issues) = self.issue_cache.lookup(repo_path) {
            return issues;
        }

        let issues = self.provider.list_issues(repo_path);
        self.issue_cache.store(repo_path, issues.clone());
        issues
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
        status: ProviderStatus,
        pr_status: PrStatus,
        issues: Vec<IssueInfo>,
        pr_fetch_count: AtomicUsize,
        issue_fetch_count: AtomicUsize,
    }

    impl FakeProvider {
        fn new(status: ProviderStatus, pr_status: PrStatus, issues: Vec<IssueInfo>) -> Self {
            Self {
                status,
                pr_status,
                issues,
                pr_fetch_count: AtomicUsize::new(0),
                issue_fetch_count: AtomicUsize::new(0),
            }
        }

        fn empty(status: ProviderStatus) -> Self {
            Self::new(status, PrStatus::default(), Vec::new())
        }

        fn pr_fetch_count(&self) -> usize {
            self.pr_fetch_count.load(Ordering::SeqCst)
        }

        fn issue_fetch_count(&self) -> usize {
            self.issue_fetch_count.load(Ordering::SeqCst)
        }
    }

    impl GitHostProvider for FakeProvider {
        fn provider_status(&self, _repo_path: &str) -> ProviderStatus {
            self.status.clone()
        }

        fn fetch_pr_status(&self, _repo_path: &str) -> PrStatus {
            self.pr_fetch_count.fetch_add(1, Ordering::SeqCst);
            self.pr_status.clone()
        }

        fn list_issues(&self, _repo_path: &str) -> Vec<IssueInfo> {
            self.issue_fetch_count.fetch_add(1, Ordering::SeqCst);
            self.issues.clone()
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
    fn provider_absent_fetches_empty_values() {
        let provider = Arc::new(FakeProvider::empty(ProviderStatus::NoRemote));
        let uc = usecase_with(
            provider,
            Arc::new(FakePrCache::default()),
            Arc::new(FakeIssueCache::default()),
        );

        assert_eq!(uc.fetch_pr_status("/repo"), PrStatus::default());
        assert!(uc.fetch_issues("/repo").is_empty());
    }

    #[test]
    fn provider_available_status_is_returned() {
        let provider = Arc::new(FakeProvider::empty(ProviderStatus::Available));
        let uc = usecase_with(
            provider,
            Arc::new(FakePrCache::default()),
            Arc::new(FakeIssueCache::default()),
        );

        assert_eq!(uc.check_provider_status("/repo"), ProviderStatus::Available);
    }

    #[test]
    fn provider_unavailable_statuses_are_returned() {
        for status in [
            ProviderStatus::NoRemote,
            ProviderStatus::UnsupportedPlatform,
            ProviderStatus::CliNotFound {
                cli: "gh".to_string(),
            },
            ProviderStatus::NotAuthenticated,
        ] {
            let provider = Arc::new(FakeProvider::empty(status.clone()));
            let uc = usecase_with(
                provider,
                Arc::new(FakePrCache::default()),
                Arc::new(FakeIssueCache::default()),
            );

            assert_eq!(uc.check_provider_status("/repo"), status);
        }
    }

    #[test]
    fn cached_pr_status_hit_does_not_fetch_provider() {
        let cached = sample_pr_status();
        let provider = Arc::new(FakeProvider::new(
            ProviderStatus::Available,
            PrStatus::default(),
            Vec::new(),
        ));
        let uc = usecase_with(
            provider.clone(),
            Arc::new(FakePrCache::with_lookup(Some(cached.clone()))),
            Arc::new(FakeIssueCache::default()),
        );

        assert_eq!(uc.get_cached_pr_status("/repo"), cached);
        assert_eq!(provider.pr_fetch_count(), 0);
    }

    #[test]
    fn cached_pr_status_miss_fetches_and_stores() {
        let fetched = sample_pr_status();
        let provider = Arc::new(FakeProvider::new(
            ProviderStatus::Available,
            fetched.clone(),
            Vec::new(),
        ));
        let pr_cache = Arc::new(FakePrCache::with_lookup(None));
        let uc = usecase_with(
            provider.clone(),
            pr_cache.clone(),
            Arc::new(FakeIssueCache::default()),
        );

        assert_eq!(uc.get_cached_pr_status("/repo"), fetched);
        assert_eq!(provider.pr_fetch_count(), 1);
        assert_eq!(pr_cache.stored_values(), vec![fetched]);
    }

    #[test]
    fn cached_pr_status_lookup_none_refetches_and_updates_cache() {
        let fetched = sample_pr_status();
        let provider = Arc::new(FakeProvider::new(
            ProviderStatus::Available,
            fetched.clone(),
            Vec::new(),
        ));
        let pr_cache = Arc::new(FakePrCache::with_lookup(None));
        let uc = usecase_with(
            provider.clone(),
            pr_cache.clone(),
            Arc::new(FakeIssueCache::default()),
        );

        assert_eq!(uc.get_cached_pr_status("/repo"), fetched);
        assert_eq!(provider.pr_fetch_count(), 1);
        assert_eq!(pr_cache.stored_values(), vec![fetched]);
    }

    #[test]
    fn cached_issues_hit_does_not_fetch_provider() {
        let cached = vec![sample_issue(1)];
        let provider = Arc::new(FakeProvider::new(
            ProviderStatus::Available,
            PrStatus::default(),
            Vec::new(),
        ));
        let uc = usecase_with(
            provider.clone(),
            Arc::new(FakePrCache::default()),
            Arc::new(FakeIssueCache::with_lookup(Some(cached.clone()))),
        );

        assert_eq!(uc.get_cached_issues("/repo"), cached);
        assert_eq!(provider.issue_fetch_count(), 0);
    }

    #[test]
    fn cached_issues_miss_fetches_and_stores() {
        let fetched = vec![sample_issue(2)];
        let provider = Arc::new(FakeProvider::new(
            ProviderStatus::Available,
            PrStatus::default(),
            fetched.clone(),
        ));
        let issue_cache = Arc::new(FakeIssueCache::with_lookup(None));
        let uc = usecase_with(
            provider.clone(),
            Arc::new(FakePrCache::default()),
            issue_cache.clone(),
        );

        assert_eq!(uc.get_cached_issues("/repo"), fetched);
        assert_eq!(provider.issue_fetch_count(), 1);
        assert_eq!(issue_cache.stored_values(), vec![fetched]);
    }
}
