use std::sync::Arc;

use parking_lot::Mutex;

use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventCancellation, TerminalSurfaceEventReceiveError,
    TerminalSurfaceEventSink, TerminalSurfaceEventSource, TerminalSurfaceEventStream,
    TerminalSurfaceEventSubscription,
};

const TERMINAL_SURFACE_STREAM_CAPACITY: usize = 256;

pub(crate) struct TerminalSurfaceEventHub {
    sender: tokio::sync::broadcast::Sender<TerminalSurfaceEvent>,
}

impl TerminalSurfaceEventHub {
    pub(crate) fn new() -> Self {
        Self::with_capacity(TERMINAL_SURFACE_STREAM_CAPACITY)
    }

    pub(crate) fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(capacity);
        Self { sender }
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

impl TerminalSurfaceEventCancellation for EventCancellation {
    fn cancel(&self) {
        if let Some(sender) = self.sender.lock().take() {
            let _ = sender.send(());
        }
    }
}

impl TerminalSurfaceEventSource for TerminalSurfaceEventHub {
    fn subscribe(&self) -> TerminalSurfaceEventStream {
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let cancellation = Arc::new(EventCancellation {
            sender: Mutex::new(Some(cancel)),
        });
        TerminalSurfaceEventStream {
            subscription: Box::new(EventSubscription {
                receiver: self.sender.subscribe(),
                cancelled,
                _cancellation: Arc::clone(&cancellation),
            }),
            cancellation,
        }
    }
}

impl TerminalSurfaceEventSink for TerminalSurfaceEventHub {
    fn publish(&self, event: TerminalSurfaceEvent) {
        let _ = self.sender.send(event);
    }
}
