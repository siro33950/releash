use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WatchSubscriptionError {
    NotFound,
    AlreadyExists,
    Limit,
}
impl std::fmt::Display for WatchSubscriptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotFound => "Push subscription ended",
            Self::AlreadyExists => "Push subscription already exists",
            Self::Limit => "Too many subscriptions",
        })
    }
}
impl std::error::Error for WatchSubscriptionError {}

#[derive(Default)]
pub(crate) struct WatchSubscriptions {
    subscriptions: HashMap<String, WatchSubscription>,
    next_generation: u64,
}

#[derive(Default)]
struct WatchSubscription {
    generation: u64,
    pending: usize,
    watchers: HashSet<u64>,
}

pub(crate) struct WatchReservation {
    subscription: String,
    generation: u64,
}
impl WatchSubscriptions {
    pub fn subscribe(&mut self, id: String) -> Result<(), WatchSubscriptionError> {
        if self.subscriptions.contains_key(&id) {
            return Err(WatchSubscriptionError::AlreadyExists);
        }
        if self.subscriptions.len() >= 16 {
            return Err(WatchSubscriptionError::Limit);
        }
        self.next_generation += 1;
        self.subscriptions.insert(
            id,
            WatchSubscription {
                generation: self.next_generation,
                ..Default::default()
            },
        );
        Ok(())
    }
    pub fn ensure_can_watch(&self, id: &str) -> Result<(), WatchSubscriptionError> {
        if !self.subscriptions.contains_key(id) {
            return Err(WatchSubscriptionError::NotFound);
        }
        if self
            .subscriptions
            .values()
            .map(|entry| entry.watchers.len() + entry.pending)
            .sum::<usize>()
            >= 64
        {
            return Err(WatchSubscriptionError::Limit);
        }
        Ok(())
    }
    pub fn reserve(&mut self, id: &str) -> Result<WatchReservation, WatchSubscriptionError> {
        self.ensure_can_watch(id)?;
        let entry = self
            .subscriptions
            .get_mut(id)
            .ok_or(WatchSubscriptionError::NotFound)?;
        entry.pending += 1;
        Ok(WatchReservation {
            subscription: id.into(),
            generation: entry.generation,
        })
    }
    pub fn complete(
        &mut self,
        reservation: WatchReservation,
        watcher: Option<u64>,
    ) -> Result<(), WatchSubscriptionError> {
        let entry = self
            .subscriptions
            .get_mut(&reservation.subscription)
            .filter(|entry| entry.generation == reservation.generation)
            .ok_or(WatchSubscriptionError::NotFound)?;
        entry.pending -= 1;
        if let Some(watcher) = watcher {
            entry.watchers.insert(watcher);
        }
        Ok(())
    }
    pub fn stopped(&mut self, watcher: u64) {
        for watchers in self.subscriptions.values_mut() {
            watchers.watchers.remove(&watcher);
        }
    }
    pub fn unsubscribe(&mut self, id: &str) -> HashSet<u64> {
        self.subscriptions
            .remove(id)
            .map(|entry| entry.watchers)
            .unwrap_or_default()
    }
}

#[cfg(test)]
#[path = "watch_subscriptions_test.rs"]
mod watch_subscriptions_tests;
