use std::collections::HashMap;
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
    registrations: Mutex<HashMap<u64, TerminalRegistration>>,
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
        let mut latest: HashMap<&str, &TerminalRegistration> = HashMap::new();
        for registration in registrations.values() {
            let current = latest
                .entry(&registration.session_key)
                .or_insert(registration);
            if current.runtime_generation < registration.runtime_generation {
                *current = registration;
            }
        }
        for registration in latest.values() {
            sink.initialize(registration)?;
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
        registrations.insert(registration.runtime_generation, registration);
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
        let Some(registration) = registrations.remove(&runtime_generation) else {
            return false;
        };
        let subscribed = self.state_sink.lock().as_ref().is_some_and(|sink| {
            !registrations.values().any(|other| {
                other.session_key == registration.session_key
                    && other.runtime_generation > runtime_generation
            }) && sink.remove(&registration)
        });
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
        {
            let registrations = self.registrations.lock();
            if let Some(registration) = registrations
                .values()
                .filter(|registration| registration.session_key == event.session_key())
                .max_by_key(|registration| registration.runtime_generation)
            {
                if let Some(sink) = self.state_sink.lock().as_ref() {
                    sink.publish(registration, event.clone());
                }
            }
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
