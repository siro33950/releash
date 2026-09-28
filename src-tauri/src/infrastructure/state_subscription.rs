use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use futures_util::{Stream, StreamExt};
use parking_lot::Mutex;
use tokio::sync::Notify;

pub(crate) enum StateSubscriptionEvent<T> {
    Ready,
    Item(String, Event<T>),
}

const BOOKMARK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone)]
pub(crate) struct StateSubscriptionRuntime<T> {
    state: Arc<Mutex<Subscriptions<T>>>,
    changed: Arc<Notify>,
    terminal_routes: Arc<Mutex<HashMap<String, String>>>,
    terminal_boot: String,
}

impl<T: Clone + PartialEq + Send + Sync + 'static> StateSubscriptionRuntime<T> {
    pub(crate) fn new(epoch: String) -> Self {
        Self {
            state: Arc::new(Mutex::new(Subscriptions::new(epoch))),
            changed: Arc::new(Notify::new()),
            terminal_routes: Default::default(),
            terminal_boot: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub(crate) fn terminal_version(&self, runtime_generation: u64, sequence: u64) -> Version {
        Version {
            epoch: format!("{}:{runtime_generation}", self.terminal_boot),
            sequence,
        }
    }

    pub(crate) fn register_terminal(
        &self,
        session: &str,
        target: &str,
        version: Version,
        pending_limit: usize,
    ) -> Result<(), SubscriptionError> {
        let mut routes = self.terminal_routes.lock();
        let previous = routes.insert(session.into(), target.into());
        if let Err(error) =
            self.update(|state| state.register_delta(target, version, pending_limit))
        {
            match previous {
                Some(previous) => {
                    routes.insert(session.into(), previous);
                }
                None => {
                    routes.remove(session);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn unregister_terminal(&self, session: &str, epoch: &str) -> Option<bool> {
        let mut routes = self.terminal_routes.lock();
        let target = routes.get(session)?.clone();
        let subscribed = self.mutate(|state| {
            if state
                .current_version(&target)
                .is_none_or(|version| version.epoch != epoch)
            {
                return None;
            }
            state.unregister(&target).ok()?;
            Some(state.has_subscribers(&target))
        });
        if subscribed.is_some() {
            routes.remove(session);
        }
        subscribed
    }

    pub(crate) fn terminal_route(&self, session: &str) -> Option<String> {
        self.terminal_routes.lock().get(session).cloned()
    }

    #[cfg(test)]
    pub(crate) fn test_terminal_route_count(&self) -> usize {
        self.terminal_routes.lock().len()
    }

    pub(crate) fn update(
        &self,
        update: impl FnOnce(&mut Subscriptions<T>) -> Result<(), SubscriptionError>,
    ) -> Result<(), SubscriptionError> {
        let result = update(&mut self.state.lock());
        if result.is_ok() {
            self.changed.notify_waiters();
        }
        result
    }

    pub(crate) fn inspect<R>(&self, read: impl FnOnce(&Subscriptions<T>) -> R) -> R {
        read(&self.state.lock())
    }

    pub(crate) fn mutate<R>(&self, update: impl FnOnce(&mut Subscriptions<T>) -> R) -> R {
        let result = update(&mut self.state.lock());
        self.changed.notify_waiters();
        result
    }

    pub(crate) fn stream<P, F>(
        &self,
        id: String,
        permit: P,
        refresh: F,
    ) -> impl Stream<Item = StateSubscriptionEvent<T>> + Send + use<T, P, F>
    where
        P: Send + 'static,
        F: Fn(String, Vec<String>) + Send + Sync + 'static,
    {
        let runtime = self.clone();
        let mut timer = tokio::time::interval_at(
            tokio::time::Instant::now() + BOOKMARK_INTERVAL,
            BOOKMARK_INTERVAL,
        );
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let events = futures_util::stream::unfold(
            (id, permit, timer, runtime, refresh),
            |(id, permit, mut timer, runtime, refresh)| async move {
                loop {
                    let changed = runtime.changed.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    let requests = {
                        let state = runtime.state.lock();
                        state
                            .snapshot_requests(&id)
                            .into_iter()
                            .map(|raw| {
                                let clients = state.snapshot_request_clients(&raw);
                                (raw, clients)
                            })
                            .collect::<Vec<_>>()
                    };
                    for (raw, clients) in requests {
                        refresh(raw, clients);
                    }
                    if let Some((target, event)) = runtime.state.lock().next(&id) {
                        return Some((
                            StateSubscriptionEvent::Item(target, event),
                            (id, permit, timer, runtime.clone(), refresh),
                        ));
                    }
                    tokio::select! {
                        _ = changed => {},
                        _ = timer.tick() => runtime.state.lock().bookmark(&id),
                    }
                }
            },
        );
        futures_util::stream::once(async { StateSubscriptionEvent::Ready }).chain(events)
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

struct Target<T> {
    version: Version,
    snapshot: Option<Arc<T>>,
    delivery: Delivery,
    history: VecDeque<(Event<T>, bool)>,
    history_units: VecDeque<usize>,
    discarded_through: Option<u64>,
    pending_limit: Option<usize>,
    persistent: bool,
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
            .collect()
    }

    fn pending_sizes(&self, version: Option<&Version>) -> VecDeque<usize> {
        if self.resumable(version) {
            let mut sizes = self.replay_sizes(version.unwrap());
            sizes.push_back(0);
            sizes
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
    subscriptions: HashMap<String, Subscription<T>>,
    order: VecDeque<String>,
}

pub(crate) struct Subscriptions<T> {
    epoch: String,
    target_generation: u64,
    targets: HashMap<String, Target<T>>,
    clients: HashMap<String, Client<T>>,
}

impl<T: Clone + PartialEq> Subscriptions<T> {
    pub fn new(epoch: String) -> Self {
        Self {
            epoch,
            target_generation: 0,
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
                persistent: false,
            },
        );
        Ok(())
    }

    pub fn unregister(&mut self, target: &str) -> Result<(), SubscriptionError> {
        let target = target.to_string();
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

    pub fn start(
        &mut self,
        client: &str,
        target: &str,
        version: Option<&Version>,
    ) -> Result<(), SubscriptionError> {
        let target = target.to_string();
        let value = self
            .targets
            .get(&target)
            .ok_or(SubscriptionError::UnknownTarget)?;
        let awaiting_snapshot = value.snapshot.is_none() && !value.resumable(version);
        if awaiting_snapshot && value.delivery != Delivery::Delta {
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
                pending: if awaiting_snapshot {
                    VecDeque::new()
                } else {
                    value.resume(version)
                },
                sent: version.filter(|v| v.epoch == value.version.epoch).cloned(),
                overflowed: awaiting_snapshot,
                pending_units: if awaiting_snapshot {
                    0
                } else {
                    value.pending_sizes(version).iter().sum()
                },
                sizes: if awaiting_snapshot {
                    VecDeque::new()
                } else {
                    value.pending_sizes(version)
                },
            },
        );
        Ok(())
    }

    pub fn pending_amount(&self, client: &str, target: &str) -> usize {
        let target = target.to_string();
        self.clients
            .get(client)
            .and_then(|client| client.subscriptions.get(&target))
            .map(|subscription| subscription.pending_units)
            .unwrap_or(0)
    }

    pub fn current_version(&self, target: &str) -> Option<Version> {
        self.targets
            .get(target)
            .map(|target| target.version.clone())
    }

    pub fn register_delta(
        &mut self,
        target: &str,
        version: Version,
        pending_limit: usize,
    ) -> Result<(), SubscriptionError> {
        let id = target.to_string();
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
                persistent: false,
            },
        );
        for client in self.clients.values_mut() {
            if let Some(subscription) = client.subscriptions.get_mut(&id) {
                subscription.overflowed = true;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn is_subscribed(&self, client: &str, target: &str) -> bool {
        self.clients
            .get(client)
            .is_some_and(|client| client.subscriptions.contains_key(target))
    }

    pub fn has_subscribers(&self, target: &str) -> bool {
        self.clients
            .values()
            .any(|client| client.subscriptions.contains_key(target))
    }

    #[cfg(test)]
    pub fn needs_snapshot(
        &self,
        target: &str,
        version: Option<&Version>,
    ) -> Result<bool, SubscriptionError> {
        let id = target.to_string();
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
        let id = target.to_string();
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
        let units = units.max(1);
        let id = target.to_string();
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
        value.history_units.push_back(units);
        if value.history.len() > RETAINED_CHANGES {
            value.discarded_through = value
                .history
                .pop_front()
                .map(|(event, _)| event.version().sequence);
            value.history_units.pop_front();
        }
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
        let id = target.to_string();
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
            .filter(|(id, subscription)| self.snapshot_required(id, subscription))
            .map(|(id, _)| id.to_string())
            .collect()
    }

    pub fn snapshot_request_clients(&self, target: &str) -> Vec<String> {
        self.clients
            .iter()
            .filter(|(_, client)| {
                client
                    .subscriptions
                    .get(target)
                    .is_some_and(|subscription| self.snapshot_required(target, subscription))
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn snapshot_required(&self, target: &str, subscription: &Subscription<T>) -> bool {
        self.targets.get(target).is_some_and(|value| {
            subscription.overflowed
                && value.snapshot.is_none()
                && !value.resumable(subscription.sent.as_ref())
        })
    }

    pub fn stop(&mut self, client: &str, target: &str) -> Result<(), SubscriptionError> {
        let client = self
            .clients
            .get_mut(client)
            .ok_or(SubscriptionError::StreamEnded)?;
        let target = target.to_string();
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
        let target = target.to_string();
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

    pub fn ensure_active(&mut self, target: &str) -> Result<(), SubscriptionError> {
        if self.active_targets().contains(target) {
            return Ok(());
        }
        if self
            .targets
            .get(target)
            .is_some_and(|value| !value.persistent && value.delivery == Delivery::Full)
        {
            self.target_generation = self
                .target_generation
                .checked_add(1)
                .ok_or(SubscriptionError::VersionExhausted)?;
            self.targets.remove(target);
        }
        Err(SubscriptionError::StreamEnded)
    }

    #[cfg(test)]
    pub fn release_inactive_snapshots(&mut self) {
        self.release_inactive_snapshots_except(&Default::default());
    }

    pub fn release_inactive_snapshots_except(
        &mut self,
        protected: &std::collections::HashSet<String>,
    ) {
        let active = self.active_targets();
        for (id, target) in &mut self.targets {
            if !target.persistent && !active.contains(id) && !protected.contains(id) {
                target.snapshot = None;
                if target.delivery == Delivery::Full {
                    target.history.clear();
                    target.history_units.clear();
                }
            }
        }
    }

    pub fn active_targets(&self) -> std::collections::HashSet<String> {
        self.clients
            .values()
            .flat_map(|client| client.subscriptions.keys().cloned())
            .collect()
    }

    pub fn registered(&self, target: &str) -> bool {
        self.targets.contains_key(target)
    }

    pub fn protect(&mut self, target: &str) {
        if let Some(value) = self.targets.get_mut(target) {
            value.persistent = true;
        }
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
#[path = "state_subscription_test.rs"]
mod subscriptions_tests;
