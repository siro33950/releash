use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::domain::terminal_surface::gateway::TerminalSurfaceEvent;
use crate::usecase::terminal_surface::error::UsecaseError;
use crate::usecase::terminal_surface::output::{
    TerminalRegistration, TerminalSurfaceEventSink, TerminalSurfaceOutputControl,
    TerminalSurfaceOutputEvent,
};

use crate::infrastructure::terminal::output_flow_control::TerminalOutputFlow;

const TERMINAL_SURFACE_STREAM_CAPACITY: usize = 256;

pub(crate) struct TerminalSurfaceEventHub {
    sender: tokio::sync::broadcast::Sender<TerminalSurfaceEvent>,
    state_sink:
        Mutex<Option<Arc<dyn crate::usecase::terminal_surface::output::TerminalSurfaceStateSink>>>,
    registrations: Mutex<HashMap<String, BTreeMap<u64, Arc<TerminalRegistration>>>>,
    output: TerminalOutputFlow,
}

impl TerminalSurfaceEventHub {
    pub(crate) fn new() -> Self {
        let switches = crate::infrastructure::performance_switches::terminal_performance_switches();
        Self::with_flags(
            TERMINAL_SURFACE_STREAM_CAPACITY,
            !switches.disable_output_flow_control,
        )
    }

    pub(crate) fn with_flags(capacity: usize, flow_control_enabled: bool) -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(capacity);
        Self {
            sender,
            state_sink: Mutex::new(None),
            registrations: Mutex::new(HashMap::new()),
            output: TerminalOutputFlow::new(flow_control_enabled),
        }
    }

    pub(crate) fn event_sender(&self) -> tokio::sync::broadcast::Sender<TerminalSurfaceEvent> {
        self.sender.clone()
    }

    #[cfg(test)]
    pub(crate) fn test_subscribed(&self, session: &str, client: &str) -> bool {
        self.output.test_subscribed(session, client)
    }

    #[cfg(test)]
    pub(crate) fn test_pending_amount(&self, session: &str, client: &str) -> Option<usize> {
        self.output.test_pending_amount(session, client)
    }
}

impl TerminalSurfaceOutputControl for TerminalSurfaceEventHub {
    fn set_state_sink(
        &self,
        sink: Arc<dyn crate::usecase::terminal_surface::output::TerminalSurfaceStateSink>,
    ) -> Result<(), UsecaseError> {
        let registrations = self.registrations.lock();
        for generations in registrations.values() {
            if let Some((_, registration)) = generations.last_key_value() {
                sink.initialize(registration)?;
            }
        }
        *self.state_sink.lock() = Some(sink);
        Ok(())
    }
    fn initialize(&self, registration: TerminalRegistration) -> Result<(), UsecaseError> {
        let mut registrations = self.registrations.lock();
        if let Some(sink) = self.state_sink.lock().clone() {
            sink.initialize(&registration)?;
        }
        self.output
            .reset(&registration.session_key, registration.latest_sequence);
        registrations
            .entry(registration.session_key.clone())
            .or_default()
            .insert(registration.runtime_generation, Arc::new(registration));
        Ok(())
    }
    fn subscribe_output(&self, session_key: &str, client: &str, units: usize) {
        self.output.subscribe(session_key, client, units);
    }
    fn unsubscribe_output(&self, session_key: &str, client: &str) {
        self.output.unsubscribe(session_key, client);
    }
    fn processed_output(&self, session_key: &str, client: &str, units: usize) {
        self.output.processed(session_key, client, units);
    }
}

#[cfg(test)]
#[path = "terminal_event_hub_test.rs"]
mod terminal_event_hub_tests;

impl TerminalSurfaceEventSink for TerminalSurfaceEventHub {
    fn remove(&self, runtime_generation: u64) -> bool {
        let mut registrations = self.registrations.lock();
        let Some(session) = registrations.iter().find_map(|(session, generations)| {
            generations
                .contains_key(&runtime_generation)
                .then(|| session.clone())
        }) else {
            return false;
        };
        let generations = registrations.get_mut(&session).unwrap();
        let registration = generations.remove(&runtime_generation).unwrap();
        let newer = generations
            .last_key_value()
            .is_some_and(|(generation, _)| *generation > runtime_generation);
        if generations.is_empty() {
            registrations.remove(&session);
        }
        let subscribed = self
            .state_sink
            .lock()
            .as_ref()
            .is_some_and(|sink| !newer && sink.remove(&registration));
        self.release_output(&registration.session_key);
        subscribed
    }
    fn release_output(&self, session_key: &str) {
        self.output.release(session_key);
    }
    fn wait_output(&self, session_key: &str) {
        self.output.wait(session_key);
    }
    fn publish(&self, event: TerminalSurfaceOutputEvent) {
        if let TerminalSurfaceOutputEvent::Output { data, sequence, .. } = &event {
            self.output
                .output(event.session_key(), *sequence, data.encode_utf16().count());
        }
        let registration = self
            .registrations
            .lock()
            .get(event.session_key())
            .and_then(|generations| generations.last_key_value())
            .map(|(_, registration)| registration.clone());
        let sink = self.state_sink.lock().clone();
        if let (Some(registration), Some(sink)) = (registration, sink) {
            sink.publish(&registration, event.clone());
        }

        if let TerminalSurfaceOutputEvent::Exit {
            session_key,
            runtime_generation,
            exit_code,
            sequence,
        } = event
        {
            let _ = self.sender.send(TerminalSurfaceEvent::Exit {
                session_key,
                runtime_generation,
                exit_code,
                sequence,
            });
        }
    }
}
