use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crate::domain::git_host::{CacheTtl, IssueCache, IssueInfo, PrStatus, PrStatusCache};

struct Entry<T> {
    value: T,
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
    fn lookup_value(&self, repo_path: &str) -> Option<T> {
        let now = Instant::now();
        let map = self.entries.lock().ok()?;
        let entry = map.get(repo_path)?;
        if self.ttl.is_fresh(entry.fetched_at, now) {
            Some(entry.value.clone())
        } else {
            None
        }
    }

    fn store_value(&self, repo_path: &str, value: T) {
        let now = Instant::now();
        if let Ok(mut map) = self.entries.lock() {
            map.retain(|_, entry| self.ttl.is_fresh(entry.fetched_at, now));
            map.insert(
                repo_path.to_string(),
                Entry {
                    value,
                    fetched_at: now,
                },
            );
        }
    }
}

/// Repository ごとに、最後に取れた PR の状態を持つ。期限では捨てない。
#[derive(Default)]
pub(crate) struct LatestPrStatuses {
    entries: Mutex<HashMap<String, PrStatus>>,
}

impl PrStatusCache for LatestPrStatuses {
    fn lookup(&self, repo_path: &str) -> Option<PrStatus> {
        self.entries.lock().ok()?.get(repo_path).cloned()
    }

    fn store(&self, repo_path: &str, value: PrStatus) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(repo_path.to_string(), value);
        }
    }
}

impl IssueCache for InMemoryTtlCache<Vec<IssueInfo>> {
    fn lookup(&self, repo_path: &str) -> Option<Vec<IssueInfo>> {
        self.lookup_value(repo_path)
    }

    fn store(&self, repo_path: &str, value: Vec<IssueInfo>) {
        self.store_value(repo_path, value);
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
        statuses.store("/repo", status.clone());
        // Then
        assert_eq!(statuses.lookup("/repo"), Some(status));
        assert!(statuses.lookup("/other").is_none());
    }

    #[test]
    fn issue_cache_returns_stored_value_for_same_key() {
        let cache = InMemoryTtlCache::<Vec<IssueInfo>>::new(CacheTtl::from_secs(30));

        IssueCache::store(&cache, "/repo", vec![sample_issue(1)]);

        assert_eq!(
            IssueCache::lookup(&cache, "/repo"),
            Some(vec![sample_issue(1)])
        );
        assert!(IssueCache::lookup(&cache, "/other").is_none());
    }

    #[test]
    fn issue_cache_returns_none_for_stale_entry() {
        let cache = InMemoryTtlCache::<Vec<IssueInfo>>::new(CacheTtl::from_secs(0));

        IssueCache::store(&cache, "/repo", vec![sample_issue(1)]);

        assert!(IssueCache::lookup(&cache, "/repo").is_none());
    }

    #[test]
    fn store_evicts_stale_entries_before_inserting_new_value() {
        let cache = InMemoryTtlCache::<Vec<IssueInfo>>::new(CacheTtl::from_secs(0));

        IssueCache::store(&cache, "/old", Vec::new());
        IssueCache::store(&cache, "/new", Vec::new());

        let map = cache.entries.lock().unwrap();
        assert!(!map.contains_key("/old"));
        assert!(map.contains_key("/new"));
    }
}
