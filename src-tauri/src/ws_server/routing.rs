use crate::adaptor::protocol::*;

use super::WsServerState;

pub(super) async fn route_message(msg: &WsMessage, state: &WsServerState) -> Option<WsMessage> {
    if let WsMessage::ResyncStream(req) = msg {
        return match crate::usecase::agent_session::session::resync_streaming_message(
            state.stream_resync_read_model.as_ref(),
            &req.session_id,
            &req.message_id,
            req.since_seq,
        )
        .await
        {
            Ok(Some(snapshot)) => {
                state.broadcaster.send_stream_snapshot(snapshot.into());
                None
            }
            Ok(None) => Some(WsMessage::Error(ErrorMsg {
                code: "STREAM_NOT_FOUND".to_string(),
                message: "Streaming message not found".to_string(),
            })),
            Err(err) => Some(WsMessage::Error(ErrorMsg {
                code: "STREAM_RESYNC_FAILED".to_string(),
                message: err,
            })),
        };
    }

    Some(WsMessage::Error(ErrorMsg {
        code: "INVALID_MESSAGE".to_string(),
        message: "Unexpected message from client".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::adaptor::protocol::*;
    use crate::domain::app_config::repository::{ConfigRepository, ConfigUpdate};
    use crate::domain::app_config::value_objects::{
        AgentShortcutConfig, AppConfigDocument, AppSettings, ServerConfig, TelemetryConfig,
        TlsConfig, WorkflowConfig,
    };
    use crate::domain::app_config::AppConfigError;
    use crate::domain::notification::{DesktopNotifyMode, NotifyConfig};
    use crate::usecase::agent_session::session::{
        AgentStreamResyncReadModel, StreamResyncSnapshot,
    };
    use crate::usecase::pty_session::query_service::PtySessionReplayReader;
    use crate::ws_bridge::WsBroadcaster;

    use super::super::WsServerState;
    use super::route_message;

    struct EmptyReplayReader;

    impl PtySessionReplayReader for EmptyReplayReader {
        fn replay_outputs(&self) -> Vec<crate::usecase::pty_session::dto::PtyReplayOutput> {
            Vec::new()
        }
    }

    struct MockConfigRepository;

    impl ConfigRepository for MockConfigRepository {
        fn load(&self) -> Result<AppConfigDocument, AppConfigError> {
            Ok(AppConfigDocument {
                server: ServerConfig {
                    bind: "127.0.0.1".to_string(),
                    port: 0,
                    hook_port: 0,
                    token: "token".to_string(),
                    tls: TlsConfig {
                        enabled: false,
                        cert: String::new(),
                        key: String::new(),
                    },
                    notify: NotifyConfig {
                        webhook_url: String::new(),
                        on_running: false,
                        on_done: false,
                        on_error: false,
                        on_waiting: false,
                        desktop_mode: DesktopNotifyMode::Always,
                        inactive_timeout_minutes: 0,
                    },
                },
                telemetry: TelemetryConfig {
                    crash_reporting: false,
                    performance_telemetry: false,
                },
                app: AppSettings {
                    close_to_tray: false,
                    auto_launch: false,
                    start_minimized: false,
                    last_root_path: String::new(),
                    last_repo_paths: Vec::new(),
                    external_editor: String::new(),
                    agent_shortcuts: AgentShortcutConfig::default(),
                },
                workflow: WorkflowConfig {
                    approval_auto_approve: false,
                },
            })
        }

        fn save(&self, _config: AppConfigDocument) -> Result<(), AppConfigError> {
            Ok(())
        }

        fn update(&self, _f: ConfigUpdate) -> Result<(), AppConfigError> {
            Ok(())
        }
    }

    struct StaticStreamResyncReadModel {
        result: Result<Option<StreamResyncSnapshot>, String>,
    }

    #[async_trait::async_trait]
    impl AgentStreamResyncReadModel for StaticStreamResyncReadModel {
        async fn resync_streaming_message(
            &self,
            _session_id: &str,
            _message_id: &str,
            _since_seq: u64,
        ) -> Result<Option<StreamResyncSnapshot>, String> {
            self.result.clone()
        }
    }

    fn test_state_with_read_model(
        broadcaster: Arc<WsBroadcaster>,
        read_model: Arc<dyn AgentStreamResyncReadModel>,
    ) -> WsServerState {
        WsServerState::new(
            broadcaster,
            Arc::new(EmptyReplayReader),
            Arc::new(MockConfigRepository),
            read_model,
            false,
        )
    }

    fn test_state() -> WsServerState {
        test_state_with_read_model(
            Arc::new(WsBroadcaster::default()),
            Arc::new(StaticStreamResyncReadModel { result: Ok(None) }),
        )
    }

    #[tokio::test]
    async fn test_route_known_inbound_message_returns_error() {
        let state = test_state();
        let msg = WsMessage::AuthChallenge(AuthChallenge {
            challenge: "x".to_string(),
        });
        let result = route_message(&msg, &state).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "INVALID_MESSAGE"),
            _ => panic!("expected error"),
        }
    }

    #[tokio::test]
    async fn route_resync_stream_missing_message_returns_stream_not_found() {
        let state = test_state();
        let result = route_message(
            &WsMessage::ResyncStream(ResyncStreamReq {
                session_id: "missing".to_string(),
                message_id: "m1".to_string(),
                since_seq: 0,
            }),
            &state,
        )
        .await;

        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "STREAM_NOT_FOUND"),
            _ => panic!("expected STREAM_NOT_FOUND"),
        }
    }

    #[tokio::test]
    async fn route_resync_stream_enqueues_snapshot_for_ws_forwarder() {
        let broadcaster = Arc::new(WsBroadcaster::default());
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));
        let state = test_state_with_read_model(
            Arc::clone(&broadcaster),
            Arc::new(StaticStreamResyncReadModel {
                result: Ok(Some(StreamResyncSnapshot {
                    session_id: "s1".to_string(),
                    message_id: "m1".to_string(),
                    seq: 4,
                    parts: vec![crate::usecase::agent_session::session::MessagePart::Text {
                        content: "snapshot".to_string(),
                        parent_tool_use_id: None,
                    }],
                })),
            }),
        );

        let result = route_message(
            &WsMessage::ResyncStream(ResyncStreamReq {
                session_id: "s1".to_string(),
                message_id: "m1".to_string(),
                since_seq: 2,
            }),
            &state,
        )
        .await;

        assert!(result.is_none());
        let drained = broadcaster.drain_stream_messages();
        assert!(matches!(
            &drained[..],
            [WsMessage::AgentStreamSync(snapshot)]
                if snapshot.session_id == "s1" && snapshot.message_id == "m1" && snapshot.seq == 4
        ));
    }

    #[tokio::test]
    async fn route_resync_stream_read_model_error_returns_stream_resync_failed() {
        let state = test_state_with_read_model(
            Arc::new(WsBroadcaster::default()),
            Arc::new(StaticStreamResyncReadModel {
                result: Err("read model exploded".to_string()),
            }),
        );

        let result = route_message(
            &WsMessage::ResyncStream(ResyncStreamReq {
                session_id: "s1".to_string(),
                message_id: "m1".to_string(),
                since_seq: 2,
            }),
            &state,
        )
        .await;

        match result {
            Some(WsMessage::Error(e)) => {
                assert_eq!(e.code, "STREAM_RESYNC_FAILED");
                assert_eq!(e.message, "read model exploded");
            }
            _ => panic!("expected STREAM_RESYNC_FAILED"),
        }
    }
}
