use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crate::domain::git_host::{
    CacheTtl, CachedResult, GitHostError, IssueCache, IssueInfo, PrStatus, PrStatusCache,
};

struct Entry<T> {
    value: CachedResult<T>,
    fetched_at: Instant,
}

pub(crate) struct InMemoryTtlCache<T> {
    ttl: CacheTtl,
    entries: Mutex<HashMap<String, Entry<T>>>,
}

impl<T> InMemoryTtlCache<T> {
    pub(crate) fn new(ttl: CacheTtl) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }
}

impl<T> InMemoryTtlCache<T>
where
    T: Clone,
{
    fn lookup_result(&self, repo_path: &str) -> CachedResult<T> {
        let now = Instant::now();
        let map = self.entries.lock().unwrap();
        match map.get(repo_path) {
            Some(entry)
                if self.ttl.is_fresh(entry.fetched_at, now) || entry.value.error.is_some() =>
            {
                entry.value.clone()
            }
            _ => CachedResult::default(),
        }
    }
    fn record_result(&self, repo_path: &str, result: Result<T, GitHostError>) {
        let now = Instant::now();
        let mut map = self.entries.lock().unwrap();
        map.retain(|key, entry| key == repo_path || self.ttl.is_fresh(entry.fetched_at, now));
        let entry = map.entry(repo_path.into()).or_insert_with(|| Entry {
            value: CachedResult::default(),
            fetched_at: now,
        });
        entry.value.record(result);
        entry.fetched_at = now;
    }
}

/// Repository ごとに、最後に取れた PR の状態を持つ。期限では捨てない。
#[derive(Default)]
pub(crate) struct LatestPrStatuses {
    entries: Mutex<HashMap<String, CachedResult<PrStatus>>>,
}

impl PrStatusCache for LatestPrStatuses {
    fn result(&self, repo_path: &str) -> CachedResult<PrStatus> {
        self.entries
            .lock()
            .unwrap()
            .get(repo_path)
            .cloned()
            .unwrap_or_default()
    }
    fn record(&self, repo_path: &str, result: Result<PrStatus, GitHostError>) {
        self.entries
            .lock()
            .unwrap()
            .entry(repo_path.into())
            .or_default()
            .record(result);
    }
}
impl IssueCache for InMemoryTtlCache<Vec<IssueInfo>> {
    fn result(&self, repo_path: &str) -> CachedResult<Vec<IssueInfo>> {
        self.lookup_result(repo_path)
    }
    fn record(&self, repo_path: &str, result: Result<Vec<IssueInfo>, GitHostError>) {
        self.record_result(repo_path, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::git_host::{IssueLabel, Milestone, PrAuthor};

    fn sample_issue(number: u64) -> IssueInfo {
        IssueInfo {
            number,
            title: format!("Issue {number}"),
            state: "OPEN".to_string(),
            url: format!("https://github.com/owner/repo/issues/{number}"),
            author: PrAuthor {
                login: "author".to_string(),
            },
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
            labels: vec![IssueLabel {
                name: "bug".to_string(),
                color: "d73a4a".to_string(),
            }],
            assignees: vec![PrAuthor {
                login: "assignee".to_string(),
            }],
            body: "body".to_string(),
            milestone: Some(Milestone {
                title: "M1".to_string(),
            }),
        }
    }

    #[test]
    fn test_pr状態の保持_最後に取れた値をrepositoryごとに返す() {
        // Given
        let statuses = LatestPrStatuses::default();
        let status = PrStatus::default();
        // When
        statuses.record("/repo", Ok(status.clone()));
        // Then
        assert_eq!(statuses.result("/repo").value, Some(status));
        assert!(statuses.result("/other").value.is_none());
    }

    #[test]
    fn issue_cache_returns_stored_value_for_same_key() {
        // Given
        let cache = InMemoryTtlCache::<Vec<IssueInfo>>::new(CacheTtl::from_secs(30));

        // When
        IssueCache::record(&cache, "/repo", Ok(vec![sample_issue(1)]));
        // Then

        assert_eq!(
            IssueCache::result(&cache, "/repo").value,
            Some(vec![sample_issue(1)])
        );
        assert!(IssueCache::result(&cache, "/other").value.is_none());
    }

    #[test]
    fn issue_cache_returns_none_for_stale_entry() {
        // Given
        let cache = InMemoryTtlCache::<Vec<IssueInfo>>::new(CacheTtl::from_secs(0));

        // When
        IssueCache::record(&cache, "/repo", Ok(vec![sample_issue(1)]));
        // Then

        assert!(IssueCache::result(&cache, "/repo").value.is_none());
    }

    #[test]
    fn store_evicts_stale_entries_before_inserting_new_value() {
        // Given
        let cache = InMemoryTtlCache::<Vec<IssueInfo>>::new(CacheTtl::from_secs(0));

        IssueCache::record(&cache, "/old", Ok(Vec::new()));
        // When
        IssueCache::record(&cache, "/new", Ok(Vec::new()));
        // Then

        let map = cache.entries.lock().unwrap();
        assert!(!map.contains_key("/old"));
        assert!(map.contains_key("/new"));
    }
}
