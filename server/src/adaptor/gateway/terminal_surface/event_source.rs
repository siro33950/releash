use std::sync::Arc;

use parking_lot::Mutex;

use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventCancellation, TerminalSurfaceEventReceiveError,
    TerminalSurfaceEventSource, TerminalSurfaceEventStream, TerminalSurfaceEventSubscription,
};

pub struct TerminalSurfaceEventSourceGateway {
    sender: tokio::sync::broadcast::Sender<TerminalSurfaceEvent>,
}

impl TerminalSurfaceEventSourceGateway {
    pub fn new(sender: tokio::sync::broadcast::Sender<TerminalSurfaceEvent>) -> Self {
        Self { sender }
    }
}

impl TerminalSurfaceEventSource for TerminalSurfaceEventSourceGateway {
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

#[cfg(test)]
#[path = "event_source_test.rs"]
mod event_source_tests;
