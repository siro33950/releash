use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{Stream, StreamExt};
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::infrastructure::state_subscription::{Delivery, Event, Subscriptions, Version};
use crate::infrastructure::terminal::output_flow_control::{
    OUTPUT_PENDING_LIMIT, OUTPUT_REPORT_UNITS,
};
use crate::usecase::state_subscription::{
    StateChangeSource, StateSubscriptionOutput, StateSubscriptionUsecase, StateValue,
    SubscriptionError, REPO_PATHS,
};
use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
use crate::usecase::terminal_surface::output::{
    TerminalSurfaceOutputEvent, TerminalSurfaceOutputSummary, TerminalSurfaceStateSink,
};

const BOOKMARK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

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

pub(crate) enum StateSubscriptionEvent {
    Ready,
    Item(String, Event<StateValue>),
}

#[derive(Clone)]
pub(crate) struct StateSubscriptionPresenter {
    pub(crate) state: Arc<Mutex<Subscriptions<StateValue>>>,
    changed: Arc<Notify>,
    invalidated: tokio::sync::broadcast::Sender<StateChangeSource>,
    pub(crate) terminal_routes: Arc<Mutex<HashMap<String, String>>>,
    pub(crate) terminal_inputs: Arc<Mutex<HashMap<(String, String), String>>>,
    boot: String,
}

impl StateSubscriptionPresenter {
    pub(crate) fn new(paths: Vec<String>) -> Self {
        let mut state = Subscriptions::new(uuid::Uuid::new_v4().to_string());
        state
            .register(
                REPO_PATHS.into(),
                StateValue::RepositoryPaths(paths),
                Delivery::Full,
            )
            .expect("unique target");
        state.protect(REPO_PATHS);
        Self {
            state: Arc::new(Mutex::new(state)),
            changed: Arc::new(Notify::new()),
            invalidated: tokio::sync::broadcast::channel(64).0,
            terminal_routes: Default::default(),
            terminal_inputs: Default::default(),
            boot: uuid::Uuid::new_v4().to_string(),
        }
    }

    fn update(
        &self,
        update: impl FnOnce(
            &mut Subscriptions<StateValue>,
        )
            -> Result<(), crate::infrastructure::state_subscription::SubscriptionError>,
    ) -> Result<(), SubscriptionError> {
        update(&mut self.state.lock())?;
        self.changed.notify_waiters();
        Ok(())
    }

    pub(crate) fn stream(
        &self,
        usecase: StateSubscriptionUsecase,
        id: String,
    ) -> Result<impl Stream<Item = StateSubscriptionEvent> + Send + use<>, SubscriptionError> {
        usecase.open_client(id.clone())?;
        let permit = StreamPermit { usecase, id };
        let timer = permit.usecase.timer().interval(BOOKMARK_INTERVAL);
        let presenter = self.clone();
        let events = futures_util::stream::unfold(
            (permit, timer, presenter),
            |(permit, mut timer, presenter)| async move {
                loop {
                    let changed = presenter.changed.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    let requests = presenter.state.lock().snapshot_requests(&permit.id);
                    for raw in requests {
                        permit.usecase.schedule_terminal_refresh(raw);
                    }
                    if let Some((target, event)) = presenter.state.lock().next(&permit.id) {
                        return Some((
                            StateSubscriptionEvent::Item(target, event),
                            (permit, timer, presenter.clone()),
                        ));
                    }
                    tokio::select! {
                        _ = changed => {},
                        _ = timer.next() => presenter.state.lock().bookmark(&permit.id),
                    }
                }
            },
        );
        Ok(futures_util::stream::once(async { StateSubscriptionEvent::Ready }).chain(events))
    }
}

#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
pub(crate) fn test_output() -> crate::usecase::state_subscription::StateSubscriptionOutputRef {
    Arc::new(StateSubscriptionPresenter::new(vec![]))
}

struct StreamPermit {
    usecase: StateSubscriptionUsecase,
    id: String,
}

impl Drop for StreamPermit {
    fn drop(&mut self) {
        self.usecase.close_client(&self.id);
    }
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

    fn subscribe_changes(&self) -> tokio::sync::broadcast::Receiver<StateChangeSource> {
        self.invalidated.subscribe()
    }

    fn invalidate(&self, source: StateChangeSource) {
        let _ = self.invalidated.send(source);
    }

    fn publish(
        &self,
        target: &str,
        snapshot: StateValue,
        delta: Option<StateValue>,
    ) -> Result<(), SubscriptionError> {
        self.update(|state| state.publish(target, snapshot, delta))
    }

    fn open(&self, client: String) -> Result<(), SubscriptionError> {
        self.state.lock().open(client).map_err(Into::into)
    }

    fn close(&self, client: &str) {
        self.state.lock().close(client);
    }

    fn start(
        &self,
        client: &str,
        target: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), SubscriptionError> {
        let version = cursor_version(cursor);
        self.update(|state| state.start(client, target, version.as_ref()))
    }

    fn start_with_snapshot(
        &self,
        client: &str,
        target: &str,
        snapshot: StateValue,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), SubscriptionError> {
        let version = cursor_version(cursor);
        self.update(|state| state.start_with_snapshot(client, target, snapshot, version.as_ref()))
    }

    fn stop(&self, client: &str, target: &str) -> Result<(), SubscriptionError> {
        self.update(|state| state.stop(client, target))
    }

    fn active_targets(&self) -> std::collections::HashSet<String> {
        self.state.lock().active_targets()
    }

    fn ensure_active(&self, target: &str) -> Result<(), SubscriptionError> {
        self.state.lock().ensure_active(target).map_err(Into::into)
    }

    fn release_inactive_snapshots(&self) {
        self.state.lock().release_inactive_snapshots();
    }

    fn needs_snapshot(
        &self,
        target: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<bool, SubscriptionError> {
        let version = cursor_version(cursor);
        self.state
            .lock()
            .needs_snapshot(target, version.as_ref())
            .map_err(Into::into)
    }

    fn terminal_pending_amount(&self, client: &str, target: &str) -> usize {
        self.state
            .lock()
            .pending_amount(client, target, |value| match value {
                StateValue::Terminal(TerminalSurfaceStreamItem::Output { data, .. }) => {
                    data.encode_utf16().count()
                }
                _ => 0,
            })
    }

    fn set_terminal_snapshot(
        &self,
        target: &str,
        runtime_generation: u64,
        sequence: u64,
        snapshot: StateValue,
    ) -> Result<(), SubscriptionError> {
        let version = Version {
            epoch: format!("{}:{runtime_generation}", self.boot),
            sequence,
        };
        self.update(|state| state.set_delta_snapshot(target, version, snapshot))
    }

    fn terminal_reset_clients(&self, target: &str) -> Vec<String> {
        let state = self.state.lock();
        self.terminal_inputs
            .lock()
            .keys()
            .filter(|(client, raw)| {
                raw == target
                    && state
                        .snapshot_requests(client)
                        .iter()
                        .any(|request| request == target)
            })
            .map(|(client, _)| client.clone())
            .collect()
    }

    fn terminal_report_units(&self) -> usize {
        OUTPUT_REPORT_UNITS
    }

    fn set_terminal_input(&self, client: &str, target: &str, input_id: &str) {
        self.terminal_inputs
            .lock()
            .insert((client.into(), target.into()), input_id.into());
    }

    fn remove_terminal_input(&self, client: &str, target: &str) -> Option<String> {
        self.terminal_inputs
            .lock()
            .remove(&(client.into(), target.into()))
    }

    fn has_terminal_input(&self, client: &str, target: &str) -> bool {
        self.terminal_inputs
            .lock()
            .contains_key(&(client.into(), target.into()))
    }

    fn terminal_targets(&self, client: &str) -> Vec<String> {
        self.terminal_inputs
            .lock()
            .keys()
            .filter(|(id, _)| id == client)
            .map(|(_, target)| target.clone())
            .collect()
    }

    fn terminal_state_sink(&self) -> Arc<dyn TerminalSurfaceStateSink> {
        Arc::new(self.clone())
    }
}

#[cfg(test)]
impl StateSubscriptionUsecase {
    pub(crate) fn new(
        paths: Vec<String>,
        timer: Arc<dyn crate::usecase::state_subscription::SubscriptionTimer>,
    ) -> Self {
        Self::new_with_output(Arc::new(StateSubscriptionPresenter::new(paths)), timer)
    }

    pub(crate) fn test_presenter(&self) -> Option<&StateSubscriptionPresenter> {
        self.output_ref().as_any().downcast_ref()
    }

    pub(crate) fn open(
        &self,
        id: String,
    ) -> Result<impl Stream<Item = StateSubscriptionEvent> + Send + use<>, SubscriptionError> {
        self.test_presenter()
            .expect("test presenter")
            .stream(self.clone(), id)
    }
}

impl TerminalSurfaceStateSink for StateSubscriptionPresenter {
    fn initialize(&self, surface: &TerminalSurfaceOutputSummary) {
        let target = surface.target.clone();
        self.terminal_routes
            .lock()
            .insert(surface.session_key.clone(), target.clone());
        let version = Version {
            epoch: format!("{}:{}", self.boot, surface.runtime_generation),
            sequence: surface.latest_sequence,
        };
        if let Err(error) =
            self.update(|state| state.register_delta(&target, version, OUTPUT_PENDING_LIMIT))
        {
            log::error!("Terminal registration failed: {error}");
        }
    }

    fn remove(&self, surface: &TerminalSurfaceOutputSummary) -> bool {
        let mut routes = self.terminal_routes.lock();
        let Some(target) = routes.get(&surface.session_key) else {
            return false;
        };
        let mut state = self.state.lock();
        let epoch = format!("{}:{}", self.boot, surface.runtime_generation);
        if state
            .current_version(target)
            .is_none_or(|version| version.epoch != epoch)
        {
            return false;
        }
        if let Err(error) = state.unregister(target) {
            log::error!("Terminal removal failed: {error}");
            return false;
        }
        let mut inputs = self.terminal_inputs.lock();
        inputs.retain(|(client, raw), _| raw != target || state.is_subscribed(client, raw));
        let subscribed = inputs.keys().any(|(_, raw)| raw == target);
        routes.remove(&surface.session_key);
        self.changed.notify_waiters();
        subscribed
    }

    fn publish(&self, event: TerminalSurfaceOutputEvent) {
        let Some(target) = self
            .terminal_routes
            .lock()
            .get(event.session_key())
            .cloned()
        else {
            return;
        };
        let mut state = self.state.lock();
        let Some(mut version) = state.current_version(&target) else {
            return;
        };
        let sequence = match &event {
            TerminalSurfaceOutputEvent::Output { sequence, .. }
            | TerminalSurfaceOutputEvent::Resize { sequence, .. }
            | TerminalSurfaceOutputEvent::Exit { sequence, .. } => *sequence,
        };
        let advances_version = matches!(&event, TerminalSurfaceOutputEvent::Output { .. });
        if sequence < version.sequence && !advances_version {
            if let Err(error) = state.require_delta_snapshot(&target) {
                log::error!("Terminal resynchronization failed: {error}");
            }
            drop(state);
            self.changed.notify_waiters();
            return;
        }
        if advances_version && sequence <= version.sequence {
            return;
        }
        let (item, units) = match event {
            TerminalSurfaceOutputEvent::Output {
                session_key,
                data,
                sequence,
            } => {
                version.sequence = sequence;
                let units = data.encode_utf16().count();
                (
                    TerminalSurfaceStreamItem::Output {
                        session_key,
                        data,
                        sequence,
                    },
                    units,
                )
            }
            TerminalSurfaceOutputEvent::Resize {
                session_key,
                cols,
                rows,
                sequence,
            } => {
                version.sequence = sequence;
                (
                    TerminalSurfaceStreamItem::Resize {
                        session_key,
                        cols,
                        rows,
                        sequence,
                    },
                    0,
                )
            }
            TerminalSurfaceOutputEvent::Exit {
                session_key,
                exit_code,
                sequence,
                ..
            } => {
                version.sequence = sequence;
                (
                    TerminalSurfaceStreamItem::Exit {
                        session_key,
                        exit_code,
                        sequence,
                    },
                    0,
                )
            }
        };
        if let Err(error) = state.publish_delta(
            &target,
            version,
            StateValue::Terminal(item),
            units,
            advances_version,
        ) {
            log::error!("Terminal publication failed: {error}");
        }
        drop(state);
        self.changed.notify_waiters();
    }
}

#[cfg(test)]
#[path = "state_subscription_test.rs"]
mod state_subscription_tests;

#[cfg(test)]
#[path = "state_subscription_flow_test.rs"]
mod state_subscription_flow_tests;

#[cfg(test)]
#[path = "state_subscription_terminal_test.rs"]
mod state_subscription_terminal_tests;
