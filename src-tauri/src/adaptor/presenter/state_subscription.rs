#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PublishedState {
    Value(Box<crate::adaptor::presenter::client::StatePayload>),
    Failure(crate::adaptor::presenter::client::StateReadFailure),
}

impl From<crate::adaptor::presenter::client::StatePayload> for PublishedState {
    fn from(value: crate::adaptor::presenter::client::StatePayload) -> Self {
        Self::Value(Box::new(value))
    }
}

impl From<StateReadError> for PublishedState {
    fn from(error: StateReadError) -> Self {
        use crate::adaptor::presenter::connect::ConnectFailure;
        Self::Failure(crate::adaptor::presenter::client::StateReadFailure {
            code: error.connect_code().grpc_code() as i32,
            message: error.message,
        })
    }
}

use parking_lot::Mutex;
use std::sync::Arc;

use futures_util::Stream;

use crate::infrastructure::state_subscription::{
    Delivery, StateSubscriptionRuntime, Subscriptions, Version,
};
pub(crate) type StateSubscriptionEvent =
    crate::infrastructure::state_subscription::StateSubscriptionEvent<PublishedState>;
use crate::usecase::state_subscription::{
    StateReadError, StateSubscriptionOutput, StateValue, SubscriptionError, SubscriptionTarget,
};
impl From<crate::infrastructure::state_subscription::SubscriptionError> for SubscriptionError {
    fn from(error: crate::infrastructure::state_subscription::SubscriptionError) -> Self {
        use crate::infrastructure::state_subscription::SubscriptionError as DeliveryError;
        match error {
            DeliveryError::InvalidId => Self::InvalidId,
            DeliveryError::AlreadyExists => Self::AlreadyExists,
            DeliveryError::StreamEnded => Self::StreamEnded,
            DeliveryError::UnknownTarget => Self::UnknownTarget,
            DeliveryError::VersionExhausted => Self::VersionExhausted,
            DeliveryError::SnapshotRequired => Self::SnapshotRequired,
        }
    }
}

type RequestedArgs = Arc<
    Mutex<
        std::collections::HashMap<
            String,
            std::collections::HashMap<String, std::collections::HashSet<Vec<String>>>,
        >,
    >,
>;

#[derive(Clone)]
pub(crate) struct StateSubscriptionPresenter {
    runtime: StateSubscriptionRuntime<PublishedState>,
    // ponytail: Notion request registration is serialized; split by client if starts contend.
    pub(crate) request_lock: Arc<tokio::sync::Mutex<()>>,
    requested_args: RequestedArgs,
}

impl StateSubscriptionPresenter {
    pub(crate) fn runtime(&self) -> StateSubscriptionRuntime<PublishedState> {
        self.runtime.clone()
    }

    #[cfg(test)]
    pub(crate) fn test_runtime(&self) -> &StateSubscriptionRuntime<PublishedState> {
        &self.runtime
    }

    pub(crate) fn new() -> Self {
        let runtime = StateSubscriptionRuntime::new(uuid::Uuid::new_v4().to_string());
        Self {
            runtime,
            request_lock: Default::default(),
            requested_args: Default::default(),
        }
    }

    pub(crate) fn add_request(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        args: Vec<String>,
    ) -> (bool, bool) {
        let mut requested = self.requested_args.lock();
        let aliases = requested
            .entry(client.into())
            .or_default()
            .entry(target.to_string())
            .or_default();
        let replay = !aliases.is_empty();
        let inserted = aliases.insert(args);
        (inserted, inserted && replay)
    }

    pub(crate) fn has_other_requests(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        args: &[String],
    ) -> bool {
        self.requested_args
            .lock()
            .get(client)
            .and_then(|targets| targets.get(&target.to_string()))
            .is_some_and(|aliases| aliases.iter().any(|alias| alias != args))
    }

    pub(crate) fn remove_request(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        args: &[String],
    ) -> bool {
        let mut requested = self.requested_args.lock();
        let Some(targets) = requested.get_mut(client) else {
            return true;
        };
        let key = target.to_string();
        if let Some(aliases) = targets.get_mut(&key) {
            aliases.remove(args);
            if !aliases.is_empty() {
                return false;
            }
        }
        targets.remove(&key);
        if targets.is_empty() {
            requested.remove(client);
        }
        true
    }

    pub(crate) fn replay_request(
        &self,
        client: &str,
        target: &SubscriptionTarget,
    ) -> Result<(), SubscriptionError> {
        let raw = target.to_string();
        self.update(|state| {
            state.stop(client, &raw)?;
            state.start(client, &raw, None)?;
            Ok(true)
        })
    }

    pub(crate) fn wire_events(
        &self,
        client: &str,
        event: StateSubscriptionEvent,
    ) -> Vec<
        Result<
            crate::adaptor::presenter::connect_wire::rpc::StateSubscriptionEvent,
            connectrpc::ConnectError,
        >,
    > {
        let aliases = match &event {
            StateSubscriptionEvent::Item(target, _) => self
                .requested_args
                .lock()
                .get(client)
                .and_then(|targets| targets.get(target))
                .cloned(),
            _ => None,
        };
        match aliases {
            Some(aliases) => aliases
                .into_iter()
                .map(|args| {
                    crate::adaptor::presenter::state_subscription_wire::event_with_args(
                        &event,
                        Some(args),
                    )
                })
                .collect::<Vec<_>>(),
            None => vec![crate::adaptor::presenter::state_subscription_wire::event(
                event,
            )],
        }
    }

    fn update(
        &self,
        update: impl FnOnce(
            &mut Subscriptions<PublishedState>,
        ) -> Result<
            bool,
            crate::infrastructure::state_subscription::SubscriptionError,
        >,
    ) -> Result<(), SubscriptionError> {
        self.runtime.update(update).map_err(Into::into)
    }

    pub(crate) fn start(
        &self,
        client: &str,
        target: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), SubscriptionError> {
        let version = cursor_version(cursor);
        self.runtime
            .mutate(|state| {
                let subscribed = state.is_subscribed(client, target);
                let result = state.start(client, target, version.as_ref());
                let mut changed = result.is_ok() && !subscribed;
                if result.is_err() {
                    let existed = state.registered(target);
                    let _ = state.ensure_active(target);
                    changed = existed && !state.registered(target);
                }
                (result, changed)
            })
            .map_err(Into::into)
    }

    pub(crate) fn stop(
        &self,
        client: &str,
        target: &str,
        active: &std::collections::HashSet<SubscriptionTarget>,
    ) -> Result<(), SubscriptionError> {
        let protected = protected_targets(active);
        self.update(|state| {
            let stopped = state.stop(client, target)?;
            Ok(state.release_inactive_snapshots_except(&protected) || stopped)
        })?;
        Ok(())
    }

    pub(crate) fn open(&self, id: String) -> Result<(), SubscriptionError> {
        self.runtime
            .update(|state| state.open(id).map(|_| true))
            .map_err(Into::into)
    }

    pub(crate) fn close(&self, id: &str, active: &std::collections::HashSet<SubscriptionTarget>) {
        self.requested_args.lock().remove(id);
        let protected = protected_targets(active);
        self.runtime.mutate(|state| {
            let targets = state.active_targets();
            state.close(id);
            for target in targets {
                if !protected.contains(&target) {
                    let _ = state.ensure_active(&target);
                }
            }
            state.release_inactive_snapshots_except(&protected);
            ((), true)
        });
    }

    pub(crate) fn stream<P: Send + 'static, F: Fn(String, Vec<String>) + Send + Sync + 'static>(
        &self,
        id: String,
        permit: P,
        refresh: F,
    ) -> impl Stream<Item = StateSubscriptionEvent> + Send + use<P, F> {
        self.runtime.stream(id, permit, refresh)
    }
}

#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
pub(crate) fn test_output() -> crate::usecase::state_subscription::StateSubscriptionOutputRef {
    Arc::new(StateSubscriptionPresenter::new())
}

fn protected_targets(
    active: &std::collections::HashSet<SubscriptionTarget>,
) -> std::collections::HashSet<String> {
    active
        .iter()
        .map(ToString::to_string)
        .chain(std::iter::once(
            SubscriptionTarget::RepositoryPaths.to_string(),
        ))
        .collect()
}

fn cursor_version(cursor: Option<(&str, u64)>) -> Option<Version> {
    cursor.map(|(epoch, sequence)| Version {
        epoch: epoch.into(),
        sequence,
    })
}

impl StateSubscriptionOutput for StateSubscriptionPresenter {
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), StateReadError> {
        self.start(client, &target.to_string(), cursor)
            .map_err(StateReadError::from_error)
    }

    fn stop(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        active: &std::collections::HashSet<SubscriptionTarget>,
    ) -> Result<(), SubscriptionError> {
        self.stop(client, &target.to_string(), active)
    }

    fn publish_failure(
        &self,
        target: &SubscriptionTarget,
        error: StateReadError,
    ) -> Result<(), SubscriptionError> {
        let snapshot = PublishedState::from(error);
        let target = target.to_string();
        self.update(|state| {
            if state.registered(&target) {
                state.publish(&target, snapshot, None)
            } else {
                state
                    .register(target, snapshot, Delivery::Full)
                    .map(|_| true)
            }
        })
    }

    fn publish_initial(
        &self,
        target: &SubscriptionTarget,
        snapshot: StateValue,
    ) -> Result<(), SubscriptionError> {
        let target = target.to_string();
        let snapshot = crate::adaptor::presenter::state_subscription_wire::payload(&snapshot)
            .map(PublishedState::from)
            .map_err(|_| SubscriptionError::EncodingFailed)?;
        self.update(|state| {
            if state.registered(&target) {
                state.publish(&target, snapshot, None)
            } else {
                state
                    .register(target, snapshot, Delivery::Full)
                    .map(|_| true)
            }
        })
    }

    fn publish(
        &self,
        target: &SubscriptionTarget,
        snapshot: StateValue,
        delta: Option<StateValue>,
    ) -> Result<(), SubscriptionError> {
        let target = target.to_string();
        let snapshot = crate::adaptor::presenter::state_subscription_wire::payload(&snapshot)
            .map(PublishedState::from)
            .map_err(|_| SubscriptionError::EncodingFailed)?;
        let delta = delta
            .as_ref()
            .map(|value| {
                crate::adaptor::presenter::state_subscription_wire::payload(value)
                    .map(PublishedState::from)
            })
            .transpose()
            .map_err(|_| SubscriptionError::EncodingFailed)?;
        self.update(|state| state.publish(&target, snapshot, delta))
    }
}

#[cfg(test)]
#[path = "state_subscription_test.rs"]
mod state_subscription_tests;
