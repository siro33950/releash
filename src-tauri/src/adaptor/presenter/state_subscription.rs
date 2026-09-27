use std::sync::Arc;

use futures_util::Stream;
#[cfg(test)]
use futures_util::StreamExt;
use parking_lot::Mutex;

use crate::infrastructure::state_subscription::{
    Delivery, StateSubscriptionRuntime, Subscriptions, Version,
};
pub(crate) type StateSubscriptionEvent =
    crate::infrastructure::state_subscription::StateSubscriptionEvent<
        crate::adaptor::presenter::client::StatePayload,
    >;
use crate::infrastructure::terminal::output_flow_control::{
    OUTPUT_PENDING_LIMIT, OUTPUT_REPORT_UNITS,
};
use crate::usecase::state_subscription::{
    StateChangeSource, StateReadError, StateSubscriptionChange, StateSubscriptionOutput,
    StateSubscriptionUsecase, StateValue, SubscriptionError, SubscriptionTarget,
};
use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
use crate::usecase::terminal_surface::output::{
    TerminalSurfaceOutputEvent, TerminalSurfaceOutputSummary, TerminalSurfaceStateSink,
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
    runtime: StateSubscriptionRuntime<crate::adaptor::presenter::client::StatePayload>,
    invalidated: tokio::sync::broadcast::Sender<StateChangeSource>,
    terminal: Arc<
        Mutex<
            Option<Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>>,
        >,
    >,
}

impl StateSubscriptionPresenter {
    pub(crate) fn connect_terminal(
        &self,
        terminal: &Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>,
    ) {
        *self.terminal.lock() = Some(terminal.clone());
        terminal.connect_state(Arc::new(self.clone()));
        for summary in terminal.summaries() {
            self.initialize(&(&summary).into());
        }
    }

    pub(crate) fn change_sender(&self) -> tokio::sync::broadcast::Sender<StateChangeSource> {
        self.invalidated.clone()
    }

    pub(crate) fn new(paths: Vec<String>) -> Self {
        let runtime = StateSubscriptionRuntime::new(uuid::Uuid::new_v4().to_string());
        let target = SubscriptionTarget::RepositoryPaths.to_string();
        runtime
            .state
            .lock()
            .register(
                target.clone(),
                crate::adaptor::presenter::state_subscription_wire::payload(
                    &StateValue::RepositoryPaths(paths),
                )
                .expect("repository paths encode"),
                Delivery::Full,
            )
            .expect("unique target");
        runtime.state.lock().protect(&target);
        Self {
            runtime,
            invalidated: tokio::sync::broadcast::channel(64).0,
            terminal: Default::default(),
        }
    }

    fn update(
        &self,
        update: impl FnOnce(
            &mut Subscriptions<crate::adaptor::presenter::client::StatePayload>,
        )
            -> Result<(), crate::infrastructure::state_subscription::SubscriptionError>,
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
        self.update(|state| {
            let result = state.start(client, target, version.as_ref());
            if result.is_err() {
                let _ = state.ensure_active(target);
            }
            result
        })
    }

    pub(crate) fn stop(
        &self,
        usecase: &StateSubscriptionUsecase,
        client: &str,
        target: &str,
    ) -> Result<(), SubscriptionError> {
        self.update(|state| state.stop(client, target))?;
        let protected: std::collections::HashSet<String> = usecase
            .active_targets()
            .into_iter()
            .map(|target| target.to_string())
            .collect();
        self.runtime
            .state
            .lock()
            .release_inactive_snapshots_except(&protected);
        Ok(())
    }

    pub(crate) fn needs_snapshot(
        &self,
        target: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<bool, SubscriptionError> {
        let version = cursor_version(cursor);
        self.runtime
            .state
            .lock()
            .needs_snapshot(target, version.as_ref())
            .map_err(Into::into)
    }

    pub(crate) fn terminal_pending_amount(&self, client: &str, target: &str) -> usize {
        self.runtime.state.lock().pending_amount(client, target)
    }

    pub(crate) fn terminal_report_units(&self) -> usize {
        OUTPUT_REPORT_UNITS
    }

    pub(crate) fn stream(
        &self,
        usecase: StateSubscriptionUsecase,
        id: String,
    ) -> Result<impl Stream<Item = StateSubscriptionEvent> + Send + use<>, SubscriptionError> {
        usecase.open_client(id.clone())?;
        if let Err(error) = self.runtime.state.lock().open(id.clone()) {
            usecase.close_client(&id);
            return Err(error.into());
        }
        let permit = StreamPermit {
            usecase,
            id,
            runtime: self.runtime.clone(),
        };
        let refresh = permit.usecase.clone();
        Ok(self
            .runtime
            .stream(permit.id.clone(), permit, move |raw, clients| {
                if let Ok(target) = SubscriptionTarget::parse(&raw) {
                    refresh.schedule_terminal_refresh(clients, target);
                }
            }))
    }
}

impl StateSubscriptionPresenter {
    pub(crate) async fn present_start(
        &self,
        usecase: &StateSubscriptionUsecase,
        change: &StateSubscriptionChange,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), StateReadError> {
        let raw = change.target.to_string();
        let result = async {
            if let Some(input_id) = &change.terminal_input_id {
                if self
                    .needs_snapshot(&raw, cursor)
                    .map_err(StateReadError::from_error)?
                {
                    usecase.refresh_terminal(&change.target).await?;
                }
                self.present_terminal_start(usecase, change, &raw, input_id, cursor)
            } else {
                self.start(&change.client, &raw, cursor)
                    .map_err(StateReadError::from_error)
            }
        }
        .await;
        if result.is_err() {
            if let Err(cleanup) = usecase.stop(&change.client, &change.target) {
                log::error!("State subscription rollback failed: {cleanup}");
            }
        }
        result
    }

    fn present_terminal_start(
        &self,
        usecase: &StateSubscriptionUsecase,
        change: &StateSubscriptionChange,
        raw: &str,
        input_id: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), StateReadError> {
        let SubscriptionTarget::Terminal(owner) = &change.target else {
            return Err(terminal_read_error("Not a terminal target"));
        };
        let terminal = self
            .terminal
            .lock()
            .clone()
            .ok_or_else(|| terminal_read_error("Terminal unavailable"))?;
        let summary = terminal.get_summary(owner).map_err(terminal_read_error)?;
        let mut result = Ok(());
        let mut started = false;
        terminal.with_output_order(summary.runtime_generation.value(), &mut || {
            result = self
                .start(&change.client, raw, cursor)
                .map_err(StateReadError::from_error);
            if result.is_ok() {
                started = true;
                let pending = self.terminal_pending_amount(&change.client, raw);
                terminal.subscribe_output(owner, &change.client, input_id, pending);
                result = usecase.attach_terminal(&change.client, &change.target, input_id);
                if result.is_err() {
                    terminal.unsubscribe_output(owner, &change.client, input_id);
                }
            }
        });
        if started && result.is_err() {
            if let Err(cleanup) = self.stop(usecase, &change.client, raw) {
                log::error!("Terminal delivery rollback failed: {cleanup}");
            }
        }
        result
    }

    pub(crate) fn present_stop(
        &self,
        usecase: &StateSubscriptionUsecase,
        change: &StateSubscriptionChange,
    ) -> Result<(), SubscriptionError> {
        self.stop(usecase, &change.client, &change.target.to_string())
    }
}

fn terminal_read_error(error: impl std::fmt::Display) -> StateReadError {
    StateReadError::from_error(crate::domain::failure::TechnicalFailure {
        nature: crate::domain::failure::TechnicalFailureNature::Other,
        message: error.to_string(),
    })
}

#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
pub(crate) fn test_output() -> crate::usecase::state_subscription::StateSubscriptionOutputRef {
    Arc::new(StateSubscriptionPresenter::new(vec![]))
}

struct StreamPermit {
    usecase: StateSubscriptionUsecase,
    id: String,
    runtime: StateSubscriptionRuntime<crate::adaptor::presenter::client::StatePayload>,
}

impl Drop for StreamPermit {
    fn drop(&mut self) {
        self.usecase.close_client(&self.id);
        let mut state = self.runtime.state.lock();
        let targets = state.active_targets();
        state.close(&self.id);
        let protected: std::collections::HashSet<String> = self
            .usecase
            .active_targets()
            .into_iter()
            .map(|target| target.to_string())
            .collect();
        for target in targets {
            if !protected.contains(&target) {
                let _ = state.ensure_active(&target);
            }
        }
        state.release_inactive_snapshots_except(&protected);
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

    fn invalidate(&self, source: StateChangeSource) {
        let _ = self.invalidated.send(source);
    }

    fn publish_initial(
        &self,
        target: &SubscriptionTarget,
        snapshot: StateValue,
    ) -> Result<(), SubscriptionError> {
        let target = target.to_string();
        let snapshot = crate::adaptor::presenter::state_subscription_wire::payload(&snapshot)
            .map_err(|_| SubscriptionError::EncodingFailed)?;
        self.update(|state| {
            if state.registered(&target) {
                state.publish(&target, snapshot, None)
            } else {
                state.register(target, snapshot, Delivery::Full)
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
            .map_err(|_| SubscriptionError::EncodingFailed)?;
        let delta = delta
            .as_ref()
            .map(crate::adaptor::presenter::state_subscription_wire::payload)
            .transpose()
            .map_err(|_| SubscriptionError::EncodingFailed)?;
        self.update(|state| state.publish(&target, snapshot, delta))
    }

    fn set_terminal_snapshot(
        &self,
        target: &SubscriptionTarget,
        runtime_generation: u64,
        sequence: u64,
        snapshot: StateValue,
    ) -> Result<(), SubscriptionError> {
        let version = self.runtime.terminal_version(runtime_generation, sequence);
        let snapshot = crate::adaptor::presenter::state_subscription_wire::payload(&snapshot)
            .map_err(|_| SubscriptionError::EncodingFailed)?;
        self.update(|state| state.set_delta_snapshot(&target.to_string(), version, snapshot))
    }
}

impl TerminalSurfaceStateSink for StateSubscriptionPresenter {
    fn initialize(&self, surface: &TerminalSurfaceOutputSummary) {
        let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
        self.runtime
            .terminal_routes
            .lock()
            .insert(surface.session_key.clone(), target.clone());
        let version = self
            .runtime
            .terminal_version(surface.runtime_generation, surface.latest_sequence);
        if let Err(error) =
            self.update(|state| state.register_delta(&target, version, OUTPUT_PENDING_LIMIT))
        {
            log::error!("Terminal registration failed: {error}");
        }
    }

    fn remove(&self, surface: &TerminalSurfaceOutputSummary) -> bool {
        let mut routes = self.runtime.terminal_routes.lock();
        let Some(target) = routes.get(&surface.session_key) else {
            return false;
        };
        let mut state = self.runtime.state.lock();
        let epoch = self
            .runtime
            .terminal_version(surface.runtime_generation, 0)
            .epoch;
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
        let subscribed = state.has_subscribers(target);
        routes.remove(&surface.session_key);
        self.runtime.notify();
        subscribed
    }

    fn publish(&self, event: TerminalSurfaceOutputEvent) {
        let Some(target) = self
            .runtime
            .terminal_routes
            .lock()
            .get(event.session_key())
            .cloned()
        else {
            return;
        };
        let mut state = self.runtime.state.lock();
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
            self.runtime.notify();
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
        let payload = match crate::adaptor::presenter::state_subscription_wire::payload(
            &StateValue::Terminal(item),
        ) {
            Ok(payload) => payload,
            Err(error) => {
                log::error!("Terminal publication encoding failed: {error}");
                return;
            }
        };
        if let Err(error) = state.publish_delta(&target, version, payload, units, advances_version)
        {
            log::error!("Terminal publication failed: {error}");
        }
        drop(state);
        self.runtime.notify();
    }
}

#[cfg(test)]
#[path = "state_subscription_test.rs"]
mod state_subscription_tests;

#[cfg(test)]
#[path = "../../../tests/state_subscription/flow_test.rs"]
mod state_subscription_flow_tests;

#[cfg(test)]
#[path = "../../../tests/state_subscription/terminal_test.rs"]
mod state_subscription_terminal_tests;
