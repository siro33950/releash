use tauri::Emitter;

use crate::adaptor::protocol::agent_session_notice::AgentSessionNoticeSnapshotMessage;
use crate::usecase::agent_session::notice::{
    AgentSessionNoticePublisher, AgentSessionNoticeSnapshot,
};

pub(crate) struct TauriAgentSessionNoticePublisher<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> TauriAgentSessionNoticePublisher<R> {
    pub(crate) fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: tauri::Runtime> AgentSessionNoticePublisher for TauriAgentSessionNoticePublisher<R> {
    fn publish(&self, snapshot: AgentSessionNoticeSnapshot) {
        let payload = AgentSessionNoticeSnapshotMessage::from(snapshot);
        if let Err(error) = self.app.emit("agent-session-notice-changed", payload) {
            log::warn!("failed to emit agent session notice snapshot: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::notice::{
        AgentSessionNoticeOperation, AgentSessionNoticeUpdate, AgentSessionNoticeUsecase,
    };
    use std::sync::{Arc, Mutex};
    use tauri::Listener;

    #[test]
    fn publisher_emits_failure_and_clear_snapshots_on_the_notice_event() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("tauri mock app must build");
        let received = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let received_for_listener = received.clone();
        app.listen("agent-session-notice-changed", move |event| {
            received_for_listener
                .lock()
                .unwrap()
                .push(serde_json::from_str(event.payload()).unwrap());
        });
        let usecase = AgentSessionNoticeUsecase::default();
        usecase.register_publisher(Arc::new(TauriAgentSessionNoticePublisher::new(
            app.handle().clone(),
        )));

        usecase.update(
            "session-a",
            AgentSessionNoticeUpdate::Failure {
                operation: AgentSessionNoticeOperation::Send,
                message: "send failed".to_string(),
            },
        );
        usecase.update(
            "session-a",
            AgentSessionNoticeUpdate::Success {
                operation: AgentSessionNoticeOperation::Send,
            },
        );

        assert_eq!(
            received.lock().unwrap().as_slice(),
            [
                serde_json::json!({
                    "sessionId": "session-a",
                    "revision": 1,
                    "notice": { "message": "send failed" },
                }),
                serde_json::json!({
                    "sessionId": "session-a",
                    "revision": 2,
                    "notice": null,
                }),
            ]
        );
    }
}
