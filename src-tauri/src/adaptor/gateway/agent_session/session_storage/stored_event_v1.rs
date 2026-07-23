//! Versioned legacy JSON DTO and deterministic codec for agent-session events.
//!
//! The domain event vocabulary is serde-free. Legacy JSON crosses this gateway-
//! owned V1 DTO and preserves additive source payloads for later writeback.

use serde::{Deserialize, Serialize};

use crate::domain::agent_session::entities::{Attachment, PermissionPartStatus};
use crate::domain::agent_session::events::{
    AgentSessionDomainEvent as AgentSessionEvent, BackendSessionRecoveryReason,
    GoalReactivationOutcome, InterruptReason, ObligationKind, ObligationState, PermissionDecision,
    PromptInput, RecoveryActionKind, RecoveryResultClassification, SendDisposition,
    SessionLifecycleKind, StopResolution, TurnStopReason, TurnTokenUsage,
};
use crate::domain::agent_session::value_objects::{
    JsonPayload, SystemNotificationType, TodoListItem, ToolOutputRef, ToolOutputSummary,
};
use crate::domain::code::MentionReference;

use super::stored_message_part_v1::{
    contains_additive_fields, IncompatibleStoredEvent, PreservedStoredPayload, StoredMessagePartV1,
    StoredPayloadSource, StoredPermissionRequestV1,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DecodedStoredAgentSessionEventV1 {
    pub event: AgentSessionEvent,
    pub preserved_additive_payload: Option<PreservedStoredPayload>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredPromptInputV1 {
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mentions: Vec<StoredMentionReferenceV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attachment_refs: Vec<StoredAttachmentV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parts: Vec<StoredMessagePartV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredMentionReferenceV1 {
    file_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    start_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredAttachmentV1 {
    id: String,
    media_type: String,
    byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredToolOutputRefV1 {
    id: String,
    byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredToolOutputSummaryV1 {
    line_count: u64,
    byte_size: u64,
    is_error: bool,
    #[serde(default)]
    truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredTodoListItemV1 {
    text: String,
    completed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredSystemNotificationTypeV1 {
    Compaction,
    SessionRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredTurnStopReasonV1 {
    Refusal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredInterruptReasonV1 {
    Abort,
    Timeout,
    Crash,
    SessionClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredPermissionDecisionV1 {
    Allowed,
    Denied,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredTurnTokenUsageV1 {
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredBackendSessionRecoveryReasonV1 {
    ResumeMismatch,
    BackendSessionLost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum StoredGoalReactivationOutcomeV1 {
    NoCurrentGoal,
    TerminalGoalUnchanged {
        goal_id: String,
        goal_revision: u64,
    },
    Restored {
        goal_id: String,
        goal_revision: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_goal_ref: Option<String>,
    },
    ObservedUnchanged {
        goal_id: String,
        goal_revision: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum StoredSendDispositionV1 {
    StartedTurn { turn_id: String },
    Queued { queue_item_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredObligationKindV1 {
    ProviderEstablish,
    TurnExecution,
    PermissionResponse,
    ProviderInterrupt,
    SessionClose,
    QueuePause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredObligationStateV1 {
    Pending,
    EffectReserved,
    Completed,
    ReconciliationRequired,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredStopResolutionV1 {
    Succeeded,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredSessionLifecycleKindV1 {
    Close,
    Archive,
    BackendSwitch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredRecoveryActionKindV1 {
    ReadAgain,
    RetrySameEffect,
    UseObservedResult,
    CancelIfSafe,
    KeepForManualResolution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredRecoveryResultClassificationV1 {
    Pending,
    Succeeded,
    ConfirmedNoEffect,
    Ambiguous,
    CancelledBeforeEffect,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum StoredAgentSessionEventV1 {
    BackendSessionRecoveryStarted {
        recovery_id: String,
        old_provider_session_generation: u64,
        reason: StoredBackendSessionRecoveryReasonV1,
        at: f64,
    },
    SessionConfigurationReactivated {
        recovery_id: String,
        provider_session_generation: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        consumed_observation_id: Option<String>,
        at: f64,
    },
    SessionGoalReactivated {
        recovery_id: String,
        outcome: StoredGoalReactivationOutcomeV1,
        provider_session_generation: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        restoring_turn_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        consumed_observation_id: Option<String>,
        at: f64,
    },
    BackendSessionRecoveryCompleted {
        recovery_id: String,
        provider_session_generation: u64,
        at: f64,
    },
    BackendSessionRecoveryFailed {
        recovery_id: String,
        error: String,
        at: f64,
    },
    TurnStarted {
        turn_id: u64,
        message_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assistant_message_id: Option<String>,
        prompt: StoredPromptInputV1,
        at: f64,
    },
    TurnInterruptRequested {
        turn_id: u64,
        at: f64,
    },
    QueuePaused {
        at: f64,
    },
    QueueResumed {
        expected_paused_at: f64,
        at: f64,
    },
    TextRecorded {
        turn_id: u64,
        message_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    ReasoningRecorded {
        turn_id: u64,
        message_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    ErrorRecorded {
        turn_id: u64,
        message_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    ToolCallStarted {
        turn_id: u64,
        tool_use_id: String,
        tool: String,
        input: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    ToolCallSucceeded {
        turn_id: u64,
        tool_use_id: String,
        content: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "contentRef"
        )]
        content_ref: Option<StoredToolOutputRefV1>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<StoredToolOutputSummaryV1>,
    },
    ToolCallFailed {
        turn_id: u64,
        tool_use_id: String,
        content: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "contentRef"
        )]
        content_ref: Option<StoredToolOutputRefV1>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<StoredToolOutputSummaryV1>,
    },
    ToolResultRecorded {
        turn_id: u64,
        message_id: String,
        content: String,
        is_error: bool,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "contentRef"
        )]
        content_ref: Option<StoredToolOutputRefV1>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<StoredToolOutputSummaryV1>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    ToolCallRetried {
        turn_id: u64,
        tool_use_id: String,
        attempt: u32,
    },
    PermissionRequested {
        turn_id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
        request: StoredPermissionRequestV1,
    },
    PermissionResolved {
        turn_id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        decision: StoredPermissionDecisionV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answers: Option<serde_json::Value>,
    },
    TaskStatusChanged {
        turn_id: u64,
        message_id: String,
        task_tool_use_id: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    TodoListSnapshotRecorded {
        turn_id: u64,
        message_id: String,
        items: Vec<StoredTodoListItemV1>,
    },
    SystemNotificationRecorded {
        turn_id: u64,
        message_id: String,
        notification_type: StoredSystemNotificationTypeV1,
        status: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hook_id: Option<String>,
    },
    ImageRecorded {
        turn_id: u64,
        message_id: String,
        data: String,
        media_type: String,
    },
    ImageRefRecorded {
        turn_id: u64,
        message_id: String,
        attachment: StoredAttachmentV1,
    },
    FinalPartsRecorded {
        turn_id: u64,
        message_id: String,
        parts: Vec<StoredMessagePartV1>,
    },
    TurnCompleted {
        turn_id: u64,
        exit_code: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_reason: Option<StoredTurnStopReasonV1>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_usage: Option<StoredTurnTokenUsageV1>,
    },
    TurnInterrupted {
        turn_id: u64,
        reason: StoredInterruptReasonV1,
        exit_code: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    SessionErrored {
        message_id: String,
        reason: String,
        at: f64,
    },
    SessionClosed {
        at: f64,
    },
    SendOperationAccepted {
        operation_id: String,
        disposition: StoredSendDispositionV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        human_message_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<StoredPromptInputV1>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reserved_turn_id: Option<String>,
        at: f64,
    },
    StopOperationAccepted {
        operation_id: String,
        target_turn_id: u64,
        at: f64,
    },
    SessionLifecycleOperationAccepted {
        operation_id: String,
        kind: StoredSessionLifecycleKindV1,
        at: f64,
    },
    ObligationRecorded {
        obligation_id: String,
        kind: StoredObligationKindV1,
        state: StoredObligationStateV1,
        at: f64,
    },
    StopResolutionRecorded {
        operation_id: String,
        turn_id: u64,
        resolution: StoredStopResolutionV1,
        at: f64,
    },
    PendingRecoveryPublished {
        obligation_id: String,
        kind: StoredObligationKindV1,
        at: f64,
    },
    RecoveryActionResolved {
        action_id: String,
        obligation_id: String,
        kind: StoredRecoveryActionKindV1,
        classification: StoredRecoveryResultClassificationV1,
        at: f64,
    },
}

pub(crate) fn decode_stored_agent_session_event_v1(
    raw: &[u8],
    payload_version: u32,
    source: StoredPayloadSource,
) -> Result<DecodedStoredAgentSessionEventV1, IncompatibleStoredEvent> {
    if payload_version != 1 {
        return Err(IncompatibleStoredEvent {
            type_tag: "agent_session_event".to_string(),
            payload_version,
            reason: "unsupported required payload version".to_string(),
        });
    }
    let value: serde_json::Value = serde_json::from_slice(raw).map_err(|error| {
        incompatible_event("agent_session_event", format!("invalid JSON: {error}"))
    })?;
    let type_tag = value
        .as_object()
        .and_then(|object| object.get("type"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| incompatible_event("agent_session_event", "missing required type tag"))?
        .to_string();
    let stored: StoredAgentSessionEventV1 =
        serde_json::from_value(value.clone()).map_err(|error| {
            incompatible_event(type_tag.clone(), format!("invalid known payload: {error}"))
        })?;
    let canonical = serde_json::to_value(&stored)
        .expect("stored agent-session event serialization must be deterministic");
    let has_additive_fields = contains_additive_fields(&value, &canonical);
    let event = stored
        .try_into()
        .map_err(|reason| incompatible_event(type_tag.clone(), reason))?;
    Ok(DecodedStoredAgentSessionEventV1 {
        event,
        preserved_additive_payload: has_additive_fields.then(|| PreservedStoredPayload {
            source,
            payload_version,
            type_tag,
            raw_bytes: raw.to_vec(),
        }),
    })
}

pub(crate) fn encode_agent_session_events_v1(
    events: &[AgentSessionEvent],
    pretty: bool,
) -> Result<Vec<u8>, serde_json::Error> {
    let stored = events
        .iter()
        .map(StoredAgentSessionEventV1::from)
        .collect::<Vec<_>>();
    if pretty {
        serde_json::to_vec_pretty(&stored)
    } else {
        serde_json::to_vec(&stored)
    }
}

pub(crate) fn decode_agent_session_events_v1(
    raw: &[u8],
) -> Result<Vec<AgentSessionEvent>, IncompatibleStoredEvent> {
    let stored: Vec<StoredAgentSessionEventV1> = serde_json::from_slice(raw)
        .map_err(|error| incompatible_event("agent_session_event", error.to_string()))?;
    stored
        .into_iter()
        .map(|event| {
            event
                .try_into()
                .map_err(|reason| incompatible_event("agent_session_event", reason))
        })
        .collect()
}

fn incompatible_event(
    type_tag: impl Into<String>,
    reason: impl Into<String>,
) -> IncompatibleStoredEvent {
    IncompatibleStoredEvent {
        type_tag: type_tag.into(),
        payload_version: 1,
        reason: reason.into(),
    }
}

impl From<&AgentSessionEvent> for StoredAgentSessionEventV1 {
    fn from(event: &AgentSessionEvent) -> Self {
        use AgentSessionEvent as Domain;
        use StoredAgentSessionEventV1 as Stored;
        match event {
            Domain::BackendSessionRecoveryStarted {
                recovery_id,
                old_provider_session_generation,
                reason,
                at,
            } => Stored::BackendSessionRecoveryStarted {
                recovery_id: recovery_id.clone(),
                old_provider_session_generation: *old_provider_session_generation,
                reason: (*reason).into(),
                at: *at,
            },
            Domain::SessionConfigurationReactivated {
                recovery_id,
                provider_session_generation,
                consumed_observation_id,
                at,
            } => Stored::SessionConfigurationReactivated {
                recovery_id: recovery_id.clone(),
                provider_session_generation: *provider_session_generation,
                consumed_observation_id: consumed_observation_id.clone(),
                at: *at,
            },
            Domain::SessionGoalReactivated {
                recovery_id,
                outcome,
                provider_session_generation,
                restoring_turn_id,
                consumed_observation_id,
                at,
            } => Stored::SessionGoalReactivated {
                recovery_id: recovery_id.clone(),
                outcome: outcome.into(),
                provider_session_generation: *provider_session_generation,
                restoring_turn_id: restoring_turn_id.clone(),
                consumed_observation_id: consumed_observation_id.clone(),
                at: *at,
            },
            Domain::BackendSessionRecoveryCompleted {
                recovery_id,
                provider_session_generation,
                at,
            } => Stored::BackendSessionRecoveryCompleted {
                recovery_id: recovery_id.clone(),
                provider_session_generation: *provider_session_generation,
                at: *at,
            },
            Domain::BackendSessionRecoveryFailed {
                recovery_id,
                error,
                at,
            } => Stored::BackendSessionRecoveryFailed {
                recovery_id: recovery_id.clone(),
                error: error.clone(),
                at: *at,
            },
            Domain::TurnStarted {
                turn_id,
                message_id,
                assistant_message_id,
                prompt,
                at,
            } => Stored::TurnStarted {
                turn_id: *turn_id,
                message_id: message_id.clone(),
                assistant_message_id: assistant_message_id.clone(),
                prompt: prompt.into(),
                at: *at,
            },
            Domain::TurnInterruptRequested { turn_id, at } => Stored::TurnInterruptRequested {
                turn_id: *turn_id,
                at: *at,
            },
            Domain::QueuePaused { at } => Stored::QueuePaused { at: *at },
            Domain::QueueResumed {
                expected_paused_at,
                at,
            } => Stored::QueueResumed {
                expected_paused_at: *expected_paused_at,
                at: *at,
            },
            Domain::TextRecorded {
                turn_id,
                message_id,
                content,
                parent_tool_use_id,
            } => Stored::TextRecorded {
                turn_id: *turn_id,
                message_id: message_id.clone(),
                content: content.clone(),
                parent_tool_use_id: parent_tool_use_id.clone(),
            },
            Domain::ReasoningRecorded {
                turn_id,
                message_id,
                content,
                parent_tool_use_id,
            } => Stored::ReasoningRecorded {
                turn_id: *turn_id,
                message_id: message_id.clone(),
                content: content.clone(),
                parent_tool_use_id: parent_tool_use_id.clone(),
            },
            Domain::ErrorRecorded {
                turn_id,
                message_id,
                content,
                parent_tool_use_id,
            } => Stored::ErrorRecorded {
                turn_id: *turn_id,
                message_id: message_id.clone(),
                content: content.clone(),
                parent_tool_use_id: parent_tool_use_id.clone(),
            },
            Domain::ToolCallStarted {
                turn_id,
                tool_use_id,
                tool,
                input,
                parent_tool_use_id,
            } => Stored::ToolCallStarted {
                turn_id: *turn_id,
                tool_use_id: tool_use_id.clone(),
                tool: tool.clone(),
                input: json_value(input),
                parent_tool_use_id: parent_tool_use_id.clone(),
            },
            Domain::ToolCallSucceeded {
                turn_id,
                tool_use_id,
                content,
                content_ref,
                summary,
            } => Stored::ToolCallSucceeded {
                turn_id: *turn_id,
                tool_use_id: tool_use_id.clone(),
                content: content.clone(),
                content_ref: content_ref.as_ref().map(Into::into),
                summary: summary.as_ref().map(Into::into),
            },
            Domain::ToolCallFailed {
                turn_id,
                tool_use_id,
                content,
                content_ref,
                summary,
            } => Stored::ToolCallFailed {
                turn_id: *turn_id,
                tool_use_id: tool_use_id.clone(),
                content: content.clone(),
                content_ref: content_ref.as_ref().map(Into::into),
                summary: summary.as_ref().map(Into::into),
            },
            Domain::ToolResultRecorded {
                turn_id,
                message_id,
                content,
                is_error,
                content_ref,
                summary,
                tool_use_id,
                parent_tool_use_id,
            } => Stored::ToolResultRecorded {
                turn_id: *turn_id,
                message_id: message_id.clone(),
                content: content.clone(),
                is_error: *is_error,
                content_ref: content_ref.as_ref().map(Into::into),
                summary: summary.as_ref().map(Into::into),
                tool_use_id: tool_use_id.clone(),
                parent_tool_use_id: parent_tool_use_id.clone(),
            },
            Domain::ToolCallRetried {
                turn_id,
                tool_use_id,
                attempt,
            } => Stored::ToolCallRetried {
                turn_id: *turn_id,
                tool_use_id: tool_use_id.clone(),
                attempt: *attempt,
            },
            Domain::PermissionRequested {
                turn_id,
                tool_use_id,
                request,
            } => Stored::PermissionRequested {
                turn_id: *turn_id,
                tool_use_id: tool_use_id.clone(),
                request: request.into(),
            },
            Domain::PermissionResolved {
                turn_id,
                tool_use_id,
                request_id,
                decision,
                answers,
            } => Stored::PermissionResolved {
                turn_id: *turn_id,
                tool_use_id: tool_use_id.clone(),
                request_id: request_id.clone(),
                decision: (*decision).into(),
                answers: answers.as_ref().map(json_value),
            },
            Domain::TaskStatusChanged {
                turn_id,
                message_id,
                task_tool_use_id,
                status,
                description,
                summary,
            } => Stored::TaskStatusChanged {
                turn_id: *turn_id,
                message_id: message_id.clone(),
                task_tool_use_id: task_tool_use_id.clone(),
                status: status.clone(),
                description: description.clone(),
                summary: summary.clone(),
            },
            Domain::TodoListSnapshotRecorded {
                turn_id,
                message_id,
                items,
            } => Stored::TodoListSnapshotRecorded {
                turn_id: *turn_id,
                message_id: message_id.clone(),
                items: items.iter().map(Into::into).collect(),
            },
            Domain::SystemNotificationRecorded {
                turn_id,
                message_id,
                notification_type,
                status,
                label,
                detail,
                hook_id,
            } => Stored::SystemNotificationRecorded {
                turn_id: *turn_id,
                message_id: message_id.clone(),
                notification_type: (*notification_type).into(),
                status: status.clone(),
                label: label.clone(),
                detail: detail.clone(),
                hook_id: hook_id.clone(),
            },
            Domain::ImageRecorded {
                turn_id,
                message_id,
                data,
                media_type,
            } => Stored::ImageRecorded {
                turn_id: *turn_id,
                message_id: message_id.clone(),
                data: data.clone(),
                media_type: media_type.clone(),
            },
            Domain::ImageRefRecorded {
                turn_id,
                message_id,
                attachment,
            } => Stored::ImageRefRecorded {
                turn_id: *turn_id,
                message_id: message_id.clone(),
                attachment: attachment.into(),
            },
            Domain::FinalPartsRecorded {
                turn_id,
                message_id,
                parts,
            } => Stored::FinalPartsRecorded {
                turn_id: *turn_id,
                message_id: message_id.clone(),
                parts: parts.iter().map(Into::into).collect(),
            },
            Domain::TurnCompleted {
                turn_id,
                exit_code,
                stop_reason,
                token_usage,
            } => Stored::TurnCompleted {
                turn_id: *turn_id,
                exit_code: *exit_code,
                stop_reason: stop_reason.map(Into::into),
                token_usage: token_usage.map(Into::into),
            },
            Domain::TurnInterrupted {
                turn_id,
                reason,
                exit_code,
                error,
            } => Stored::TurnInterrupted {
                turn_id: *turn_id,
                reason: (*reason).into(),
                exit_code: *exit_code,
                error: error.clone(),
            },
            Domain::SessionErrored {
                message_id,
                reason,
                at,
            } => Stored::SessionErrored {
                message_id: message_id.clone(),
                reason: reason.clone(),
                at: *at,
            },
            Domain::SessionClosed { at } => Stored::SessionClosed { at: *at },
            Domain::SendOperationAccepted {
                operation_id,
                disposition,
                human_message_id,
                prompt,
                reserved_turn_id,
                at,
            } => Stored::SendOperationAccepted {
                operation_id: operation_id.clone(),
                disposition: disposition.into(),
                human_message_id: human_message_id.clone(),
                prompt: prompt.as_ref().map(Into::into),
                reserved_turn_id: reserved_turn_id.clone(),
                at: *at,
            },
            Domain::StopOperationAccepted {
                operation_id,
                target_turn_id,
                at,
            } => Stored::StopOperationAccepted {
                operation_id: operation_id.clone(),
                target_turn_id: *target_turn_id,
                at: *at,
            },
            Domain::SessionLifecycleOperationAccepted {
                operation_id,
                kind,
                at,
            } => Stored::SessionLifecycleOperationAccepted {
                operation_id: operation_id.clone(),
                kind: (*kind).into(),
                at: *at,
            },
            Domain::ObligationRecorded {
                obligation_id,
                kind,
                state,
                at,
            } => Stored::ObligationRecorded {
                obligation_id: obligation_id.clone(),
                kind: (*kind).into(),
                state: (*state).into(),
                at: *at,
            },
            Domain::StopResolutionRecorded {
                operation_id,
                turn_id,
                resolution,
                at,
            } => Stored::StopResolutionRecorded {
                operation_id: operation_id.clone(),
                turn_id: *turn_id,
                resolution: (*resolution).into(),
                at: *at,
            },
            Domain::PendingRecoveryPublished {
                obligation_id,
                kind,
                at,
            } => Stored::PendingRecoveryPublished {
                obligation_id: obligation_id.clone(),
                kind: (*kind).into(),
                at: *at,
            },
            Domain::RecoveryActionResolved {
                action_id,
                obligation_id,
                kind,
                classification,
                at,
            } => Stored::RecoveryActionResolved {
                action_id: action_id.clone(),
                obligation_id: obligation_id.clone(),
                kind: (*kind).into(),
                classification: (*classification).into(),
                at: *at,
            },
        }
    }
}

impl TryFrom<StoredAgentSessionEventV1> for AgentSessionEvent {
    type Error = String;
    fn try_from(event: StoredAgentSessionEventV1) -> Result<Self, Self::Error> {
        use AgentSessionEvent as Domain;
        use StoredAgentSessionEventV1 as Stored;
        Ok(match event {
            Stored::BackendSessionRecoveryStarted {
                recovery_id,
                old_provider_session_generation,
                reason,
                at,
            } => Domain::BackendSessionRecoveryStarted {
                recovery_id,
                old_provider_session_generation,
                reason: reason.into(),
                at,
            },
            Stored::SessionConfigurationReactivated {
                recovery_id,
                provider_session_generation,
                consumed_observation_id,
                at,
            } => Domain::SessionConfigurationReactivated {
                recovery_id,
                provider_session_generation,
                consumed_observation_id,
                at,
            },
            Stored::SessionGoalReactivated {
                recovery_id,
                outcome,
                provider_session_generation,
                restoring_turn_id,
                consumed_observation_id,
                at,
            } => Domain::SessionGoalReactivated {
                recovery_id,
                outcome: outcome.into(),
                provider_session_generation,
                restoring_turn_id,
                consumed_observation_id,
                at,
            },
            Stored::BackendSessionRecoveryCompleted {
                recovery_id,
                provider_session_generation,
                at,
            } => Domain::BackendSessionRecoveryCompleted {
                recovery_id,
                provider_session_generation,
                at,
            },
            Stored::BackendSessionRecoveryFailed {
                recovery_id,
                error,
                at,
            } => Domain::BackendSessionRecoveryFailed {
                recovery_id,
                error,
                at,
            },
            Stored::TurnStarted {
                turn_id,
                message_id,
                assistant_message_id,
                prompt,
                at,
            } => Domain::TurnStarted {
                turn_id,
                message_id,
                assistant_message_id,
                prompt: prompt.try_into()?,
                at,
            },
            Stored::TurnInterruptRequested { turn_id, at } => {
                Domain::TurnInterruptRequested { turn_id, at }
            }
            Stored::QueuePaused { at } => Domain::QueuePaused { at },
            Stored::QueueResumed {
                expected_paused_at,
                at,
            } => Domain::QueueResumed {
                expected_paused_at,
                at,
            },
            Stored::TextRecorded {
                turn_id,
                message_id,
                content,
                parent_tool_use_id,
            } => Domain::TextRecorded {
                turn_id,
                message_id,
                content,
                parent_tool_use_id,
            },
            Stored::ReasoningRecorded {
                turn_id,
                message_id,
                content,
                parent_tool_use_id,
            } => Domain::ReasoningRecorded {
                turn_id,
                message_id,
                content,
                parent_tool_use_id,
            },
            Stored::ErrorRecorded {
                turn_id,
                message_id,
                content,
                parent_tool_use_id,
            } => Domain::ErrorRecorded {
                turn_id,
                message_id,
                content,
                parent_tool_use_id,
            },
            Stored::ToolCallStarted {
                turn_id,
                tool_use_id,
                tool,
                input,
                parent_tool_use_id,
            } => Domain::ToolCallStarted {
                turn_id,
                tool_use_id,
                tool,
                input: json_payload(input),
                parent_tool_use_id,
            },
            Stored::ToolCallSucceeded {
                turn_id,
                tool_use_id,
                content,
                content_ref,
                summary,
            } => Domain::ToolCallSucceeded {
                turn_id,
                tool_use_id,
                content,
                content_ref: content_ref.map(Into::into),
                summary: summary.map(Into::into),
            },
            Stored::ToolCallFailed {
                turn_id,
                tool_use_id,
                content,
                content_ref,
                summary,
            } => Domain::ToolCallFailed {
                turn_id,
                tool_use_id,
                content,
                content_ref: content_ref.map(Into::into),
                summary: summary.map(Into::into),
            },
            Stored::ToolResultRecorded {
                turn_id,
                message_id,
                content,
                is_error,
                content_ref,
                summary,
                tool_use_id,
                parent_tool_use_id,
            } => Domain::ToolResultRecorded {
                turn_id,
                message_id,
                content,
                is_error,
                content_ref: content_ref.map(Into::into),
                summary: summary.map(Into::into),
                tool_use_id,
                parent_tool_use_id,
            },
            Stored::ToolCallRetried {
                turn_id,
                tool_use_id,
                attempt,
            } => Domain::ToolCallRetried {
                turn_id,
                tool_use_id,
                attempt,
            },
            Stored::PermissionRequested {
                turn_id,
                tool_use_id,
                request,
            } => Domain::PermissionRequested {
                turn_id,
                tool_use_id,
                request: request
                    .into_domain(PermissionPartStatus::Pending, None)
                    .map_err(|error| error.to_string())?,
            },
            Stored::PermissionResolved {
                turn_id,
                tool_use_id,
                request_id,
                decision,
                answers,
            } => Domain::PermissionResolved {
                turn_id,
                tool_use_id,
                request_id,
                decision: decision.into(),
                answers: answers.map(json_payload),
            },
            Stored::TaskStatusChanged {
                turn_id,
                message_id,
                task_tool_use_id,
                status,
                description,
                summary,
            } => Domain::TaskStatusChanged {
                turn_id,
                message_id,
                task_tool_use_id,
                status,
                description,
                summary,
            },
            Stored::TodoListSnapshotRecorded {
                turn_id,
                message_id,
                items,
            } => Domain::TodoListSnapshotRecorded {
                turn_id,
                message_id,
                items: items.into_iter().map(Into::into).collect(),
            },
            Stored::SystemNotificationRecorded {
                turn_id,
                message_id,
                notification_type,
                status,
                label,
                detail,
                hook_id,
            } => Domain::SystemNotificationRecorded {
                turn_id,
                message_id,
                notification_type: notification_type.into(),
                status,
                label,
                detail,
                hook_id,
            },
            Stored::ImageRecorded {
                turn_id,
                message_id,
                data,
                media_type,
            } => Domain::ImageRecorded {
                turn_id,
                message_id,
                data,
                media_type,
            },
            Stored::ImageRefRecorded {
                turn_id,
                message_id,
                attachment,
            } => Domain::ImageRefRecorded {
                turn_id,
                message_id,
                attachment: attachment.into(),
            },
            Stored::FinalPartsRecorded {
                turn_id,
                message_id,
                parts,
            } => Domain::FinalPartsRecorded {
                turn_id,
                message_id,
                parts: parts
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_, _>>()
                    .map_err(|error: IncompatibleStoredEvent| error.to_string())?,
            },
            Stored::TurnCompleted {
                turn_id,
                exit_code,
                stop_reason,
                token_usage,
            } => Domain::TurnCompleted {
                turn_id,
                exit_code,
                stop_reason: stop_reason.map(Into::into),
                token_usage: token_usage.map(Into::into),
            },
            Stored::TurnInterrupted {
                turn_id,
                reason,
                exit_code,
                error,
            } => Domain::TurnInterrupted {
                turn_id,
                reason: reason.into(),
                exit_code,
                error,
            },
            Stored::SessionErrored {
                message_id,
                reason,
                at,
            } => Domain::SessionErrored {
                message_id,
                reason,
                at,
            },
            Stored::SessionClosed { at } => Domain::SessionClosed { at },
            Stored::SendOperationAccepted {
                operation_id,
                disposition,
                human_message_id,
                prompt,
                reserved_turn_id,
                at,
            } => Domain::SendOperationAccepted {
                operation_id,
                disposition: disposition.into(),
                human_message_id,
                prompt: prompt.map(TryInto::try_into).transpose()?,
                reserved_turn_id,
                at,
            },
            Stored::StopOperationAccepted {
                operation_id,
                target_turn_id,
                at,
            } => Domain::StopOperationAccepted {
                operation_id,
                target_turn_id,
                at,
            },
            Stored::SessionLifecycleOperationAccepted {
                operation_id,
                kind,
                at,
            } => Domain::SessionLifecycleOperationAccepted {
                operation_id,
                kind: kind.into(),
                at,
            },
            Stored::ObligationRecorded {
                obligation_id,
                kind,
                state,
                at,
            } => Domain::ObligationRecorded {
                obligation_id,
                kind: kind.into(),
                state: state.into(),
                at,
            },
            Stored::StopResolutionRecorded {
                operation_id,
                turn_id,
                resolution,
                at,
            } => Domain::StopResolutionRecorded {
                operation_id,
                turn_id,
                resolution: resolution.into(),
                at,
            },
            Stored::PendingRecoveryPublished {
                obligation_id,
                kind,
                at,
            } => Domain::PendingRecoveryPublished {
                obligation_id,
                kind: kind.into(),
                at,
            },
            Stored::RecoveryActionResolved {
                action_id,
                obligation_id,
                kind,
                classification,
                at,
            } => Domain::RecoveryActionResolved {
                action_id,
                obligation_id,
                kind: kind.into(),
                classification: classification.into(),
                at,
            },
        })
    }
}

impl From<&PromptInput> for StoredPromptInputV1 {
    fn from(value: &PromptInput) -> Self {
        Self {
            content: value.content.clone(),
            mentions: value.mentions.iter().map(Into::into).collect(),
            attachment_refs: value.attachment_refs.iter().map(Into::into).collect(),
            parts: value.parts.iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<StoredPromptInputV1> for PromptInput {
    type Error = String;
    fn try_from(value: StoredPromptInputV1) -> Result<Self, Self::Error> {
        Ok(Self {
            content: value.content,
            mentions: value.mentions.into_iter().map(Into::into).collect(),
            attachment_refs: value.attachment_refs.into_iter().map(Into::into).collect(),
            parts: value
                .parts
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()
                .map_err(|error: IncompatibleStoredEvent| error.to_string())?,
        })
    }
}

impl From<&MentionReference> for StoredMentionReferenceV1 {
    fn from(value: &MentionReference) -> Self {
        Self {
            file_path: value.file_path.clone(),
            start_line: value.start_line,
            end_line: value.end_line,
        }
    }
}
impl From<StoredMentionReferenceV1> for MentionReference {
    fn from(value: StoredMentionReferenceV1) -> Self {
        Self {
            file_path: value.file_path,
            start_line: value.start_line,
            end_line: value.end_line,
        }
    }
}
impl From<&Attachment> for StoredAttachmentV1 {
    fn from(value: &Attachment) -> Self {
        Self {
            id: value.id.clone(),
            media_type: value.media_type.clone(),
            byte_size: value.byte_size,
        }
    }
}
impl From<StoredAttachmentV1> for Attachment {
    fn from(value: StoredAttachmentV1) -> Self {
        Self {
            id: value.id,
            media_type: value.media_type,
            byte_size: value.byte_size,
        }
    }
}
impl From<&ToolOutputRef> for StoredToolOutputRefV1 {
    fn from(value: &ToolOutputRef) -> Self {
        Self {
            id: value.id.clone(),
            byte_size: value.byte_size,
        }
    }
}
impl From<StoredToolOutputRefV1> for ToolOutputRef {
    fn from(value: StoredToolOutputRefV1) -> Self {
        Self {
            id: value.id,
            byte_size: value.byte_size,
        }
    }
}
impl From<&ToolOutputSummary> for StoredToolOutputSummaryV1 {
    fn from(value: &ToolOutputSummary) -> Self {
        Self {
            line_count: value.line_count,
            byte_size: value.byte_size,
            is_error: value.is_error,
            truncated: value.truncated,
        }
    }
}
impl From<StoredToolOutputSummaryV1> for ToolOutputSummary {
    fn from(value: StoredToolOutputSummaryV1) -> Self {
        Self {
            line_count: value.line_count,
            byte_size: value.byte_size,
            is_error: value.is_error,
            truncated: value.truncated,
        }
    }
}
impl From<&TodoListItem> for StoredTodoListItemV1 {
    fn from(value: &TodoListItem) -> Self {
        Self {
            text: value.text.clone(),
            completed: value.completed,
        }
    }
}
impl From<StoredTodoListItemV1> for TodoListItem {
    fn from(value: StoredTodoListItemV1) -> Self {
        Self {
            text: value.text,
            completed: value.completed,
        }
    }
}
impl From<SystemNotificationType> for StoredSystemNotificationTypeV1 {
    fn from(value: SystemNotificationType) -> Self {
        match value {
            SystemNotificationType::Compaction => Self::Compaction,
            SystemNotificationType::SessionRecovery => Self::SessionRecovery,
        }
    }
}
impl From<StoredSystemNotificationTypeV1> for SystemNotificationType {
    fn from(value: StoredSystemNotificationTypeV1) -> Self {
        match value {
            StoredSystemNotificationTypeV1::Compaction => Self::Compaction,
            StoredSystemNotificationTypeV1::SessionRecovery => Self::SessionRecovery,
        }
    }
}
impl From<TurnStopReason> for StoredTurnStopReasonV1 {
    fn from(_: TurnStopReason) -> Self {
        Self::Refusal
    }
}
impl From<StoredTurnStopReasonV1> for TurnStopReason {
    fn from(_: StoredTurnStopReasonV1) -> Self {
        Self::Refusal
    }
}
impl From<InterruptReason> for StoredInterruptReasonV1 {
    fn from(value: InterruptReason) -> Self {
        match value {
            InterruptReason::Abort => Self::Abort,
            InterruptReason::Timeout => Self::Timeout,
            InterruptReason::Crash => Self::Crash,
            InterruptReason::SessionClosed => Self::SessionClosed,
        }
    }
}
impl From<StoredInterruptReasonV1> for InterruptReason {
    fn from(value: StoredInterruptReasonV1) -> Self {
        match value {
            StoredInterruptReasonV1::Abort => Self::Abort,
            StoredInterruptReasonV1::Timeout => Self::Timeout,
            StoredInterruptReasonV1::Crash => Self::Crash,
            StoredInterruptReasonV1::SessionClosed => Self::SessionClosed,
        }
    }
}
impl From<PermissionDecision> for StoredPermissionDecisionV1 {
    fn from(value: PermissionDecision) -> Self {
        match value {
            PermissionDecision::Allowed => Self::Allowed,
            PermissionDecision::Denied => Self::Denied,
            PermissionDecision::Cancelled => Self::Cancelled,
        }
    }
}
impl From<StoredPermissionDecisionV1> for PermissionDecision {
    fn from(value: StoredPermissionDecisionV1) -> Self {
        match value {
            StoredPermissionDecisionV1::Allowed => Self::Allowed,
            StoredPermissionDecisionV1::Denied => Self::Denied,
            StoredPermissionDecisionV1::Cancelled => Self::Cancelled,
        }
    }
}
impl From<TurnTokenUsage> for StoredTurnTokenUsageV1 {
    fn from(value: TurnTokenUsage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
        }
    }
}
impl From<StoredTurnTokenUsageV1> for TurnTokenUsage {
    fn from(value: StoredTurnTokenUsageV1) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
        }
    }
}
impl From<BackendSessionRecoveryReason> for StoredBackendSessionRecoveryReasonV1 {
    fn from(value: BackendSessionRecoveryReason) -> Self {
        match value {
            BackendSessionRecoveryReason::ResumeMismatch => Self::ResumeMismatch,
            BackendSessionRecoveryReason::BackendSessionLost => Self::BackendSessionLost,
        }
    }
}
impl From<StoredBackendSessionRecoveryReasonV1> for BackendSessionRecoveryReason {
    fn from(value: StoredBackendSessionRecoveryReasonV1) -> Self {
        match value {
            StoredBackendSessionRecoveryReasonV1::ResumeMismatch => Self::ResumeMismatch,
            StoredBackendSessionRecoveryReasonV1::BackendSessionLost => Self::BackendSessionLost,
        }
    }
}
impl From<&GoalReactivationOutcome> for StoredGoalReactivationOutcomeV1 {
    fn from(value: &GoalReactivationOutcome) -> Self {
        match value {
            GoalReactivationOutcome::NoCurrentGoal => Self::NoCurrentGoal,
            GoalReactivationOutcome::TerminalGoalUnchanged {
                goal_id,
                goal_revision,
            } => Self::TerminalGoalUnchanged {
                goal_id: goal_id.clone(),
                goal_revision: *goal_revision,
            },
            GoalReactivationOutcome::Restored {
                goal_id,
                goal_revision,
                provider_goal_ref,
            } => Self::Restored {
                goal_id: goal_id.clone(),
                goal_revision: *goal_revision,
                provider_goal_ref: provider_goal_ref.clone(),
            },
            GoalReactivationOutcome::ObservedUnchanged {
                goal_id,
                goal_revision,
            } => Self::ObservedUnchanged {
                goal_id: goal_id.clone(),
                goal_revision: *goal_revision,
            },
        }
    }
}
impl From<StoredGoalReactivationOutcomeV1> for GoalReactivationOutcome {
    fn from(value: StoredGoalReactivationOutcomeV1) -> Self {
        match value {
            StoredGoalReactivationOutcomeV1::NoCurrentGoal => Self::NoCurrentGoal,
            StoredGoalReactivationOutcomeV1::TerminalGoalUnchanged {
                goal_id,
                goal_revision,
            } => Self::TerminalGoalUnchanged {
                goal_id,
                goal_revision,
            },
            StoredGoalReactivationOutcomeV1::Restored {
                goal_id,
                goal_revision,
                provider_goal_ref,
            } => Self::Restored {
                goal_id,
                goal_revision,
                provider_goal_ref,
            },
            StoredGoalReactivationOutcomeV1::ObservedUnchanged {
                goal_id,
                goal_revision,
            } => Self::ObservedUnchanged {
                goal_id,
                goal_revision,
            },
        }
    }
}

impl From<&SendDisposition> for StoredSendDispositionV1 {
    fn from(value: &SendDisposition) -> Self {
        match value {
            SendDisposition::StartedTurn { turn_id } => Self::StartedTurn {
                turn_id: turn_id.clone(),
            },
            SendDisposition::Queued { queue_item_id } => Self::Queued {
                queue_item_id: queue_item_id.clone(),
            },
        }
    }
}
impl From<StoredSendDispositionV1> for SendDisposition {
    fn from(value: StoredSendDispositionV1) -> Self {
        match value {
            StoredSendDispositionV1::StartedTurn { turn_id } => Self::StartedTurn { turn_id },
            StoredSendDispositionV1::Queued { queue_item_id } => Self::Queued { queue_item_id },
        }
    }
}
impl From<ObligationKind> for StoredObligationKindV1 {
    fn from(value: ObligationKind) -> Self {
        match value {
            ObligationKind::ProviderEstablish => Self::ProviderEstablish,
            ObligationKind::TurnExecution => Self::TurnExecution,
            ObligationKind::PermissionResponse => Self::PermissionResponse,
            ObligationKind::ProviderInterrupt => Self::ProviderInterrupt,
            ObligationKind::SessionClose => Self::SessionClose,
            ObligationKind::QueuePause => Self::QueuePause,
        }
    }
}
impl From<StoredObligationKindV1> for ObligationKind {
    fn from(value: StoredObligationKindV1) -> Self {
        match value {
            StoredObligationKindV1::ProviderEstablish => Self::ProviderEstablish,
            StoredObligationKindV1::TurnExecution => Self::TurnExecution,
            StoredObligationKindV1::PermissionResponse => Self::PermissionResponse,
            StoredObligationKindV1::ProviderInterrupt => Self::ProviderInterrupt,
            StoredObligationKindV1::SessionClose => Self::SessionClose,
            StoredObligationKindV1::QueuePause => Self::QueuePause,
        }
    }
}
impl From<ObligationState> for StoredObligationStateV1 {
    fn from(value: ObligationState) -> Self {
        match value {
            ObligationState::Pending => Self::Pending,
            ObligationState::EffectReserved => Self::EffectReserved,
            ObligationState::Completed => Self::Completed,
            ObligationState::ReconciliationRequired => Self::ReconciliationRequired,
            ObligationState::Cancelled => Self::Cancelled,
        }
    }
}
impl From<StoredObligationStateV1> for ObligationState {
    fn from(value: StoredObligationStateV1) -> Self {
        match value {
            StoredObligationStateV1::Pending => Self::Pending,
            StoredObligationStateV1::EffectReserved => Self::EffectReserved,
            StoredObligationStateV1::Completed => Self::Completed,
            StoredObligationStateV1::ReconciliationRequired => Self::ReconciliationRequired,
            StoredObligationStateV1::Cancelled => Self::Cancelled,
        }
    }
}
impl From<StopResolution> for StoredStopResolutionV1 {
    fn from(value: StopResolution) -> Self {
        match value {
            StopResolution::Succeeded => Self::Succeeded,
            StopResolution::Superseded => Self::Superseded,
        }
    }
}
impl From<StoredStopResolutionV1> for StopResolution {
    fn from(value: StoredStopResolutionV1) -> Self {
        match value {
            StoredStopResolutionV1::Succeeded => Self::Succeeded,
            StoredStopResolutionV1::Superseded => Self::Superseded,
        }
    }
}
impl From<SessionLifecycleKind> for StoredSessionLifecycleKindV1 {
    fn from(value: SessionLifecycleKind) -> Self {
        match value {
            SessionLifecycleKind::Close => Self::Close,
            SessionLifecycleKind::Archive => Self::Archive,
            SessionLifecycleKind::BackendSwitch => Self::BackendSwitch,
        }
    }
}
impl From<StoredSessionLifecycleKindV1> for SessionLifecycleKind {
    fn from(value: StoredSessionLifecycleKindV1) -> Self {
        match value {
            StoredSessionLifecycleKindV1::Close => Self::Close,
            StoredSessionLifecycleKindV1::Archive => Self::Archive,
            StoredSessionLifecycleKindV1::BackendSwitch => Self::BackendSwitch,
        }
    }
}
impl From<RecoveryActionKind> for StoredRecoveryActionKindV1 {
    fn from(value: RecoveryActionKind) -> Self {
        match value {
            RecoveryActionKind::ReadAgain => Self::ReadAgain,
            RecoveryActionKind::RetrySameEffect => Self::RetrySameEffect,
            RecoveryActionKind::UseObservedResult => Self::UseObservedResult,
            RecoveryActionKind::CancelIfSafe => Self::CancelIfSafe,
            RecoveryActionKind::KeepForManualResolution => Self::KeepForManualResolution,
        }
    }
}
impl From<StoredRecoveryActionKindV1> for RecoveryActionKind {
    fn from(value: StoredRecoveryActionKindV1) -> Self {
        match value {
            StoredRecoveryActionKindV1::ReadAgain => Self::ReadAgain,
            StoredRecoveryActionKindV1::RetrySameEffect => Self::RetrySameEffect,
            StoredRecoveryActionKindV1::UseObservedResult => Self::UseObservedResult,
            StoredRecoveryActionKindV1::CancelIfSafe => Self::CancelIfSafe,
            StoredRecoveryActionKindV1::KeepForManualResolution => Self::KeepForManualResolution,
        }
    }
}
impl From<RecoveryResultClassification> for StoredRecoveryResultClassificationV1 {
    fn from(value: RecoveryResultClassification) -> Self {
        match value {
            RecoveryResultClassification::Pending => Self::Pending,
            RecoveryResultClassification::Succeeded => Self::Succeeded,
            RecoveryResultClassification::ConfirmedNoEffect => Self::ConfirmedNoEffect,
            RecoveryResultClassification::Ambiguous => Self::Ambiguous,
            RecoveryResultClassification::CancelledBeforeEffect => Self::CancelledBeforeEffect,
            RecoveryResultClassification::Unchanged => Self::Unchanged,
        }
    }
}
impl From<StoredRecoveryResultClassificationV1> for RecoveryResultClassification {
    fn from(value: StoredRecoveryResultClassificationV1) -> Self {
        match value {
            StoredRecoveryResultClassificationV1::Pending => Self::Pending,
            StoredRecoveryResultClassificationV1::Succeeded => Self::Succeeded,
            StoredRecoveryResultClassificationV1::ConfirmedNoEffect => Self::ConfirmedNoEffect,
            StoredRecoveryResultClassificationV1::Ambiguous => Self::Ambiguous,
            StoredRecoveryResultClassificationV1::CancelledBeforeEffect => {
                Self::CancelledBeforeEffect
            }
            StoredRecoveryResultClassificationV1::Unchanged => Self::Unchanged,
        }
    }
}

fn json_payload(value: serde_json::Value) -> JsonPayload {
    JsonPayload::new_unchecked(
        serde_json::to_string(&value).expect("JSON value serialization cannot fail"),
    )
}

fn json_value(value: &JsonPayload) -> serde_json::Value {
    serde_json::from_str(value.as_str())
        .expect("domain JsonPayload must be validated at its boundary")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::entities::{
        MessagePart, PermissionRequest, PermissionRequestBody, PermissionRequestStatus,
    };

    fn source() -> StoredPayloadSource {
        StoredPayloadSource {
            source_id: "sessions/s-1/events/1.json".into(),
            record_ordinal: Some(1),
        }
    }

    fn pending_permission() -> PermissionRequest {
        PermissionRequest {
            id: "permission-1".to_string(),
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
            tool_name: "Bash".to_string(),
            body: PermissionRequestBody::ToolApproval {
                input: json_payload(serde_json::json!({"command": "cargo test"})),
            },
            title: None,
            display_name: None,
            description: None,
            decision_reason: None,
            status: PermissionRequestStatus::Pending,
        }
    }

    fn all_domain_events() -> Vec<AgentSessionEvent> {
        use AgentSessionEvent as E;
        vec![
            E::BackendSessionRecoveryStarted {
                recovery_id: "r-1".into(),
                old_provider_session_generation: 1,
                reason: BackendSessionRecoveryReason::ResumeMismatch,
                at: 1.0,
            },
            E::SessionConfigurationReactivated {
                recovery_id: "r-1".into(),
                provider_session_generation: 2,
                consumed_observation_id: Some("o-1".into()),
                at: 1.0,
            },
            E::SessionGoalReactivated {
                recovery_id: "r-1".into(),
                outcome: GoalReactivationOutcome::Restored {
                    goal_id: "g-1".into(),
                    goal_revision: 3,
                    provider_goal_ref: None,
                },
                provider_session_generation: 2,
                restoring_turn_id: None,
                consumed_observation_id: None,
                at: 1.0,
            },
            E::BackendSessionRecoveryCompleted {
                recovery_id: "r-1".into(),
                provider_session_generation: 2,
                at: 1.0,
            },
            E::BackendSessionRecoveryFailed {
                recovery_id: "r-1".into(),
                error: "boom".into(),
                at: 1.0,
            },
            E::TurnStarted {
                turn_id: 1,
                message_id: "m-1".into(),
                assistant_message_id: Some("m-2".into()),
                prompt: PromptInput {
                    content: "hi".into(),
                    mentions: vec![MentionReference {
                        file_path: "src/lib.rs".into(),
                        start_line: Some(1),
                        end_line: None,
                    }],
                    attachment_refs: vec![Attachment {
                        id: "a-1".into(),
                        media_type: "image/png".into(),
                        byte_size: 1,
                    }],
                    parts: vec![MessagePart::Text {
                        content: "hi".into(),
                        parent_tool_use_id: None,
                    }],
                },
                at: 1.0,
            },
            E::TurnInterruptRequested {
                turn_id: 1,
                at: 1.0,
            },
            E::QueuePaused { at: 1.0 },
            E::QueueResumed {
                expected_paused_at: 1.0,
                at: 2.0,
            },
            E::TextRecorded {
                turn_id: 1,
                message_id: "m-2".into(),
                content: "text".into(),
                parent_tool_use_id: None,
            },
            E::ReasoningRecorded {
                turn_id: 1,
                message_id: "m-2".into(),
                content: "think".into(),
                parent_tool_use_id: Some("t-0".into()),
            },
            E::ErrorRecorded {
                turn_id: 1,
                message_id: "m-2".into(),
                content: "err".into(),
                parent_tool_use_id: None,
            },
            E::ToolCallStarted {
                turn_id: 1,
                tool_use_id: "t-1".into(),
                tool: "Read".into(),
                input: json_payload(serde_json::json!({"path": "a.rs"})),
                parent_tool_use_id: None,
            },
            E::ToolCallSucceeded {
                turn_id: 1,
                tool_use_id: "t-1".into(),
                content: "ok".into(),
                content_ref: Some(ToolOutputRef {
                    id: "blob-1".into(),
                    byte_size: 2,
                }),
                summary: Some(ToolOutputSummary {
                    line_count: 1,
                    byte_size: 2,
                    is_error: false,
                    truncated: false,
                }),
            },
            E::ToolCallFailed {
                turn_id: 1,
                tool_use_id: "t-1".into(),
                content: "ng".into(),
                content_ref: None,
                summary: None,
            },
            E::ToolResultRecorded {
                turn_id: 1,
                message_id: "m-2".into(),
                content: "out".into(),
                is_error: false,
                content_ref: None,
                summary: None,
                tool_use_id: Some("t-1".into()),
                parent_tool_use_id: None,
            },
            E::ToolCallRetried {
                turn_id: 1,
                tool_use_id: "t-1".into(),
                attempt: 2,
            },
            E::PermissionRequested {
                turn_id: 1,
                tool_use_id: Some("t-1".into()),
                request: pending_permission(),
            },
            E::PermissionResolved {
                turn_id: 1,
                tool_use_id: Some("t-1".into()),
                request_id: Some("permission-1".into()),
                decision: PermissionDecision::Allowed,
                answers: Some(json_payload(serde_json::json!({"approved": true}))),
            },
            E::TaskStatusChanged {
                turn_id: 1,
                message_id: "m-2".into(),
                task_tool_use_id: "task-1".into(),
                status: "completed".into(),
                description: Some("done".into()),
                summary: None,
            },
            E::TodoListSnapshotRecorded {
                turn_id: 1,
                message_id: "m-2".into(),
                items: vec![TodoListItem {
                    text: "todo".into(),
                    completed: false,
                }],
            },
            E::SystemNotificationRecorded {
                turn_id: 1,
                message_id: "m-2".into(),
                notification_type: SystemNotificationType::Compaction,
                status: "done".into(),
                label: "Compacted".into(),
                detail: None,
                hook_id: None,
            },
            E::ImageRecorded {
                turn_id: 1,
                message_id: "m-2".into(),
                data: "AA==".into(),
                media_type: "image/png".into(),
            },
            E::ImageRefRecorded {
                turn_id: 1,
                message_id: "m-2".into(),
                attachment: Attachment {
                    id: "a-1".into(),
                    media_type: "image/png".into(),
                    byte_size: 1,
                },
            },
            E::FinalPartsRecorded {
                turn_id: 1,
                message_id: "m-2".into(),
                parts: vec![MessagePart::Text {
                    content: "final".into(),
                    parent_tool_use_id: None,
                }],
            },
            E::TurnCompleted {
                turn_id: 1,
                exit_code: 0,
                stop_reason: Some(TurnStopReason::Refusal),
                token_usage: Some(TurnTokenUsage {
                    input_tokens: 1,
                    output_tokens: 2,
                }),
            },
            E::TurnInterrupted {
                turn_id: 1,
                reason: InterruptReason::Timeout,
                exit_code: 1,
                error: Some("timeout".into()),
            },
            E::SessionErrored {
                message_id: "m-2".into(),
                reason: "fatal".into(),
                at: 1.0,
            },
            E::SessionClosed { at: 1.0 },
            E::SendOperationAccepted {
                operation_id: "op-1".into(),
                disposition: SendDisposition::StartedTurn {
                    turn_id: "7".into(),
                },
                human_message_id: None,
                prompt: None,
                reserved_turn_id: None,
                at: 1.0,
            },
            E::StopOperationAccepted {
                operation_id: "op-2".into(),
                target_turn_id: 7,
                at: 1.0,
            },
            E::SessionLifecycleOperationAccepted {
                operation_id: "op-3".into(),
                kind: SessionLifecycleKind::BackendSwitch,
                at: 1.0,
            },
            E::ObligationRecorded {
                obligation_id: "ob-1".into(),
                kind: ObligationKind::ProviderEstablish,
                state: ObligationState::EffectReserved,
                at: 1.0,
            },
            E::StopResolutionRecorded {
                operation_id: "op-2".into(),
                turn_id: 7,
                resolution: StopResolution::Superseded,
                at: 2.0,
            },
            E::PendingRecoveryPublished {
                obligation_id: "ob-1".into(),
                kind: ObligationKind::TurnExecution,
                at: 2.0,
            },
            E::RecoveryActionResolved {
                action_id: "act-1".into(),
                obligation_id: "ob-1".into(),
                kind: RecoveryActionKind::RetrySameEffect,
                classification: RecoveryResultClassification::Succeeded,
                at: 3.0,
            },
        ]
    }

    #[test]
    fn all_domain_event_variants_round_trip_through_stored_v1_without_semantic_loss() {
        for event in all_domain_events() {
            let stored = StoredAgentSessionEventV1::from(&event);
            let json = serde_json::to_vec(&stored).unwrap();
            let decoded: StoredAgentSessionEventV1 = serde_json::from_slice(&json).unwrap();
            assert_eq!(AgentSessionEvent::try_from(decoded).unwrap(), event);
        }
    }

    #[test]
    fn issues_1499_operation_variants_have_fixed_snake_case_tags() {
        let event = AgentSessionEvent::SendOperationAccepted {
            operation_id: "op-1".into(),
            disposition: SendDisposition::Queued {
                queue_item_id: "q-1".into(),
            },
            human_message_id: None,
            prompt: None,
            reserved_turn_id: None,
            at: 1.0,
        };
        let json = serde_json::to_value(StoredAgentSessionEventV1::from(&event)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "send_operation_accepted",
                "operation_id": "op-1",
                "disposition": { "type": "queued", "queue_item_id": "q-1" },
                "at": 1.0,
            })
        );
        let event = AgentSessionEvent::RecoveryActionResolved {
            action_id: "act-1".into(),
            obligation_id: "ob-1".into(),
            kind: RecoveryActionKind::KeepForManualResolution,
            classification: RecoveryResultClassification::ConfirmedNoEffect,
            at: 2.0,
        };
        let json = serde_json::to_value(StoredAgentSessionEventV1::from(&event)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "recovery_action_resolved",
                "action_id": "act-1",
                "obligation_id": "ob-1",
                "kind": "keep_for_manual_resolution",
                "classification": "confirmed_no_effect",
                "at": 2.0,
            })
        );
    }

    #[test]
    fn known_legacy_event_spelling_and_optional_omission_remain_stable() {
        let event = AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "t-1".into(),
            content: "ok".into(),
            content_ref: None,
            summary: None,
        };
        assert_eq!(
            serde_json::to_string(&StoredAgentSessionEventV1::from(&event)).unwrap(),
            r#"{"type":"tool_call_succeeded","turn_id":1,"tool_use_id":"t-1","content":"ok"}"#
        );
        let event = AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "t-1".into(),
            content: "ok".into(),
            content_ref: Some(ToolOutputRef {
                id: "blob-1".into(),
                byte_size: 2,
            }),
            summary: None,
        };
        assert_eq!(
            serde_json::to_string(&StoredAgentSessionEventV1::from(&event)).unwrap(),
            r#"{"type":"tool_call_succeeded","turn_id":1,"tool_use_id":"t-1","content":"ok","contentRef":{"id":"blob-1","byteSize":2}}"#
        );
    }

    #[test]
    fn additive_event_payload_is_raw_preserved_and_decoded_to_domain() {
        let raw = br#"{"type":"queue_paused","at":1.0,"future":{"value":1}}"#;
        let decoded = decode_stored_agent_session_event_v1(raw, 1, source()).unwrap();
        assert!(matches!(
            decoded.event,
            AgentSessionEvent::QueuePaused { at: 1.0 }
        ));
        assert_eq!(decoded.preserved_additive_payload.unwrap().raw_bytes, raw);
    }

    #[test]
    fn nested_additive_event_payload_is_raw_preserved() {
        let raw = br#"{"type":"turn_started","turn_id":1,"message_id":"m","prompt":{"content":"hi","future":true},"at":1.0}"#;
        let decoded = decode_stored_agent_session_event_v1(raw, 1, source()).unwrap();
        assert_eq!(decoded.preserved_additive_payload.unwrap().raw_bytes, raw);
    }

    #[test]
    fn unknown_required_event_variant_and_version_fail_closed() {
        let variant =
            decode_stored_agent_session_event_v1(br#"{"type":"future_required"}"#, 1, source())
                .unwrap_err();
        assert_eq!(variant.type_tag, "future_required");
        let version = decode_stored_agent_session_event_v1(
            br#"{"type":"queue_paused","at":1.0}"#,
            2,
            source(),
        )
        .unwrap_err();
        assert_eq!(version.payload_version, 2);
    }
}
