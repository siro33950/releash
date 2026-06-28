use crate::adaptor::gateway::shared::ws_broadcaster::WsBroadcaster;
use crate::adaptor::protocol::*;
use crate::usecase::agent_session::session::AgentStreamResyncReadModel;
use crate::usecase::agent_session::status::AgentStatusCenter;

pub(crate) async fn route_message(
    msg: &WsMessage,
    broadcaster: &WsBroadcaster,
    resync_read_model: &dyn AgentStreamResyncReadModel,
    status_center: &AgentStatusCenter,
) -> Option<WsMessage> {
    if let WsMessage::ResyncStream(req) = msg {
        return match crate::usecase::agent_session::session::resync_streaming_message(
            resync_read_model,
            &req.session_id,
            &req.message_id,
            req.since_seq,
        )
        .await
        {
            Ok(Some(snapshot)) => {
                broadcaster.send_stream_snapshot(snapshot.into());
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

    if let WsMessage::WorktreeStepStatusResync(req) = msg {
        return Some(WsMessage::WorktreeStepStatusSync(
            status_center
                .query_worktree_step_statuses(&req.worktree_path)
                .into(),
        ));
    }

    Some(WsMessage::Error(ErrorMsg {
        code: "INVALID_MESSAGE".to_string(),
        message: "Unexpected message from client".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::adaptor::gateway::shared::ws_broadcaster::WsBroadcaster;
    use crate::adaptor::protocol::*;
    use crate::usecase::agent_session::session::{
        AgentStreamResyncReadModel, StreamResyncSnapshot,
    };
    use crate::usecase::agent_session::status::AgentStatusCenter;

    use super::route_message;

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

    fn test_dependencies() -> (
        Arc<WsBroadcaster>,
        Arc<dyn AgentStreamResyncReadModel>,
        Arc<AgentStatusCenter>,
    ) {
        (
            Arc::new(WsBroadcaster::default()),
            Arc::new(StaticStreamResyncReadModel { result: Ok(None) }),
            Arc::new(AgentStatusCenter::new()),
        )
    }

    #[tokio::test]
    async fn test_route_known_inbound_message_returns_error() {
        let (broadcaster, read_model, status_center) = test_dependencies();
        let msg = WsMessage::AuthChallenge(AuthChallenge {
            challenge: "x".to_string(),
        });
        let result = route_message(
            &msg,
            broadcaster.as_ref(),
            read_model.as_ref(),
            status_center.as_ref(),
        )
        .await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "INVALID_MESSAGE"),
            _ => panic!("expected error"),
        }
    }

    #[tokio::test]
    async fn route_resync_stream_missing_message_returns_stream_not_found() {
        let (broadcaster, read_model, status_center) = test_dependencies();
        let result = route_message(
            &WsMessage::ResyncStream(ResyncStreamReq {
                session_id: "missing".to_string(),
                message_id: "m1".to_string(),
                since_seq: 0,
            }),
            broadcaster.as_ref(),
            read_model.as_ref(),
            status_center.as_ref(),
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
        let read_model = Arc::new(StaticStreamResyncReadModel {
            result: Ok(Some(StreamResyncSnapshot {
                session_id: "s1".to_string(),
                message_id: "m1".to_string(),
                seq: 4,
                parts: vec![crate::usecase::agent_session::session::MessagePart::Text {
                    content: "snapshot".to_string(),
                    parent_tool_use_id: None,
                }],
            })),
        });

        let result = route_message(
            &WsMessage::ResyncStream(ResyncStreamReq {
                session_id: "s1".to_string(),
                message_id: "m1".to_string(),
                since_seq: 2,
            }),
            broadcaster.as_ref(),
            read_model.as_ref(),
            &AgentStatusCenter::new(),
        )
        .await;

        assert!(result.is_none());
        let drained = broadcaster.drain_messages();
        assert!(matches!(
            &drained[..],
            [WsMessage::AgentStreamDelta(snapshot)]
                if snapshot.session_id == "s1"
                    && snapshot.message_id == "m1"
                    && snapshot.seq == 4
                    && snapshot.snapshot
        ));
    }

    #[tokio::test]
    async fn route_resync_stream_read_model_error_returns_stream_resync_failed() {
        let broadcaster = Arc::new(WsBroadcaster::default());
        let read_model = Arc::new(StaticStreamResyncReadModel {
            result: Err("read model exploded".to_string()),
        });

        let result = route_message(
            &WsMessage::ResyncStream(ResyncStreamReq {
                session_id: "s1".to_string(),
                message_id: "m1".to_string(),
                since_seq: 2,
            }),
            broadcaster.as_ref(),
            read_model.as_ref(),
            &AgentStatusCenter::new(),
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

    #[tokio::test]
    async fn route_worktree_step_status_resync_returns_backend_view() {
        let (broadcaster, read_model, status_center) = test_dependencies();

        let result = route_message(
            &WsMessage::WorktreeStepStatusResync(WorktreeStepStatusResyncReq {
                worktree_path: "/repo".to_string(),
            }),
            broadcaster.as_ref(),
            read_model.as_ref(),
            status_center.as_ref(),
        )
        .await;

        match result {
            Some(WsMessage::WorktreeStepStatusSync(sync)) => {
                assert_eq!(sync.worktree_path, "/repo");
                assert_eq!(sync.version, 0);
                assert!(sync.steps.is_empty());
                assert!(sync.workflows.is_empty());
            }
            other => panic!("expected worktree step status sync, got {other:?}"),
        }
    }

    #[test]
    fn test_deserialize_invalid_json() {
        let result = deserialize_message("not valid json at all");
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_empty_payload() {
        let result = deserialize_message("");
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_missing_type_field() {
        let result = deserialize_message(r#"{"data": "hello"}"#);
        assert!(result.is_err());
    }
}
