use std::sync::Arc;

use parking_lot::Mutex;

use crate::domain::terminal_surface::gateway::TerminalSurfaceEvent;
use crate::usecase::terminal_surface::output::{
    TerminalSurfaceEventSink, TerminalSurfaceOutputControl, TerminalSurfaceOutputEvent,
    TerminalSurfaceOutputSummary,
};

use crate::infrastructure::terminal::output_flow_control::TerminalOutputFlow;

const TERMINAL_SURFACE_STREAM_CAPACITY: usize = 256;

pub(crate) struct TerminalSurfaceEventHub {
    sender: tokio::sync::broadcast::Sender<TerminalSurfaceEvent>,
    state_sink:
        Mutex<Option<Arc<dyn crate::usecase::terminal_surface::output::TerminalSurfaceStateSink>>>,
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
        *self.state_sink.lock() = Some(sink);
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
    fn initialize(&self, surface: &TerminalSurfaceOutputSummary) {
        self.output
            .reset(&surface.session_key, surface.latest_sequence);
        if let Some(sink) = self.state_sink.lock().clone() {
            sink.initialize(surface);
        }
    }
    fn remove(&self, surface: &TerminalSurfaceOutputSummary) -> bool {
        let subscribed = self
            .state_sink
            .lock()
            .as_ref()
            .is_some_and(|sink| sink.remove(surface));
        self.release_output(&surface.session_key);
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
