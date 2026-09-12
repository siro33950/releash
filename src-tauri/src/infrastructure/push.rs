use std::sync::Arc;

use serde::Serialize;
use tauri::Emitter;
use tokio::sync::broadcast;

#[derive(Serialize)]
struct PushEnvelope<'a, P> {
    status: &'static str,
    event: &'a str,
    payload: P,
}

pub(crate) struct PushSink {
    sender: broadcast::Sender<Arc<str>>,
}

impl PushSink {
    pub(crate) fn new() -> Self {
        Self {
            sender: broadcast::channel(64).0,
        }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Arc<str>> {
        self.sender.subscribe()
    }

    pub(crate) fn emit<R: tauri::Runtime, P: Serialize + Clone>(
        &self,
        app: &tauri::AppHandle<R>,
        event: &str,
        payload: P,
    ) {
        if let Err(error) = app.emit(event, &payload) {
            log::error!("Tauri push failed for {event}: {error}");
        }
        match serde_json::to_string(&PushEnvelope {
            status: "push",
            event,
            payload,
        }) {
            Ok(frame) => {
                let _ = self.sender.send(Arc::from(frame));
            }
            Err(error) => log::error!("WebSocket push serialization failed for {event}: {error}"),
        }
    }
}
