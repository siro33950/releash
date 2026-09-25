use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::time::Duration;

#[derive(Default)]
struct Item {
    processing: bool,
    dirty: bool,
    due: Duration,
    failures: u64,
}

pub struct WorkQueue<K> {
    items: HashMap<K, Item>,
    order: VecDeque<K>,
    stopped: HashSet<K>,
}

impl<K: Eq + Hash + Clone> Default for WorkQueue<K> {
    fn default() -> Self {
        Self {
            items: HashMap::new(),
            order: VecDeque::new(),
            stopped: HashSet::new(),
        }
    }
}

impl<K: Eq + Hash + Clone> WorkQueue<K> {
    pub fn add(&mut self, key: K, due: Duration) -> bool {
        if self.stopped.contains(&key) {
            return false;
        }
        let item = self.items.entry(key.clone()).or_default();
        if !item.dirty {
            item.dirty = true;
            item.due = due;
            if !item.processing {
                self.order.push_back(key);
            }
        } else if item.failures == 0 {
            item.due = item.due.min(due);
        }
        true
    }

    pub fn stop(&mut self, key: &K) {
        self.items.remove(key);
        self.order.retain(|pending| pending != key);
        self.stopped.insert(key.clone());
    }

    pub fn reset(&mut self, key: &K) {
        self.stopped.remove(key);
    }

    #[cfg(test)]
    pub fn get(&mut self, now: Duration) -> Option<K> {
        self.get_matching(now, |_| true)
    }

    pub fn get_matching(&mut self, now: Duration, matches: impl Fn(&K) -> bool) -> Option<K> {
        let index = self
            .order
            .iter()
            .position(|key| matches(key) && self.items[key].due <= now)?;
        let key = self.order.remove(index)?;
        let item = self.items.get_mut(&key)?;
        item.processing = true;
        item.dirty = false;
        Some(key)
    }

    pub fn failed(&mut self, key: &K) -> u64 {
        let item = self.items.get_mut(key).expect("processing item");
        item.failures = item.failures.saturating_add(1);
        item.failures
    }

    pub fn done(&mut self, key: &K, next: Option<Duration>, forget: bool) -> bool {
        let item = self.items.get_mut(key).expect("processing item");
        item.processing = false;
        if forget {
            item.failures = 0;
        }
        if let Some(due) = next {
            // Retry delay also applies when the item was added during its attempt.
            item.due = due;
            item.dirty = true;
        }
        if item.dirty {
            self.order.push_back(key.clone());
        } else {
            self.items.remove(key);
        }
        self.items.contains_key(key)
    }

    pub fn retry(
        &mut self,
        key: &K,
        kind: super::failure::FailureKind,
        policy: super::retry::RetryBackoff,
        now: Duration,
        jitter: f64,
    ) -> Duration {
        let count = self.failed(key);
        let policy = if kind.retry_action() == super::failure::RetryAction::Restart {
            super::retry::RetryBackoff::CONFLICT
        } else {
            policy
        };
        now + policy.delay(count, jitter)
    }

    #[cfg(test)]
    pub fn failure_count(&self, key: &K) -> u64 {
        self.items.get(key).map_or(0, |item| item.failures)
    }

    pub fn remove(&mut self, key: &K) {
        self.items.remove(key);
        self.order.retain(|pending| pending != key);
    }

    #[cfg(test)]
    pub fn next_due(&self) -> Option<Duration> {
        self.next_due_matching(|_| true)
    }
    pub fn next_due_matching(&self, matches: impl Fn(&K) -> bool) -> Option<Duration> {
        self.order
            .iter()
            .filter(|key| matches(key))
            .map(|key| self.items[key].due)
            .min()
    }
}

#[cfg(test)]
#[path = "work_queue_test.rs"]
mod work_queue_tests;
