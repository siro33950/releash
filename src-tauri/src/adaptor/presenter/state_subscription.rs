#[derive(Debug, Clone, PartialEq)]
pub enum PublishedState {
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

use std::sync::Arc;

use futures_util::Stream;

use crate::infrastructure::state_subscription::{
    Delivery, StateSubscriptionRuntime, Subscriptions, Version,
};
pub type StateSubscriptionEvent =
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
pub struct StateSubscriptionPresenter {
    runtime: StateSubscriptionRuntime<PublishedState>,
}

pub struct SubscriptionDelivery {
    presenter: StateSubscriptionPresenter,
    client: String,
    id: String,
    identity: Arc<()>,
    cursor: Option<Version>,
    pending: std::sync::atomic::AtomicBool,
}

impl crate::usecase::state_subscription::StateSubscriptionDelivery for SubscriptionDelivery {
    fn start(&self) -> Result<Option<usize>, StateReadError> {
        let result = self.presenter.runtime.mutate(|state| {
            if !state.matches_identity(&self.id, &self.identity) {
                return (Err(crate::infrastructure::state_subscription::SubscriptionError::StreamEnded), false);
            }
            let result = state.activate(&self.id, self.cursor.as_ref()).map(|()| {
                (!state.awaiting_snapshot(&self.client, &self.id))
                    .then(|| state.pending_amount(&self.client, &self.id))
            });
            let changed = result.is_ok();
            (result, changed)
        }).map_err(SubscriptionError::from);
        if result.is_ok() {
            self.pending
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
        result.map_err(StateReadError::from_error)
    }

    fn claim(&self) -> bool {
        self.presenter
            .runtime
            .mutate(|state| (state.claim(&self.id, &self.identity), false))
    }

    fn finish(
        &self,
        active: &std::collections::HashSet<SubscriptionTarget>,
    ) -> Result<(), SubscriptionError> {
        let protected = protected_targets(active);
        self.presenter.update(|state| {
            let stopped = if state.matches_identity(&self.id, &self.identity) {
                state.stop(&self.client, &self.id)?
            } else {
                false
            };
            Ok(state.release_inactive_snapshots_except(&protected) || stopped)
        })
    }
}

impl Drop for SubscriptionDelivery {
    fn drop(&mut self) {
        if self.pending.load(std::sync::atomic::Ordering::Relaxed) {
            use crate::usecase::state_subscription::StateSubscriptionDelivery;
            let _ = self.finish(&Default::default());
        }
    }
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

    pub fn reserve_delivery(
        &self,
        client: &str,
        id: &str,
        target: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<SubscriptionDelivery, SubscriptionError> {
        let identity = self
            .runtime
            .mutate(|state| {
                let result = state
                    .reserve(client, id, target)
                    .map(|()| state.identity(id).unwrap());
                let changed = if result.is_err() {
                    let existed = state.registered(target);
                    let _ = state.ensure_active(target);
                    existed && !state.registered(target)
                } else {
                    false
                };
                (result, changed)
            })
            .map_err(SubscriptionError::from)?;
        Ok(SubscriptionDelivery {
            presenter: self.clone(),
            client: client.into(),
            id: id.into(),
            identity,
            cursor: cursor_version(cursor),
            pending: std::sync::atomic::AtomicBool::new(true),
        })
    }

    pub fn delivery(&self, id: &str) -> Option<(String, String, SubscriptionDelivery)> {
        self.runtime.mutate(|state| {
            let value = state.lookup(id).map(|(client, target)| {
                let delivery = SubscriptionDelivery {
                    presenter: self.clone(),
                    client: client.clone(),
                    id: id.into(),
                    identity: state.identity(id).unwrap(),
                    cursor: None,
                    pending: std::sync::atomic::AtomicBool::new(false),
                };
                (client, target, delivery)
            });
            (value, false)
        })
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
}

#[cfg(any(test, feature = "test-support"))]
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
    #[cfg(any(test, feature = "test-support"))]
    fn as_any(&self) -> &dyn std::any::Any {
        self
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
pub(crate) mod state_subscription_tests;
