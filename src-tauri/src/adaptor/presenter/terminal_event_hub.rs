use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::domain::terminal_surface::gateway::TerminalSurfaceEvent;
use crate::usecase::terminal_surface::output::{
    TerminalSurfaceEventSink, TerminalSurfaceOutputControl, TerminalSurfaceOutputEvent,
    TerminalSurfaceOutputSummary,
};

use crate::infrastructure::terminal::output_pause::OutputPause;

const TERMINAL_SURFACE_STREAM_CAPACITY: usize = 256;

pub(crate) struct TerminalSurfaceEventHub {
    sender: tokio::sync::broadcast::Sender<TerminalSurfaceEvent>,
    flow_control_enabled: bool,
    state_sink:
        Mutex<Option<Arc<dyn crate::usecase::terminal_surface::output::TerminalSurfaceStateSink>>>,
    output: Mutex<
        HashMap<
            String,
            (
                crate::infrastructure::terminal::output_flow_control::OutputFlowControl,
                Arc<OutputPause>,
            ),
        >,
    >,
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
            flow_control_enabled,
            state_sink: Mutex::new(None),
            output: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn event_sender(&self) -> tokio::sync::broadcast::Sender<TerminalSurfaceEvent> {
        self.sender.clone()
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
        if !self.flow_control_enabled {
            return;
        }
        let mut output = self.output.lock();
        let (flow, pause) = output.entry(session_key.into()).or_default();
        pause.set(flow.subscribe(client, units));
    }
    fn unsubscribe_output(&self, session_key: &str, client: &str) {
        if let Some((flow, pause)) = self.output.lock().get_mut(session_key) {
            pause.set(flow.unsubscribe(client));
        }
    }
    fn processed_output(&self, session_key: &str, client: &str, units: usize) {
        if let Some((flow, pause)) = self.output.lock().get_mut(session_key) {
            pause.set(flow.processed(client, units));
        }
    }
}

#[cfg(test)]
#[path = "terminal_event_hub_test.rs"]
mod terminal_event_hub_tests;

impl TerminalSurfaceEventSink for TerminalSurfaceEventHub {
    fn initialize(&self, surface: &TerminalSurfaceOutputSummary) {
        if let Some((flow, pause)) = self.output.lock().get_mut(&surface.session_key) {
            flow.reset(surface.latest_sequence);
            pause.set(false);
        }
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
        if let Some((_, pause)) = self.output.lock().remove(session_key) {
            pause.set(false);
        }
    }
    fn wait_output(&self, session_key: &str) {
        let pause = self
            .output
            .lock()
            .get(session_key)
            .map(|(_, pause)| pause.clone());
        if let Some(pause) = pause {
            pause.wait();
        }
    }
    fn publish(&self, event: TerminalSurfaceOutputEvent) {
        if self.flow_control_enabled {
            if let TerminalSurfaceOutputEvent::Output { data, sequence, .. } = &event {
                if let Some((flow, pause)) = self.output.lock().get_mut(event.session_key()) {
                    pause.set(flow.output(*sequence, data.encode_utf16().count()));
                }
            }
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
