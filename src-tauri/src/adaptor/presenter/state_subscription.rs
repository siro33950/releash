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

#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
use std::sync::Arc;

use futures_util::Stream;
use futures_util::StreamExt;

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

#[derive(Clone)]
pub(crate) struct StateSubscriptionPresenter {
    runtime: StateSubscriptionRuntime<PublishedState>,
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
        Self { runtime }
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

    pub(crate) fn stream_wire<
        P: Send + 'static,
        F: Fn(String, Vec<String>) + Send + Sync + 'static,
    >(
        &self,
        id: String,
        permit: P,
        refresh: F,
    ) -> impl Stream<
        Item = Result<
            crate::adaptor::presenter::connect_wire::rpc::StateSubscriptionEvent,
            connectrpc::ConnectError,
        >,
    > + Send
           + use<P, F> {
        self.stream(id, permit, refresh)
            .map(crate::adaptor::presenter::state_subscription_wire::event)
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
        use crate::adaptor::presenter::connect::ConnectFailure;
        let failure = crate::adaptor::presenter::client::StateReadFailure {
            code: error.connect_code().grpc_code() as i32,
            message: error.message,
        };
        let snapshot = PublishedState::Failure(failure);
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
