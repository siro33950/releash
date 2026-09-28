use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::domain::terminal_surface::gateway::TerminalSurfaceEvent;
use crate::usecase::terminal_surface::output::{
    TerminalSurfaceEventSink, TerminalSurfaceOutputControl, TerminalSurfaceOutputEvent,
};

use crate::infrastructure::terminal::output_flow_control::TerminalOutputFlow;

const TERMINAL_SURFACE_STREAM_CAPACITY: usize = 256;

struct TerminalRegistration {
    session_key: String,
    workspace_path: String,
    session_id: Option<String>,
    latest_sequence: u64,
}

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
}

impl TerminalSurfaceOutputControl for TerminalSurfaceEventHub {
    fn set_state_sink(
        &self,
        sink: Arc<dyn crate::usecase::terminal_surface::output::TerminalSurfaceStateSink>,
    ) {
        let registrations = self.registrations.lock();
        *self.state_sink.lock() = Some(sink.clone());
        for (&runtime_generation, registration) in registrations.iter() {
            sink.initialize(
                &registration.session_key,
                &registration.workspace_path,
                registration.session_id.as_deref(),
                runtime_generation,
                registration.latest_sequence,
            );
        }
    }
    fn initialize(
        &self,
        session_key: &str,
        workspace_path: &str,
        session_id: Option<&str>,
        runtime_generation: u64,
        latest_sequence: u64,
    ) {
        let mut registrations = self.registrations.lock();
        registrations.insert(
            runtime_generation,
            TerminalRegistration {
                session_key: session_key.into(),
                workspace_path: workspace_path.into(),
                session_id: session_id.map(str::to_owned),
                latest_sequence,
            },
        );
        self.output.reset(session_key, latest_sequence);
        if let Some(sink) = self.state_sink.lock().clone() {
            sink.initialize(
                session_key,
                workspace_path,
                session_id,
                runtime_generation,
                latest_sequence,
            );
        }
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
        let Some(registration) = self.registrations.lock().remove(&runtime_generation) else {
            return false;
        };
        let subscribed = self
            .state_sink
            .lock()
            .as_ref()
            .is_some_and(|sink| sink.remove(&registration.session_key, runtime_generation));
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
        if let Some(sink) = self.state_sink.lock().clone() {
            sink.publish(event.clone());
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
