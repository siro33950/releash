use parking_lot::Mutex;
use std::collections::HashMap;
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
    fn retained(&self, entry: &Entry<T>, now: Instant) -> bool {
        self.ttl.is_fresh(entry.fetched_at, now) || entry.value.error.is_some()
    }

    fn lookup_result(&self, repo_path: &str) -> CachedResult<T> {
        let now = Instant::now();
        let map = self.entries.lock();
        match map.get(repo_path) {
            Some(entry) if self.retained(entry, now) => entry.value.clone(),
            _ => CachedResult::default(),
        }
    }
    fn record_result(&self, repo_path: &str, result: Result<T, GitHostError>) {
        let now = Instant::now();
        let mut map = self.entries.lock();
        map.retain(|key, entry| key == repo_path || self.retained(entry, now));
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
            .get(repo_path)
            .cloned()
            .unwrap_or_default()
    }
    fn record(&self, repo_path: &str, result: Result<PrStatus, GitHostError>) {
        self.entries
            .lock()
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
#[path = "cache_test.rs"]
mod cache_tests;
