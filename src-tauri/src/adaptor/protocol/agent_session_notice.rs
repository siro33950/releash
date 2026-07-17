use serde::{Deserialize, Serialize};

use crate::usecase::agent_session::notice::{
    AgentSessionNoticeOperation, AgentSessionNoticeSnapshot, AgentSessionNoticeUpdate,
};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionNoticeOperationMessage {
    Send,
    LoadSession,
    LoadOlder,
    CancelQueue,
    ResumeQueue,
    CloseSession,
    RestoreSession,
    ArchiveSession,
    ForkSession,
    SetTitle,
    RespondPermission,
    SetBackend,
}

impl From<AgentSessionNoticeOperationMessage> for AgentSessionNoticeOperation {
    fn from(value: AgentSessionNoticeOperationMessage) -> Self {
        match value {
            AgentSessionNoticeOperationMessage::Send => Self::Send,
            AgentSessionNoticeOperationMessage::LoadSession => Self::LoadSession,
            AgentSessionNoticeOperationMessage::LoadOlder => Self::LoadOlder,
            AgentSessionNoticeOperationMessage::CancelQueue => Self::CancelQueue,
            AgentSessionNoticeOperationMessage::ResumeQueue => Self::ResumeQueue,
            AgentSessionNoticeOperationMessage::CloseSession => Self::CloseSession,
            AgentSessionNoticeOperationMessage::RestoreSession => Self::RestoreSession,
            AgentSessionNoticeOperationMessage::ArchiveSession => Self::ArchiveSession,
            AgentSessionNoticeOperationMessage::ForkSession => Self::ForkSession,
            AgentSessionNoticeOperationMessage::SetTitle => Self::SetTitle,
            AgentSessionNoticeOperationMessage::RespondPermission => Self::RespondPermission,
            AgentSessionNoticeOperationMessage::SetBackend => Self::SetBackend,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AgentSessionNoticeUpdateMessage {
    Failure {
        operation: AgentSessionNoticeOperationMessage,
        message: String,
    },
    Success {
        operation: AgentSessionNoticeOperationMessage,
    },
    Dismiss,
    RemoveSession,
}

impl From<AgentSessionNoticeUpdateMessage> for AgentSessionNoticeUpdate {
    fn from(value: AgentSessionNoticeUpdateMessage) -> Self {
        match value {
            AgentSessionNoticeUpdateMessage::Failure { operation, message } => Self::Failure {
                operation: operation.into(),
                message,
            },
            AgentSessionNoticeUpdateMessage::Success { operation } => Self::Success {
                operation: operation.into(),
            },
            AgentSessionNoticeUpdateMessage::Dismiss => Self::Dismiss,
            AgentSessionNoticeUpdateMessage::RemoveSession => Self::RemoveSession,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionNoticeMessage {
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionNoticeSnapshotMessage {
    pub session_id: String,
    pub revision: u64,
    pub notice: Option<AgentSessionNoticeMessage>,
}

impl From<AgentSessionNoticeSnapshot> for AgentSessionNoticeSnapshotMessage {
    fn from(snapshot: AgentSessionNoticeSnapshot) -> Self {
        Self {
            session_id: snapshot.session_id,
            revision: snapshot.revision,
            notice: snapshot.notice.map(|notice| AgentSessionNoticeMessage {
                message: notice.message,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::usecase::agent_session::notice_state::AgentSessionNotice;

    #[test]
    fn shared_command_response_and_event_snapshot_has_one_wire_shape() {
        let message = AgentSessionNoticeSnapshotMessage::from(AgentSessionNoticeSnapshot {
            session_id: "session-a".to_string(),
            revision: 7,
            notice: Some(AgentSessionNotice {
                message: "send failed".to_string(),
            }),
        });

        assert_eq!(
            serde_json::to_value(message).unwrap(),
            json!({
                "sessionId": "session-a",
                "revision": 7,
                "notice": { "message": "send failed" },
            })
        );
    }

    #[test]
    fn update_message_rejects_query_actions() {
        assert!(
            serde_json::from_value::<AgentSessionNoticeUpdateMessage>(json!({
                "action": "query"
            }))
            .is_err()
        );
    }
}
