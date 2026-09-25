use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventCancellation, TerminalSurfaceEventReceiveError,
    TerminalSurfaceEventSink, TerminalSurfaceEventSource, TerminalSurfaceEventStream,
    TerminalSurfaceEventSubscription,
};

use super::output_flow_control::OutputPause;

const TERMINAL_SURFACE_STREAM_CAPACITY: usize = 256;

pub(crate) struct TerminalSurfaceEventHub {
    sender: tokio::sync::broadcast::Sender<TerminalSurfaceEvent>,
    flow_control_enabled: bool,
    state_sink: Mutex<Option<Arc<dyn crate::domain::terminal_surface::gateway::TerminalSurfaceStateSink>>>,
    output: Mutex<HashMap<String, (crate::domain::terminal_surface::value_objects::output_flow_control::OutputFlowControl, Arc<OutputPause>)>>,
}

impl TerminalSurfaceEventHub {
    pub(crate) fn new() -> Self {
        let switches = crate::other::performance_switches::terminal_performance_switches();
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

    fn stream(
        receiver: tokio::sync::broadcast::Receiver<TerminalSurfaceEvent>,
    ) -> TerminalSurfaceEventStream {
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let cancellation = Arc::new(EventCancellation {
            sender: Mutex::new(Some(cancel)),
        });
        TerminalSurfaceEventStream {
            subscription: Box::new(EventSubscription {
                receiver,
                cancelled,
                _cancellation: Arc::clone(&cancellation),
            }),
            cancellation,
        }
    }
}

struct EventSubscription {
    receiver: tokio::sync::broadcast::Receiver<TerminalSurfaceEvent>,
    cancelled: tokio::sync::oneshot::Receiver<()>,
    _cancellation: Arc<EventCancellation>,
}

impl TerminalSurfaceEventSubscription for EventSubscription {
    fn recv(
        &mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<TerminalSurfaceEvent, TerminalSurfaceEventReceiveError>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            tokio::select! {
                biased;
                _ = &mut self.cancelled => Err(TerminalSurfaceEventReceiveError::Closed),
                event = self.receiver.recv() => event.map_err(|error| match error {
                    tokio::sync::broadcast::error::RecvError::Lagged(count) => {
                        TerminalSurfaceEventReceiveError::Lagged(count)
                    }
                    tokio::sync::broadcast::error::RecvError::Closed => {
                        TerminalSurfaceEventReceiveError::Closed
                    }
                }),
            }
        })
    }
}

struct EventCancellation {
    sender: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl EventCancellation {
    fn finish(&self) {
        if let Some(sender) = self.sender.lock().take() {
            let _ = sender.send(());
        }
    }
}

impl TerminalSurfaceEventCancellation for EventCancellation {
    fn cancel(&self) {
        self.finish();
    }
}

impl Drop for EventCancellation {
    fn drop(&mut self) {
        self.finish();
    }
}

impl TerminalSurfaceEventSource for TerminalSurfaceEventHub {
    fn set_state_sink(
        &self,
        sink: Arc<dyn crate::domain::terminal_surface::gateway::TerminalSurfaceStateSink>,
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

    fn subscribe(&self) -> TerminalSurfaceEventStream {
        Self::stream(self.sender.subscribe())
    }
}

#[cfg(test)]
#[path = "event_hub_test.rs"]
mod event_hub_tests;

impl TerminalSurfaceEventSink for TerminalSurfaceEventHub {
    fn initialize(
        &self,
        surface: &crate::domain::terminal_surface::entities::TerminalSurfaceSummary,
    ) {
        if let Some((flow, pause)) = self.output.lock().get_mut(&surface.session_key) {
            flow.reset(surface.latest_sequence);
            pause.set(false);
        }
        if let Some(sink) = self.state_sink.lock().clone() {
            sink.initialize(surface);
        }
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
    fn publish(&self, event: TerminalSurfaceEvent) {
        if self.flow_control_enabled {
            if let TerminalSurfaceEvent::Output { data, sequence, .. } = &event {
                if let Some((flow, pause)) = self.output.lock().get_mut(event.session_key()) {
                    pause.set(flow.output(*sequence, data.encode_utf16().count()));
                }
            }
        }
        if let Some(sink) = self.state_sink.lock().clone() {
            sink.publish(event.clone());
        }

        if matches!(event, TerminalSurfaceEvent::Exit { .. }) {
            let _ = self.sender.send(event);
        }
    }
}
