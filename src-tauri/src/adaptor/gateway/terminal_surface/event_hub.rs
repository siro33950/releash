use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventCancellation, TerminalSurfaceEventReceiveError,
    TerminalSurfaceEventSink, TerminalSurfaceEventSource, TerminalSurfaceEventStream,
    TerminalSurfaceEventSubscription,
};

use super::output_flow_control::TerminalOutputFlowControl;

const TERMINAL_SURFACE_STREAM_CAPACITY: usize = 256;

pub(crate) struct TerminalSurfaceEventHub {
    sender: tokio::sync::broadcast::Sender<TerminalSurfaceEvent>,
    owner_streams: Arc<Mutex<HashMap<String, OwnerEventStream>>>,
    capacity: usize,
    flow_control_enabled: bool,
}

struct OwnerEventStream {
    sender: tokio::sync::broadcast::Sender<TerminalSurfaceEvent>,
    flow_control: Arc<TerminalOutputFlowControl>,
    active_subscription: Arc<()>,
}

impl TerminalSurfaceEventHub {
    pub(crate) fn new() -> Self {
        let switches = crate::other::performance_switches::terminal_performance_switches();
        Self::with_flags(
            TERMINAL_SURFACE_STREAM_CAPACITY,
            !switches.disable_output_flow_control,
        )
    }

    #[cfg(test)]
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self::with_flags(capacity, true)
    }

    pub(crate) fn with_flags(capacity: usize, flow_control_enabled: bool) -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(capacity);
        Self {
            sender,
            owner_streams: Arc::new(Mutex::new(HashMap::new())),
            capacity,
            flow_control_enabled,
        }
    }

    fn stream(
        receiver: tokio::sync::broadcast::Receiver<TerminalSurfaceEvent>,
        on_cancel: Option<Box<dyn FnOnce() + Send>>,
    ) -> TerminalSurfaceEventStream {
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let cancellation = Arc::new(EventCancellation {
            sender: Mutex::new(Some(cancel)),
            on_cancel: Mutex::new(on_cancel),
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

    #[cfg(test)]
    pub(crate) fn owner_stream_count(&self) -> usize {
        self.owner_streams.lock().len()
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
    on_cancel: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl EventCancellation {
    fn finish(&self) {
        if let Some(sender) = self.sender.lock().take() {
            let _ = sender.send(());
        }
        if let Some(on_cancel) = self.on_cancel.lock().take() {
            on_cancel();
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
    fn subscribe(&self) -> TerminalSurfaceEventStream {
        Self::stream(self.sender.subscribe(), None)
    }

    fn subscribe_owner(
        &self,
        session_key: &str,
        attachment_id: &str,
    ) -> TerminalSurfaceEventStream {
        let subscription_identity = Arc::new(());
        let (receiver, flow_control) = {
            let mut owner_streams = self.owner_streams.lock();
            let stream = owner_streams
                .entry(session_key.to_string())
                .or_insert_with(|| OwnerEventStream {
                    sender: tokio::sync::broadcast::channel(self.capacity).0,
                    flow_control: Arc::new(TerminalOutputFlowControl::new(
                        self.flow_control_enabled,
                    )),
                    active_subscription: Arc::new(()),
                });
            stream.active_subscription = Arc::clone(&subscription_identity);
            stream.flow_control.activate(attachment_id);
            (stream.sender.subscribe(), Arc::clone(&stream.flow_control))
        };
        let attachment_id = attachment_id.to_string();
        let session_key = session_key.to_string();
        let owner_streams = Arc::clone(&self.owner_streams);
        Self::stream(
            receiver,
            Some(Box::new(move || {
                let mut streams = owner_streams.lock();
                let is_current = streams.get(&session_key).is_some_and(|stream| {
                    Arc::ptr_eq(&stream.flow_control, &flow_control)
                        && Arc::ptr_eq(&stream.active_subscription, &subscription_identity)
                });
                if !is_current || !flow_control.deactivate(&attachment_id) {
                    return;
                }
                streams.remove(&session_key);
            })),
        )
    }

    fn acknowledge_owner_output(&self, session_key: &str, attachment_id: &str, sequence: u64) {
        let flow_control = self
            .owner_streams
            .lock()
            .get(session_key)
            .map(|stream| Arc::clone(&stream.flow_control));
        if let Some(flow_control) = flow_control {
            flow_control.acknowledge(attachment_id, sequence);
        }
    }
}

#[cfg(test)]
#[path = "event_hub_test.rs"]
mod event_hub_tests;

impl TerminalSurfaceEventSink for TerminalSurfaceEventHub {
    fn publish(&self, event: TerminalSurfaceEvent) {
        let owner_stream = self
            .owner_streams
            .lock()
            .get(event.session_key())
            .map(|stream| (stream.sender.clone(), Arc::clone(&stream.flow_control)));
        if let Some((owner_sender, flow_control)) = owner_stream {
            if let TerminalSurfaceEvent::Output { data, sequence, .. } = &event {
                flow_control.reserve(*sequence, data.encode_utf16().count());
            }
            if matches!(event, TerminalSurfaceEvent::Exit { .. }) {
                let _ = owner_sender.send(event.clone());
                let _ = self.sender.send(event);
            } else {
                let _ = owner_sender.send(event);
            }
        } else if matches!(event, TerminalSurfaceEvent::Exit { .. }) {
            let _ = self.sender.send(event);
        }
    }
}
