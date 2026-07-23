//! Versioned public DTOs for agent-session command results and notifications.

use serde::{Deserialize, Serialize};

use super::agent::{
    JsonValueDtoV1, MessagePartDtoV1, PermissionAllowedPromptDtoV1, PermissionQuestionDtoV1,
    PermissionQuestionOptionDtoV1, PermissionRequestDtoV1, PermissionRequestKindDtoV1,
    ToolOutputRefDtoV1, ToolOutputSummaryDtoV1,
};
use crate::usecase::agent_session::runtime::usecase::InitSessionsResponse;
use crate::usecase::agent_session::session::{
    ActivityEntry, ChatMessage, ChatSession, ContextCarryState, GetSessionResponse,
    InitialSessionPage, MessageMention, MessagePageMetadata, MessageRole, ModelInfo,
    PermissionRequestKindMsg, PermissionRequestMsg, QueuedAgentTurn, SessionPage, SessionState,
    SessionSummary, TokenUsage, TurnInterruption, TurnInterruptionReason, WorkflowNodeContextDto,
};
use crate::usecase::agent_session::status::TurnPhase;

/// Decode the canonical public representation of a persisted non-negative
/// integer. Public semantic integers are deliberately strings: accepting the
/// Rust parser's `+1`, `01`, whitespace, or values above SQLite's signed
/// range would make Tauri and WebSocket requests non-canonical.
pub(crate) fn decode_nonnegative_i64_decimal(raw: &str) -> Option<i64> {
    if raw.is_empty()
        || (raw.len() > 1 && raw.starts_with('0'))
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    raw.parse::<i64>().ok()
}

pub(crate) fn decode_nonnegative_u64_decimal(raw: &str) -> Option<u64> {
    decode_nonnegative_i64_decimal(raw).map(|value| value as u64)
}

/// Turn identities are positive SQLite-bounded semantic integers. Keep this
/// decoder next to the non-negative public decoder so Tauri and WebSocket
/// cannot drift on zero, leading-zero, sign, Unicode, or overflow handling.
pub(crate) fn decode_positive_i64_decimal(raw: &str) -> Option<i64> {
    decode_nonnegative_i64_decimal(raw).filter(|value| *value > 0)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StopOperationRequestDtoV1 {
    pub request_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub expected_session_revision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StopOperationReceiptDtoV1 {
    pub operation_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub accepted_revision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopResolutionDtoV1 {
    Succeeded,
    Superseded,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StopOperationStateDtoV1 {
    Accepted,
    Completed { resolution: StopResolutionDtoV1 },
    ReconciliationRequired { failure: SafeOperationFailureDtoV1 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StopCommandOutcomeDtoV1 {
    Accepted {
        receipt: StopOperationReceiptDtoV1,
        state: StopOperationStateDtoV1,
    },
    RejectedBeforeCommit {
        failure: SafeOperationFailureDtoV1,
    },
    OutcomeUnknown {
        request_id: String,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SessionLifecycleActionDtoV1 {
    Close,
    ArchiveOpen,
    ArchiveClosed,
    SwitchBackend { backend_id: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionLifecycleRequestDtoV1 {
    pub request_id: String,
    pub session_id: String,
    pub expected_session_revision: String,
    pub action: SessionLifecycleActionDtoV1,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionLifecycleReceiptDtoV1 {
    pub operation_id: String,
    pub session_id: String,
    pub action: String,
    pub backend_id: Option<String>,
    pub first_accepted_revision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingCallerAttemptDtoV1 {
    pub kind: String,
    pub caller_request_id: String,
    pub operation_id: Option<String>,
    pub resolution: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingCallerAttemptPageDtoV1 {
    pub entries: Vec<PendingCallerAttemptDtoV1>,
    pub next_cursor: Option<String>,
}

impl From<crate::usecase::agent_session::operation::PendingCallerAttemptPage>
    for PendingCallerAttemptPageDtoV1
{
    fn from(value: crate::usecase::agent_session::operation::PendingCallerAttemptPage) -> Self {
        Self {
            entries: value.entries.into_iter().map(Into::into).collect(),
            next_cursor: value.next_cursor,
        }
    }
}

impl From<crate::usecase::agent_session::operation::PendingCallerAttempt>
    for PendingCallerAttemptDtoV1
{
    fn from(value: crate::usecase::agent_session::operation::PendingCallerAttempt) -> Self {
        Self {
            kind: value.kind.label().to_string(),
            caller_request_id: value.caller_request_id,
            operation_id: value.operation_id,
            resolution: value.resolution.label().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SessionLifecycleStateDtoV1 {
    Accepted,
    Completed,
    ReconciliationRequired { failure: SafeOperationFailureDtoV1 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SessionLifecycleRejectionDtoV1 {
    Busy,
    PendingOperation,
    RevisionConflict { current_revision: String },
    InvalidState,
    Failed { failure: SafeOperationFailureDtoV1 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SessionLifecycleCommandResultDtoV1 {
    Accepted {
        receipt: SessionLifecycleReceiptDtoV1,
        state: SessionLifecycleStateDtoV1,
    },
    Rejected {
        rejection: SessionLifecycleRejectionDtoV1,
    },
    OutcomeUnknown {
        request_id: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SafeOperationFailureDtoV1 {
    pub kind: String,
    pub retryable: bool,
    pub label: String,
    pub detail: Option<String>,
    pub correlation_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum OperationApplicationErrorDtoV1 {
    InvalidRequest,
    PayloadConflict,
    NotFound,
    CapacityExceeded,
    RateLimited,
    RequestIdConflict,
    MigrationInProgress,
    ShutdownInProgress,
    FeedbackCapacityExceeded,
    StaleTarget,
    RevisionConflict { current_revision: String },
    CursorMismatch,
    CursorExpired,
    SnapshotMismatch,
    DetailsCompacted,
    QueryBusy,
    DeadlineExceeded,
    ResponseTooLarge,
    OutcomeUnknown { operation_id: String },
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

/// The public Tauri surface uses endpoint-owned closed error enums.  The
/// WebSocket envelope keeps `OperationApplicationErrorDtoV1` as its tagged
/// union, but each route can only construct the variants admitted by its
/// corresponding endpoint enum.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApplicationQuitErrorDtoV1 {
    InvalidRequest,
    PayloadConflict,
    CapacityExceeded,
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApplicationQuitLookupErrorDtoV1 {
    InvalidRequest,
    NotFound,
    QueryBusy,
    DeadlineExceeded,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum CurrentShutdownErrorDtoV1 {
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ShutdownPlanQueryErrorDtoV1 {
    InvalidRequest,
    NotFound,
    DetailsCompacted,
    CursorMismatch,
    CursorExpired,
    QueryBusy,
    DeadlineExceeded,
    ResponseTooLarge,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum MigrationQueryErrorDtoV1 {
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ShutdownDetailsMutationErrorDtoV1 {
    InvalidRequest,
    Internal { correlation_id: String },
}

impl From<ApplicationQuitErrorDtoV1> for OperationApplicationErrorDtoV1 {
    fn from(value: ApplicationQuitErrorDtoV1) -> Self {
        match value {
            ApplicationQuitErrorDtoV1::InvalidRequest => Self::InvalidRequest,
            ApplicationQuitErrorDtoV1::PayloadConflict => Self::PayloadConflict,
            ApplicationQuitErrorDtoV1::CapacityExceeded => Self::CapacityExceeded,
            ApplicationQuitErrorDtoV1::Internal { correlation_id } => {
                Self::Internal { correlation_id }
            }
        }
    }
}

impl From<ApplicationQuitLookupErrorDtoV1> for OperationApplicationErrorDtoV1 {
    fn from(value: ApplicationQuitLookupErrorDtoV1) -> Self {
        match value {
            ApplicationQuitLookupErrorDtoV1::InvalidRequest => Self::InvalidRequest,
            ApplicationQuitLookupErrorDtoV1::NotFound => Self::NotFound,
            ApplicationQuitLookupErrorDtoV1::QueryBusy => Self::QueryBusy,
            ApplicationQuitLookupErrorDtoV1::DeadlineExceeded => Self::DeadlineExceeded,
            ApplicationQuitLookupErrorDtoV1::StorageUnavailable { failure } => {
                Self::StorageUnavailable { failure }
            }
            ApplicationQuitLookupErrorDtoV1::Internal { correlation_id } => {
                Self::Internal { correlation_id }
            }
        }
    }
}

impl From<CurrentShutdownErrorDtoV1> for OperationApplicationErrorDtoV1 {
    fn from(value: CurrentShutdownErrorDtoV1) -> Self {
        match value {
            CurrentShutdownErrorDtoV1::Internal { correlation_id } => {
                Self::Internal { correlation_id }
            }
        }
    }
}

impl From<ShutdownPlanQueryErrorDtoV1> for OperationApplicationErrorDtoV1 {
    fn from(value: ShutdownPlanQueryErrorDtoV1) -> Self {
        match value {
            ShutdownPlanQueryErrorDtoV1::InvalidRequest => Self::InvalidRequest,
            ShutdownPlanQueryErrorDtoV1::NotFound => Self::NotFound,
            ShutdownPlanQueryErrorDtoV1::DetailsCompacted => Self::DetailsCompacted,
            ShutdownPlanQueryErrorDtoV1::CursorMismatch => Self::CursorMismatch,
            ShutdownPlanQueryErrorDtoV1::CursorExpired => Self::CursorExpired,
            ShutdownPlanQueryErrorDtoV1::QueryBusy => Self::QueryBusy,
            ShutdownPlanQueryErrorDtoV1::DeadlineExceeded => Self::DeadlineExceeded,
            ShutdownPlanQueryErrorDtoV1::ResponseTooLarge => Self::ResponseTooLarge,
            ShutdownPlanQueryErrorDtoV1::StorageUnavailable { failure } => {
                Self::StorageUnavailable { failure }
            }
            ShutdownPlanQueryErrorDtoV1::Internal { correlation_id } => {
                Self::Internal { correlation_id }
            }
        }
    }
}

impl From<MigrationQueryErrorDtoV1> for OperationApplicationErrorDtoV1 {
    fn from(value: MigrationQueryErrorDtoV1) -> Self {
        match value {
            MigrationQueryErrorDtoV1::StorageUnavailable { failure } => {
                Self::StorageUnavailable { failure }
            }
            MigrationQueryErrorDtoV1::Internal { correlation_id } => {
                Self::Internal { correlation_id }
            }
        }
    }
}

impl From<ShutdownDetailsMutationErrorDtoV1> for OperationApplicationErrorDtoV1 {
    fn from(value: ShutdownDetailsMutationErrorDtoV1) -> Self {
        match value {
            ShutdownDetailsMutationErrorDtoV1::InvalidRequest => Self::InvalidRequest,
            ShutdownDetailsMutationErrorDtoV1::Internal { correlation_id } => {
                Self::Internal { correlation_id }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SendCommandErrorDtoV1 {
    InvalidRequest,
    PayloadConflict,
    NotFound,
    CapacityExceeded,
    FeedbackCapacityExceeded,
    MigrationInProgress,
    ShutdownInProgress,
    ResponseTooLarge,
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SendLookupErrorDtoV1 {
    InvalidRequest,
    OutcomeUnknown { operation_id: String },
    NotFound,
    QueryBusy,
    DeadlineExceeded,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum PermissionResponseCommandErrorDtoV1 {
    InvalidRequest,
    PayloadConflict,
    NotFound,
    CapacityExceeded,
    FeedbackCapacityExceeded,
    MigrationInProgress,
    ShutdownInProgress,
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum PermissionResponseLookupErrorDtoV1 {
    InvalidRequest,
    NotFound,
    QueryBusy,
    DeadlineExceeded,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

impl From<PermissionResponseCommandErrorDtoV1> for OperationApplicationErrorDtoV1 {
    fn from(value: PermissionResponseCommandErrorDtoV1) -> Self {
        match value {
            PermissionResponseCommandErrorDtoV1::InvalidRequest => Self::InvalidRequest,
            PermissionResponseCommandErrorDtoV1::PayloadConflict => Self::PayloadConflict,
            PermissionResponseCommandErrorDtoV1::NotFound => Self::NotFound,
            PermissionResponseCommandErrorDtoV1::CapacityExceeded => Self::CapacityExceeded,
            PermissionResponseCommandErrorDtoV1::FeedbackCapacityExceeded => {
                Self::FeedbackCapacityExceeded
            }
            PermissionResponseCommandErrorDtoV1::MigrationInProgress => Self::MigrationInProgress,
            PermissionResponseCommandErrorDtoV1::ShutdownInProgress => Self::ShutdownInProgress,
            PermissionResponseCommandErrorDtoV1::Internal { correlation_id } => {
                Self::Internal { correlation_id }
            }
        }
    }
}

impl From<PermissionResponseLookupErrorDtoV1> for OperationApplicationErrorDtoV1 {
    fn from(value: PermissionResponseLookupErrorDtoV1) -> Self {
        match value {
            PermissionResponseLookupErrorDtoV1::InvalidRequest => Self::InvalidRequest,
            PermissionResponseLookupErrorDtoV1::NotFound => Self::NotFound,
            PermissionResponseLookupErrorDtoV1::QueryBusy => Self::QueryBusy,
            PermissionResponseLookupErrorDtoV1::DeadlineExceeded => Self::DeadlineExceeded,
            PermissionResponseLookupErrorDtoV1::StorageUnavailable { failure } => {
                Self::StorageUnavailable { failure }
            }
            PermissionResponseLookupErrorDtoV1::Internal { correlation_id } => {
                Self::Internal { correlation_id }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StopCommandErrorDtoV1 {
    InvalidRequest,
    PayloadConflict,
    FeedbackCapacityExceeded,
    MigrationInProgress,
    ShutdownInProgress,
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StopLookupErrorDtoV1 {
    InvalidRequest,
    NotFound,
    QueryBusy,
    DeadlineExceeded,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SessionLifecycleCommandErrorDtoV1 {
    InvalidRequest,
    PayloadConflict,
    FeedbackCapacityExceeded,
    MigrationInProgress,
    ShutdownInProgress,
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SessionLifecycleLookupErrorDtoV1 {
    InvalidRequest,
    NotFound,
    QueryBusy,
    DeadlineExceeded,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum PendingRecoveryQueryErrorDtoV1 {
    InvalidRequest,
    CursorMismatch,
    CursorExpired,
    QueryBusy,
    DeadlineExceeded,
    ResponseTooLarge,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum PendingRecoverySnapshotQueryErrorDtoV1 {
    InvalidRequest,
    NotFound,
    SnapshotMismatch,
    CursorMismatch,
    CursorExpired,
    DetailsCompacted,
    QueryBusy,
    DeadlineExceeded,
    ResponseTooLarge,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RecoveryActionCommandErrorDtoV1 {
    InvalidRequest,
    NotFound,
    MigrationInProgress,
    ShutdownInProgress,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RecoveryActionLookupErrorDtoV1 {
    InvalidRequest,
    NotFound,
    QueryBusy,
    DeadlineExceeded,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PendingPartitionDtoV1 {
    Owner,
    ClosedSession,
    ArchivedSession,
    UnownedRuntime,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum PendingRecoveryOwnerTargetDtoV1 {
    Session {
        session_id: String,
    },
    WorkflowExecution {
        execution_id: String,
    },
    WorkflowNode {
        execution_id: String,
        node_execution_id: String,
        workflow_name: String,
        node_name: String,
        attempt: String,
    },
    ClosedSession {
        session_id: String,
    },
    ArchivedSession {
        session_id: String,
    },
    UnownedRuntime {
        runtime_id: String,
    },
    UnknownOwner {
        owner: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoveryActionKindDtoV1 {
    ReadAgain,
    RetrySameEffect,
    UseObservedResult,
    CancelIfSafe,
    KeepForManualResolution,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PendingRecoveryCategoryDtoV1 {
    TurnExecution,
    QueueExecution,
    PermissionDelivery,
    ProviderEstablish,
    TerminalCommit,
    BackendRecovery,
    SessionClose,
    WorkflowShutdown,
    RecoveryPublication,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PendingRecoveryKnownStatusDtoV1 {
    Prepared,
    Pending,
    EffectReserved,
    Running,
    WaitingApproval,
    ReconciliationRequired,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryActionRequestDtoV1 {
    pub action_id: String,
    pub obligation_id: String,
    pub origin_revision: String,
    pub action: RecoveryActionKindDtoV1,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingRecoveryEntryDtoV1 {
    pub obligation_id: String,
    pub category: PendingRecoveryCategoryDtoV1,
    pub original_identity: String,
    pub owner: String,
    pub owner_target: PendingRecoveryOwnerTargetDtoV1,
    pub partition: PendingPartitionDtoV1,
    pub shutdown_plan: Option<ShutdownPlanReferenceDtoV1>,
    pub revision: String,
    pub state: String,
    pub known_status: PendingRecoveryKnownStatusDtoV1,
    pub safe_label: String,
    pub actions: Vec<RecoveryActionKindDtoV1>,
    pub action_identities: Vec<RecoveryActionIdentityDtoV1>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryActionIdentityDtoV1 {
    pub action_id: String,
    pub action: RecoveryActionKindDtoV1,
    pub origin_revision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShutdownPlanReferenceDtoV1 {
    pub plan_id: String,
    pub epoch: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingRecoveryPageDtoV1 {
    pub entries: Vec<PendingRecoveryEntryDtoV1>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RecoveryActionOutcomeDtoV1 {
    Completed {
        action_id: String,
        result: RecoveryActionCompletedResultDtoV1,
    },
    InProgress {
        action_id: String,
    },
    Rejected {
        action_id: String,
        rejection: RecoveryActionRejectionDtoV1,
    },
    ActionOutcomeUnknown {
        action_id: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryActionCompletedResultDtoV1 {
    pub outcome: String,
    pub classification: String,
    pub resource_revision: String,
    pub canonical_result_sha256: String,
    pub resource_view: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RecoveryActionRejectionDtoV1 {
    RevisionConflict { current_revision: String },
    ActionUnavailable,
    TargetRevisionChanged,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RecoveryActionStatusDtoV1 {
    InProgress {
        action_id: String,
    },
    OutcomeUnknown {
        action_id: String,
    },
    ReconciliationRequired {
        action_id: String,
        failure: SafeOperationFailureDtoV1,
    },
    Completed {
        action_id: String,
        result: RecoveryActionCompletedResultDtoV1,
    },
}

impl From<crate::domain::local_event::PendingPartition> for PendingPartitionDtoV1 {
    fn from(value: crate::domain::local_event::PendingPartition) -> Self {
        match value {
            crate::domain::local_event::PendingPartition::Owner => Self::Owner,
            crate::domain::local_event::PendingPartition::ClosedSession => Self::ClosedSession,
            crate::domain::local_event::PendingPartition::ArchivedSession => Self::ArchivedSession,
            crate::domain::local_event::PendingPartition::UnownedRuntime => Self::UnownedRuntime,
        }
    }
}

impl From<crate::usecase::agent_session::operation::PendingRecoveryOwnerTarget>
    for PendingRecoveryOwnerTargetDtoV1
{
    fn from(value: crate::usecase::agent_session::operation::PendingRecoveryOwnerTarget) -> Self {
        use crate::usecase::agent_session::operation::PendingRecoveryOwnerTarget;
        match value {
            PendingRecoveryOwnerTarget::Session { session_id } => Self::Session { session_id },
            PendingRecoveryOwnerTarget::WorkflowExecution { execution_id } => {
                Self::WorkflowExecution { execution_id }
            }
            PendingRecoveryOwnerTarget::WorkflowNode {
                execution_id,
                node_execution_id,
                workflow_name,
                node_name,
                attempt,
            } => Self::WorkflowNode {
                execution_id,
                node_execution_id,
                workflow_name,
                node_name,
                attempt: attempt.to_string(),
            },
            PendingRecoveryOwnerTarget::ClosedSession { session_id } => {
                Self::ClosedSession { session_id }
            }
            PendingRecoveryOwnerTarget::ArchivedSession { session_id } => {
                Self::ArchivedSession { session_id }
            }
            PendingRecoveryOwnerTarget::UnownedRuntime { runtime_id } => {
                Self::UnownedRuntime { runtime_id }
            }
            PendingRecoveryOwnerTarget::UnknownOwner { owner } => Self::UnknownOwner { owner },
        }
    }
}

impl From<crate::usecase::agent_session::operation::PendingRecoveryCategory>
    for PendingRecoveryCategoryDtoV1
{
    fn from(value: crate::usecase::agent_session::operation::PendingRecoveryCategory) -> Self {
        use crate::usecase::agent_session::operation::PendingRecoveryCategory;
        match value {
            PendingRecoveryCategory::TurnExecution => Self::TurnExecution,
            PendingRecoveryCategory::QueueExecution => Self::QueueExecution,
            PendingRecoveryCategory::PermissionDelivery => Self::PermissionDelivery,
            PendingRecoveryCategory::ProviderEstablish => Self::ProviderEstablish,
            PendingRecoveryCategory::TerminalCommit => Self::TerminalCommit,
            PendingRecoveryCategory::BackendRecovery => Self::BackendRecovery,
            PendingRecoveryCategory::SessionClose => Self::SessionClose,
            PendingRecoveryCategory::WorkflowShutdown => Self::WorkflowShutdown,
            PendingRecoveryCategory::RecoveryPublication => Self::RecoveryPublication,
            PendingRecoveryCategory::Unknown => Self::Unknown,
        }
    }
}

impl From<crate::usecase::agent_session::operation::PendingRecoveryKnownStatus>
    for PendingRecoveryKnownStatusDtoV1
{
    fn from(value: crate::usecase::agent_session::operation::PendingRecoveryKnownStatus) -> Self {
        use crate::usecase::agent_session::operation::PendingRecoveryKnownStatus;
        match value {
            PendingRecoveryKnownStatus::Prepared => Self::Prepared,
            PendingRecoveryKnownStatus::Pending => Self::Pending,
            PendingRecoveryKnownStatus::EffectReserved => Self::EffectReserved,
            PendingRecoveryKnownStatus::Running => Self::Running,
            PendingRecoveryKnownStatus::WaitingApproval => Self::WaitingApproval,
            PendingRecoveryKnownStatus::ReconciliationRequired => Self::ReconciliationRequired,
            PendingRecoveryKnownStatus::Failed => Self::Failed,
            PendingRecoveryKnownStatus::Unknown => Self::Unknown,
        }
    }
}

impl From<RecoveryActionKindDtoV1> for crate::domain::agent_session::events::RecoveryActionKind {
    fn from(value: RecoveryActionKindDtoV1) -> Self {
        match value {
            RecoveryActionKindDtoV1::ReadAgain => Self::ReadAgain,
            RecoveryActionKindDtoV1::RetrySameEffect => Self::RetrySameEffect,
            RecoveryActionKindDtoV1::UseObservedResult => Self::UseObservedResult,
            RecoveryActionKindDtoV1::CancelIfSafe => Self::CancelIfSafe,
            RecoveryActionKindDtoV1::KeepForManualResolution => Self::KeepForManualResolution,
        }
    }
}

impl From<crate::domain::agent_session::events::RecoveryActionKind> for RecoveryActionKindDtoV1 {
    fn from(value: crate::domain::agent_session::events::RecoveryActionKind) -> Self {
        match value {
            crate::domain::agent_session::events::RecoveryActionKind::ReadAgain => Self::ReadAgain,
            crate::domain::agent_session::events::RecoveryActionKind::RetrySameEffect => {
                Self::RetrySameEffect
            }
            crate::domain::agent_session::events::RecoveryActionKind::UseObservedResult => {
                Self::UseObservedResult
            }
            crate::domain::agent_session::events::RecoveryActionKind::CancelIfSafe => {
                Self::CancelIfSafe
            }
            crate::domain::agent_session::events::RecoveryActionKind::KeepForManualResolution => {
                Self::KeepForManualResolution
            }
        }
    }
}

pub(crate) const PENDING_RECOVERY_PUBLIC_PAGE_MAX_BYTES: usize = 4 * 1024 * 1024;

fn pending_recovery_entry(
    entry: crate::usecase::agent_session::operation::PendingRecoveryEntry,
) -> PendingRecoveryEntryDtoV1 {
    PendingRecoveryEntryDtoV1 {
        obligation_id: entry.obligation_id,
        category: entry.category.into(),
        original_identity: entry.original_identity,
        owner: entry.owner,
        owner_target: entry.owner_target.into(),
        partition: entry.partition.into(),
        shutdown_plan: entry.shutdown_plan.map(|plan| ShutdownPlanReferenceDtoV1 {
            plan_id: plan.plan_id,
            epoch: plan.epoch.to_string(),
        }),
        revision: entry.revision.to_string(),
        state: match entry.state {
            crate::usecase::agent_session::operation::recovery::RecoveryResourceState::Pending => {
                "pending"
            }
            crate::usecase::agent_session::operation::recovery::RecoveryResourceState::Failed => {
                "failed"
            }
        }
        .to_string(),
        known_status: entry.known_status.into(),
        safe_label: entry.safe_label,
        actions: entry.actions.into_iter().map(Into::into).collect(),
        action_identities: entry
            .action_identities
            .into_iter()
            .map(|identity| RecoveryActionIdentityDtoV1 {
                action_id: identity.action_id,
                action: identity.action.into(),
                origin_revision: identity.origin_revision.to_string(),
            })
            .collect(),
    }
}

#[derive(Serialize)]
struct PendingRecoveryPageDtoRefV1<'a> {
    entries: &'a [PendingRecoveryEntryDtoV1],
    next_cursor: Option<&'a str>,
}

fn pending_recovery_encoded_len(
    entries: &[PendingRecoveryEntryDtoV1],
    next_cursor: Option<&str>,
) -> Result<usize, crate::usecase::agent_session::operation::RecoveryActionError> {
    serde_json::to_vec(&PendingRecoveryPageDtoRefV1 {
        entries,
        next_cursor,
    })
    .map(|encoded| encoded.len())
    .map_err(|error| {
        let correlation_id = uuid::Uuid::new_v4().to_string();
        log::error!(
            "pending recovery public page serialization failed [{correlation_id}]: {error}"
        );
        crate::usecase::agent_session::operation::RecoveryActionError::Internal { correlation_id }
    })
}

/// Convert and enforce the exact canonical public page encoding shared by
/// Tauri and authenticated WebSocket. When more than one entry is present,
/// the largest prefix that fits is returned with the source cursor directly
/// after its last entry. An oversized first entry fails without a partial
/// result.
pub(crate) fn checked_pending_recovery_page(
    value: crate::usecase::agent_session::operation::PendingRecoveryPage,
) -> Result<PendingRecoveryPageDtoV1, crate::usecase::agent_session::operation::RecoveryActionError>
{
    if value.entries.len() > 200 {
        return Err(
            crate::usecase::agent_session::operation::RecoveryActionError::Internal {
                correlation_id: uuid::Uuid::new_v4().to_string(),
            },
        );
    }
    let source_next_cursor = value.next_cursor;
    let continuation_cursors = value
        .entries
        .iter()
        .map(|entry| entry.continuation_cursor.clone())
        .collect::<Vec<_>>();
    let mut entries = value
        .entries
        .into_iter()
        .map(pending_recovery_entry)
        .collect::<Vec<_>>();
    let total = entries.len();
    let cursor_after = |count: usize| -> Option<&str> {
        if count < total {
            continuation_cursors
                .get(count.saturating_sub(1))
                .map(String::as_str)
        } else {
            source_next_cursor.as_deref()
        }
    };

    if pending_recovery_encoded_len(&entries, cursor_after(total))?
        <= PENDING_RECOVERY_PUBLIC_PAGE_MAX_BYTES
    {
        return Ok(PendingRecoveryPageDtoV1 {
            entries,
            next_cursor: source_next_cursor,
        });
    }
    if total == 0
        || pending_recovery_encoded_len(&entries[..1], cursor_after(1))?
            > PENDING_RECOVERY_PUBLIC_PAGE_MAX_BYTES
    {
        return Err(
            crate::usecase::agent_session::operation::RecoveryActionError::ResponseTooLarge,
        );
    }

    let mut low = 1usize;
    let mut high = total - 1;
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if pending_recovery_encoded_len(&entries[..middle], cursor_after(middle))?
            <= PENDING_RECOVERY_PUBLIC_PAGE_MAX_BYTES
        {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let next_cursor = cursor_after(low).map(str::to_string);
    entries.truncate(low);
    Ok(PendingRecoveryPageDtoV1 {
        entries,
        next_cursor,
    })
}

impl From<crate::usecase::agent_session::operation::RecoveryActionOutcome>
    for RecoveryActionOutcomeDtoV1
{
    fn from(value: crate::usecase::agent_session::operation::RecoveryActionOutcome) -> Self {
        match value {
            crate::usecase::agent_session::operation::RecoveryActionOutcome::Completed {
                action_id,
                result,
            } => Self::Completed {
                action_id,
                result: result.into(),
            },
            crate::usecase::agent_session::operation::RecoveryActionOutcome::ActionOutcomeUnknown {
                action_id,
            } => Self::ActionOutcomeUnknown { action_id },
            crate::usecase::agent_session::operation::RecoveryActionOutcome::InProgress {
                action_id,
            } => Self::InProgress { action_id },
            crate::usecase::agent_session::operation::RecoveryActionOutcome::Rejected {
                action_id,
                rejection,
            } => Self::Rejected {
                action_id,
                rejection: rejection.into(),
            },
        }
    }
}

impl From<crate::usecase::agent_session::operation::recovery::RecoveryActionRejection>
    for RecoveryActionRejectionDtoV1
{
    fn from(
        value: crate::usecase::agent_session::operation::recovery::RecoveryActionRejection,
    ) -> Self {
        use crate::usecase::agent_session::operation::recovery::RecoveryActionRejection as R;
        match value {
            R::RevisionConflict { current_revision } => Self::RevisionConflict {
                current_revision: current_revision.to_string(),
            },
            R::ActionUnavailable => Self::ActionUnavailable,
            R::TargetRevisionChanged => Self::TargetRevisionChanged,
        }
    }
}

impl From<crate::usecase::agent_session::operation::recovery::RecoveryActionCompletedResult>
    for RecoveryActionCompletedResultDtoV1
{
    fn from(
        value: crate::usecase::agent_session::operation::recovery::RecoveryActionCompletedResult,
    ) -> Self {
        use crate::usecase::agent_session::operation::recovery::RecoveryActionResultOutcome as O;
        Self {
            outcome: match value.outcome {
                O::Pending => "pending",
                O::Terminal => "terminal",
                O::Unchanged => "unchanged",
            }
            .to_string(),
            classification: recovery_classification_label(value.classification).to_string(),
            resource_revision: value.resource_revision.to_string(),
            canonical_result_sha256: value.canonical_result_sha256,
            resource_view: value.resource_view,
        }
    }
}

impl From<crate::usecase::agent_session::operation::recovery::RecoveryActionStatus>
    for RecoveryActionStatusDtoV1
{
    fn from(
        value: crate::usecase::agent_session::operation::recovery::RecoveryActionStatus,
    ) -> Self {
        use crate::usecase::agent_session::operation::recovery::RecoveryActionStatus as S;
        match value {
            S::InProgress { action_id } => Self::InProgress { action_id },
            S::OutcomeUnknown { action_id } => Self::OutcomeUnknown { action_id },
            S::ReconciliationRequired { action_id, failure } => Self::ReconciliationRequired {
                action_id,
                failure: failure.into(),
            },
            S::Completed { action_id, result } => Self::Completed {
                action_id,
                result: result.into(),
            },
        }
    }
}

impl RecoveryActionOutcomeDtoV1 {
    pub(crate) fn from_durable_status(
        value: crate::usecase::agent_session::operation::recovery::RecoveryActionStatus,
    ) -> Self {
        use crate::usecase::agent_session::operation::recovery::RecoveryActionStatus as S;
        match value {
            S::Completed { action_id, result } => Self::Completed {
                action_id,
                result: result.into(),
            },
            S::InProgress { action_id } | S::ReconciliationRequired { action_id, .. } => {
                Self::InProgress { action_id }
            }
            S::OutcomeUnknown { action_id } => Self::ActionOutcomeUnknown { action_id },
        }
    }
}

pub(crate) fn recovery_classification_label(
    value: crate::domain::agent_session::events::RecoveryResultClassification,
) -> &'static str {
    use crate::domain::agent_session::events::RecoveryResultClassification as C;
    match value {
        C::Pending => "pending",
        C::Succeeded => "succeeded",
        C::ConfirmedNoEffect => "confirmed_no_effect",
        C::Ambiguous => "ambiguous",
        C::CancelledBeforeEffect => "cancelled_before_effect",
        C::Unchanged => "unchanged",
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionResponseRequestDtoV1 {
    pub operation_id: String,
    pub session_id: String,
    pub request_id: String,
    pub behavior: String,
    pub message: Option<String>,
    pub updated_input: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionResponseOperationReceiptDtoV1 {
    pub operation_id: String,
    pub session_id: String,
    pub request_id: String,
    pub input_ref: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PermissionResponseDecisionDtoV1 {
    Allowed,
    Denied,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum PermissionResponseExecutionStatusDtoV1 {
    AwaitingProviderResponse {
        obligation_id: String,
    },
    ReconciliationRequired {
        failure: SafeOperationFailureDtoV1,
    },
    Failed {
        failure: SafeOperationFailureDtoV1,
    },
    Completed {
        decision: PermissionResponseDecisionDtoV1,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionResponseOperationViewDtoV1 {
    pub receipt: PermissionResponseOperationReceiptDtoV1,
    pub latest_status: PermissionResponseExecutionStatusDtoV1,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum PermissionResponseCommandOutcomeDtoV1 {
    Accepted {
        operation: PermissionResponseOperationViewDtoV1,
    },
    RejectedBeforeCommit {
        failure: SafeOperationFailureDtoV1,
    },
    OutcomeUnknown {
        operation_id: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SendOperationReceiptDtoV1 {
    pub operation_id: String,
    pub session_id: String,
    pub input_ref: String,
    pub disposition: SendDispositionDtoV1,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SendDispositionDtoV1 {
    StartedTurn { turn_id: String },
    Queued { queue_item_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SendExecutionStatusDtoV1 {
    AwaitingProviderStart {
        dependency_obligation_ids: Vec<String>,
    },
    Queued {
        queue_item_id: String,
        reserved_turn_id: String,
    },
    ProviderStartReserved {
        obligation_id: String,
    },
    Running {
        turn_id: String,
    },
    ReconciliationRequired {
        failure: SafeOperationFailureDtoV1,
    },
    Failed {
        failure: SafeOperationFailureDtoV1,
    },
    Terminal {
        result: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SendOperationViewDtoV1 {
    pub receipt: SendOperationReceiptDtoV1,
    pub latest_status: SendExecutionStatusDtoV1,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SendCommandOutcomeDtoV1 {
    Accepted { operation: SendOperationViewDtoV1 },
    RejectedBeforeCommit { failure: SafeOperationFailureDtoV1 },
    OutcomeUnknown { operation_id: String },
}

impl From<crate::usecase::agent_session::operation::SendCommandOutcome>
    for SendCommandOutcomeDtoV1
{
    fn from(value: crate::usecase::agent_session::operation::SendCommandOutcome) -> Self {
        match value {
            crate::usecase::agent_session::operation::SendCommandOutcome::Accepted(operation) => {
                Self::Accepted {
                    operation: operation.into(),
                }
            }
            crate::usecase::agent_session::operation::SendCommandOutcome::RejectedBeforeCommit {
                failure,
            } => Self::RejectedBeforeCommit {
                failure: failure.into(),
            },
            crate::usecase::agent_session::operation::SendCommandOutcome::OutcomeUnknown {
                operation_id,
            } => Self::OutcomeUnknown { operation_id },
        }
    }
}

impl From<crate::usecase::agent_session::operation::AcceptedSendOperation>
    for SendOperationViewDtoV1
{
    fn from(value: crate::usecase::agent_session::operation::AcceptedSendOperation) -> Self {
        use crate::domain::agent_session::events::SendDisposition;
        use crate::usecase::agent_session::operation::SendExecutionStatus as S;
        let disposition = match value.receipt.disposition {
            SendDisposition::StartedTurn { turn_id } => SendDispositionDtoV1::StartedTurn {
                turn_id: turn_id.to_string(),
            },
            SendDisposition::Queued { queue_item_id } => {
                SendDispositionDtoV1::Queued { queue_item_id }
            }
        };
        let latest_status = match value.latest_status {
            S::AwaitingProviderStart {
                dependency_obligation_ids,
            } => SendExecutionStatusDtoV1::AwaitingProviderStart {
                dependency_obligation_ids,
            },
            S::Queued {
                queue_item_id,
                reserved_turn_id,
            } => SendExecutionStatusDtoV1::Queued {
                queue_item_id,
                reserved_turn_id,
            },
            S::ProviderStartReserved { obligation_id } => {
                SendExecutionStatusDtoV1::ProviderStartReserved { obligation_id }
            }
            S::Running { turn_id } => SendExecutionStatusDtoV1::Running { turn_id },
            S::ReconciliationRequired { failure } => {
                SendExecutionStatusDtoV1::ReconciliationRequired {
                    failure: failure.into(),
                }
            }
            S::Failed { failure } => SendExecutionStatusDtoV1::Failed {
                failure: failure.into(),
            },
            S::Terminal { result } => SendExecutionStatusDtoV1::Terminal {
                result: format!("{result:?}"),
            },
        };
        Self {
            receipt: SendOperationReceiptDtoV1 {
                operation_id: value.receipt.operation_id,
                session_id: value.receipt.session_id,
                input_ref: value.receipt.input_ref,
                disposition,
            },
            latest_status,
        }
    }
}

impl From<crate::usecase::agent_session::operation::PermissionResponseCommandOutcome>
    for PermissionResponseCommandOutcomeDtoV1
{
    fn from(
        value: crate::usecase::agent_session::operation::PermissionResponseCommandOutcome,
    ) -> Self {
        use crate::usecase::agent_session::operation::PermissionResponseCommandOutcome as O;
        match value {
            O::Accepted(operation) => Self::Accepted {
                operation: operation.into(),
            },
            O::RejectedBeforeCommit { failure } => Self::RejectedBeforeCommit {
                failure: failure.into(),
            },
            O::OutcomeUnknown { operation_id } => Self::OutcomeUnknown { operation_id },
        }
    }
}

impl From<crate::usecase::agent_session::operation::AcceptedPermissionResponseOperation>
    for PermissionResponseOperationViewDtoV1
{
    fn from(
        value: crate::usecase::agent_session::operation::AcceptedPermissionResponseOperation,
    ) -> Self {
        use crate::usecase::agent_session::operation::{
            PermissionResponseDecisionKind as D, PermissionResponseExecutionStatus as S,
        };
        let latest_status = match value.latest_status {
            S::AwaitingProviderResponse { obligation_id } => {
                PermissionResponseExecutionStatusDtoV1::AwaitingProviderResponse { obligation_id }
            }
            S::ReconciliationRequired { failure } => {
                PermissionResponseExecutionStatusDtoV1::ReconciliationRequired {
                    failure: failure.into(),
                }
            }
            S::Failed { failure } => PermissionResponseExecutionStatusDtoV1::Failed {
                failure: failure.into(),
            },
            S::Completed { decision } => PermissionResponseExecutionStatusDtoV1::Completed {
                decision: match decision {
                    D::Allowed => PermissionResponseDecisionDtoV1::Allowed,
                    D::Denied => PermissionResponseDecisionDtoV1::Denied,
                },
            },
        };
        Self {
            receipt: PermissionResponseOperationReceiptDtoV1 {
                operation_id: value.receipt.operation_id,
                session_id: value.receipt.session_id,
                request_id: value.receipt.request_id,
                input_ref: value.receipt.input_ref,
            },
            latest_status,
        }
    }
}

impl From<crate::domain::local_event::SafeOperationFailure> for SafeOperationFailureDtoV1 {
    fn from(value: crate::domain::local_event::SafeOperationFailure) -> Self {
        Self {
            kind: crate::usecase::agent_session::operation::record::failure_kind_label(value.kind)
                .to_string(),
            retryable: value.retryable,
            label: value.label.value().to_string(),
            detail: value.detail.map(|detail| detail.value().to_string()),
            correlation_id: value.correlation_id,
        }
    }
}

impl From<crate::usecase::agent_session::operation::StopOperationReceipt>
    for StopOperationReceiptDtoV1
{
    fn from(value: crate::usecase::agent_session::operation::StopOperationReceipt) -> Self {
        Self {
            operation_id: value.operation_id,
            session_id: value.session_id,
            turn_id: value.turn_id,
            accepted_revision: value.accepted_revision.to_string(),
        }
    }
}

impl From<crate::usecase::agent_session::operation::StopOperationState>
    for StopOperationStateDtoV1
{
    fn from(value: crate::usecase::agent_session::operation::StopOperationState) -> Self {
        match value {
            crate::usecase::agent_session::operation::StopOperationState::Accepted => Self::Accepted,
            crate::usecase::agent_session::operation::StopOperationState::Completed { resolution } => Self::Completed {
                resolution: match resolution {
                    crate::domain::agent_session::events::StopResolution::Succeeded => StopResolutionDtoV1::Succeeded,
                    crate::domain::agent_session::events::StopResolution::Superseded => StopResolutionDtoV1::Superseded,
                },
            },
            crate::usecase::agent_session::operation::StopOperationState::ReconciliationRequired { failure } => Self::ReconciliationRequired { failure: failure.into() },
        }
    }
}

impl From<crate::usecase::agent_session::operation::StopCommandOutcome>
    for StopCommandOutcomeDtoV1
{
    fn from(value: crate::usecase::agent_session::operation::StopCommandOutcome) -> Self {
        match value {
            crate::usecase::agent_session::operation::StopCommandOutcome::Accepted { receipt, state } => Self::Accepted { receipt: receipt.into(), state: state.into() },
            crate::usecase::agent_session::operation::StopCommandOutcome::RejectedBeforeCommit { failure } => Self::RejectedBeforeCommit { failure: failure.into() },
            crate::usecase::agent_session::operation::StopCommandOutcome::OutcomeUnknown { request_id } => Self::OutcomeUnknown { request_id },
        }
    }
}

fn lifecycle_action_fields(
    value: crate::usecase::agent_session::operation::SessionLifecycleAction,
) -> (String, Option<String>) {
    match value {
        crate::usecase::agent_session::operation::SessionLifecycleAction::Close => {
            ("close".to_string(), None)
        }
        crate::usecase::agent_session::operation::SessionLifecycleAction::ArchiveOpen => {
            ("archive_open".to_string(), None)
        }
        crate::usecase::agent_session::operation::SessionLifecycleAction::ArchiveClosed => {
            ("archive_closed".to_string(), None)
        }
        crate::usecase::agent_session::operation::SessionLifecycleAction::SwitchBackend {
            backend_id,
        } => ("switch_backend".to_string(), Some(backend_id)),
    }
}

impl From<crate::usecase::agent_session::operation::SessionLifecycleReceipt>
    for SessionLifecycleReceiptDtoV1
{
    fn from(value: crate::usecase::agent_session::operation::SessionLifecycleReceipt) -> Self {
        let (action, backend_id) = lifecycle_action_fields(value.action);
        Self {
            operation_id: value.operation_id,
            session_id: value.session_id,
            action,
            backend_id,
            first_accepted_revision: value.first_accepted_revision.to_string(),
        }
    }
}

impl From<crate::usecase::agent_session::operation::SessionLifecycleOperationState>
    for SessionLifecycleStateDtoV1
{
    fn from(
        value: crate::usecase::agent_session::operation::SessionLifecycleOperationState,
    ) -> Self {
        match value {
            crate::usecase::agent_session::operation::SessionLifecycleOperationState::Accepted => Self::Accepted,
            crate::usecase::agent_session::operation::SessionLifecycleOperationState::Completed => Self::Completed,
            crate::usecase::agent_session::operation::SessionLifecycleOperationState::ReconciliationRequired { failure } => Self::ReconciliationRequired { failure: failure.into() },
        }
    }
}

impl From<crate::usecase::agent_session::operation::SessionLifecycleCommandResult>
    for SessionLifecycleCommandResultDtoV1
{
    fn from(
        value: crate::usecase::agent_session::operation::SessionLifecycleCommandResult,
    ) -> Self {
        use crate::usecase::agent_session::operation::{
            SessionLifecycleCommandResult as R, SessionLifecycleRejection as J,
        };
        match value {
            R::Accepted { receipt, state } => Self::Accepted {
                receipt: receipt.into(),
                state: state.into(),
            },
            R::OutcomeUnknown { request_id } => Self::OutcomeUnknown { request_id },
            R::Rejected(rejection) => Self::Rejected {
                rejection: match rejection {
                    J::Busy => SessionLifecycleRejectionDtoV1::Busy,
                    J::PendingOperation => SessionLifecycleRejectionDtoV1::PendingOperation,
                    J::RevisionConflict { current_revision } => {
                        SessionLifecycleRejectionDtoV1::RevisionConflict {
                            current_revision: current_revision.to_string(),
                        }
                    }
                    J::InvalidState => SessionLifecycleRejectionDtoV1::InvalidState,
                    J::Failed { failure } => SessionLifecycleRejectionDtoV1::Failed {
                        failure: failure.into(),
                    },
                },
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatMessageDtoV1 {
    pub id: String,
    pub role: MessageRoleDtoV1,
    pub content: String,
    pub thinking: Option<String>,
    pub activities: Option<Vec<ActivityEntryDtoV1>>,
    pub parts: Option<Vec<MessagePartDtoV1>>,
    pub streaming_final_seq: String,
    pub timestamp_ms: String,
    pub mentions: Option<Vec<MessageMentionDtoV1>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatSessionDtoV1 {
    pub id: String,
    pub worktree_path: String,
    pub messages: Vec<ChatMessageDtoV1>,
    pub state: SessionStateDtoV1,
    pub error_reason: Option<String>,
    pub created_at_ms: String,
    pub updated_at_ms: String,
    pub agent_session_id: Option<String>,
    pub context_carry: Option<ContextCarryStateDtoV1>,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub selected_model: Option<String>,
    pub permission_profile_id: Option<String>,
    pub backend_id: Option<String>,
    pub workflow_node_session: bool,
    pub workflow_node_context: Option<WorkflowNodeContextDtoV1>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetSessionResponseDtoV1 {
    pub id: String,
    pub worktree_path: String,
    pub messages: Vec<ChatMessageDtoV1>,
    pub state: SessionStateDtoV1,
    pub error_reason: Option<String>,
    pub created_at_ms: String,
    pub updated_at_ms: String,
    pub agent_session_id: Option<String>,
    pub context_carry: Option<ContextCarryStateDtoV1>,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub selected_model: Option<String>,
    pub permission_profile_id: Option<String>,
    pub backend_id: Option<String>,
    pub workflow_node_session: bool,
    pub workflow_node_context: Option<WorkflowNodeContextDtoV1>,
    pub session_revision: String,
    pub active_turn_id: Option<String>,
    pub turn_phase: TurnPhaseDtoV1,
    pub available_models: Vec<ModelInfoDtoV1>,
    pub can_change_backend: bool,
    pub pending_queue: Vec<QueuedAgentTurnDtoV1>,
    pub pending_queue_count: String,
    pub queue_paused: bool,
    pub pending_permission_request: Option<PermissionRequestDtoV1>,
    pub pending_permission_state_revision: String,
    pub initial_page: Option<InitialSessionPageDtoV1>,
    pub latest_token_usage: Option<TokenUsageDtoV1>,
    pub last_turn_interruption: Option<TurnInterruptionDtoV1>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionPageDtoV1 {
    pub messages: Vec<ChatMessageDtoV1>,
    pub message_metadata: Vec<MessagePageMetadataDtoV1>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub total_count: String,
    pub latest_token_usage: Option<TokenUsageDtoV1>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InitSessionsResponseDtoV1 {
    pub sessions: Vec<SessionSummaryDtoV1>,
    pub active_session: Option<GetSessionResponseDtoV1>,
    pub permission_mode: String,
    pub plan_mode: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionSummaryDtoV1 {
    pub id: String,
    pub worktree_path: String,
    pub state: SessionStateDtoV1,
    pub error_reason: Option<String>,
    pub created_at_ms: String,
    pub updated_at_ms: String,
    pub first_message: String,
    pub message_count: String,
    pub agent_session_id: Option<String>,
    pub context_carry: Option<ContextCarryStateDtoV1>,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
    pub backend_id: Option<String>,
    pub workflow_node_session: bool,
    pub workflow_node_context: Option<WorkflowNodeContextDtoV1>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TurnPhaseDtoV1 {
    Idle,
    Streaming,
    WaitingPermission,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelInfoDtoV1 {
    pub id: String,
    pub display_name: String,
    pub backend: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueuedAgentTurnDtoV1 {
    pub id: String,
    pub content_preview: String,
    pub created_at_ms: String,
    pub permission_mode: String,
    pub image_count: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InitialSessionPageDtoV1 {
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub total_count: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TokenUsageDtoV1 {
    pub input_tokens: String,
    pub output_tokens: String,
    pub total_tokens: Option<String>,
    pub context_window_tokens: Option<String>,
}

fn public_millis_from_legacy_seconds(value: f64) -> String {
    let millis = (value * 1000.0).round();
    if !millis.is_finite() || millis <= 0.0 {
        return "0".to_string();
    }
    if millis >= i64::MAX as f64 {
        return i64::MAX.to_string();
    }
    (millis as i64).to_string()
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TurnInterruptionDtoV1 {
    pub message_id: String,
    pub reason: TurnInterruptionReasonDtoV1,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TurnInterruptionReasonDtoV1 {
    Abort,
    Timeout,
    Crash,
    SessionClosed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessagePageMetadataDtoV1 {
    pub message_id: String,
    pub token_meta: Option<JsonValueDtoV1>,
    pub run_meta: Option<JsonValueDtoV1>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MessageRoleDtoV1 {
    Human,
    Agent,
    System,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionStateDtoV1 {
    Active,
    Idle,
    Done,
    Error,
    Closed,
    Archived,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ContextCarryStateDtoV1 {
    Resumed,
    Reinjected,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessageMentionDtoV1 {
    pub file_path: String,
    pub start_line: Option<String>,
    pub end_line: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkflowNodeContextDtoV1 {
    pub execution_id: String,
    pub node_execution_id: String,
    pub workflow_name: String,
    pub node_name: String,
    pub attempt: String,
    pub parent_node_name: Option<String>,
    pub parent_attempt: Option<String>,
    pub order: String,
    pub startup_timeout_secs: Option<String>,
    pub startup_max_retries: Option<String>,
    pub stale_timeout_secs: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ActivityEntryDtoV1 {
    ToolUse {
        tool: String,
        input: JsonValueDtoV1,
        id: String,
    },
    ToolResult {
        content: String,
        is_error: bool,
        tool_use_id: Option<String>,
        content_ref: Option<ToolOutputRefDtoV1>,
        summary: Option<ToolOutputSummaryDtoV1>,
    },
    PermissionResult {
        tool_name: String,
        status: String,
        summary: String,
    },
}

impl From<&ChatMessage> for ChatMessageDtoV1 {
    fn from(v: &ChatMessage) -> Self {
        Self {
            id: v.id.clone(),
            role: (&v.role).into(),
            content: v.content.clone(),
            thinking: v.thinking.clone(),
            activities: v
                .activities
                .as_ref()
                .map(|items| items.iter().map(Into::into).collect()),
            parts: v
                .parts
                .as_ref()
                .map(|items| items.iter().map(Into::into).collect()),
            streaming_final_seq: v.streaming_final_seq.to_string(),
            timestamp_ms: public_millis_from_legacy_seconds(v.timestamp),
            mentions: v
                .mentions
                .as_ref()
                .map(|items| items.iter().map(Into::into).collect()),
        }
    }
}
impl From<ChatMessage> for ChatMessageDtoV1 {
    fn from(v: ChatMessage) -> Self {
        (&v).into()
    }
}
impl From<&ChatSession> for ChatSessionDtoV1 {
    fn from(v: &ChatSession) -> Self {
        Self {
            id: v.id.clone(),
            worktree_path: v.worktree_path.clone(),
            messages: v.messages.iter().map(Into::into).collect(),
            state: (&v.state).into(),
            error_reason: v.error_reason.clone(),
            created_at_ms: public_millis_from_legacy_seconds(v.created_at),
            updated_at_ms: public_millis_from_legacy_seconds(v.updated_at),
            agent_session_id: v.agent_session_id.clone(),
            context_carry: v.context_carry.as_ref().map(Into::into),
            permission_mode: v.permission_mode.clone(),
            plan_mode: v.plan_mode,
            selected_model: v.selected_model.clone(),
            permission_profile_id: v.permission_profile_id.clone(),
            backend_id: v.backend_id.clone(),
            workflow_node_session: v.workflow_node_session,
            workflow_node_context: v.workflow_node_context.as_ref().map(Into::into),
        }
    }
}
impl From<ChatSession> for ChatSessionDtoV1 {
    fn from(v: ChatSession) -> Self {
        (&v).into()
    }
}
impl From<GetSessionResponse> for GetSessionResponseDtoV1 {
    fn from(v: GetSessionResponse) -> Self {
        let s = ChatSessionDtoV1::from(&v.session);
        Self {
            id: s.id,
            worktree_path: s.worktree_path,
            messages: s.messages,
            state: s.state,
            error_reason: s.error_reason,
            created_at_ms: s.created_at_ms,
            updated_at_ms: s.updated_at_ms,
            agent_session_id: s.agent_session_id,
            context_carry: s.context_carry,
            permission_mode: s.permission_mode,
            plan_mode: s.plan_mode,
            selected_model: s.selected_model,
            permission_profile_id: s.permission_profile_id,
            backend_id: s.backend_id,
            workflow_node_session: s.workflow_node_session,
            workflow_node_context: s.workflow_node_context,
            session_revision: v.session_revision.to_string(),
            active_turn_id: v.active_turn_id.map(|value| value.to_string()),
            turn_phase: v.turn_phase.into(),
            available_models: v.available_models.into_iter().map(Into::into).collect(),
            can_change_backend: v.can_change_backend,
            pending_queue: v.pending_queue.into_iter().map(Into::into).collect(),
            pending_queue_count: v.pending_queue_count.to_string(),
            queue_paused: v.queue_paused,
            pending_permission_request: v.pending_permission_request.map(Into::into),
            pending_permission_state_revision: v.pending_permission_state_revision.to_string(),
            initial_page: v.initial_page.map(Into::into),
            latest_token_usage: v.latest_token_usage.map(Into::into),
            last_turn_interruption: v.last_turn_interruption.map(Into::into),
        }
    }
}
impl From<SessionPage> for SessionPageDtoV1 {
    fn from(v: SessionPage) -> Self {
        Self {
            messages: v.messages.into_iter().map(Into::into).collect(),
            message_metadata: v.message_metadata.into_iter().map(Into::into).collect(),
            next_cursor: v.next_cursor.map(|cursor| cursor.0.to_string()),
            has_more: v.has_more,
            total_count: v.total_count.to_string(),
            latest_token_usage: v.latest_token_usage.map(Into::into),
        }
    }
}
impl From<InitSessionsResponse> for InitSessionsResponseDtoV1 {
    fn from(v: InitSessionsResponse) -> Self {
        Self {
            sessions: v.sessions.into_iter().map(Into::into).collect(),
            active_session: v.active_session.map(Into::into),
            permission_mode: v.permission_mode,
            plan_mode: v.plan_mode,
        }
    }
}
impl From<SessionSummary> for SessionSummaryDtoV1 {
    fn from(v: SessionSummary) -> Self {
        Self {
            id: v.id,
            worktree_path: v.worktree_path,
            state: (&v.state).into(),
            error_reason: v.error_reason,
            created_at_ms: public_millis_from_legacy_seconds(v.created_at),
            updated_at_ms: public_millis_from_legacy_seconds(v.updated_at),
            first_message: v.first_message,
            message_count: v.message_count.to_string(),
            agent_session_id: v.agent_session_id,
            context_carry: v.context_carry.as_ref().map(Into::into),
            permission_mode: v.permission_mode,
            plan_mode: v.plan_mode,
            permission_profile_id: v.permission_profile_id,
            backend_id: v.backend_id,
            workflow_node_session: v.workflow_node_session,
            workflow_node_context: v.workflow_node_context.as_ref().map(Into::into),
        }
    }
}

impl From<TurnPhase> for TurnPhaseDtoV1 {
    fn from(value: TurnPhase) -> Self {
        match value {
            TurnPhase::Idle => Self::Idle,
            TurnPhase::Streaming => Self::Streaming,
            TurnPhase::WaitingPermission => Self::WaitingPermission,
        }
    }
}

impl From<ModelInfo> for ModelInfoDtoV1 {
    fn from(value: ModelInfo) -> Self {
        Self {
            id: value.id,
            display_name: value.display_name,
            backend: value.backend,
            model_id: value.model_id,
        }
    }
}

impl From<QueuedAgentTurn> for QueuedAgentTurnDtoV1 {
    fn from(value: QueuedAgentTurn) -> Self {
        Self {
            id: value.id,
            content_preview: value.content_preview,
            created_at_ms: public_millis_from_legacy_seconds(value.created_at),
            permission_mode: value.permission_mode,
            image_count: value.image_count.to_string(),
        }
    }
}

impl From<InitialSessionPage> for InitialSessionPageDtoV1 {
    fn from(value: InitialSessionPage) -> Self {
        Self {
            next_cursor: value.next_cursor.map(|cursor| cursor.0.to_string()),
            has_more: value.has_more,
            total_count: value.total_count.to_string(),
        }
    }
}

impl From<TokenUsage> for TokenUsageDtoV1 {
    fn from(value: TokenUsage) -> Self {
        Self {
            input_tokens: value.input_tokens.to_string(),
            output_tokens: value.output_tokens.to_string(),
            total_tokens: value.total_tokens.map(|tokens| tokens.to_string()),
            context_window_tokens: value.context_window_tokens.map(|tokens| tokens.to_string()),
        }
    }
}

impl From<TurnInterruption> for TurnInterruptionDtoV1 {
    fn from(value: TurnInterruption) -> Self {
        Self {
            message_id: value.message_id,
            reason: value.reason.into(),
        }
    }
}

impl From<TurnInterruptionReason> for TurnInterruptionReasonDtoV1 {
    fn from(value: TurnInterruptionReason) -> Self {
        match value {
            TurnInterruptionReason::Abort => Self::Abort,
            TurnInterruptionReason::Timeout => Self::Timeout,
            TurnInterruptionReason::Crash => Self::Crash,
            TurnInterruptionReason::SessionClosed => Self::SessionClosed,
        }
    }
}

impl From<MessagePageMetadata> for MessagePageMetadataDtoV1 {
    fn from(value: MessagePageMetadata) -> Self {
        Self {
            message_id: value.message_id,
            token_meta: value.token_meta.as_ref().map(JsonValueDtoV1::from_value),
            run_meta: value.run_meta.as_ref().map(JsonValueDtoV1::from_value),
        }
    }
}

impl From<PermissionRequestMsg> for PermissionRequestDtoV1 {
    fn from(value: PermissionRequestMsg) -> Self {
        Self {
            id: value.id,
            tool_use_id: value.tool_use_id,
            tool_name: value.tool_name,
            kind: match value.kind {
                PermissionRequestKindMsg::ToolApproval => PermissionRequestKindDtoV1::ToolApproval,
                PermissionRequestKindMsg::PlanApproval => PermissionRequestKindDtoV1::PlanApproval,
                PermissionRequestKindMsg::Question => PermissionRequestKindDtoV1::Question,
                PermissionRequestKindMsg::PermissionGrant => {
                    PermissionRequestKindDtoV1::PermissionGrant
                }
            },
            input: value.input.as_ref().map(JsonValueDtoV1::from_value),
            plan: value.plan,
            allowed_prompts: value
                .allowed_prompts
                .into_iter()
                .map(|prompt| PermissionAllowedPromptDtoV1 {
                    tool: prompt.tool,
                    prompt: prompt.prompt,
                })
                .collect(),
            questions: value
                .questions
                .into_iter()
                .map(|question| PermissionQuestionDtoV1 {
                    question: question.question,
                    header: question.header,
                    options: question
                        .options
                        .into_iter()
                        .map(|option| PermissionQuestionOptionDtoV1 {
                            label: option.label,
                            description: option.description,
                        })
                        .collect(),
                    multi_select: question.multi_select,
                })
                .collect(),
            title: value.title,
            display_name: value.display_name,
            description: value.description,
            decision_reason: value.decision_reason,
        }
    }
}

impl From<&MessageRole> for MessageRoleDtoV1 {
    fn from(v: &MessageRole) -> Self {
        match v {
            MessageRole::Human => Self::Human,
            MessageRole::Agent => Self::Agent,
            MessageRole::System => Self::System,
        }
    }
}
impl From<&SessionState> for SessionStateDtoV1 {
    fn from(v: &SessionState) -> Self {
        match v {
            SessionState::Active => Self::Active,
            SessionState::Idle => Self::Idle,
            SessionState::Done => Self::Done,
            SessionState::Error => Self::Error,
            SessionState::Closed => Self::Closed,
            SessionState::Archived => Self::Archived,
        }
    }
}
impl From<&ContextCarryState> for ContextCarryStateDtoV1 {
    fn from(v: &ContextCarryState) -> Self {
        match v {
            ContextCarryState::Resumed => Self::Resumed,
            ContextCarryState::Reinjected => Self::Reinjected,
            ContextCarryState::Failed => Self::Failed,
        }
    }
}
impl From<&MessageMention> for MessageMentionDtoV1 {
    fn from(v: &MessageMention) -> Self {
        Self {
            file_path: v.file_path.clone(),
            start_line: v.start_line.map(|line| line.to_string()),
            end_line: v.end_line.map(|line| line.to_string()),
        }
    }
}
impl From<&WorkflowNodeContextDto> for WorkflowNodeContextDtoV1 {
    fn from(v: &WorkflowNodeContextDto) -> Self {
        Self {
            execution_id: v.execution_id.clone(),
            node_execution_id: v.node_execution_id.clone(),
            workflow_name: v.workflow_name.clone(),
            node_name: v.node_name.clone(),
            attempt: v.attempt.to_string(),
            parent_node_name: v.parent_node_name.clone(),
            parent_attempt: v.parent_attempt.map(|attempt| attempt.to_string()),
            order: v.order.to_string(),
            startup_timeout_secs: v.startup_timeout_secs.map(|value| value.to_string()),
            startup_max_retries: v.startup_max_retries.map(|value| value.to_string()),
            stale_timeout_secs: v.stale_timeout_secs.map(|value| value.to_string()),
        }
    }
}
impl From<&ActivityEntry> for ActivityEntryDtoV1 {
    fn from(v: &ActivityEntry) -> Self {
        match v {
            ActivityEntry::ToolUse { tool, input, id } => Self::ToolUse {
                tool: tool.clone(),
                input: JsonValueDtoV1::from_value(input),
                id: id.clone(),
            },
            ActivityEntry::ToolResult {
                content,
                is_error,
                tool_use_id,
                content_ref,
                summary,
            } => Self::ToolResult {
                content: content.clone(),
                is_error: *is_error,
                tool_use_id: tool_use_id.clone(),
                content_ref: content_ref.as_ref().map(|v| ToolOutputRefDtoV1 {
                    id: v.id.clone(),
                    byte_size: v.byte_size.to_string(),
                }),
                summary: summary.as_ref().map(|v| ToolOutputSummaryDtoV1 {
                    line_count: v.line_count.to_string(),
                    byte_size: v.byte_size.to_string(),
                    is_error: v.is_error,
                    truncated: v.truncated,
                }),
            },
            ActivityEntry::PermissionResult {
                tool_name,
                status,
                summary,
            } => Self::PermissionResult {
                tool_name: tool_name.clone(),
                status: status.clone(),
                summary: summary.clone(),
            },
        }
    }
}

#[cfg(test)]
mod architecture_tests {
    use super::{
        checked_pending_recovery_page, decode_nonnegative_i64_decimal,
        decode_nonnegative_u64_decimal, decode_positive_i64_decimal, ChatMessageDtoV1,
        MessageRoleDtoV1, PendingRecoveryQueryErrorDtoV1, PendingRecoverySnapshotQueryErrorDtoV1,
        RecoveryActionCommandErrorDtoV1, RecoveryActionLookupErrorDtoV1,
        RecoveryActionRequestDtoV1, SafeOperationFailureDtoV1, SendCommandErrorDtoV1,
        SendLookupErrorDtoV1, SessionLifecycleCommandErrorDtoV1, SessionLifecycleLookupErrorDtoV1,
        StopCommandErrorDtoV1, StopLookupErrorDtoV1, StopOperationRequestDtoV1,
        StopOperationStateDtoV1,
    };

    fn error_tags<T: serde::Serialize>(errors: Vec<T>) -> Vec<String> {
        errors
            .into_iter()
            .map(|error| {
                serde_json::to_value(error).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }

    fn failure() -> SafeOperationFailureDtoV1 {
        SafeOperationFailureDtoV1 {
            kind: "storage_unavailable".to_string(),
            retryable: true,
            label: "unavailable".to_string(),
            detail: None,
            correlation_id: "storage-1".to_string(),
        }
    }

    #[test]
    fn public_v1_fields_do_not_embed_usecase_or_unversioned_json_types() {
        let source = include_str!("agent_session_v1.rs");
        for line in source.lines().map(str::trim) {
            if line.starts_with("pub ") {
                assert!(!line.contains("crate::usecase::"), "{line}");
                assert!(!line.contains("serde_json::Value"), "{line}");
            }
        }
    }

    #[test]
    fn recovery_request_rejects_client_supplied_observation_proof() {
        let raw = serde_json::json!({
            "action_id": "action-1",
            "obligation_id": "obligation-1",
            "origin_revision": "0",
            "action": "use_observed_result",
            "observed_result": "client-asserted-success",
        });
        assert!(serde_json::from_value::<RecoveryActionRequestDtoV1>(raw).is_err());
    }

    #[test]
    fn b075_public_semantic_integer_decoder_is_canonical_and_i64_bounded() {
        for (raw, expected) in [
            ("0", Some(0)),
            ("1", Some(1)),
            ("9223372036854775807", Some(i64::MAX)),
            ("", None),
            ("01", None),
            ("+1", None),
            ("-1", None),
            ("1e0", None),
            ("１", None),
            (" 1", None),
            ("1 ", None),
            ("9223372036854775808", None),
        ] {
            assert_eq!(decode_nonnegative_i64_decimal(raw), expected, "{raw:?}");
            assert_eq!(
                decode_nonnegative_u64_decimal(raw),
                expected.map(|value| value as u64),
                "{raw:?}"
            );
            assert_eq!(
                decode_positive_i64_decimal(raw),
                expected.filter(|value| *value > 0),
                "{raw:?}"
            );
        }
        assert!(
            serde_json::from_value::<StopOperationRequestDtoV1>(serde_json::json!({
                "request_id": "stop-json-number",
                "session_id": "session-1",
                "turn_id": 1,
                "expected_session_revision": "0",
            }))
            .is_err()
        );

        for field in ["turn_id", "expected_session_revision"] {
            let mut raw = serde_json::json!({
                "request_id": "stop-json-number",
                "session_id": "session-1",
                "turn_id": "1",
                "expected_session_revision": "0",
            });
            raw[field] = serde_json::json!(1);
            assert!(
                serde_json::from_value::<StopOperationRequestDtoV1>(raw).is_err(),
                "Stop.{field} accepted a JSON number"
            );
        }

        let mut lifecycle = serde_json::json!({
            "request_id": "lifecycle-json-number",
            "session_id": "session-1",
            "expected_session_revision": "0",
            "action": { "type": "close" },
        });
        lifecycle["expected_session_revision"] = serde_json::json!(0);
        assert!(serde_json::from_value::<super::SessionLifecycleRequestDtoV1>(lifecycle).is_err());

        let mut recovery = serde_json::json!({
            "action_id": "action-1",
            "obligation_id": "obligation-1",
            "origin_revision": "0",
            "action": "read_again",
        });
        recovery["origin_revision"] = serde_json::json!(0);
        assert!(serde_json::from_value::<RecoveryActionRequestDtoV1>(recovery).is_err());
    }

    fn assert_string_field<T>(_: fn(&T) -> &String) {}

    fn assert_optional_string_field<T>(_: fn(&T) -> &Option<String>) {}

    #[test]
    fn b075_all_agent_session_v1_struct_semantic_integer_fields_are_strings() {
        assert_string_field(|value: &super::StopOperationRequestDtoV1| &value.turn_id);
        assert_string_field(|value: &super::StopOperationRequestDtoV1| {
            &value.expected_session_revision
        });
        assert_string_field(|value: &super::StopOperationReceiptDtoV1| &value.turn_id);
        assert_string_field(|value: &super::StopOperationReceiptDtoV1| &value.accepted_revision);
        assert_string_field(|value: &super::SessionLifecycleRequestDtoV1| {
            &value.expected_session_revision
        });
        assert_string_field(|value: &super::SessionLifecycleReceiptDtoV1| {
            &value.first_accepted_revision
        });
        assert_string_field(|value: &super::RecoveryActionRequestDtoV1| &value.origin_revision);
        assert_string_field(|value: &super::PendingRecoveryEntryDtoV1| &value.revision);
        assert_string_field(|value: &super::RecoveryActionIdentityDtoV1| &value.origin_revision);
        assert_string_field(|value: &super::ShutdownPlanReferenceDtoV1| &value.epoch);
        assert_string_field(|value: &super::RecoveryActionCompletedResultDtoV1| {
            &value.resource_revision
        });
        assert_string_field(|value: &super::ChatMessageDtoV1| &value.streaming_final_seq);
        assert_string_field(|value: &super::ChatMessageDtoV1| &value.timestamp_ms);
        assert_string_field(|value: &super::ChatSessionDtoV1| &value.created_at_ms);
        assert_string_field(|value: &super::ChatSessionDtoV1| &value.updated_at_ms);
        assert_string_field(|value: &super::GetSessionResponseDtoV1| &value.created_at_ms);
        assert_string_field(|value: &super::GetSessionResponseDtoV1| &value.updated_at_ms);
        assert_string_field(|value: &super::GetSessionResponseDtoV1| &value.session_revision);
        assert_optional_string_field(|value: &super::GetSessionResponseDtoV1| {
            &value.active_turn_id
        });
        assert_string_field(|value: &super::GetSessionResponseDtoV1| &value.pending_queue_count);
        assert_string_field(|value: &super::GetSessionResponseDtoV1| {
            &value.pending_permission_state_revision
        });
        assert_string_field(|value: &super::SessionPageDtoV1| &value.total_count);
        assert_string_field(|value: &super::SessionSummaryDtoV1| &value.created_at_ms);
        assert_string_field(|value: &super::SessionSummaryDtoV1| &value.updated_at_ms);
        assert_string_field(|value: &super::SessionSummaryDtoV1| &value.message_count);
        assert_string_field(|value: &super::QueuedAgentTurnDtoV1| &value.created_at_ms);
        assert_string_field(|value: &super::QueuedAgentTurnDtoV1| &value.image_count);
        assert_string_field(|value: &super::InitialSessionPageDtoV1| &value.total_count);
        assert_string_field(|value: &super::TokenUsageDtoV1| &value.input_tokens);
        assert_string_field(|value: &super::TokenUsageDtoV1| &value.output_tokens);
        assert_optional_string_field(|value: &super::TokenUsageDtoV1| &value.total_tokens);
        assert_optional_string_field(|value: &super::TokenUsageDtoV1| &value.context_window_tokens);
        assert_optional_string_field(|value: &super::MessageMentionDtoV1| &value.start_line);
        assert_optional_string_field(|value: &super::MessageMentionDtoV1| &value.end_line);
        assert_string_field(|value: &super::WorkflowNodeContextDtoV1| &value.attempt);
        assert_optional_string_field(|value: &super::WorkflowNodeContextDtoV1| {
            &value.parent_attempt
        });
        assert_string_field(|value: &super::WorkflowNodeContextDtoV1| &value.order);
        assert_optional_string_field(|value: &super::WorkflowNodeContextDtoV1| {
            &value.startup_timeout_secs
        });
        assert_optional_string_field(|value: &super::WorkflowNodeContextDtoV1| {
            &value.startup_max_retries
        });
        assert_optional_string_field(|value: &super::WorkflowNodeContextDtoV1| {
            &value.stale_timeout_secs
        });
    }

    #[test]
    fn b075_agent_session_v1_enum_semantic_integer_fields_encode_as_strings() {
        let maximum = i64::MAX.to_string();
        for (value, field) in [
            (
                serde_json::to_value(super::SendDispositionDtoV1::StartedTurn {
                    turn_id: maximum.clone(),
                })
                .unwrap(),
                "turn_id",
            ),
            (
                serde_json::to_value(super::SendExecutionStatusDtoV1::Queued {
                    queue_item_id: "queue-1".to_string(),
                    reserved_turn_id: maximum.clone(),
                })
                .unwrap(),
                "reserved_turn_id",
            ),
            (
                serde_json::to_value(super::SendExecutionStatusDtoV1::Running {
                    turn_id: maximum.clone(),
                })
                .unwrap(),
                "turn_id",
            ),
            (
                serde_json::to_value(super::PendingRecoveryOwnerTargetDtoV1::WorkflowNode {
                    execution_id: "execution-1".to_string(),
                    node_execution_id: "node-execution-1".to_string(),
                    workflow_name: "workflow".to_string(),
                    node_name: "node".to_string(),
                    attempt: maximum.clone(),
                })
                .unwrap(),
                "attempt",
            ),
            (
                serde_json::to_value(super::OperationApplicationErrorDtoV1::RevisionConflict {
                    current_revision: maximum.clone(),
                })
                .unwrap(),
                "current_revision",
            ),
            (
                serde_json::to_value(super::SessionLifecycleRejectionDtoV1::RevisionConflict {
                    current_revision: maximum.clone(),
                })
                .unwrap(),
                "current_revision",
            ),
            (
                serde_json::to_value(super::RecoveryActionRejectionDtoV1::RevisionConflict {
                    current_revision: maximum.clone(),
                })
                .unwrap(),
                "current_revision",
            ),
        ] {
            assert_eq!(value[field], serde_json::Value::String(maximum.clone()));
        }
    }

    #[test]
    fn strict_public_message_golden_uses_decimal_strings_snake_case_and_explicit_null() {
        let value = serde_json::to_value(ChatMessageDtoV1 {
            id: "message-1".to_string(),
            role: MessageRoleDtoV1::Agent,
            content: "done".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            streaming_final_seq: i64::MAX.to_string(),
            timestamp_ms: i64::MAX.to_string(),
            mentions: None,
        })
        .unwrap();
        assert_eq!(value["streaming_final_seq"], i64::MAX.to_string());
        assert_eq!(value["timestamp_ms"], i64::MAX.to_string());
        assert!(value["thinking"].is_null());
        assert!(value["activities"].is_null());
        assert!(value["parts"].is_null());
        assert!(value["mentions"].is_null());
        assert!(value.get("streamingFinalSeq").is_none());
        assert!(value.get("timestampMs").is_none());
    }

    #[test]
    fn completed_stop_state_exposes_the_closed_resolution() {
        for (resolution, expected) in [
            (
                crate::domain::agent_session::events::StopResolution::Succeeded,
                "succeeded",
            ),
            (
                crate::domain::agent_session::events::StopResolution::Superseded,
                "superseded",
            ),
        ] {
            let value = serde_json::to_value(StopOperationStateDtoV1::from(
                crate::usecase::agent_session::operation::StopOperationState::Completed {
                    resolution,
                },
            ))
            .unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "completed",
                    "resolution": expected,
                })
            );
        }
    }

    #[test]
    fn tauri_operation_endpoints_publish_only_their_named_exact_error_sets() {
        assert_eq!(
            error_tags(vec![
                SendCommandErrorDtoV1::InvalidRequest,
                SendCommandErrorDtoV1::PayloadConflict,
                SendCommandErrorDtoV1::NotFound,
                SendCommandErrorDtoV1::CapacityExceeded,
                SendCommandErrorDtoV1::FeedbackCapacityExceeded,
                SendCommandErrorDtoV1::MigrationInProgress,
                SendCommandErrorDtoV1::ShutdownInProgress,
                SendCommandErrorDtoV1::ResponseTooLarge,
                SendCommandErrorDtoV1::Internal {
                    correlation_id: "internal-1".to_string(),
                },
            ]),
            [
                "invalid_request",
                "payload_conflict",
                "not_found",
                "capacity_exceeded",
                "feedback_capacity_exceeded",
                "migration_in_progress",
                "shutdown_in_progress",
                "response_too_large",
                "internal",
            ]
        );
        assert_eq!(
            error_tags(vec![
                SendLookupErrorDtoV1::InvalidRequest,
                SendLookupErrorDtoV1::OutcomeUnknown {
                    operation_id: "send-unknown-1".to_string(),
                },
                SendLookupErrorDtoV1::NotFound,
                SendLookupErrorDtoV1::QueryBusy,
                SendLookupErrorDtoV1::DeadlineExceeded,
                SendLookupErrorDtoV1::StorageUnavailable { failure: failure() },
                SendLookupErrorDtoV1::Internal {
                    correlation_id: "internal-1".to_string(),
                },
            ]),
            [
                "invalid_request",
                "outcome_unknown",
                "not_found",
                "query_busy",
                "deadline_exceeded",
                "storage_unavailable",
                "internal",
            ]
        );
        assert_eq!(
            error_tags(vec![
                StopCommandErrorDtoV1::InvalidRequest,
                StopCommandErrorDtoV1::PayloadConflict,
                StopCommandErrorDtoV1::FeedbackCapacityExceeded,
                StopCommandErrorDtoV1::MigrationInProgress,
                StopCommandErrorDtoV1::ShutdownInProgress,
                StopCommandErrorDtoV1::Internal {
                    correlation_id: "internal-1".to_string(),
                },
            ]),
            [
                "invalid_request",
                "payload_conflict",
                "feedback_capacity_exceeded",
                "migration_in_progress",
                "shutdown_in_progress",
                "internal",
            ]
        );
        assert_eq!(
            error_tags(vec![
                StopLookupErrorDtoV1::InvalidRequest,
                StopLookupErrorDtoV1::NotFound,
                StopLookupErrorDtoV1::QueryBusy,
                StopLookupErrorDtoV1::DeadlineExceeded,
                StopLookupErrorDtoV1::StorageUnavailable { failure: failure() },
                StopLookupErrorDtoV1::Internal {
                    correlation_id: "internal-1".to_string(),
                },
            ]),
            [
                "invalid_request",
                "not_found",
                "query_busy",
                "deadline_exceeded",
                "storage_unavailable",
                "internal",
            ]
        );
        assert_eq!(
            error_tags(vec![
                SessionLifecycleCommandErrorDtoV1::InvalidRequest,
                SessionLifecycleCommandErrorDtoV1::PayloadConflict,
                SessionLifecycleCommandErrorDtoV1::FeedbackCapacityExceeded,
                SessionLifecycleCommandErrorDtoV1::MigrationInProgress,
                SessionLifecycleCommandErrorDtoV1::ShutdownInProgress,
                SessionLifecycleCommandErrorDtoV1::Internal {
                    correlation_id: "internal-1".to_string(),
                },
            ]),
            [
                "invalid_request",
                "payload_conflict",
                "feedback_capacity_exceeded",
                "migration_in_progress",
                "shutdown_in_progress",
                "internal",
            ]
        );
        assert_eq!(
            error_tags(vec![
                SessionLifecycleLookupErrorDtoV1::InvalidRequest,
                SessionLifecycleLookupErrorDtoV1::NotFound,
                SessionLifecycleLookupErrorDtoV1::QueryBusy,
                SessionLifecycleLookupErrorDtoV1::DeadlineExceeded,
                SessionLifecycleLookupErrorDtoV1::StorageUnavailable { failure: failure() },
                SessionLifecycleLookupErrorDtoV1::Internal {
                    correlation_id: "internal-1".to_string(),
                },
            ]),
            [
                "invalid_request",
                "not_found",
                "query_busy",
                "deadline_exceeded",
                "storage_unavailable",
                "internal",
            ]
        );
        assert_eq!(
            error_tags(vec![
                PendingRecoveryQueryErrorDtoV1::InvalidRequest,
                PendingRecoveryQueryErrorDtoV1::CursorMismatch,
                PendingRecoveryQueryErrorDtoV1::CursorExpired,
                PendingRecoveryQueryErrorDtoV1::QueryBusy,
                PendingRecoveryQueryErrorDtoV1::DeadlineExceeded,
                PendingRecoveryQueryErrorDtoV1::ResponseTooLarge,
                PendingRecoveryQueryErrorDtoV1::StorageUnavailable { failure: failure() },
                PendingRecoveryQueryErrorDtoV1::Internal {
                    correlation_id: "internal-1".to_string(),
                },
            ]),
            [
                "invalid_request",
                "cursor_mismatch",
                "cursor_expired",
                "query_busy",
                "deadline_exceeded",
                "response_too_large",
                "storage_unavailable",
                "internal",
            ]
        );
        assert_eq!(
            error_tags(vec![
                PendingRecoverySnapshotQueryErrorDtoV1::InvalidRequest,
                PendingRecoverySnapshotQueryErrorDtoV1::NotFound,
                PendingRecoverySnapshotQueryErrorDtoV1::SnapshotMismatch,
                PendingRecoverySnapshotQueryErrorDtoV1::CursorMismatch,
                PendingRecoverySnapshotQueryErrorDtoV1::CursorExpired,
                PendingRecoverySnapshotQueryErrorDtoV1::DetailsCompacted,
                PendingRecoverySnapshotQueryErrorDtoV1::QueryBusy,
                PendingRecoverySnapshotQueryErrorDtoV1::DeadlineExceeded,
                PendingRecoverySnapshotQueryErrorDtoV1::ResponseTooLarge,
                PendingRecoverySnapshotQueryErrorDtoV1::StorageUnavailable { failure: failure() },
                PendingRecoverySnapshotQueryErrorDtoV1::Internal {
                    correlation_id: "internal-1".to_string(),
                },
            ]),
            [
                "invalid_request",
                "not_found",
                "snapshot_mismatch",
                "cursor_mismatch",
                "cursor_expired",
                "details_compacted",
                "query_busy",
                "deadline_exceeded",
                "response_too_large",
                "storage_unavailable",
                "internal",
            ]
        );
        assert_eq!(
            error_tags(vec![
                RecoveryActionCommandErrorDtoV1::InvalidRequest,
                RecoveryActionCommandErrorDtoV1::NotFound,
                RecoveryActionCommandErrorDtoV1::MigrationInProgress,
                RecoveryActionCommandErrorDtoV1::ShutdownInProgress,
                RecoveryActionCommandErrorDtoV1::StorageUnavailable { failure: failure() },
                RecoveryActionCommandErrorDtoV1::Internal {
                    correlation_id: "internal-1".to_string(),
                },
            ]),
            [
                "invalid_request",
                "not_found",
                "migration_in_progress",
                "shutdown_in_progress",
                "storage_unavailable",
                "internal",
            ]
        );
        assert_eq!(
            error_tags(vec![
                RecoveryActionLookupErrorDtoV1::InvalidRequest,
                RecoveryActionLookupErrorDtoV1::NotFound,
                RecoveryActionLookupErrorDtoV1::QueryBusy,
                RecoveryActionLookupErrorDtoV1::DeadlineExceeded,
                RecoveryActionLookupErrorDtoV1::StorageUnavailable { failure: failure() },
                RecoveryActionLookupErrorDtoV1::Internal {
                    correlation_id: "internal-1".to_string(),
                },
            ]),
            [
                "invalid_request",
                "not_found",
                "query_busy",
                "deadline_exceeded",
                "storage_unavailable",
                "internal",
            ]
        );
    }

    fn expanded_pending_entry(
        obligation_id: &str,
        escaped_owner_bytes: usize,
        continuation_cursor: &str,
    ) -> crate::usecase::agent_session::operation::PendingRecoveryEntry {
        let owner = "\"".repeat(escaped_owner_bytes);
        crate::usecase::agent_session::operation::PendingRecoveryEntry {
            obligation_id: obligation_id.to_string(),
            category: crate::usecase::agent_session::operation::PendingRecoveryCategory::Unknown,
            original_identity: obligation_id.to_string(),
            owner: owner.clone(),
            owner_target:
                crate::usecase::agent_session::operation::PendingRecoveryOwnerTarget::UnknownOwner {
                    owner,
                },
            partition: crate::domain::local_event::PendingPartition::Owner,
            shutdown_plan: None,
            revision: 0,
            state:
                crate::usecase::agent_session::operation::recovery::RecoveryResourceState::Pending,
            known_status:
                crate::usecase::agent_session::operation::PendingRecoveryKnownStatus::Unknown,
            safe_label: "Pending local operation".to_string(),
            actions: Vec::new(),
            action_identities: Vec::new(),
            continuation_cursor: continuation_cursor.to_string(),
        }
    }

    #[test]
    fn b038_public_encoded_expansion_truncates_at_exact_entry_cursor() {
        let page = crate::usecase::agent_session::operation::PendingRecoveryPage {
            entries: vec![
                expanded_pending_entry("expanded-1", 550_000, "cursor-after-first"),
                expanded_pending_entry("expanded-2", 550_000, "cursor-after-second"),
            ],
            next_cursor: None,
        };
        let checked = checked_pending_recovery_page(page).unwrap();
        assert_eq!(checked.entries.len(), 1);
        assert_eq!(checked.next_cursor.as_deref(), Some("cursor-after-first"));
        assert!(serde_json::to_vec(&checked).unwrap().len() <= 4 * 1024 * 1024);
    }

    #[test]
    fn b038_single_public_entry_overflow_is_response_too_large_without_partial_page() {
        let page = crate::usecase::agent_session::operation::PendingRecoveryPage {
            entries: vec![expanded_pending_entry(
                "expanded-single",
                1_100_000,
                "cursor-after-single",
            )],
            next_cursor: None,
        };
        assert!(matches!(
            checked_pending_recovery_page(page),
            Err(crate::usecase::agent_session::operation::RecoveryActionError::ResponseTooLarge)
        ));
    }

    #[test]
    fn b090_public_pending_recovery_page_preserves_plan_and_workflow_owner() {
        let plan = crate::domain::local_event::ShutdownPlanKey {
            plan_id: "b090-plan-1".to_string(),
            epoch: 7,
        };
        let page = crate::usecase::agent_session::operation::PendingRecoveryPage {
            entries: vec![crate::usecase::agent_session::operation::PendingRecoveryEntry {
                obligation_id: "b090-workflow-obligation".to_string(),
                category:
                    crate::usecase::agent_session::operation::PendingRecoveryCategory::WorkflowShutdown,
                original_identity: "workflow-run-7".to_string(),
                owner: "workflow-session".to_string(),
                owner_target:
                    crate::usecase::agent_session::operation::PendingRecoveryOwnerTarget::WorkflowNode {
                        execution_id: "workflow-run-7".to_string(),
                        node_execution_id: "node-execution-11".to_string(),
                        workflow_name: "release-workflow".to_string(),
                        node_name: "review".to_string(),
                        attempt: 3,
                    },
                partition: crate::domain::local_event::PendingPartition::Owner,
                shutdown_plan: Some(plan),
                revision: 4,
                state:
                    crate::usecase::agent_session::operation::recovery::RecoveryResourceState::Pending,
                known_status:
                    crate::usecase::agent_session::operation::PendingRecoveryKnownStatus::Pending,
                safe_label: "Pending workflow shutdown".to_string(),
                actions: Vec::new(),
                action_identities: Vec::new(),
                continuation_cursor: "b090-after-workflow".to_string(),
            }],
            next_cursor: None,
        };

        let public = checked_pending_recovery_page(page).unwrap();
        let public = serde_json::to_value(public).unwrap();
        assert_eq!(
            public["entries"][0]["shutdown_plan"]["plan_id"],
            "b090-plan-1"
        );
        assert_eq!(public["entries"][0]["shutdown_plan"]["epoch"], "7");
        assert_eq!(
            public["entries"][0]["owner_target"]["type"],
            "workflow_node"
        );
        assert_eq!(
            public["entries"][0]["owner_target"]["execution_id"],
            "workflow-run-7"
        );
        assert_eq!(public["entries"][0]["owner_target"]["node_name"], "review");
        assert_eq!(public["entries"][0]["owner_target"]["attempt"], "3");
        assert!(serde_json::to_vec(&public).unwrap().len() <= 4 * 1024 * 1024);
    }
}
