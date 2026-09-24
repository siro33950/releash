#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StateValue {
    RepositoryPaths(Vec<String>),
}

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

const RETAINED_CHANGES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Version {
    pub epoch: String,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delivery {
    Full,
    Delta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event<T> {
    Snapshot(Version, Arc<T>),
    Change(Version, Delivery, Arc<T>),
    Bookmark(Version),
}

impl<T> Event<T> {
    pub fn version(&self) -> &Version {
        match self {
            Self::Snapshot(version, _) | Self::Change(version, _, _) | Self::Bookmark(version) => {
                version
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubscriptionError {
    InvalidId,
    AlreadyExists,
    StreamEnded,
    UnknownTarget,
    VersionExhausted,
}

impl std::fmt::Display for SubscriptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SubscriptionError {}

struct Target<T> {
    version: Version,
    snapshot: Arc<T>,
    delivery: Delivery,
    history: VecDeque<Event<T>>,
}

impl<T: Clone> Target<T> {
    fn resume(&self, version: Option<&Version>) -> VecDeque<Event<T>> {
        let mut pending = VecDeque::new();
        let resumable = version.is_some_and(|version| {
            version.epoch == self.version.epoch
                && version.sequence <= self.version.sequence
                && self.version.sequence - version.sequence <= self.history.len() as u64
        });
        if resumable {
            let sequence = version.unwrap().sequence;
            pending.extend(
                self.history
                    .iter()
                    .filter(|event| event.version().sequence > sequence)
                    .cloned(),
            );
        } else {
            pending.push_back(Event::Snapshot(self.version.clone(), self.snapshot.clone()));
        }
        pending.push_back(Event::Bookmark(self.version.clone()));
        pending
    }
}

struct Subscription<T> {
    pending: VecDeque<Event<T>>,
    sent: Option<Version>,
    overflowed: bool,
}

struct Client<T> {
    subscriptions: HashMap<String, Subscription<T>>,
    order: VecDeque<String>,
}

pub(crate) struct Subscriptions<T> {
    epoch: String,
    targets: HashMap<String, Target<T>>,
    clients: HashMap<String, Client<T>>,
}

impl<T: Clone + PartialEq> Subscriptions<T> {
    pub fn new(epoch: String) -> Self {
        Self {
            epoch,
            targets: HashMap::new(),
            clients: HashMap::new(),
        }
    }

    pub fn register(
        &mut self,
        id: String,
        snapshot: T,
        delivery: Delivery,
    ) -> Result<(), SubscriptionError> {
        if self.targets.contains_key(&id) {
            return Err(SubscriptionError::AlreadyExists);
        }
        self.targets.insert(
            id,
            Target {
                version: Version {
                    epoch: self.epoch.clone(),
                    sequence: 0,
                },
                snapshot: Arc::new(snapshot),
                delivery,
                history: VecDeque::new(),
            },
        );
        Ok(())
    }

    pub fn open(&mut self, id: String) -> Result<(), SubscriptionError> {
        if id.is_empty() || id.len() > 128 {
            return Err(SubscriptionError::InvalidId);
        }
        if self.clients.contains_key(&id) {
            return Err(SubscriptionError::AlreadyExists);
        }
        self.clients.insert(
            id,
            Client {
                subscriptions: HashMap::new(),
                order: VecDeque::new(),
            },
        );
        Ok(())
    }

    pub fn close(&mut self, id: &str) {
        self.clients.remove(id);
    }

    pub fn start(
        &mut self,
        client: &str,
        target: &str,
        version: Option<&Version>,
    ) -> Result<(), SubscriptionError> {
        let value = self
            .targets
            .get(target)
            .ok_or(SubscriptionError::UnknownTarget)?;
        let client = self
            .clients
            .get_mut(client)
            .ok_or(SubscriptionError::StreamEnded)?;
        if client.subscriptions.contains_key(target) {
            return Ok(());
        }
        client.order.push_back(target.into());
        client.subscriptions.insert(
            target.into(),
            Subscription {
                pending: value.resume(version),
                sent: version.filter(|v| v.epoch == self.epoch).cloned(),
                overflowed: false,
            },
        );
        Ok(())
    }

    pub fn stop(&mut self, client: &str, target: &str) -> Result<(), SubscriptionError> {
        let client = self
            .clients
            .get_mut(client)
            .ok_or(SubscriptionError::StreamEnded)?;
        client.subscriptions.remove(target);
        client.order.retain(|id| id != target);
        Ok(())
    }

    pub fn publish(
        &mut self,
        target: &str,
        snapshot: T,
        delta: Option<T>,
    ) -> Result<(), SubscriptionError> {
        let value = self
            .targets
            .get_mut(target)
            .ok_or(SubscriptionError::UnknownTarget)?;
        if *value.snapshot == snapshot {
            return Ok(());
        }
        value.version.sequence = value
            .version
            .sequence
            .checked_add(1)
            .ok_or(SubscriptionError::VersionExhausted)?;
        value.snapshot = Arc::new(snapshot);
        let change = match (value.delivery, delta) {
            (Delivery::Delta, Some(delta)) => {
                Event::Change(value.version.clone(), Delivery::Delta, Arc::new(delta))
            }
            _ => Event::Change(
                value.version.clone(),
                Delivery::Full,
                value.snapshot.clone(),
            ),
        };
        value.history.push_back(change.clone());
        if value.history.len() > RETAINED_CHANGES {
            value.history.pop_front();
        }
        for client in self.clients.values_mut() {
            if let Some(subscription) = client.subscriptions.get_mut(target) {
                if subscription.pending.len() >= RETAINED_CHANGES {
                    subscription.pending.clear();
                    subscription.overflowed = true;
                }
                if !subscription.overflowed {
                    subscription.pending.push_back(change.clone());
                }
            }
        }
        Ok(())
    }

    pub fn bookmark(&mut self, client: &str) {
        if let Some(client) = self.clients.get_mut(client) {
            for (id, subscription) in &mut client.subscriptions {
                if subscription.pending.is_empty() && !subscription.overflowed {
                    subscription
                        .pending
                        .push_back(Event::Bookmark(self.targets[id].version.clone()));
                }
            }
        }
    }

    pub fn next(&mut self, client: &str) -> Option<(String, Event<T>)> {
        let client = self.clients.get_mut(client)?;
        for _ in 0..client.order.len() {
            let id = client.order.pop_front()?;
            client.order.push_back(id.clone());
            let subscription = client.subscriptions.get_mut(&id)?;
            if subscription.overflowed {
                subscription.pending = self.targets[&id].resume(subscription.sent.as_ref());
                subscription.overflowed = false;
            }
            if let Some(event) = subscription.pending.pop_front() {
                subscription.sent = Some(event.version().clone());
                return Some((id, event));
            }
        }
        None
    }
}

#[cfg(test)]
#[path = "subscriptions_test.rs"]
mod subscriptions_tests;
