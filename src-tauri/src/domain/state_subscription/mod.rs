mod subscription_target;
pub(crate) use subscription_target::{StateChangeSource, SubscriptionTarget, WatchRequirement};

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
    SnapshotRequired,
}

impl std::fmt::Display for SubscriptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SubscriptionError {}

impl crate::domain::failure::ClassifiedFailure for SubscriptionError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind as F;
        match self {
            Self::InvalidId => F::InvalidInput,
            Self::AlreadyExists => F::AlreadyPresent,
            Self::StreamEnded | Self::UnknownTarget | Self::SnapshotRequired => F::Missing,
            Self::VersionExhausted => F::Internal,
        }
    }
}

struct Target<T> {
    version: Version,
    snapshot: Option<Arc<T>>,
    delivery: Delivery,
    history: VecDeque<(Event<T>, bool)>,
    history_units: VecDeque<usize>,
    discarded_through: Option<u64>,
    pending_limit: Option<usize>,
}

impl<T: Clone> Target<T> {
    fn resumable(&self, version: Option<&Version>) -> bool {
        version.is_some_and(|version| {
            version.epoch == self.version.epoch
                && (self.delivery != Delivery::Delta
                    || self
                        .discarded_through
                        .is_none_or(|discarded| version.sequence > discarded))
                && version.sequence <= self.version.sequence
                && (version.sequence == self.version.sequence
                    || self.history.front().is_some_and(|(event, _)| {
                        event.version().sequence <= version.sequence.saturating_add(1)
                    }))
                && self
                    .pending_limit
                    .is_none_or(|limit| self.replay_sizes(version).iter().sum::<usize>() <= limit)
        })
    }

    fn replay_sizes(&self, version: &Version) -> VecDeque<usize> {
        self.history
            .iter()
            .zip(self.history_units.iter())
            .filter(|((event, same_version), _)| {
                event.version().sequence > version.sequence
                    || (*same_version && event.version().sequence == version.sequence)
            })
            .map(|(_, units)| *units)
            .chain(std::iter::once(0))
            .collect()
    }

    fn pending_sizes(&self, version: Option<&Version>) -> VecDeque<usize> {
        if self.resumable(version) {
            self.replay_sizes(version.unwrap())
        } else {
            VecDeque::from([0, 0])
        }
    }

    fn resume(&self, version: Option<&Version>) -> VecDeque<Event<T>> {
        let mut pending = VecDeque::new();
        if self.resumable(version) {
            let sequence = version.unwrap().sequence;
            pending.extend(
                self.history
                    .iter()
                    .filter(|(event, same_version)| {
                        event.version().sequence > sequence
                            || (*same_version && event.version().sequence == sequence)
                    })
                    .map(|(event, _)| event.clone()),
            );
        } else {
            pending.push_back(Event::Snapshot(
                self.version.clone(),
                self.snapshot.clone().expect("active target has a snapshot"),
            ));
        }
        pending.push_back(Event::Bookmark(self.version.clone()));
        pending
    }
}

struct Subscription<T> {
    pending: VecDeque<Event<T>>,
    sent: Option<Version>,
    overflowed: bool,
    pending_units: usize,
    sizes: VecDeque<usize>,
}

struct Client<T> {
    subscriptions: HashMap<SubscriptionTarget, Subscription<T>>,
    order: VecDeque<SubscriptionTarget>,
}

pub(crate) struct Subscriptions<T> {
    epoch: String,
    target_generation: u64,
    targets: HashMap<SubscriptionTarget, Target<T>>,
    clients: HashMap<String, Client<T>>,
    watches: std::collections::HashSet<WatchRequirement>,
}

impl<T: Clone + PartialEq> Subscriptions<T> {
    pub fn new(epoch: String) -> Self {
        Self {
            epoch,
            target_generation: 0,
            targets: HashMap::new(),
            clients: HashMap::new(),
            watches: Default::default(),
        }
    }

    pub fn register(
        &mut self,
        id: String,
        snapshot: T,
        delivery: Delivery,
    ) -> Result<(), SubscriptionError> {
        let id = SubscriptionTarget::parse(&id)?;
        if self.targets.contains_key(&id) {
            return Err(SubscriptionError::AlreadyExists);
        }
        self.targets.insert(
            id,
            Target {
                version: Version {
                    epoch: if self.target_generation == 0 {
                        self.epoch.clone()
                    } else {
                        format!("{}:{}", self.epoch, self.target_generation)
                    },
                    sequence: 0,
                },
                snapshot: Some(Arc::new(snapshot)),
                delivery,
                history: VecDeque::new(),
                history_units: VecDeque::new(),
                discarded_through: None,
                pending_limit: None,
            },
        );
        Ok(())
    }

    pub fn unregister(&mut self, target: &str) -> Result<(), SubscriptionError> {
        let target = SubscriptionTarget::parse(target)?;
        self.targets.remove(&target);
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

    pub fn start_with_snapshot(
        &mut self,
        client: &str,
        raw: &str,
        snapshot: T,
        version: Option<&Version>,
    ) -> Result<(), SubscriptionError> {
        let target = SubscriptionTarget::parse(raw)?;
        if !self.clients.contains_key(client) {
            self.ensure_active(&target)?;
            return Err(SubscriptionError::StreamEnded);
        }
        if self.registered(&target) {
            self.publish(raw, snapshot, None)?;
        } else {
            self.register(raw.into(), snapshot, Delivery::Full)?;
        }
        self.start(client, raw, version)
    }

    pub fn start(
        &mut self,
        client: &str,
        target: &str,
        version: Option<&Version>,
    ) -> Result<(), SubscriptionError> {
        let target = SubscriptionTarget::parse(target)?;
        let value = self
            .targets
            .get(&target)
            .ok_or(SubscriptionError::UnknownTarget)?;
        if value.snapshot.is_none() && !value.resumable(version) {
            return Err(SubscriptionError::UnknownTarget);
        }
        let client = self
            .clients
            .get_mut(client)
            .ok_or(SubscriptionError::StreamEnded)?;
        if client.subscriptions.contains_key(&target) {
            return Ok(());
        }
        client.order.push_back(target.clone());
        client.subscriptions.insert(
            target,
            Subscription {
                pending: value.resume(version),
                sent: version.filter(|v| v.epoch == value.version.epoch).cloned(),
                overflowed: false,
                pending_units: value.pending_sizes(version).iter().sum(),
                sizes: value.pending_sizes(version),
            },
        );
        Ok(())
    }

    pub fn pending_amount(
        &self,
        client: &str,
        target: &str,
        measure: impl Fn(&T) -> usize,
    ) -> usize {
        let Ok(target) = SubscriptionTarget::parse(target) else {
            return 0;
        };
        self.clients
            .get(client)
            .and_then(|client| client.subscriptions.get(&target))
            .map(|subscription| {
                subscription
                    .pending
                    .iter()
                    .map(|event| match event {
                        Event::Change(_, _, value) => measure(value),
                        _ => 0,
                    })
                    .sum()
            })
            .unwrap_or(0)
    }

    pub fn current_version(&self, target: &str) -> Option<Version> {
        self.targets
            .get(&SubscriptionTarget::parse(target).ok()?)
            .map(|target| target.version.clone())
    }

    pub fn register_delta(
        &mut self,
        target: &str,
        version: Version,
        pending_limit: usize,
    ) -> Result<(), SubscriptionError> {
        let id = SubscriptionTarget::parse(target)?;
        if self
            .targets
            .get(&id)
            .is_some_and(|value| value.version.epoch == version.epoch)
        {
            return Ok(());
        }
        self.targets.insert(
            id.clone(),
            Target {
                version,
                snapshot: None,
                delivery: Delivery::Delta,
                history: VecDeque::new(),
                history_units: VecDeque::new(),
                discarded_through: None,
                pending_limit: Some(pending_limit),
            },
        );
        for client in self.clients.values_mut() {
            if let Some(subscription) = client.subscriptions.get_mut(&id) {
                subscription.overflowed = true;
            }
        }
        Ok(())
    }

    pub fn is_subscribed(&self, client: &str, target: &str) -> bool {
        SubscriptionTarget::parse(target).is_ok_and(|target| {
            self.clients
                .get(client)
                .is_some_and(|client| client.subscriptions.contains_key(&target))
        })
    }

    pub fn needs_snapshot(
        &self,
        target: &str,
        version: Option<&Version>,
    ) -> Result<bool, SubscriptionError> {
        let id = SubscriptionTarget::parse(target)?;
        let value = self
            .targets
            .get(&id)
            .ok_or(SubscriptionError::UnknownTarget)?;
        Ok(!value.resumable(version))
    }

    pub fn set_delta_snapshot(
        &mut self,
        target: &str,
        version: Version,
        snapshot: T,
    ) -> Result<(), SubscriptionError> {
        let id = SubscriptionTarget::parse(target)?;
        let value = self
            .targets
            .get_mut(&id)
            .ok_or(SubscriptionError::UnknownTarget)?;
        if value.version.epoch != version.epoch || value.version.sequence > version.sequence {
            return Err(SubscriptionError::SnapshotRequired);
        }
        if value.version.sequence < version.sequence {
            value.history.clear();
            value.history_units.clear();
        }
        value.version = version;
        value.snapshot = Some(Arc::new(snapshot));
        Ok(())
    }

    pub fn publish_delta(
        &mut self,
        target: &str,
        version: Version,
        delta: T,
        units: usize,
        advances_version: bool,
    ) -> Result<(), SubscriptionError> {
        let id = SubscriptionTarget::parse(target)?;
        let value = self
            .targets
            .get_mut(&id)
            .ok_or(SubscriptionError::UnknownTarget)?;
        if value.version.epoch != version.epoch || version.sequence < value.version.sequence {
            return Err(SubscriptionError::SnapshotRequired);
        }
        if version.sequence
            > value
                .version
                .sequence
                .saturating_add(u64::from(advances_version))
        {
            value.discarded_through = Some(version.sequence.saturating_sub(1));
            value.history.clear();
            value.history_units.clear();
            for client in self.clients.values_mut() {
                if let Some(subscription) = client.subscriptions.get_mut(&id) {
                    subscription.pending.clear();
                    subscription.sizes.clear();
                    subscription.pending_units = 0;
                    subscription.overflowed = true;
                }
            }
        }
        let same_version = !advances_version;
        value.version = version.clone();
        value.snapshot = None;
        let event = Event::Change(version, Delivery::Delta, Arc::new(delta));
        value.history.push_back((event.clone(), same_version));
        value.history_units.push_back(units.max(1));
        if value.history.len() > RETAINED_CHANGES {
            value.discarded_through = value
                .history
                .pop_front()
                .map(|(event, _)| event.version().sequence);
            value.history_units.pop_front();
        }
        let units = units.max(1);
        for client in self.clients.values_mut() {
            if let Some(subscription) = client.subscriptions.get_mut(&id) {
                if subscription.overflowed {
                    continue;
                }
                if subscription.pending_units.saturating_add(units)
                    > value.pending_limit.unwrap_or(0)
                {
                    subscription.pending.clear();
                    subscription.sizes.clear();
                    subscription.pending_units = 0;
                    subscription.overflowed = true;
                }
                if !subscription.overflowed {
                    while subscription.sizes.len() < subscription.pending.len() {
                        subscription.sizes.push_back(0);
                    }
                    subscription.pending.push_back(event.clone());
                    subscription.sizes.push_back(units);
                    subscription.pending_units += units;
                }
            }
        }
        Ok(())
    }

    pub fn require_delta_snapshot(&mut self, target: &str) -> Result<(), SubscriptionError> {
        let id = SubscriptionTarget::parse(target)?;
        let value = self
            .targets
            .get_mut(&id)
            .ok_or(SubscriptionError::UnknownTarget)?;
        value.snapshot = None;
        value.history.clear();
        value.history_units.clear();
        value.discarded_through = Some(value.version.sequence);
        for client in self.clients.values_mut() {
            if let Some(subscription) = client.subscriptions.get_mut(&id) {
                subscription.pending.clear();
                subscription.sizes.clear();
                subscription.pending_units = 0;
                subscription.overflowed = true;
            }
        }
        Ok(())
    }

    pub fn snapshot_requests(&self, client: &str) -> Vec<String> {
        self.clients
            .get(client)
            .into_iter()
            .flat_map(|client| client.subscriptions.iter())
            .filter(|(id, subscription)| {
                let Some(value) = self.targets.get(*id) else {
                    return false;
                };
                subscription.overflowed
                    && value.snapshot.is_none()
                    && !value.resumable(subscription.sent.as_ref())
            })
            .map(|(id, _)| id.to_string())
            .collect()
    }

    pub fn stop(&mut self, client: &str, target: &str) -> Result<(), SubscriptionError> {
        let client = self
            .clients
            .get_mut(client)
            .ok_or(SubscriptionError::StreamEnded)?;
        let target = SubscriptionTarget::parse(target)?;
        client.subscriptions.remove(&target);
        client.order.retain(|id| id != &target);
        Ok(())
    }

    pub fn publish(
        &mut self,
        target: &str,
        snapshot: T,
        delta: Option<T>,
    ) -> Result<(), SubscriptionError> {
        let target = SubscriptionTarget::parse(target)?;
        let value = self
            .targets
            .get_mut(&target)
            .ok_or(SubscriptionError::UnknownTarget)?;
        if value.snapshot.as_deref() == Some(&snapshot) {
            return Ok(());
        }
        value.version.sequence = value
            .version
            .sequence
            .checked_add(1)
            .ok_or(SubscriptionError::VersionExhausted)?;
        value.snapshot = Some(Arc::new(snapshot));
        let change = match (value.delivery, delta) {
            (Delivery::Delta, Some(delta)) => {
                Event::Change(value.version.clone(), Delivery::Delta, Arc::new(delta))
            }
            _ => Event::Change(
                value.version.clone(),
                Delivery::Full,
                value.snapshot.clone().expect("published snapshot"),
            ),
        };
        value.history.push_back((change.clone(), false));
        value.history_units.push_back(0);
        if value.history.len() > RETAINED_CHANGES {
            value.discarded_through = value
                .history
                .pop_front()
                .map(|(event, _)| event.version().sequence);
            value.history_units.pop_front();
        }
        for client in self.clients.values_mut() {
            if let Some(subscription) = client.subscriptions.get_mut(&target) {
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

    pub fn ensure_active(&mut self, target: &SubscriptionTarget) -> Result<(), SubscriptionError> {
        if self.active_targets().contains(target) {
            return Ok(());
        }
        if *target != SubscriptionTarget::RepositoryPaths && self.targets.contains_key(target) {
            self.target_generation = self
                .target_generation
                .checked_add(1)
                .ok_or(SubscriptionError::VersionExhausted)?;
            self.targets.remove(target);
        }
        Err(SubscriptionError::StreamEnded)
    }

    pub fn release_inactive_snapshots(&mut self) {
        let active = self.active_targets();
        for (id, target) in &mut self.targets {
            if *id != SubscriptionTarget::RepositoryPaths && !active.contains(id) {
                target.snapshot = None;
                if target.delivery == Delivery::Full {
                    target.history.clear();
                    target.history_units.clear();
                }
            }
        }
    }

    pub fn active_targets(&self) -> std::collections::HashSet<SubscriptionTarget> {
        self.clients
            .values()
            .flat_map(|client| client.subscriptions.keys().cloned())
            .collect()
    }

    pub fn watch_failed(&mut self, requirement: &WatchRequirement) {
        self.watches.remove(requirement);
    }

    pub fn watch_changes(
        &mut self,
        repositories: &[String],
        history_paths: &[String],
    ) -> (Vec<WatchRequirement>, Vec<WatchRequirement>) {
        let required = self.required_watches(repositories, history_paths);
        let start = required.difference(&self.watches).cloned().collect();
        let stop = self.watches.difference(&required).cloned().collect();
        self.watches = required;
        (start, stop)
    }

    pub fn registered(&self, target: &SubscriptionTarget) -> bool {
        self.targets.contains_key(target)
    }

    pub fn required_watches(
        &self,
        repositories: &[String],
        history_paths: &[String],
    ) -> std::collections::HashSet<WatchRequirement> {
        self.active_targets()
            .iter()
            .flat_map(|target| target.watches(repositories, history_paths))
            .collect()
    }

    pub fn bookmark(&mut self, client: &str) {
        if let Some(client) = self.clients.get_mut(client) {
            for (id, subscription) in &mut client.subscriptions {
                if subscription.pending.is_empty() && !subscription.overflowed {
                    if let Some(target) = self.targets.get(id) {
                        subscription
                            .pending
                            .push_back(Event::Bookmark(target.version.clone()));
                    }
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
            if subscription.overflowed && subscription.pending.is_empty() {
                let Some(value) = self.targets.get(&id) else {
                    continue;
                };
                if value.snapshot.is_none() && !value.resumable(subscription.sent.as_ref()) {
                    continue;
                }
                subscription.sizes = value.pending_sizes(subscription.sent.as_ref());
                subscription.pending_units = subscription.sizes.iter().sum();
                subscription.pending = self.targets[&id].resume(subscription.sent.as_ref());
                subscription.overflowed = false;
            }
            if let Some(event) = subscription.pending.pop_front() {
                subscription.pending_units = subscription
                    .pending_units
                    .saturating_sub(subscription.sizes.pop_front().unwrap_or(0));
                subscription.sent = Some(event.version().clone());
                return Some((id.to_string(), event));
            }
        }
        None
    }
}

#[cfg(test)]
#[path = "subscriptions_test.rs"]
mod subscriptions_tests;
