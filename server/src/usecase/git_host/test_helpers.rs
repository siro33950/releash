use crate::domain::git_host::PrInfo;
use crate::domain::git_host::{GitHostError, IssueCache, IssueInfo, PrStatus, PrStatusCache};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
#[derive(Default)]
pub(crate) struct FakePrCache {
    pub(crate) lookup_value: Mutex<crate::domain::git_host::CachedResult<PrStatus>>,
    pub(crate) stored_values: Mutex<Vec<PrStatus>>,
    pub(crate) records: AtomicUsize,
}

impl FakePrCache {
    pub(crate) fn with_lookup(value: Option<PrStatus>) -> Self {
        Self {
            lookup_value: Mutex::new(crate::domain::git_host::CachedResult { value, error: None }),
            stored_values: Mutex::new(Vec::new()),
            records: AtomicUsize::new(0),
        }
    }

    pub(crate) fn stored_values(&self) -> Vec<PrStatus> {
        self.stored_values.lock().unwrap().clone()
    }
}

impl PrStatusCache for FakePrCache {
    fn result(&self, _: &str) -> crate::domain::git_host::CachedResult<PrStatus> {
        self.lookup_value.lock().unwrap().clone()
    }
    fn record(&self, _: &str, result: Result<PrStatus, GitHostError>) {
        self.records.fetch_add(1, Ordering::SeqCst);
        if let Ok(value) = &result {
            self.stored_values.lock().unwrap().push(value.clone());
        }
        self.lookup_value.lock().unwrap().record(result);
    }
}

#[derive(Default)]
pub(crate) struct FakeIssueCache {
    pub(crate) lookup_value: Mutex<crate::domain::git_host::CachedResult<Vec<IssueInfo>>>,
    pub(crate) stored_values: Mutex<Vec<Vec<IssueInfo>>>,
}

impl FakeIssueCache {
    pub(crate) fn with_lookup(value: Option<Vec<IssueInfo>>) -> Self {
        Self {
            lookup_value: Mutex::new(crate::domain::git_host::CachedResult { value, error: None }),
            stored_values: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn stored_values(&self) -> Vec<Vec<IssueInfo>> {
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

pub(crate) fn sample_pr_status() -> PrStatus {
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

pub(crate) fn sample_issue(number: u64) -> IssueInfo {
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
