//! Canonical closed event vocabulary for an agent session.
//!
//! Persistence envelopes and public DTOs convert to this type at adapter boundaries;
//! this module deliberately has no serde, filesystem, database, or transport dependency.

use crate::domain::agent_session::entities::{Attachment, MessagePart, PermissionRequest};
pub use crate::domain::agent_session::entities::{InterruptReason, TurnStopReason};
use crate::domain::agent_session::value_objects::{
    JsonPayload, SystemNotificationType, TodoListItem, ToolOutputRef, ToolOutputSummary,
};
use crate::domain::code::MentionReference;
use crate::domain::provider_lifecycle::ProviderKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderSessionOwnershipEvent {
    Claimed {
        provider: ProviderKind,
        provider_session_id: String,
        agent_session_id: String,
    },
    Released {
        provider: ProviderKind,
        provider_session_id: String,
        agent_session_id: String,
    },
}

pub type TurnId = u64;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PromptInput {
    pub content: String,
    pub mentions: Vec<MentionReference>,
    pub attachment_refs: Vec<Attachment>,
    pub parts: Vec<MessagePart>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Allowed,
    Denied,
    Cancelled,
}

impl PermissionDecision {
    pub fn from_status(status: &str) -> Option<Self> {
        match status {
            "allowed" | "allow" => Some(Self::Allowed),
            "denied" | "deny" => Some(Self::Denied),
            "cancelled" | "canceled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn status(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl TurnTokenUsage {
    pub fn total_tokens(self) -> Option<u64> {
        self.input_tokens.checked_add(self.output_tokens)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendSessionRecoveryReason {
    ResumeMismatch,
    BackendSessionLost,
}

/// issues-1499: one-shot send disposition decided at acceptance commit time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendDisposition {
    StartedTurn { turn_id: String },
    Queued { queue_item_id: String },
}

/// issues-1499: durable work reserved before an external effect starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObligationKind {
    ProviderEstablish,
    TurnExecution,
    /// Exact permission response reserved before the provider receives it.
    PermissionResponse,
    ProviderInterrupt,
    SessionClose,
    QueuePause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObligationState {
    Pending,
    EffectReserved,
    Completed,
    ReconciliationRequired,
    Cancelled,
}

/// issues-1499: terminal resolution of an accepted Stop operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopResolution {
    Succeeded,
    Superseded,
}

/// issues-1499: backend-owned session lifecycle operations. View close never
/// creates a backend operation and is deliberately not part of this set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLifecycleKind {
    Close,
    Archive,
    BackendSwitch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryActionKind {
    ReadAgain,
    RetrySameEffect,
    UseObservedResult,
    CancelIfSafe,
    KeepForManualResolution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryResultClassification {
    Pending,
    Succeeded,
    ConfirmedNoEffect,
    Ambiguous,
    CancelledBeforeEffect,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalReactivationOutcome {
    NoCurrentGoal,
    TerminalGoalUnchanged {
        goal_id: String,
        goal_revision: u64,
    },
    Restored {
        goal_id: String,
        goal_revision: u64,
        provider_goal_ref: Option<String>,
    },
    ObservedUnchanged {
        goal_id: String,
        goal_revision: u64,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub enum AgentSessionDomainEvent {
    BackendSessionRecoveryStarted {
        recovery_id: String,
        old_provider_session_generation: u64,
        reason: BackendSessionRecoveryReason,
        at: f64,
    },
    SessionConfigurationReactivated {
        recovery_id: String,
        provider_session_generation: u64,
        consumed_observation_id: Option<String>,
        at: f64,
    },
    SessionGoalReactivated {
        recovery_id: String,
        outcome: GoalReactivationOutcome,
        provider_session_generation: u64,
        restoring_turn_id: Option<String>,
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
        turn_id: TurnId,
        message_id: String,
        assistant_message_id: Option<String>,
        prompt: PromptInput,
        at: f64,
    },
    TurnInterruptRequested {
        turn_id: TurnId,
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
        turn_id: TurnId,
        message_id: String,
        content: String,
        parent_tool_use_id: Option<String>,
    },
    ReasoningRecorded {
        turn_id: TurnId,
        message_id: String,
        content: String,
        parent_tool_use_id: Option<String>,
    },
    ErrorRecorded {
        turn_id: TurnId,
        message_id: String,
        content: String,
        parent_tool_use_id: Option<String>,
    },
    ToolCallStarted {
        turn_id: TurnId,
        tool_use_id: String,
        tool: String,
        input: JsonPayload,
        parent_tool_use_id: Option<String>,
    },
    ToolCallSucceeded {
        turn_id: TurnId,
        tool_use_id: String,
        content: String,
        content_ref: Option<ToolOutputRef>,
        summary: Option<ToolOutputSummary>,
    },
    ToolCallFailed {
        turn_id: TurnId,
        tool_use_id: String,
        content: String,
        content_ref: Option<ToolOutputRef>,
        summary: Option<ToolOutputSummary>,
    },
    ToolResultRecorded {
        turn_id: TurnId,
        message_id: String,
        content: String,
        is_error: bool,
        content_ref: Option<ToolOutputRef>,
        summary: Option<ToolOutputSummary>,
        tool_use_id: Option<String>,
        parent_tool_use_id: Option<String>,
    },
    ToolCallRetried {
        turn_id: TurnId,
        tool_use_id: String,
        attempt: u32,
    },
    PermissionRequested {
        turn_id: TurnId,
        tool_use_id: Option<String>,
        request: PermissionRequest,
    },
    PermissionResolved {
        turn_id: TurnId,
        tool_use_id: Option<String>,
        request_id: Option<String>,
        decision: PermissionDecision,
        answers: Option<JsonPayload>,
    },
    TaskStatusChanged {
        turn_id: TurnId,
        message_id: String,
        task_tool_use_id: String,
        status: String,
        description: Option<String>,
        summary: Option<String>,
    },
    TodoListSnapshotRecorded {
        turn_id: TurnId,
        message_id: String,
        items: Vec<TodoListItem>,
    },
    SystemNotificationRecorded {
        turn_id: TurnId,
        message_id: String,
        notification_type: SystemNotificationType,
        status: String,
        label: String,
        detail: Option<String>,
        hook_id: Option<String>,
    },
    ImageRecorded {
        turn_id: TurnId,
        message_id: String,
        data: String,
        media_type: String,
    },
    ImageRefRecorded {
        turn_id: TurnId,
        message_id: String,
        attachment: Attachment,
    },
    FinalPartsRecorded {
        turn_id: TurnId,
        message_id: String,
        parts: Vec<MessagePart>,
    },
    TurnCompleted {
        turn_id: TurnId,
        exit_code: i64,
        stop_reason: Option<TurnStopReason>,
        token_usage: Option<TurnTokenUsage>,
    },
    TurnInterrupted {
        turn_id: TurnId,
        reason: InterruptReason,
        exit_code: i64,
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
    // issues-1499: operation acceptance, durable obligation, Stop resolution,
    // session lifecycle, and recovery publication facts committed by the local
    // event store. These closed variants share this enum with the F1 streaming
    // vocabulary so one domain type owns the whole session event stream.
    SendOperationAccepted {
        operation_id: String,
        disposition: SendDisposition,
        /// Canonical semantic input saved with acceptance. `None` is only
        /// decoded for pre-#1499 additive records during schema evolution.
        human_message_id: Option<String>,
        prompt: Option<PromptInput>,
        reserved_turn_id: Option<String>,
        at: f64,
    },
    StopOperationAccepted {
        operation_id: String,
        target_turn_id: TurnId,
        at: f64,
    },
    SessionLifecycleOperationAccepted {
        operation_id: String,
        kind: SessionLifecycleKind,
        at: f64,
    },
    ObligationRecorded {
        obligation_id: String,
        kind: ObligationKind,
        state: ObligationState,
        at: f64,
    },
    StopResolutionRecorded {
        operation_id: String,
        turn_id: TurnId,
        resolution: StopResolution,
        at: f64,
    },
    PendingRecoveryPublished {
        obligation_id: String,
        kind: ObligationKind,
        at: f64,
    },
    RecoveryActionResolved {
        action_id: String,
        obligation_id: String,
        kind: RecoveryActionKind,
        classification: RecoveryResultClassification,
        at: f64,
    },
}
