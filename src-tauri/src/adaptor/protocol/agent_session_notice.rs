use serde::{Deserialize, Serialize};

use crate::adaptor::protocol::agent_session_v1::SafeOperationFailureDtoV1;
use crate::usecase::agent_session::feedback::{
    FeedbackAction, FeedbackActionIdentity, FeedbackRetryOutcome, SessionFeedbackEntry,
    SessionFeedbackPage,
};
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
    pub revision: String,
    pub notice: Option<AgentSessionNoticeMessage>,
}

impl From<AgentSessionNoticeSnapshot> for AgentSessionNoticeSnapshotMessage {
    fn from(snapshot: AgentSessionNoticeSnapshot) -> Self {
        Self {
            session_id: snapshot.session_id,
            revision: snapshot.revision.to_string(),
            notice: snapshot.notice.map(|notice| AgentSessionNoticeMessage {
                message: notice.message,
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionFeedbackActionMessage {
    Dismiss,
    RetryResolution,
}

impl From<FeedbackAction> for SessionFeedbackActionMessage {
    fn from(value: FeedbackAction) -> Self {
        match value {
            FeedbackAction::Dismiss => Self::Dismiss,
            FeedbackAction::RetryResolution => Self::RetryResolution,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionFeedbackActionIdentityMessage {
    pub action: SessionFeedbackActionMessage,
    pub action_id: String,
    pub origin_revision: String,
}

impl From<FeedbackActionIdentity> for SessionFeedbackActionIdentityMessage {
    fn from(value: FeedbackActionIdentity) -> Self {
        Self {
            action: value.action.into(),
            action_id: value.action_id,
            origin_revision: value.origin_revision.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionFeedbackEntryMessage {
    pub feedback_id: String,
    pub attempt_id: String,
    pub session_id: String,
    pub operation: String,
    pub revision: String,
    pub actions: Vec<SessionFeedbackActionMessage>,
    pub action_identities: Vec<SessionFeedbackActionIdentityMessage>,
    pub failure: SafeOperationFailureDtoV1,
}

impl From<SessionFeedbackEntry> for SessionFeedbackEntryMessage {
    fn from(value: SessionFeedbackEntry) -> Self {
        Self {
            feedback_id: value.feedback_id,
            attempt_id: value.attempt_id,
            session_id: value.session_id,
            operation: value.operation.label().to_string(),
            revision: value.revision.to_string(),
            actions: value.actions.into_iter().map(Into::into).collect(),
            action_identities: value
                .action_identities
                .into_iter()
                .map(Into::into)
                .collect(),
            failure: value.failure.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionFeedbackPageMessage {
    pub entries: Vec<SessionFeedbackEntryMessage>,
    pub next_cursor: Option<String>,
}

impl From<SessionFeedbackPage> for SessionFeedbackPageMessage {
    fn from(value: SessionFeedbackPage) -> Self {
        Self {
            entries: value.entries.into_iter().map(Into::into).collect(),
            next_cursor: value.next_cursor,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SessionFeedbackRetryOutcomeMessage {
    Resolved,
    Failed {
        entry: Box<SessionFeedbackEntryMessage>,
    },
}

impl From<FeedbackRetryOutcome> for SessionFeedbackRetryOutcomeMessage {
    fn from(value: FeedbackRetryOutcome) -> Self {
        match value {
            FeedbackRetryOutcome::Resolved => Self::Resolved,
            FeedbackRetryOutcome::Failed(entry) => Self::Failed {
                entry: Box::new((*entry).into()),
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SessionFeedbackErrorMessage {
    InvalidRequest,
    ShutdownInProgress,
    NotFound,
    RevisionConflict { current_revision: String },
    FeedbackCapacityExceeded,
    CursorMismatch,
    CursorExpired,
    QueryBusy,
    DeadlineExceeded,
    ResponseTooLarge,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    OutcomeUnknown { feedback_id: String },
    Internal { correlation_id: String },
}

impl From<crate::usecase::agent_session::feedback::FeedbackError> for SessionFeedbackErrorMessage {
    fn from(value: crate::usecase::agent_session::feedback::FeedbackError) -> Self {
        use crate::usecase::agent_session::feedback::FeedbackError as E;
        match value {
            E::InvalidRequest => Self::InvalidRequest,
            E::ShutdownInProgress => Self::ShutdownInProgress,
            E::NotFound => Self::NotFound,
            E::RevisionConflict { current_revision } => Self::RevisionConflict {
                current_revision: current_revision.to_string(),
            },
            E::CapacityExceeded => Self::FeedbackCapacityExceeded,
            E::CursorMismatch => Self::CursorMismatch,
            E::CursorExpired => Self::CursorExpired,
            E::QueryBusy => Self::QueryBusy,
            E::DeadlineExceeded => Self::DeadlineExceeded,
            E::ResponseTooLarge => Self::ResponseTooLarge,
            E::StorageUnavailable { failure } => Self::StorageUnavailable {
                failure: failure.into(),
            },
            E::OutcomeUnknown { feedback_id } => Self::OutcomeUnknown { feedback_id },
            E::Internal { correlation_id } => Self::Internal { correlation_id },
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::usecase::agent_session::notice_state::AgentSessionNotice;

    #[test]
    fn b075_shared_command_response_and_event_snapshot_has_one_lossless_wire_shape() {
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
                "revision": "7",
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

    #[test]
    fn update_message_rejects_session_wide_removal() {
        assert!(
            serde_json::from_value::<AgentSessionNoticeUpdateMessage>(json!({
                "action": "remove_session"
            }))
            .is_err()
        );
    }
}
