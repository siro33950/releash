//! Backend-owned single-flight application shutdown coordinator.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(test)]
use std::{future::Future, pin::Pin};

use base64::Engine as _;
use serde_json::Value;

use crate::domain::agent_session::events::{RecoveryActionKind, RecoveryResultClassification};
use crate::domain::local_event::{
    ApplicationDomainEvent, ApplicationShutdownPhase, CallerOperationKey, CommitBatchError,
    CommitBatchResult, CommitIdentity, CommitOperationKind, CommitResolution, ExpectedStreamHead,
    IdempotencyBinding, LoadStreamRequest, LocalAtomicBatch, LocalDomainEvent, LocalEventQuery,
    LocalEventQueryResult, LocalEventTransactionRepository, LocalStateMutation,
    ObligationStateRecord, OperationBindingMutation, OperationBindingView, OperationKind,
    OperationReceiptRecord, OperationRecordMutation, OperationStatusRecord, OperationStatusValue,
    PendingPartition, QueryCursor, QuitIntent, RecoveryActionMutation, RecoveryActionView,
    RecoveryAttemptRecord, RecoveryResourceViewRecord, RecoveryResultOutcomeRecord, Revision,
    RevisionGuard, SafeEffectObservation, SafeOperationFailure, SessionOperationFailureKind,
    ShutdownDetailsCompactionMutation, ShutdownDetailsState, ShutdownLatestPointerMutation,
    ShutdownOutcomeRecord, ShutdownPlanKey, ShutdownPlanMutation, ShutdownPlanPageView,
    ShutdownPlanRecord, ShutdownPlanView, ShutdownRecoverySnapshotMutation,
    ShutdownTargetKindRecord, ShutdownTargetMutation, ShutdownTargetRecord,
    ShutdownTargetRecoveryRecord, ShutdownTargetStateRecord, StreamId, StreamVersion,
    UncommittedDomainEvent,
};
use crate::usecase::agent_session::operation::{
    constant_time_eq_32, decode_recovery_completed_result, derive_recovery_action_id,
    validate_operation_identity, OperationBindingAuthority, RecoveryActionError,
    RecoveryActionIdentity, RecoveryActionOutcome, RecoveryActionRejection,
    RecoveryActionResultOutcome, RecoveryActionStatus,
};

const PREPARATION_CUTOFF: Duration = Duration::from_secs(13);
const DECISION_DEADLINE: Duration = Duration::from_secs(15);
const MAX_TARGETS: usize = 4096;
const MAX_ACCEPTANCE_MUTATIONS: usize = 8192;

#[derive(Debug, Clone, Copy)]
struct ShutdownDeadlines {
    preparation_cutoff: tokio::time::Instant,
    decision_deadline: tokio::time::Instant,
}

impl ShutdownDeadlines {
    fn from_ingress(ingress: tokio::time::Instant) -> Self {
        Self {
            preparation_cutoff: ingress + PREPARATION_CUTOFF,
            decision_deadline: ingress + DECISION_DEADLINE,
        }
    }

    fn from_receipt(receipt: &ApplicationQuitReceipt) -> Self {
        let now = tokio::time::Instant::now();
        let wall_now_ms = now_ms();
        let preparation_remaining_ms = receipt
            .t0_ms
            .saturating_add(PREPARATION_CUTOFF.as_millis() as i64)
            .saturating_sub(wall_now_ms)
            .max(0) as u64;
        let decision_remaining_ms = receipt.deadline_ms.saturating_sub(wall_now_ms).max(0) as u64;
        Self {
            preparation_cutoff: now + Duration::from_millis(preparation_remaining_ms),
            decision_deadline: now + Duration::from_millis(decision_remaining_ms),
        }
    }
}

#[cfg(test)]
type ShutdownRecoveryPreHandoffHook =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> + Send + Sync>;
#[cfg(test)]
type ShutdownPreAcceptanceHook =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> + Send + Sync>;
#[cfg(test)]
type ShutdownPreActivationHook =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationQuitIntent {
    Exit { code: i32 },
    Restart { code: i32 },
}

/// The one-shot process destination granted by a durable shutdown decision.
///
/// This deliberately remains distinct from a bare exit code: Tauri's restart
/// path is a relaunch operation and must never be reconstructed as `exit(code)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationProcessAction {
    Exit { code: i32 },
    Restart { code: i32 },
}

impl From<ApplicationQuitIntent> for ApplicationProcessAction {
    fn from(value: ApplicationQuitIntent) -> Self {
        match value {
            ApplicationQuitIntent::Exit { code } => Self::Exit { code },
            ApplicationQuitIntent::Restart { code } => Self::Restart { code },
        }
    }
}

impl ApplicationProcessAction {
    #[cfg(test)]
    pub fn code(self) -> i32 {
        match self {
            Self::Exit { code } | Self::Restart { code } => code,
        }
    }
}

fn application_quit_intent_from_domain(intent: QuitIntent) -> Option<ApplicationQuitIntent> {
    match intent {
        QuitIntent::Exit { code } => Some(ApplicationQuitIntent::Exit {
            code: i32::try_from(code).ok()?,
        }),
        QuitIntent::Restart { code } => Some(ApplicationQuitIntent::Restart {
            code: i32::try_from(code).ok()?,
        }),
    }
}

fn application_process_action_from_domain(intent: QuitIntent) -> Option<ApplicationProcessAction> {
    Some(application_quit_intent_from_domain(intent)?.into())
}

impl ApplicationQuitIntent {
    fn mode(self) -> &'static str {
        match self {
            Self::Exit { .. } => "exit",
            Self::Restart { .. } => "restart",
        }
    }

    pub fn code(self) -> i32 {
        match self {
            Self::Exit { code } | Self::Restart { code } => code,
        }
    }

    fn domain(self) -> QuitIntent {
        match self {
            Self::Exit { code } => QuitIntent::Exit {
                code: i64::from(code),
            },
            Self::Restart { code } => QuitIntent::Restart {
                code: i64::from(code),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationQuitRequest {
    pub principal: String,
    pub request_id: String,
    pub intent: ApplicationQuitIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownTarget {
    pub target_id: String,
    pub kind: String,
}

#[derive(Debug, Clone)]
struct StoredShutdownTarget {
    ordinal: i64,
    target: ShutdownTarget,
    state: ShutdownTargetStateRecord,
    revision: Revision,
    effect_identity: String,
    recovery_action: Option<ShutdownTargetRecoveryRecord>,
}

struct TargetStateTransition<'a> {
    receipt: &'a ApplicationQuitReceipt,
    plan: &'a ShutdownPlanKey,
    stored: &'a StoredShutdownTarget,
    state: ShutdownTargetStateRecord,
    expected: Revision,
    revision: Revision,
    failure: Option<&'a SafeOperationFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ActivationCommit {
    Activated,
    RejectedBeforeCommit { failure: SafeOperationFailure },
    OutcomeUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationQuitReceipt {
    pub operation_id: String,
    pub shutdown_id: String,
    pub intent: ApplicationQuitIntent,
    pub t0_ms: i64,
    pub deadline_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApplicationQuitProjection {
    Shutdown {
        receipt: ApplicationQuitReceipt,
        state: ApplicationQuitState,
    },
    OutcomeUnknown {
        operation_id: String,
        intent: ApplicationQuitIntent,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApplicationQuitState {
    Preparing,
    Activated,
    Completed,
    OutcomeUnknown {
        operation_id: String,
        shutdown_id: String,
        activation_commit_id: String,
    },
    FailedBeforeActivation {
        failure: SafeOperationFailure,
    },
    ReconciliationRequired {
        failure: SafeOperationFailure,
    },
}

impl ApplicationQuitState {
    pub fn grants_exit_permit(&self) -> bool {
        matches!(
            self,
            // Activated is the irreversible boundary.  A failure or timeout
            // while persisting the final summary must never turn a
            // post-activation quit back into an abort; unresolved targets
            // were already bound to this shutdown for next-boot readback.
            Self::Activated
                | Self::Completed
                | Self::OutcomeUnknown { .. }
                | Self::ReconciliationRequired { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApplicationQuitOutcome {
    Accepted {
        receipt: ApplicationQuitReceipt,
        state: ApplicationQuitState,
    },
    RejectedBeforeCommit {
        failure: SafeOperationFailure,
    },
    OutcomeUnknown {
        request_id: String,
        operation_id: String,
        intent: ApplicationQuitIntent,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurrentApplicationShutdownProjection {
    Current(Option<Box<ApplicationShutdownPlanReadModel>>),
    OutcomeUnknown {
        operation_id: String,
        intent: ApplicationQuitIntent,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationQuitError {
    InvalidRequest,
    PayloadConflict,
    PreviousShutdownReconciliationRequired {
        blocking: Box<ApplicationShutdownPlanReadModel>,
    },
    CapacityExceeded,
    Internal {
        correlation_id: String,
    },
}

#[derive(Clone)]
struct CompletedApplicationQuitFlight {
    receipt: ApplicationQuitReceipt,
    state: ApplicationQuitState,
    operation_binding: [u8; 32],
    join_before_ticket: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationShutdownPlanReadModel {
    pub plan: ShutdownPlanKey,
    pub phase: ApplicationShutdownPhase,
    pub details_state: ShutdownDetailsState,
    pub revision: Revision,
    pub operation_id: String,
    pub intent: String,
    pub exit_code: i32,
    pub t0_ms: i64,
    pub preparation_cutoff_ms: i64,
    pub deadline_ms: i64,
    pub target_count: Option<i64>,
    pub prepared_count: Option<i64>,
    pub effect_reserved_count: Option<i64>,
    pub terminal_count: Option<i64>,
    pub completed_count: Option<i64>,
    pub unresolved_count: Option<i64>,
    pub recovery_snapshot_count: Option<i64>,
    pub recovery_snapshot_id: Option<String>,
    pub outcome: Option<String>,
    pub safe_failure: Option<SafeOperationFailure>,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationShutdownTargetReadModel {
    pub ordinal: i64,
    pub target_key: String,
    pub target_id: String,
    pub kind: String,
    pub effect_identity: String,
    pub state: String,
    pub observation: Option<SafeEffectObservation>,
    pub revision: Revision,
    pub actions: Vec<String>,
    pub action_identities: Vec<RecoveryActionIdentity>,
}

#[derive(Debug, Clone)]
pub struct ShutdownTargetActionRequest {
    pub action_id: String,
    pub plan: ShutdownPlanKey,
    pub ordinal: i64,
    pub target_key: String,
    pub origin_revision: u64,
    pub action: RecoveryActionKind,
}

#[derive(Debug, Clone)]
pub struct ShutdownTargetActionExecution {
    pub outcome: RecoveryActionOutcome,
    pub process_action: Option<ApplicationProcessAction>,
}

struct ShutdownActionPlanClosure {
    expected_head: ExpectedStreamHead,
    event: UncommittedDomainEvent,
    mutations: Vec<LocalStateMutation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationShutdownPlanPageReadModel {
    pub plan: ApplicationShutdownPlanReadModel,
    pub targets: Vec<ApplicationShutdownTargetReadModel>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownEffectReadback {
    Completed,
    ConfirmedNotStarted,
    Ambiguous,
}

#[async_trait::async_trait]
pub trait ShutdownTargetExecutor: Send + Sync {
    /// Fixed target inventory; this must not start any shutdown effect.
    async fn targets(&self) -> Result<Vec<ShutdownTarget>, SafeOperationFailure>;

    /// Quiesce one target after its exact effect identity has been reserved
    /// durably by the coordinator.
    async fn execute_target(
        &self,
        operation_id: &str,
        effect_identity: &str,
        owner_revision: Revision,
        target: &ShutdownTarget,
    ) -> Result<(), SafeOperationFailure>;

    async fn read_target_effect(
        &self,
        operation_id: &str,
        effect_identity: &str,
        owner_revision: Revision,
        target: &ShutdownTarget,
    ) -> Result<ShutdownEffectReadback, SafeOperationFailure>;

    /// Non-target process cleanup (for example the loopback listener) runs
    /// only after all fixed domain targets have reached a terminal result.
    async fn shutdown_subordinates(&self) -> Result<(), SafeOperationFailure> {
        Ok(())
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or(0)
}

fn correlation(label: &str) -> String {
    format!("quit-{label}-{:x}", now_ms())
}

fn shutdown_deadline_failure(label: &str) -> SafeOperationFailure {
    SafeOperationFailure::new(
        SessionOperationFailureKind::DeadlineExceeded,
        true,
        "Shutdown preparation did not complete before its fixed cutoff.",
        correlation(label),
    )
}

fn shutdown_target_kind_label(kind: ShutdownTargetKindRecord) -> &'static str {
    match kind {
        ShutdownTargetKindRecord::AgentSession => "agent_session",
        ShutdownTargetKindRecord::WorkflowExecution => "workflow_execution",
        ShutdownTargetKindRecord::WorkflowNode => "workflow_node",
    }
}

fn shutdown_target_kind_from_label(value: &str) -> Option<ShutdownTargetKindRecord> {
    match value {
        "agent_session" => Some(ShutdownTargetKindRecord::AgentSession),
        "workflow_execution" => Some(ShutdownTargetKindRecord::WorkflowExecution),
        "workflow_node" => Some(ShutdownTargetKindRecord::WorkflowNode),
        _ => None,
    }
}

fn shutdown_target_state_label(state: ShutdownTargetStateRecord) -> &'static str {
    match state {
        ShutdownTargetStateRecord::Prepared => "prepared",
        ShutdownTargetStateRecord::EffectReserved => "effect_reserved",
        ShutdownTargetStateRecord::Completed => "completed",
        ShutdownTargetStateRecord::Failed => "failed",
        ShutdownTargetStateRecord::ReconciliationRequired => "reconciliation_required",
    }
}

fn shutdown_recovery_action_label(action: RecoveryActionKind) -> &'static str {
    match action {
        RecoveryActionKind::ReadAgain => "read_again",
        RecoveryActionKind::RetrySameEffect => "retry_same_effect",
        RecoveryActionKind::UseObservedResult => "use_observed_result",
        RecoveryActionKind::CancelIfSafe => "cancel_if_safe",
        RecoveryActionKind::KeepForManualResolution => "keep_for_manual_resolution",
    }
}

fn shutdown_recovery_outcome(
    classification: RecoveryResultClassification,
) -> RecoveryActionResultOutcome {
    match classification {
        RecoveryResultClassification::Pending
        | RecoveryResultClassification::ConfirmedNoEffect
        | RecoveryResultClassification::Ambiguous => RecoveryActionResultOutcome::Pending,
        RecoveryResultClassification::Succeeded
        | RecoveryResultClassification::CancelledBeforeEffect => {
            RecoveryActionResultOutcome::Terminal
        }
        RecoveryResultClassification::Unchanged => RecoveryActionResultOutcome::Unchanged,
    }
}

fn decode_shutdown_recovery_action_status(
    saved: &RecoveryActionView,
) -> Result<RecoveryActionStatus, RecoveryActionError> {
    let RecoveryAttemptRecord::ShutdownTarget { state, failure, .. } = &saved.attempt else {
        return Err(RecoveryActionError::Internal {
            correlation_id: correlation("shutdown-action-attempt-kind"),
        });
    };
    let Some(completed) = saved.completed.as_ref() else {
        return Ok(match state {
            ObligationStateRecord::OutcomeUnknown => RecoveryActionStatus::OutcomeUnknown {
                action_id: saved.action_id.clone(),
            },
            ObligationStateRecord::ReconciliationRequired | ObligationStateRecord::Failed => {
                RecoveryActionStatus::ReconciliationRequired {
                    action_id: saved.action_id.clone(),
                    failure: failure
                        .clone()
                        .ok_or_else(|| RecoveryActionError::Internal {
                            correlation_id: correlation("shutdown-action-reconciliation-failure"),
                        })?,
                }
            }
            ObligationStateRecord::Prepared
            | ObligationStateRecord::Pending
            | ObligationStateRecord::EffectReserved
            | ObligationStateRecord::Running
            | ObligationStateRecord::WaitingApproval => RecoveryActionStatus::InProgress {
                action_id: saved.action_id.clone(),
            },
            ObligationStateRecord::Completed | ObligationStateRecord::Cancelled => {
                return Err(RecoveryActionError::Internal {
                    correlation_id: correlation("shutdown-action-terminal-result-missing"),
                })
            }
        });
    };
    if *state != ObligationStateRecord::Completed {
        return Err(RecoveryActionError::Internal {
            correlation_id: correlation("shutdown-action-result-state"),
        });
    }
    let expected = decode_recovery_completed_result(completed).ok_or_else(|| {
        RecoveryActionError::Internal {
            correlation_id: correlation("shutdown-action-result-integrity"),
        }
    })?;
    Ok(RecoveryActionStatus::Completed {
        action_id: saved.action_id.clone(),
        result: expected,
    })
}

fn shutdown_action_attempt_matches(
    saved: &RecoveryActionView,
    request: &ShutdownTargetActionRequest,
    resource_ref: &str,
) -> bool {
    matches!(
        &saved.attempt,
        RecoveryAttemptRecord::ShutdownTarget {
            resource_ref: saved_resource_ref,
            plan,
            ordinal,
            target_key,
            origin_revision,
            action,
            ..
        } if saved_resource_ref == resource_ref
            && plan == &request.plan
            && *ordinal == request.ordinal
            && target_key == &request.target_key
            && *origin_revision == request.origin_revision
            && *action == request.action
    )
}

fn shutdown_action_outcome(status: RecoveryActionStatus) -> RecoveryActionOutcome {
    match status {
        RecoveryActionStatus::Completed { action_id, result } => {
            RecoveryActionOutcome::Completed { action_id, result }
        }
        RecoveryActionStatus::OutcomeUnknown { action_id } => {
            RecoveryActionOutcome::ActionOutcomeUnknown { action_id }
        }
        RecoveryActionStatus::InProgress { action_id }
        | RecoveryActionStatus::ReconciliationRequired { action_id, .. } => {
            RecoveryActionOutcome::InProgress { action_id }
        }
    }
}

fn map_shutdown_action_query_error(
    error: crate::domain::local_event::LocalEventQueryError,
) -> RecoveryActionError {
    use crate::domain::local_event::LocalEventQueryError as E;
    match error {
        E::InvalidRequest => RecoveryActionError::InvalidRequest,
        E::NotFound => RecoveryActionError::NotFound,
        E::QueryBusy => RecoveryActionError::QueryBusy,
        E::DeadlineExceeded => RecoveryActionError::DeadlineExceeded,
        E::CursorMismatch => RecoveryActionError::CursorMismatch,
        E::CursorExpired => RecoveryActionError::CursorExpired,
        E::SnapshotMismatch => RecoveryActionError::SnapshotMismatch,
        E::DetailsCompacted => RecoveryActionError::DetailsCompacted,
        E::ResponseTooLarge => RecoveryActionError::ResponseTooLarge,
        E::StorageUnavailable { failure } => RecoveryActionError::StorageUnavailable { failure },
        E::Corrupt { correlation_id }
        | E::IncompatibleStoredEvent { correlation_id }
        | E::Internal { correlation_id }
        | E::ReplayRequired { correlation_id } => RecoveryActionError::Internal { correlation_id },
    }
}

fn operation_receipt_record(
    receipt: &ApplicationQuitReceipt,
    binding_hmac: [u8; 32],
) -> OperationReceiptRecord {
    OperationReceiptRecord::ApplicationQuit {
        operation_id: receipt.operation_id.clone(),
        plan: ShutdownPlanKey {
            shutdown_id: receipt.shutdown_id.clone(),
        },
        intent: receipt.intent.domain(),
        t0_ms: receipt.t0_ms,
        deadline_ms: receipt.deadline_ms,
        binding_hmac,
    }
}

fn decode_operation_receipt_record(
    record: &OperationReceiptRecord,
) -> Option<(ApplicationQuitReceipt, [u8; 32])> {
    let OperationReceiptRecord::ApplicationQuit {
        operation_id,
        plan,
        intent,
        t0_ms,
        deadline_ms,
        binding_hmac,
    } = record
    else {
        return None;
    };
    Some((
        ApplicationQuitReceipt {
            operation_id: operation_id.clone(),
            shutdown_id: plan.shutdown_id.clone(),
            intent: application_quit_intent_from_domain(*intent)?,
            t0_ms: *t0_ms,
            deadline_ms: *deadline_ms,
        },
        *binding_hmac,
    ))
}

fn encode_quit_attempt(intent: ApplicationQuitIntent) -> Vec<u8> {
    serde_json::json!({
        "schema": "application_quit_attempt_v1",
        "intent": intent.mode(),
        "exit_code": intent.code(),
    })
    .to_string()
    .into_bytes()
}

fn decode_quit_attempt(value: &[u8]) -> Option<ApplicationQuitIntent> {
    let value: Value = serde_json::from_slice(value).ok()?;
    if value.get("schema")?.as_str()? != "application_quit_attempt_v1" {
        return None;
    }
    let code = i32::try_from(value.get("exit_code")?.as_i64()?).ok()?;
    match value.get("intent")?.as_str()? {
        "exit" => Some(ApplicationQuitIntent::Exit { code }),
        "restart" => Some(ApplicationQuitIntent::Restart { code }),
        _ => None,
    }
}

fn operation_status_record(state: &ApplicationQuitState) -> OperationStatusRecord {
    let value = match state {
        ApplicationQuitState::Preparing => OperationStatusValue::Preparing,
        ApplicationQuitState::Activated => OperationStatusValue::Activated,
        ApplicationQuitState::Completed => OperationStatusValue::Completed,
        ApplicationQuitState::OutcomeUnknown {
            operation_id,
            shutdown_id,
            activation_commit_id,
        } => OperationStatusValue::OutcomeUnknown {
            operation_id: operation_id.clone(),
            plan: ShutdownPlanKey {
                shutdown_id: shutdown_id.clone(),
            },
            activation_commit_id: activation_commit_id.clone(),
        },
        ApplicationQuitState::FailedBeforeActivation { failure } => {
            OperationStatusValue::FailedBeforeActivation {
                failure: failure.clone(),
            }
        }
        ApplicationQuitState::ReconciliationRequired { failure } => {
            OperationStatusValue::ReconciliationRequired {
                failure: failure.clone(),
            }
        }
    };
    OperationStatusRecord {
        kind: OperationKind::ApplicationQuit,
        value,
    }
}

fn decode_operation_status_record(record: &OperationStatusRecord) -> Option<ApplicationQuitState> {
    if record.kind != OperationKind::ApplicationQuit {
        return None;
    }
    Some(match &record.value {
        OperationStatusValue::Preparing => ApplicationQuitState::Preparing,
        OperationStatusValue::Activated => ApplicationQuitState::Activated,
        OperationStatusValue::Completed => ApplicationQuitState::Completed,
        OperationStatusValue::OutcomeUnknown {
            operation_id,
            plan,
            activation_commit_id,
        } => ApplicationQuitState::OutcomeUnknown {
            operation_id: operation_id.clone(),
            shutdown_id: plan.shutdown_id.clone(),
            activation_commit_id: activation_commit_id.clone(),
        },
        OperationStatusValue::FailedBeforeActivation { failure } => {
            ApplicationQuitState::FailedBeforeActivation {
                failure: failure.clone(),
            }
        }
        OperationStatusValue::ReconciliationRequired { failure } => {
            ApplicationQuitState::ReconciliationRequired {
                failure: failure.clone(),
            }
        }
        OperationStatusValue::Accepted
        | OperationStatusValue::AwaitingProviderStart { .. }
        | OperationStatusValue::AwaitingProviderResponse { .. }
        | OperationStatusValue::Queued { .. }
        | OperationStatusValue::ProviderStartReserved { .. }
        | OperationStatusValue::Running { .. }
        | OperationStatusValue::PermissionCompleted { .. }
        | OperationStatusValue::StopCompleted { .. }
        | OperationStatusValue::ExitPending
        | OperationStatusValue::Exited
        | OperationStatusValue::Failed { .. }
        | OperationStatusValue::Terminal { .. } => return None,
    })
}

fn application_quit_state_identity(state: &ApplicationQuitState) -> Vec<u8> {
    let mut bytes = b"application-quit-status-identity/v2\0".to_vec();
    match state {
        ApplicationQuitState::Preparing => bytes.extend_from_slice(b"preparing"),
        ApplicationQuitState::Activated => bytes.extend_from_slice(b"activated"),
        ApplicationQuitState::Completed => bytes.extend_from_slice(b"completed"),
        ApplicationQuitState::OutcomeUnknown {
            operation_id,
            shutdown_id,
            activation_commit_id,
        } => {
            bytes.extend_from_slice(b"outcome_unknown\0");
            bytes.extend_from_slice(operation_id.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(shutdown_id.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(activation_commit_id.as_bytes());
        }
        ApplicationQuitState::FailedBeforeActivation { failure } => {
            bytes.extend_from_slice(b"failed_before_activation\0");
            bytes.extend_from_slice(failure.correlation_id.as_bytes());
        }
        ApplicationQuitState::ReconciliationRequired { failure } => {
            bytes.extend_from_slice(b"reconciliation_required\0");
            bytes.extend_from_slice(failure.correlation_id.as_bytes());
        }
    }
    bytes
}

pub struct ShutdownCoordinator {
    repository: Arc<dyn LocalEventTransactionRepository>,
    authority: Arc<dyn OperationBindingAuthority>,
    executor: Arc<dyn ShutdownTargetExecutor>,
    installation_id: String,
    process_instance_id: String,
    request_flight: tokio::sync::Mutex<()>,
    ingress_sequence: AtomicU64,
    #[cfg(test)]
    recovery_pre_handoff_hook: StdMutex<Option<ShutdownRecoveryPreHandoffHook>>,
    #[cfg(test)]
    pre_acceptance_hook: StdMutex<Option<ShutdownPreAcceptanceHook>>,
    #[cfg(test)]
    pre_activation_hook: StdMutex<Option<ShutdownPreActivationHook>>,
    /// A completed graceful flight clears the durable current pointer. Its
    /// immutable result remains joinable only by ingress tickets registered
    /// before that flight reached its terminal fence; later requests must run
    /// the durable detail-capacity and acceptance gates for a new flight.
    completed_flight: StdMutex<Option<CompletedApplicationQuitFlight>>,
}

impl ShutdownCoordinator {
    pub fn new(
        repository: Arc<dyn LocalEventTransactionRepository>,
        authority: Arc<dyn OperationBindingAuthority>,
        executor: Arc<dyn ShutdownTargetExecutor>,
        installation_id: String,
        process_instance_id: String,
    ) -> Self {
        Self {
            repository,
            authority,
            executor,
            installation_id,
            process_instance_id,
            request_flight: tokio::sync::Mutex::new(()),
            ingress_sequence: AtomicU64::new(0),
            #[cfg(test)]
            recovery_pre_handoff_hook: StdMutex::new(None),
            #[cfg(test)]
            pre_acceptance_hook: StdMutex::new(None),
            #[cfg(test)]
            pre_activation_hook: StdMutex::new(None),
            completed_flight: StdMutex::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_recovery_pre_handoff_hook(&self, hook: ShutdownRecoveryPreHandoffHook) {
        *self
            .recovery_pre_handoff_hook
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_pre_acceptance_hook(&self, hook: ShutdownPreAcceptanceHook) {
        *self
            .pre_acceptance_hook
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_pre_activation_hook(&self, hook: ShutdownPreActivationHook) {
        *self
            .pre_activation_hook
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn registered_ingress_count(&self) -> u64 {
        self.ingress_sequence.load(Ordering::SeqCst)
    }

    fn operation_id(&self, principal: &str, request_id: &str) -> String {
        hex::encode(
            self.authority.mac(
                format!("application-quit-operation/v1\0{principal}\0{request_id}").as_bytes(),
            ),
        )
    }

    fn activation_commit_id(&self, operation_id: &str) -> String {
        hex::encode(
            self.authority
                .digest(format!("application-quit-activate/v1\0{operation_id}").as_bytes()),
        )
    }

    fn shutdown_target_key(&self, kind: &str, target_id: &str) -> String {
        fn push_lp(material: &mut Vec<u8>, value: &str) {
            let length = u32::try_from(value.len()).unwrap_or(u32::MAX);
            material.extend_from_slice(&length.to_be_bytes());
            material.extend_from_slice(value.as_bytes());
        }
        let mut material = Vec::with_capacity(kind.len() + target_id.len() + 64);
        push_lp(&mut material, "application-shutdown-target/v1");
        push_lp(&mut material, kind);
        push_lp(&mut material, target_id);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.authority.digest(&material))
    }

    fn shutdown_target_resource_ref(
        &self,
        plan: &ShutdownPlanKey,
        ordinal: i64,
        target_key: &str,
    ) -> String {
        format!(
            "shutdown-target:{}:{}:{}",
            plan.shutdown_id, ordinal, target_key
        )
    }

    fn shutdown_action_binding_hash(
        &self,
        resource_ref: &str,
        request: &ShutdownTargetActionRequest,
        effect_identity_sha256: [u8; 32],
        intent: ApplicationQuitIntent,
    ) -> [u8; 32] {
        fn push_lp(material: &mut Vec<u8>, value: &str) {
            material.extend_from_slice(&(value.len() as u64).to_be_bytes());
            material.extend_from_slice(value.as_bytes());
        }
        let mut material = Vec::new();
        push_lp(&mut material, "shutdown-target-recovery-binding/v1");
        push_lp(&mut material, resource_ref);
        push_lp(&mut material, &request.plan.shutdown_id);
        material.extend_from_slice(&request.ordinal.to_be_bytes());
        push_lp(&mut material, &request.target_key);
        material.extend_from_slice(&request.origin_revision.to_be_bytes());
        push_lp(
            &mut material,
            shutdown_recovery_action_label(request.action),
        );
        material.extend_from_slice(&effect_identity_sha256);
        push_lp(&mut material, intent.mode());
        material.extend_from_slice(&intent.code().to_be_bytes());
        self.authority.digest(&material)
    }

    fn caller_key(&self, principal: &str, request_id: &str) -> CallerOperationKey {
        CallerOperationKey {
            principal: principal.to_string(),
            installation_id: self.installation_id.clone(),
            kind: OperationKind::ApplicationQuit,
            caller_request_id: request_id.to_string(),
        }
    }

    async fn get_binding(
        &self,
        key: &CallerOperationKey,
    ) -> Result<Option<OperationBindingView>, ApplicationQuitError> {
        let result = self
            .repository
            .query(LocalEventQuery::OperationBindingByIdentity { key: key.clone() })
            .await
            .map_err(|_| ApplicationQuitError::Internal {
                correlation_id: correlation("binding-lookup"),
            })?;
        let LocalEventQueryResult::OperationBindingByIdentity(binding) = result else {
            return Err(ApplicationQuitError::Internal {
                correlation_id: correlation("binding-shape"),
            });
        };
        Ok(binding)
    }

    async fn save_join_binding(
        &self,
        key: CallerOperationKey,
        operation_id: &str,
        binding_material: &[u8],
        binding_hmac: [u8; 32],
    ) -> Result<bool, ApplicationQuitError> {
        let commit_id = CommitIdentity::parse(&hex::encode(
            self.authority.digest(
                format!(
                    "application-quit-join/v1\0{}\0{}\0{}",
                    key.principal, key.caller_request_id, operation_id
                )
                .as_bytes(),
            ),
        ))
        .map_err(|_| ApplicationQuitError::Internal {
            correlation_id: correlation("join-identity"),
        })?;
        let state_mutations = vec![LocalStateMutation::OperationBinding(
            OperationBindingMutation {
                key: key.clone(),
                operation_id: operation_id.to_string(),
                binding_hmac,
            },
        )];
        let batch = LocalAtomicBatch {
            commit_id: commit_id.clone(),
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: OperationKind::ApplicationQuit.into(),
                idempotency_key: format!("{}.join.{}", operation_id, key.caller_request_id),
                payload_hash: self.authority.digest(binding_material),
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations,
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => Ok(true),
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                match self.repository.resolve_commit(commit_id).await {
                    Ok(CommitResolution::Committed(_)) => Ok(true),
                    Ok(CommitResolution::NotCommitted) => Ok(false),
                    Err(_) => Err(ApplicationQuitError::Internal {
                        correlation_id: correlation("join-resolution"),
                    }),
                }
            }
            Err(CommitBatchError::PayloadConflict) => {
                let stored = self.get_binding(&key).await?;
                if stored.as_ref().is_some_and(|stored| {
                    stored.operation_id == operation_id
                        && constant_time_eq_32(&stored.binding_hmac, &binding_hmac)
                }) {
                    Ok(true)
                } else {
                    Err(ApplicationQuitError::PayloadConflict)
                }
            }
            Err(CommitBatchError::CapacityExceeded | CommitBatchError::SequenceExhausted) => {
                Err(ApplicationQuitError::CapacityExceeded)
            }
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                Err(self.current_shutdown_reconciliation_error().await)
            }
            Err(CommitBatchError::StorageUnavailable { failure }) => {
                Err(ApplicationQuitError::Internal {
                    correlation_id: failure.correlation_id,
                })
            }
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                Err(ApplicationQuitError::Internal { correlation_id })
            }
        }
    }

    pub async fn get_operation(
        &self,
        operation_id: &str,
    ) -> Result<
        Option<(
            ApplicationQuitReceipt,
            ApplicationQuitState,
            [u8; 32],
            Revision,
        )>,
        crate::domain::local_event::LocalEventQueryError,
    > {
        validate_operation_identity(operation_id)
            .map_err(|_| crate::domain::local_event::LocalEventQueryError::InvalidRequest)?;
        let result = self
            .repository
            .query(LocalEventQuery::OperationByIdentity {
                kind: OperationKind::ApplicationQuit,
                operation_id: operation_id.to_string(),
            })
            .await?;
        let LocalEventQueryResult::OperationByIdentity(record) = result else {
            return Err(crate::domain::local_event::LocalEventQueryError::Internal {
                correlation_id: correlation("quit-operation-shape"),
            });
        };
        let decoded = match record {
            None => None,
            Some(record) => {
                let (receipt, binding) = decode_operation_receipt_record(&record.receipt)
                    .ok_or_else(
                        || crate::domain::local_event::LocalEventQueryError::Corrupt {
                            correlation_id: correlation("quit-receipt-decode"),
                        },
                    )?;
                let state =
                    decode_operation_status_record(&record.latest_status).ok_or_else(|| {
                        crate::domain::local_event::LocalEventQueryError::Corrupt {
                            correlation_id: correlation("quit-state-decode"),
                        }
                    })?;
                Some((receipt, state, binding, record.revision))
            }
        };
        let Some((receipt, mut state, binding, revision)) = decoded else {
            return Ok(None);
        };
        if matches!(
            state,
            ApplicationQuitState::Preparing | ApplicationQuitState::Activated
        ) {
            let current = self
                .repository
                .query(LocalEventQuery::CurrentShutdown)
                .await?;
            let LocalEventQueryResult::CurrentShutdown(current) = current else {
                return Err(crate::domain::local_event::LocalEventQueryError::Internal {
                    correlation_id: correlation("current-shape"),
                });
            };
            if current.as_ref().is_some_and(|plan| {
                plan.plan.shutdown_id == receipt.shutdown_id
                    && plan.phase == ApplicationShutdownPhase::ReconciliationRequired
            }) {
                state = ApplicationQuitState::ReconciliationRequired {
                    failure: SafeOperationFailure::new(
                        SessionOperationFailureKind::DeadlineExceeded,
                        true,
                        "The previous boot ended before shutdown reached a terminal decision.",
                        format!("quit-previous-boot-{}", receipt.operation_id),
                    ),
                };
            }
        }
        Ok(Some((receipt, state, binding, revision)))
    }

    fn caller_attempt_journal(
        &self,
    ) -> crate::usecase::agent_session::operation::CallerAttemptJournal {
        crate::usecase::agent_session::operation::CallerAttemptJournal::new(
            self.repository.clone(),
            self.authority.clone(),
            self.installation_id.clone(),
        )
    }

    fn decode_pending_quit_attempt(
        &self,
        attempt: &crate::domain::local_event::CallerAttemptView,
    ) -> Result<ApplicationQuitIntent, crate::domain::local_event::LocalEventQueryError> {
        let exact_command = self
            .caller_attempt_journal()
            .open_attempt_command(attempt)
            .map_err(
                |_| crate::domain::local_event::LocalEventQueryError::Corrupt {
                    correlation_id: correlation("quit-attempt-open"),
                },
            )?;
        decode_quit_attempt(&exact_command).ok_or_else(|| {
            crate::domain::local_event::LocalEventQueryError::Corrupt {
                correlation_id: correlation("quit-attempt-decode"),
            }
        })
    }

    async fn pending_quit_by_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<ApplicationQuitIntent>, crate::domain::local_event::LocalEventQueryError>
    {
        let result = self
            .repository
            .query(LocalEventQuery::PendingCallerAttemptsByOperation {
                installation_id: self.installation_id.clone(),
                kind: OperationKind::ApplicationQuit,
                operation_id: operation_id.to_string(),
                limit: 2,
            })
            .await?;
        let LocalEventQueryResult::PendingCallerAttemptsByOperation(attempts) = result else {
            return Err(crate::domain::local_event::LocalEventQueryError::Internal {
                correlation_id: correlation("pending-quit-operation-shape"),
            });
        };
        let mut origin = attempts.into_iter().filter(|attempt| {
            self.operation_id(&attempt.key.principal, &attempt.key.caller_request_id)
                == operation_id
        });
        let Some(attempt) = origin.next() else {
            return Ok(None);
        };
        if origin.next().is_some() {
            return Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                correlation_id: correlation("pending-quit-operation-duplicate"),
            });
        }
        self.decode_pending_quit_attempt(&attempt).map(Some)
    }

    async fn pending_current_quit(
        &self,
    ) -> Result<
        Option<(String, ApplicationQuitIntent)>,
        crate::domain::local_event::LocalEventQueryError,
    > {
        let result = self
            .repository
            .query(LocalEventQuery::PendingCallerAttemptsByKind {
                installation_id: self.installation_id.clone(),
                kind: OperationKind::ApplicationQuit,
                limit: 3,
            })
            .await?;
        let LocalEventQueryResult::PendingCallerAttemptsByKind(attempts) = result else {
            return Err(crate::domain::local_event::LocalEventQueryError::Internal {
                correlation_id: correlation("pending-current-quit-shape"),
            });
        };
        let mut origins = attempts.into_iter().filter_map(|attempt| {
            let operation_id = attempt.operation_id.clone()?;
            (self.operation_id(&attempt.key.principal, &attempt.key.caller_request_id)
                == operation_id)
                .then_some((operation_id, attempt))
        });
        let Some((operation_id, attempt)) = origins.next() else {
            return Ok(None);
        };
        if origins.next().is_some() {
            return Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                correlation_id: correlation("pending-current-quit-duplicate"),
            });
        }
        Ok(Some((
            operation_id,
            self.decode_pending_quit_attempt(&attempt)?,
        )))
    }

    async fn application_quit_binding_summary(
        &self,
        operation_id: &str,
        expected_binding_hmac: Option<[u8; 32]>,
    ) -> Result<
        crate::domain::local_event::OperationBindingSummaryView,
        crate::domain::local_event::LocalEventQueryError,
    > {
        let result = self
            .repository
            .query(LocalEventQuery::OperationBindingSummaryByOperation {
                installation_id: self.installation_id.clone(),
                kind: OperationKind::ApplicationQuit,
                operation_id: operation_id.to_string(),
                expected_binding_hmac,
            })
            .await?;
        let LocalEventQueryResult::OperationBindingSummaryByOperation(summary) = result else {
            return Err(crate::domain::local_event::LocalEventQueryError::Internal {
                correlation_id: correlation("quit-projection-binding-shape"),
            });
        };
        Ok(summary)
    }

    async fn validate_normal_quit_projection_reference(
        &self,
        receipt: &ApplicationQuitReceipt,
        state: &ApplicationQuitState,
    ) -> Result<(), crate::domain::local_event::LocalEventQueryError> {
        if let ApplicationQuitState::OutcomeUnknown {
            operation_id,
            shutdown_id,
            ..
        } = state
        {
            if operation_id != &receipt.operation_id || shutdown_id != &receipt.shutdown_id {
                return Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                    correlation_id: correlation("quit-projection-unknown-reference"),
                });
            }
        }
        let result = self
            .repository
            .query(LocalEventQuery::ShutdownPlanPage {
                plan: ShutdownPlanKey {
                    shutdown_id: receipt.shutdown_id.clone(),
                },
                limit: 1,
                cursor: None,
            })
            .await;
        let page = match result {
            Ok(LocalEventQueryResult::ShutdownPlanPage(page)) => page,
            Ok(_) => {
                return Err(crate::domain::local_event::LocalEventQueryError::Internal {
                    correlation_id: correlation("quit-projection-plan-shape"),
                })
            }
            Err(crate::domain::local_event::LocalEventQueryError::NotFound)
                if matches!(state, ApplicationQuitState::OutcomeUnknown { .. }) =>
            {
                return Ok(())
            }
            Err(crate::domain::local_event::LocalEventQueryError::NotFound) => {
                return Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                    correlation_id: correlation("quit-projection-plan-missing"),
                })
            }
            Err(error) => return Err(error),
        };
        let plan = self.decode_shutdown_plan_read_model(page.plan)?;
        if plan.operation_id != receipt.operation_id
            || plan.intent != receipt.intent.mode()
            || plan.exit_code != receipt.intent.code()
            || plan.t0_ms != receipt.t0_ms
            || plan.deadline_ms != receipt.deadline_ms
        {
            return Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                correlation_id: correlation("quit-projection-plan-reference"),
            });
        }
        Ok(())
    }

    pub async fn get_application_quit_projection(
        &self,
        operation_id: &str,
    ) -> Result<Option<ApplicationQuitProjection>, crate::domain::local_event::LocalEventQueryError>
    {
        validate_operation_identity(operation_id)
            .map_err(|_| crate::domain::local_event::LocalEventQueryError::InvalidRequest)?;
        if let Some(intent) = self.pending_quit_by_operation(operation_id).await? {
            return Ok(Some(ApplicationQuitProjection::OutcomeUnknown {
                operation_id: operation_id.to_string(),
                intent,
            }));
        }
        let result = self
            .repository
            .query(LocalEventQuery::OperationByIdentity {
                kind: OperationKind::ApplicationQuit,
                operation_id: operation_id.to_string(),
            })
            .await?;
        let LocalEventQueryResult::OperationByIdentity(record) = result else {
            return Err(crate::domain::local_event::LocalEventQueryError::Internal {
                correlation_id: correlation("quit-projection-operation-shape"),
            });
        };
        let Some(record) = record else {
            let bindings = self
                .application_quit_binding_summary(operation_id, None)
                .await?;
            if bindings.total_count == 0 {
                return Ok(None);
            }
            return Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                correlation_id: correlation("quit-projection-operation-missing"),
            });
        };
        match &record.receipt {
            OperationReceiptRecord::ApplicationQuit { .. } => {
                let Some((receipt, state, binding, _)) = self.get_operation(operation_id).await?
                else {
                    return Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                        correlation_id: correlation("quit-projection-shutdown-missing"),
                    });
                };
                if receipt.operation_id != operation_id {
                    return Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                        correlation_id: correlation("quit-projection-operation-reference"),
                    });
                }
                let bindings = self
                    .application_quit_binding_summary(operation_id, Some(binding))
                    .await?;
                if bindings.total_count == 0 || bindings.matching_binding_count != 1 {
                    return Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                        correlation_id: correlation("quit-projection-binding-integrity"),
                    });
                }
                self.validate_normal_quit_projection_reference(&receipt, &state)
                    .await?;
                Ok(Some(ApplicationQuitProjection::Shutdown { receipt, state }))
            }
            OperationReceiptRecord::Send { .. }
            | OperationReceiptRecord::PermissionResponse { .. }
            | OperationReceiptRecord::Stop { .. }
            | OperationReceiptRecord::SessionLifecycle { .. } => {
                Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                    correlation_id: correlation("quit-projection-receipt-version"),
                })
            }
        }
    }

    pub async fn current_shutdown(
        &self,
    ) -> Result<Option<ShutdownPlanView>, crate::domain::local_event::LocalEventQueryError> {
        let result = self
            .repository
            .query(LocalEventQuery::CurrentShutdown)
            .await?;
        let LocalEventQueryResult::CurrentShutdown(plan) = result else {
            return Err(crate::domain::local_event::LocalEventQueryError::Internal {
                correlation_id: correlation("current-shape"),
            });
        };
        Ok(plan)
    }

    fn is_previous_boot_nonterminal_plan(
        &self,
        phase: ApplicationShutdownPhase,
        summary: &ShutdownPlanRecord,
    ) -> bool {
        matches!(
            phase,
            ApplicationShutdownPhase::Prepared
                | ApplicationShutdownPhase::Activated
                | ApplicationShutdownPhase::Quiescing
        ) && summary.process_instance_id != self.process_instance_id
    }

    fn has_exit_coupled_observation(
        &self,
        phase: ApplicationShutdownPhase,
        summary: &ShutdownPlanRecord,
    ) -> bool {
        matches!(
            phase,
            ApplicationShutdownPhase::Prepared
                | ApplicationShutdownPhase::Activated
                | ApplicationShutdownPhase::Quiescing
        ) && summary.process_instance_id != self.process_instance_id
    }

    fn decode_shutdown_plan_read_model(
        &self,
        value: ShutdownPlanView,
    ) -> Result<ApplicationShutdownPlanReadModel, crate::domain::local_event::LocalEventQueryError>
    {
        let summary = &value.summary;
        let phase = if self.is_previous_boot_nonterminal_plan(value.phase, summary) {
            ApplicationShutdownPhase::ReconciliationRequired
        } else {
            value.phase
        };
        let exact_counts_required = value.details_state == ShutdownDetailsState::Compacted
            || matches!(
                phase,
                ApplicationShutdownPhase::Completed
                    | ApplicationShutdownPhase::Failed
                    | ApplicationShutdownPhase::Cancelled
                    | ApplicationShutdownPhase::ReconciliationRequired
            );
        let count = |key: &str, raw: Option<u64>| {
            let Some(raw) = raw else {
                return if exact_counts_required {
                    Err(crate::domain::local_event::LocalEventQueryError::Internal {
                        correlation_id: correlation(&format!("shutdown-summary-{key}-missing")),
                    })
                } else {
                    Ok(None)
                };
            };
            let value = i64::try_from(raw).map_err(|_| {
                crate::domain::local_event::LocalEventQueryError::Internal {
                    correlation_id: correlation(&format!("shutdown-summary-{key}-invalid")),
                }
            })?;
            Ok(Some(value))
        };
        let target_count = count("target_count", summary.target_count)?;
        let prepared_count = count("prepared_count", summary.prepared_count)?;
        let effect_reserved_count = count("effect_reserved_count", summary.effect_reserved_count)?;
        let terminal_count = count("terminal_count", summary.terminal_count)?;
        let completed_count = count("completed_count", summary.completed_count)?;
        let unresolved_count = count("unresolved_count", summary.unresolved_count)?;
        let recovery_snapshot_count =
            count("recovery_snapshot_count", summary.recovery_snapshot_count)?;
        if let (Some(target_count), Some(completed_count), Some(unresolved_count)) =
            (target_count, completed_count, unresolved_count)
        {
            if completed_count > target_count
                || unresolved_count != target_count.saturating_sub(completed_count)
            {
                return Err(crate::domain::local_event::LocalEventQueryError::Internal {
                    correlation_id: correlation("shutdown-summary-count-integrity"),
                });
            }
        }
        let recovery_snapshot_id = summary.recovery_snapshot_id.clone();
        let outcome = summary.outcome.map(|outcome| match outcome {
            ShutdownOutcomeRecord::Completed => "completed".to_string(),
            ShutdownOutcomeRecord::AbortedBeforeActivation => {
                "aborted_before_activation".to_string()
            }
            ShutdownOutcomeRecord::ReconciliationRequired => "reconciliation_required".to_string(),
        });
        let intent = application_quit_intent_from_domain(summary.intent).ok_or_else(|| {
            crate::domain::local_event::LocalEventQueryError::Internal {
                correlation_id: correlation("shutdown-summary-exit-code"),
            }
        })?;
        let preparation_cutoff_ms = summary.preparation_cutoff_ms.ok_or_else(|| {
            crate::domain::local_event::LocalEventQueryError::Internal {
                correlation_id: correlation("shutdown-summary-preparation-cutoff-ms"),
            }
        })?;
        if summary.operation_id.is_empty() || summary.process_instance_id.is_empty() {
            return Err(crate::domain::local_event::LocalEventQueryError::Internal {
                correlation_id: correlation("shutdown-summary-reference"),
            });
        };
        Ok(ApplicationShutdownPlanReadModel {
            plan: value.plan,
            phase,
            details_state: value.details_state,
            revision: value.revision,
            operation_id: summary.operation_id.clone(),
            intent: intent.mode().to_string(),
            exit_code: intent.code(),
            t0_ms: summary.t0_ms,
            preparation_cutoff_ms,
            deadline_ms: summary.deadline_ms,
            target_count,
            prepared_count,
            effect_reserved_count,
            terminal_count,
            completed_count,
            unresolved_count,
            recovery_snapshot_count,
            recovery_snapshot_id,
            outcome,
            safe_failure: summary.failure.clone(),
            actions: Vec::new(),
        })
    }

    fn previous_shutdown_reconciliation_error(
        &self,
        plan: ShutdownPlanView,
    ) -> ApplicationQuitError {
        match self.decode_shutdown_plan_read_model(plan) {
            Ok(blocking) => ApplicationQuitError::PreviousShutdownReconciliationRequired {
                blocking: Box::new(blocking),
            },
            Err(error) => ApplicationQuitError::Internal {
                correlation_id: correlation(&format!("blocking-shutdown-decode-{error}")),
            },
        }
    }

    async fn current_shutdown_reconciliation_error(&self) -> ApplicationQuitError {
        match self.current_shutdown().await {
            Ok(Some(plan)) => self.previous_shutdown_reconciliation_error(plan),
            Ok(None) => ApplicationQuitError::Internal {
                correlation_id: correlation("blocking-shutdown-missing"),
            },
            Err(error) => ApplicationQuitError::Internal {
                correlation_id: correlation(&format!("blocking-shutdown-read-{error}")),
            },
        }
    }

    pub async fn current_shutdown_read_model(
        &self,
    ) -> Result<
        Option<ApplicationShutdownPlanReadModel>,
        crate::domain::local_event::LocalEventQueryError,
    > {
        let Some(value) = self.current_shutdown().await? else {
            return Ok(None);
        };
        let retry_quit = self.retry_quit_available(&value).await?;
        let mut projection = self.decode_shutdown_plan_read_model(value)?;
        if retry_quit {
            projection.actions.push("retry_quit".to_string());
        }
        Ok(Some(projection))
    }

    pub async fn current_application_shutdown_projection(
        &self,
    ) -> Result<
        CurrentApplicationShutdownProjection,
        crate::domain::local_event::LocalEventQueryError,
    > {
        if let Some((operation_id, intent)) = self.pending_current_quit().await? {
            return Ok(CurrentApplicationShutdownProjection::OutcomeUnknown {
                operation_id,
                intent,
            });
        }
        let current = match self.current_shutdown_read_model().await? {
            Some(current) => Some(current),
            None => {
                // A successful terminal decision atomically clears the durable
                // current pointer.  The accepting same-boot coordinator still
                // owns the completed flight, so expose that exact terminal
                // plan until process restart.  A fresh coordinator has no such
                // flight and therefore returns Current(None); callers can use
                // the exact history query with the saved plan identity.
                let completed = self
                    .completed_flight
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                match completed {
                    Some(completed)
                        if matches!(completed.state, ApplicationQuitState::Completed) =>
                    {
                        Some(
                            self.shutdown_plan_page_read_model(
                                ShutdownPlanKey {
                                    shutdown_id: completed.receipt.shutdown_id,
                                },
                                1,
                                None,
                            )
                            .await?
                            .plan,
                        )
                    }
                    _ => None,
                }
            }
        };
        let Some(mut current) = current else {
            return Ok(CurrentApplicationShutdownProjection::Current(None));
        };

        let authority = self
            .get_application_quit_projection(&current.operation_id)
            .await?;
        let (receipt, state) = match authority {
            Some(ApplicationQuitProjection::Shutdown { receipt, state }) => (receipt, state),
            Some(ApplicationQuitProjection::OutcomeUnknown {
                operation_id,
                intent,
            }) if operation_id == current.operation_id => {
                return Ok(CurrentApplicationShutdownProjection::OutcomeUnknown {
                    operation_id,
                    intent,
                })
            }
            Some(ApplicationQuitProjection::OutcomeUnknown { .. }) | None => {
                return Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                    correlation_id: correlation("current-shutdown-operation-reference"),
                })
            }
        };
        if receipt.operation_id != current.operation_id
            || receipt.shutdown_id != current.plan.shutdown_id
            || receipt.intent.mode() != current.intent
            || receipt.intent.code() != current.exit_code
            || receipt.t0_ms != current.t0_ms
            || receipt.deadline_ms != current.deadline_ms
        {
            return Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                correlation_id: correlation("current-shutdown-receipt-reference"),
            });
        }
        if let ApplicationQuitState::OutcomeUnknown {
            operation_id,
            shutdown_id,
            ..
        } = &state
        {
            if operation_id != &current.operation_id || shutdown_id != &current.plan.shutdown_id {
                return Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                    correlation_id: correlation("current-shutdown-unknown-reference"),
                });
            }
            return Ok(CurrentApplicationShutdownProjection::OutcomeUnknown {
                operation_id: receipt.operation_id,
                intent: receipt.intent,
            });
        }

        let authority_matches = matches!(
            (current.phase, &state),
            (
                ApplicationShutdownPhase::Prepared,
                ApplicationQuitState::Preparing
            ) | (
                ApplicationShutdownPhase::Activated | ApplicationShutdownPhase::Quiescing,
                ApplicationQuitState::Activated
            ) | (
                ApplicationShutdownPhase::Completed,
                ApplicationQuitState::Completed
            ) | (
                ApplicationShutdownPhase::Failed | ApplicationShutdownPhase::Cancelled,
                ApplicationQuitState::FailedBeforeActivation { .. }
            ) | (
                ApplicationShutdownPhase::ReconciliationRequired,
                ApplicationQuitState::ReconciliationRequired { .. }
            )
        );
        if !authority_matches {
            current.phase = ApplicationShutdownPhase::ReconciliationRequired;
            current.safe_failure = Some(SafeOperationFailure::new(
                SessionOperationFailureKind::ShutdownAuthorityMismatch,
                true,
                "The durable shutdown authorities disagree and require reconciliation.",
                format!("shutdown-authority-mismatch-{}", current.operation_id),
            ));
            current.actions.clear();
        } else if let ApplicationQuitState::ReconciliationRequired { failure } = state {
            current.safe_failure = Some(failure);
        }
        Ok(CurrentApplicationShutdownProjection::Current(Some(
            Box::new(current),
        )))
    }

    pub async fn retry_quit_available(
        &self,
        plan: &ShutdownPlanView,
    ) -> Result<bool, crate::domain::local_event::LocalEventQueryError> {
        let result = self
            .repository
            .query(LocalEventQuery::RetryQuitEligibility {
                plan: plan.plan.clone(),
                revision: plan.revision,
            })
            .await?;
        let LocalEventQueryResult::RetryQuitEligibility(available) = result else {
            return Err(crate::domain::local_event::LocalEventQueryError::Internal {
                correlation_id: correlation("retry-quit-eligibility-shape"),
            });
        };
        Ok(available)
    }

    async fn ensure_shutdown_detail_capacity(&self) -> Result<(), ApplicationQuitError> {
        let result = self
            .repository
            .query(LocalEventQuery::AvailableShutdownHistory { limit: 3 })
            .await
            .map_err(|error| ApplicationQuitError::Internal {
                correlation_id: correlation(&format!("shutdown-history-read-{error}")),
            })?;
        let LocalEventQueryResult::AvailableShutdownHistory(mut available) = result else {
            return Err(ApplicationQuitError::Internal {
                correlation_id: correlation("history-shape"),
            });
        };
        if available.len() < 2 {
            return Ok(());
        }
        let oldest = available.remove(0);
        if !matches!(
            oldest.phase,
            ApplicationShutdownPhase::Completed
                | ApplicationShutdownPhase::Failed
                | ApplicationShutdownPhase::Cancelled
        ) {
            return Err(self.previous_shutdown_reconciliation_error(oldest));
        }
        self.compact_shutdown_details(oldest.plan).await?;
        Ok(())
    }

    fn background_compactor(&self) -> Self {
        Self {
            repository: Arc::clone(&self.repository),
            authority: Arc::clone(&self.authority),
            executor: Arc::clone(&self.executor),
            installation_id: self.installation_id.clone(),
            process_instance_id: self.process_instance_id.clone(),
            request_flight: tokio::sync::Mutex::new(()),
            ingress_sequence: AtomicU64::new(0),
            #[cfg(test)]
            recovery_pre_handoff_hook: StdMutex::new(None),
            #[cfg(test)]
            pre_acceptance_hook: StdMutex::new(None),
            #[cfg(test)]
            pre_activation_hook: StdMutex::new(None),
            completed_flight: StdMutex::new(None),
        }
    }

    fn schedule_oldest_shutdown_detail_compaction(&self) {
        let worker = self.background_compactor();
        tokio::spawn(async move {
            let Ok(LocalEventQueryResult::AvailableShutdownHistory(mut available)) = worker
                .repository
                .query(LocalEventQuery::AvailableShutdownHistory { limit: 3 })
                .await
            else {
                return;
            };
            if available.len() < 2 {
                return;
            }
            let oldest = available.remove(0);
            if matches!(
                oldest.phase,
                ApplicationShutdownPhase::Completed
                    | ApplicationShutdownPhase::Failed
                    | ApplicationShutdownPhase::Cancelled
            ) {
                if let Err(error) = worker.compact_shutdown_details(oldest.plan).await {
                    log::warn!("background shutdown detail compaction deferred: {error:?}");
                }
            }
        });
    }

    pub async fn shutdown_plan_page(
        &self,
        plan: ShutdownPlanKey,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<ShutdownPlanPageView, crate::domain::local_event::LocalEventQueryError> {
        let result = self
            .repository
            .query(LocalEventQuery::ShutdownPlanPage {
                plan,
                limit,
                cursor: cursor.map(QueryCursor::from_opaque),
            })
            .await?;
        let LocalEventQueryResult::ShutdownPlanPage(page) = result else {
            return Err(crate::domain::local_event::LocalEventQueryError::Internal {
                correlation_id: correlation("shutdown-plan-shape"),
            });
        };
        Ok(page)
    }

    pub async fn shutdown_plan_page_read_model(
        &self,
        plan: ShutdownPlanKey,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<
        ApplicationShutdownPlanPageReadModel,
        crate::domain::local_event::LocalEventQueryError,
    > {
        let page = self.shutdown_plan_page(plan, limit, cursor).await?;
        let stored_plan_phase = page.plan.phase;
        let stored_plan_summary = page.plan.summary.clone();
        let exit_coupled =
            self.has_exit_coupled_observation(stored_plan_phase, &stored_plan_summary);
        let retry_quit = self.retry_quit_available(&page.plan).await?;
        let mut plan = self.decode_shutdown_plan_read_model(page.plan)?;
        if retry_quit {
            plan.actions.push("retry_quit".to_string());
        }
        let mut targets = Vec::with_capacity(page.targets.len());
        for target in page.targets {
            let ShutdownTargetRecord::Target {
                target_id,
                kind,
                state: stored_state,
                effect_identity,
                ..
            } = target.detail
            else {
                return Err(crate::domain::local_event::LocalEventQueryError::Corrupt {
                    correlation_id: correlation("shutdown-target-kind"),
                });
            };
            let observation = (exit_coupled
                && matches!(
                    stored_state,
                    ShutdownTargetStateRecord::Prepared | ShutdownTargetStateRecord::EffectReserved
                ))
            .then(|| SafeEffectObservation::ExitCoupledOutcomeUnknown {
                shutdown_id: plan.plan.shutdown_id.clone(),
            });
            let state = if observation.is_some() {
                "reconciliation_required".to_string()
            } else if stored_state == ShutdownTargetStateRecord::Prepared
                && matches!(
                    plan.phase,
                    ApplicationShutdownPhase::Failed | ApplicationShutdownPhase::Cancelled
                )
            {
                "cancelled_before_activation".to_string()
            } else {
                shutdown_target_state_label(stored_state).to_string()
            };
            let kind = shutdown_target_kind_label(kind).to_string();
            let target_key = self.shutdown_target_key(&kind, &target_id);
            let actions = (state == "reconciliation_required")
                .then(|| "retry_same_effect".to_string())
                .into_iter()
                .collect::<Vec<_>>();
            let action_identities = if state == "reconciliation_required" {
                let action = RecoveryActionKind::RetrySameEffect;
                let resource_ref =
                    self.shutdown_target_resource_ref(&plan.plan, target.ordinal, &target_key);
                vec![RecoveryActionIdentity {
                    action_id: derive_recovery_action_id(
                        &*self.authority,
                        &self.installation_id,
                        &resource_ref,
                        target.revision.value() as u64,
                        action,
                    ),
                    action,
                    origin_revision: target.revision.value() as u64,
                }]
            } else {
                Vec::new()
            };
            targets.push(ApplicationShutdownTargetReadModel {
                ordinal: target.ordinal,
                target_key,
                target_id,
                kind,
                effect_identity,
                state,
                observation,
                revision: target.revision,
                actions,
                action_identities,
            });
        }
        Ok(ApplicationShutdownPlanPageReadModel {
            plan,
            targets,
            next_cursor: page.next_cursor.map(|cursor| cursor.as_str().to_string()),
        })
    }

    async fn shutdown_recovery_action_record(
        &self,
        action_id: &str,
    ) -> Result<Option<RecoveryActionView>, RecoveryActionError> {
        let result = self
            .repository
            .query(LocalEventQuery::RecoveryActionByIdentity {
                action_id: action_id.to_string(),
            })
            .await
            .map_err(map_shutdown_action_query_error)?;
        let LocalEventQueryResult::RecoveryActionByIdentity(action) = result else {
            return Err(RecoveryActionError::Internal {
                correlation_id: correlation("shutdown-action-query-shape"),
            });
        };
        Ok(action)
    }

    /// Build the terminal plan participants only when the claimed target is
    /// the final unresolved target. They are appended to the action-result
    /// batch so a crash cannot expose Completed(target/action) with a
    /// permanently nonterminal plan.
    async fn prepare_shutdown_action_plan_closure(
        &self,
        receipt: &ApplicationQuitReceipt,
        binding: [u8; 32],
        plan: &ShutdownPlanKey,
        action_ordinal: i64,
    ) -> Result<Option<ShutdownActionPlanClosure>, RecoveryActionError> {
        let (_, _, targets) =
            self.prepared_targets(plan)
                .await
                .map_err(|failure| RecoveryActionError::Internal {
                    correlation_id: failure.correlation_id,
                })?;
        if !targets
            .iter()
            .any(|target| target.ordinal == action_ordinal)
            || targets.iter().any(|target| {
                target.ordinal != action_ordinal
                    && target.state != ShutdownTargetStateRecord::Completed
            })
        {
            return Ok(None);
        }
        let current_plan = self
            .current_shutdown()
            .await
            .map_err(map_shutdown_action_query_error)?
            .filter(|current| current.plan == *plan)
            .ok_or_else(|| RecoveryActionError::Internal {
                correlation_id: correlation("shutdown-action-current-plan"),
            })?;
        if current_plan.details_state != ShutdownDetailsState::Available {
            return Ok(None);
        }
        let (_, _, _, operation_revision) = self
            .get_operation(&receipt.operation_id)
            .await
            .map_err(map_shutdown_action_query_error)?
            .ok_or_else(|| RecoveryActionError::Internal {
                correlation_id: correlation("shutdown-action-current-operation"),
            })?;
        let next_operation_revision = operation_revision
            .next()
            .ok_or(RecoveryActionError::InvalidRequest)?;
        let next_plan_revision = current_plan
            .revision
            .next()
            .ok_or(RecoveryActionError::InvalidRequest)?;

        // This is the same final non-target shutdown effect used by the
        // normal coordinator. It runs only after every fixed target except
        // this durably claimed target is terminal.
        match tokio::time::timeout(
            Duration::from_secs(10),
            self.executor.shutdown_subordinates(),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => return Ok(None),
        }

        let mut summary = current_plan.summary.clone();
        let target_count =
            u64::try_from(targets.len()).map_err(|_| RecoveryActionError::InvalidRequest)?;
        summary.target_count = Some(target_count);
        summary.prepared_count = Some(0);
        summary.effect_reserved_count = Some(0);
        summary.terminal_count = Some(target_count);
        summary.completed_count = Some(target_count);
        summary.unresolved_count = Some(0);
        summary.outcome = Some(ShutdownOutcomeRecord::Completed);
        summary.failure = None;
        let completed_state = ApplicationQuitState::Completed;
        let stream_id = StreamId::application();
        let head = self
            .application_head()
            .await
            .map_err(|_| RecoveryActionError::Internal {
                correlation_id: correlation("shutdown-action-application-head"),
            })?;
        let at_ms = now_ms();
        Ok(Some(ShutdownActionPlanClosure {
            expected_head: ExpectedStreamHead {
                stream_id: stream_id.clone(),
                expected: StreamVersion::new(head).expect("nonnegative head"),
            },
            event: UncommittedDomainEvent {
                stream_id,
                event: LocalDomainEvent::Application(
                    ApplicationDomainEvent::ShutdownPhaseAdvanced {
                        shutdown_id: plan.shutdown_id.clone(),
                        phase: ApplicationShutdownPhase::Completed,
                        at_ms,
                    },
                ),
                occurred_at_ms: at_ms,
            },
            mutations: vec![
                LocalStateMutation::OperationRecord(OperationRecordMutation {
                    kind: OperationKind::ApplicationQuit,
                    operation_id: receipt.operation_id.clone(),
                    receipt: operation_receipt_record(receipt, binding),
                    latest_status: operation_status_record(&completed_state),
                    expected: RevisionGuard::Expected(operation_revision),
                    revision: next_operation_revision,
                }),
                LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                    key: plan.clone(),
                    phase: ApplicationShutdownPhase::Completed,
                    summary,
                    details_state: ShutdownDetailsState::Available,
                    expected: RevisionGuard::Expected(current_plan.revision),
                    revision: next_plan_revision,
                }),
                LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                    expected: Some(plan.clone()),
                    new: None,
                }),
            ],
        }))
    }

    pub async fn get_shutdown_target_action_status(
        &self,
        action_id: &str,
    ) -> Result<RecoveryActionStatus, RecoveryActionError> {
        validate_operation_identity(action_id).map_err(|_| RecoveryActionError::InvalidRequest)?;
        let saved = self
            .shutdown_recovery_action_record(action_id)
            .await?
            .ok_or(RecoveryActionError::NotFound)?;
        decode_shutdown_recovery_action_status(&saved)
    }

    /// Resolve one backend-issued shutdown target action. The durable action
    /// decision is always read before the mutable plan/detail resource, so a
    /// completed response remains replayable after plan compaction.
    pub async fn resolve_shutdown_target_action(
        &self,
        request: ShutdownTargetActionRequest,
    ) -> Result<ShutdownTargetActionExecution, RecoveryActionError> {
        validate_operation_identity(&request.action_id)
            .map_err(|_| RecoveryActionError::InvalidRequest)?;
        if request.ordinal < 0
            || request.origin_revision > i64::MAX as u64
            || request.target_key.is_empty()
            || request.target_key.len() > 128
        {
            return Err(RecoveryActionError::InvalidRequest);
        }
        let resource_ref =
            self.shutdown_target_resource_ref(&request.plan, request.ordinal, &request.target_key);
        if let Some(saved) = self
            .shutdown_recovery_action_record(&request.action_id)
            .await?
        {
            if !shutdown_action_attempt_matches(&saved, &request, &resource_ref) {
                return Err(RecoveryActionError::NotFound);
            }
            let process_action = match &saved.attempt {
                RecoveryAttemptRecord::ShutdownTarget { intent, .. } => {
                    application_process_action_from_domain(*intent)
                }
                RecoveryAttemptRecord::Obligation { .. }
                | RecoveryAttemptRecord::FeedbackRetry { .. } => None,
            };
            return Ok(ShutdownTargetActionExecution {
                outcome: shutdown_action_outcome(decode_shutdown_recovery_action_status(&saved)?),
                process_action,
            });
        }
        let issued = derive_recovery_action_id(
            &*self.authority,
            &self.installation_id,
            &resource_ref,
            request.origin_revision,
            request.action,
        );
        if issued != request.action_id {
            return Err(RecoveryActionError::NotFound);
        }
        if request.action != RecoveryActionKind::RetrySameEffect {
            return Ok(ShutdownTargetActionExecution {
                outcome: RecoveryActionOutcome::Rejected {
                    action_id: request.action_id,
                    rejection: RecoveryActionRejection::ActionUnavailable,
                },
                process_action: None,
            });
        }

        let target_view = match self
            .repository
            .query(LocalEventQuery::ShutdownTargetByIdentity {
                plan: request.plan.clone(),
                ordinal: request.ordinal,
            })
            .await
        {
            Ok(LocalEventQueryResult::ShutdownTargetByIdentity(Some(target))) => target,
            Ok(LocalEventQueryResult::ShutdownTargetByIdentity(None)) => {
                return Err(RecoveryActionError::NotFound)
            }
            Ok(_) => {
                return Err(RecoveryActionError::Internal {
                    correlation_id: correlation("shutdown-action-target-shape"),
                })
            }
            Err(crate::domain::local_event::LocalEventQueryError::DetailsCompacted) => {
                return Ok(ShutdownTargetActionExecution {
                    outcome: RecoveryActionOutcome::Rejected {
                        action_id: request.action_id,
                        rejection: RecoveryActionRejection::ActionUnavailable,
                    },
                    process_action: None,
                })
            }
            Err(error) => return Err(map_shutdown_action_query_error(error)),
        };
        if target_view.revision.value() as u64 != request.origin_revision {
            return Ok(ShutdownTargetActionExecution {
                outcome: RecoveryActionOutcome::Rejected {
                    action_id: request.action_id,
                    rejection: RecoveryActionRejection::RevisionConflict {
                        current_revision: target_view.revision.value() as u64,
                    },
                },
                process_action: None,
            });
        }
        let ShutdownTargetRecord::Target {
            target_id,
            kind: target_kind,
            state: stored_target_state,
            effect_identity,
            owner_operation_id,
            failure: target_failure,
            recovery_action: _,
        } = target_view.detail
        else {
            return Err(RecoveryActionError::Internal {
                correlation_id: correlation("shutdown-action-target-kind"),
            });
        };
        let kind = shutdown_target_kind_label(target_kind).to_string();
        let target_key = self.shutdown_target_key(&kind, &target_id);
        if target_key != request.target_key {
            return Err(RecoveryActionError::NotFound);
        }
        let plan_page = self
            .shutdown_plan_page(request.plan.clone(), 1, None)
            .await
            .map_err(map_shutdown_action_query_error)?;
        let plan_summary = plan_page.plan.summary;
        let exit_coupled = self.has_exit_coupled_observation(plan_page.plan.phase, &plan_summary);
        let action_available = stored_target_state
            == ShutdownTargetStateRecord::ReconciliationRequired
            || (exit_coupled
                && matches!(
                    stored_target_state,
                    ShutdownTargetStateRecord::Prepared | ShutdownTargetStateRecord::EffectReserved
                ));
        if !action_available {
            return Ok(ShutdownTargetActionExecution {
                outcome: RecoveryActionOutcome::Rejected {
                    action_id: request.action_id,
                    rejection: RecoveryActionRejection::ActionUnavailable,
                },
                process_action: None,
            });
        }
        let operation_id = (!plan_summary.operation_id.is_empty())
            .then(|| plan_summary.operation_id.clone())
            .ok_or_else(|| RecoveryActionError::Internal {
                correlation_id: correlation("shutdown-action-operation-id"),
            })?;
        let (receipt, _, operation_binding, _) = self
            .get_operation(&operation_id)
            .await
            .map_err(map_shutdown_action_query_error)?
            .ok_or_else(|| RecoveryActionError::Internal {
                correlation_id: correlation("shutdown-action-operation"),
            })?;
        let reserved_revision = target_view
            .revision
            .next()
            .ok_or(RecoveryActionError::InvalidRequest)?;
        let recovery_action = ShutdownTargetRecoveryRecord {
            action_id: request.action_id.clone(),
            origin_revision: request.origin_revision,
            action: request.action,
            state: ObligationStateRecord::EffectReserved,
        };
        let reserved_detail = ShutdownTargetRecord::Target {
            target_id: target_id.clone(),
            kind: target_kind,
            state: stored_target_state,
            effect_identity: effect_identity.clone(),
            owner_operation_id: owner_operation_id.clone(),
            failure: target_failure,
            recovery_action: Some(recovery_action),
        };
        let effect_identity_sha256 = self.authority.digest(effect_identity.as_bytes());
        let attempt = RecoveryAttemptRecord::ShutdownTarget {
            resource_ref: resource_ref.clone(),
            plan: request.plan.clone(),
            ordinal: request.ordinal,
            target_key: request.target_key.clone(),
            origin_revision: request.origin_revision,
            action: request.action,
            effect_identity_sha256,
            intent: receipt.intent.domain(),
            state: ObligationStateRecord::EffectReserved,
            failure: None,
        };
        let binding_hash = self.shutdown_action_binding_hash(
            &resource_ref,
            &request,
            effect_identity_sha256,
            receipt.intent,
        );
        let reserve = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex::encode(self.authority.digest(
                format!("shutdown-target-action-reserve/v1\0{}", request.action_id).as_bytes(),
            )))
            .map_err(|_| RecoveryActionError::InvalidRequest)?,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::Recovery,
                idempotency_key: format!("{}.reserve", request.action_id),
                payload_hash: binding_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![
                LocalStateMutation::RecoveryAction(RecoveryActionMutation {
                    action_id: request.action_id.clone(),
                    binding_hash,
                    attempt: attempt.clone(),
                    completed: None,
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).expect("zero revision"),
                }),
                LocalStateMutation::ShutdownTarget(ShutdownTargetMutation {
                    key: request.plan.clone(),
                    ordinal: request.ordinal,
                    detail: reserved_detail,
                    expected: RevisionGuard::Expected(target_view.revision),
                    revision: reserved_revision,
                }),
            ],
        };
        match self.repository.commit_batch(reserve).await {
            Ok(CommitBatchResult::Committed(_)) => {}
            Ok(CommitBatchResult::Replayed(_)) => {
                let status = self
                    .get_shutdown_target_action_status(&request.action_id)
                    .await?;
                return Ok(ShutdownTargetActionExecution {
                    outcome: shutdown_action_outcome(status),
                    process_action: Some(receipt.intent.into()),
                });
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                return Ok(ShutdownTargetActionExecution {
                    outcome: RecoveryActionOutcome::ActionOutcomeUnknown {
                        action_id: request.action_id,
                    },
                    process_action: None,
                })
            }
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                let current_revision = match self
                    .repository
                    .query(LocalEventQuery::ShutdownTargetByIdentity {
                        plan: request.plan,
                        ordinal: request.ordinal,
                    })
                    .await
                {
                    Ok(LocalEventQueryResult::ShutdownTargetByIdentity(Some(target))) => {
                        target.revision.value() as u64
                    }
                    _ => request.origin_revision,
                };
                return Ok(ShutdownTargetActionExecution {
                    outcome: RecoveryActionOutcome::Rejected {
                        action_id: request.action_id,
                        rejection: RecoveryActionRejection::RevisionConflict { current_revision },
                    },
                    process_action: None,
                });
            }
            Err(CommitBatchError::PayloadConflict) => {
                if let Some(saved) = self
                    .shutdown_recovery_action_record(&request.action_id)
                    .await?
                {
                    if shutdown_action_attempt_matches(&saved, &request, &resource_ref) {
                        return Ok(ShutdownTargetActionExecution {
                            outcome: shutdown_action_outcome(
                                decode_shutdown_recovery_action_status(&saved)?,
                            ),
                            process_action: Some(receipt.intent.into()),
                        });
                    }
                }
                return Err(RecoveryActionError::NotFound);
            }
            Err(CommitBatchError::StorageUnavailable { failure }) => {
                return Err(RecoveryActionError::StorageUnavailable { failure })
            }
            Err(CommitBatchError::CapacityExceeded | CommitBatchError::SequenceExhausted) => {
                return Err(RecoveryActionError::InvalidRequest)
            }
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                return Err(RecoveryActionError::Internal { correlation_id })
            }
        }

        #[cfg(test)]
        {
            let hook = self
                .recovery_pre_handoff_hook
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if let Some(hook) = hook {
                hook().await;
            }
        }

        // Only the process that committed the reservation may reach this
        // handoff. Re-read the exact claimed revision immediately before I/O.
        let claimed = self
            .repository
            .query(LocalEventQuery::ShutdownTargetByIdentity {
                plan: request.plan.clone(),
                ordinal: request.ordinal,
            })
            .await
            .map_err(map_shutdown_action_query_error)?;
        let LocalEventQueryResult::ShutdownTargetByIdentity(Some(claimed)) = claimed else {
            return Ok(ShutdownTargetActionExecution {
                outcome: RecoveryActionOutcome::Rejected {
                    action_id: request.action_id,
                    rejection: RecoveryActionRejection::TargetRevisionChanged,
                },
                process_action: None,
            });
        };
        let claim_matches = claimed.revision == reserved_revision
            && matches!(
                &claimed.detail,
                ShutdownTargetRecord::Target {
                    recovery_action: Some(ShutdownTargetRecoveryRecord { action_id, .. }),
                    ..
                } if action_id == &request.action_id
            );
        if !claim_matches {
            return Ok(ShutdownTargetActionExecution {
                outcome: RecoveryActionOutcome::Rejected {
                    action_id: request.action_id,
                    rejection: RecoveryActionRejection::TargetRevisionChanged,
                },
                process_action: None,
            });
        }

        let shutdown_target = ShutdownTarget { target_id, kind };
        let readback = tokio::time::timeout(
            Duration::from_secs(10),
            self.executor.read_target_effect(
                &receipt.operation_id,
                &effect_identity,
                reserved_revision,
                &shutdown_target,
            ),
        )
        .await;
        let (classification, final_state, failure) = match readback {
            Ok(Ok(ShutdownEffectReadback::Completed)) => (
                RecoveryResultClassification::Succeeded,
                ShutdownTargetStateRecord::Completed,
                None,
            ),
            Ok(Ok(ShutdownEffectReadback::ConfirmedNotStarted)) => {
                match tokio::time::timeout(
                    Duration::from_secs(10),
                    self.executor.execute_target(
                        &receipt.operation_id,
                        &effect_identity,
                        reserved_revision,
                        &shutdown_target,
                    ),
                )
                .await
                {
                    Ok(Ok(())) => (
                        RecoveryResultClassification::Succeeded,
                        ShutdownTargetStateRecord::Completed,
                        None,
                    ),
                    Ok(Err(failure)) => (
                        RecoveryResultClassification::Ambiguous,
                        ShutdownTargetStateRecord::ReconciliationRequired,
                        Some(failure),
                    ),
                    Err(_) => (
                        RecoveryResultClassification::Ambiguous,
                        ShutdownTargetStateRecord::ReconciliationRequired,
                        Some(SafeOperationFailure::new(
                            SessionOperationFailureKind::DeadlineExceeded,
                            true,
                            "The shutdown target retry reached its fixed deadline.",
                            correlation("shutdown-action-deadline"),
                        )),
                    ),
                }
            }
            Ok(Ok(ShutdownEffectReadback::Ambiguous)) | Ok(Err(_)) | Err(_) => (
                RecoveryResultClassification::Ambiguous,
                ShutdownTargetStateRecord::ReconciliationRequired,
                Some(SafeOperationFailure::new(
                    SessionOperationFailureKind::OutcomeUnknown,
                    true,
                    "The shutdown target effect remains ambiguous.",
                    correlation("shutdown-action-ambiguous"),
                )),
            ),
        };
        let final_revision = reserved_revision
            .next()
            .ok_or(RecoveryActionError::InvalidRequest)?;
        let outcome = shutdown_recovery_outcome(classification);
        let completed_payload = self
            .authority
            .canonicalize_recovery_result(
                match outcome {
                    RecoveryActionResultOutcome::Pending => RecoveryResultOutcomeRecord::Pending,
                    RecoveryActionResultOutcome::Terminal => RecoveryResultOutcomeRecord::Terminal,
                    RecoveryActionResultOutcome::Unchanged => {
                        RecoveryResultOutcomeRecord::Unchanged
                    }
                },
                classification,
                final_revision.value() as u64,
                RecoveryResourceViewRecord::ShutdownTarget {
                    plan: request.plan.clone(),
                    ordinal: request.ordinal,
                    target_id: request.target_key.clone(),
                    state: final_state,
                },
            )
            .map_err(|_| RecoveryActionError::Internal {
                correlation_id: correlation("shutdown-action-result-canonicalization"),
            })?;
        let completed_result =
            decode_recovery_completed_result(&completed_payload).ok_or_else(|| {
                RecoveryActionError::Internal {
                    correlation_id: correlation("shutdown-action-result-decode"),
                }
            })?;
        if completed_result.outcome != outcome
            || completed_result.classification != classification
            || completed_result.resource_revision != final_revision.value() as u64
        {
            return Err(RecoveryActionError::Internal {
                correlation_id: correlation("shutdown-action-result-invariant"),
            });
        }
        let final_detail = match claimed.detail {
            ShutdownTargetRecord::Target {
                target_id,
                kind,
                effect_identity,
                owner_operation_id,
                recovery_action: Some(mut recovery_action),
                ..
            } => {
                recovery_action.state = ObligationStateRecord::Completed;
                ShutdownTargetRecord::Target {
                    target_id,
                    kind,
                    state: final_state,
                    effect_identity,
                    owner_operation_id,
                    failure: failure.clone(),
                    recovery_action: Some(recovery_action),
                }
            }
            ShutdownTargetRecord::Target {
                recovery_action: None,
                ..
            }
            | ShutdownTargetRecord::RecoverySnapshot { .. } => {
                return Err(RecoveryActionError::Internal {
                    correlation_id: correlation("shutdown-action-claimed-kind"),
                })
            }
        };
        let completed_attempt = match attempt {
            RecoveryAttemptRecord::ShutdownTarget {
                resource_ref,
                plan,
                ordinal,
                target_key,
                origin_revision,
                action,
                effect_identity_sha256,
                intent,
                ..
            } => RecoveryAttemptRecord::ShutdownTarget {
                resource_ref,
                plan,
                ordinal,
                target_key,
                origin_revision,
                action,
                effect_identity_sha256,
                intent,
                state: ObligationStateRecord::Completed,
                failure: None,
            },
            RecoveryAttemptRecord::Obligation { .. }
            | RecoveryAttemptRecord::FeedbackRetry { .. } => {
                return Err(RecoveryActionError::Internal {
                    correlation_id: correlation("shutdown-action-attempt-kind"),
                })
            }
        };
        let completed_payload_sha256 = match &completed_payload {
            crate::domain::local_event::RecoveryResultRecord::Action(result) => {
                result.canonical_result_sha256
            }
            crate::domain::local_event::RecoveryResultRecord::FeedbackRetry { .. } => {
                return Err(RecoveryActionError::Internal {
                    correlation_id: correlation("shutdown-action-result-kind"),
                })
            }
        };
        let plan_closure = if classification == RecoveryResultClassification::Succeeded {
            self.prepare_shutdown_action_plan_closure(
                &receipt,
                operation_binding,
                &request.plan,
                request.ordinal,
            )
            .await?
        } else {
            None
        };
        let closes_plan = plan_closure.is_some();
        let mut expected_heads = Vec::new();
        let mut events = Vec::new();
        let mut state_mutations = vec![
            LocalStateMutation::RecoveryAction(RecoveryActionMutation {
                action_id: request.action_id.clone(),
                binding_hash,
                attempt: completed_attempt,
                completed: Some(completed_payload.clone()),
                expected: RevisionGuard::Expected(Revision::new(0).expect("zero revision")),
                revision: Revision::new(1).expect("revision one"),
            }),
            LocalStateMutation::ShutdownTarget(ShutdownTargetMutation {
                key: request.plan.clone(),
                ordinal: request.ordinal,
                detail: final_detail,
                expected: RevisionGuard::Expected(reserved_revision),
                revision: final_revision,
            }),
        ];
        if let Some(plan_closure) = plan_closure {
            expected_heads.push(plan_closure.expected_head);
            events.push(plan_closure.event);
            state_mutations.extend(plan_closure.mutations);
        }
        let finish = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex::encode(self.authority.digest(
                format!("shutdown-target-action-finish/v1\0{}", request.action_id).as_bytes(),
            )))
            .map_err(|_| RecoveryActionError::InvalidRequest)?,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::Recovery,
                idempotency_key: format!("{}.finish", request.action_id),
                payload_hash: completed_payload_sha256,
            },
            expected_heads,
            events,
            state_mutations,
        };
        match self.repository.commit_batch(finish).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {}
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                return Ok(ShutdownTargetActionExecution {
                    outcome: RecoveryActionOutcome::ActionOutcomeUnknown {
                        action_id: request.action_id,
                    },
                    process_action: None,
                })
            }
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                return Ok(ShutdownTargetActionExecution {
                    outcome: RecoveryActionOutcome::InProgress {
                        action_id: request.action_id,
                    },
                    process_action: None,
                })
            }
            Err(CommitBatchError::PayloadConflict) => {
                let status = self
                    .get_shutdown_target_action_status(&request.action_id)
                    .await?;
                return Ok(ShutdownTargetActionExecution {
                    outcome: shutdown_action_outcome(status),
                    process_action: Some(receipt.intent.into()),
                });
            }
            Err(CommitBatchError::StorageUnavailable { failure }) => {
                return Err(RecoveryActionError::StorageUnavailable { failure })
            }
            Err(CommitBatchError::CapacityExceeded | CommitBatchError::SequenceExhausted) => {
                return Err(RecoveryActionError::InvalidRequest)
            }
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                return Err(RecoveryActionError::Internal { correlation_id })
            }
        }
        let mut process_action = Some(receipt.intent.into());
        if closes_plan {
            *self
                .completed_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                Some(CompletedApplicationQuitFlight {
                    receipt: receipt.clone(),
                    state: ApplicationQuitState::Completed,
                    operation_binding,
                    join_before_ticket: self.ingress_sequence.load(Ordering::SeqCst),
                });
            self.schedule_oldest_shutdown_detail_compaction();
        } else if classification == RecoveryResultClassification::Succeeded {
            let deadlines = ShutdownDeadlines::from_receipt(&receipt);
            let continued = self
                .continue_activated(receipt, operation_binding, deadlines)
                .await;
            process_action = match &continued {
                ApplicationQuitOutcome::Accepted { receipt, state }
                    if state.grants_exit_permit() =>
                {
                    Some(receipt.intent.into())
                }
                _ => None,
            };
        }
        Ok(ShutdownTargetActionExecution {
            outcome: RecoveryActionOutcome::Completed {
                action_id: request.action_id,
                result: completed_result,
            },
            process_action,
        })
    }

    pub async fn compact_shutdown_details(
        &self,
        plan: ShutdownPlanKey,
    ) -> Result<ShutdownPlanView, ApplicationQuitError> {
        let current = self
            .shutdown_plan_page(plan.clone(), 1, None)
            .await
            .map_err(|_| ApplicationQuitError::Internal {
                correlation_id: correlation("compact-read"),
            })?
            .plan;
        if !matches!(
            current.phase,
            ApplicationShutdownPhase::Completed
                | ApplicationShutdownPhase::Failed
                | ApplicationShutdownPhase::Cancelled
        ) {
            return Err(self.previous_shutdown_reconciliation_error(current));
        }
        if current.details_state == ShutdownDetailsState::Compacted {
            return Ok(current);
        }
        let next_revision = current
            .revision
            .next()
            .ok_or(ApplicationQuitError::CapacityExceeded)?;
        let stream_id = StreamId::application();
        let head = self
            .application_head()
            .await
            .map_err(|_| ApplicationQuitError::Internal {
                correlation_id: correlation("compact-head"),
            })?;
        let payload_hash = self.authority.digest(
            format!(
                "shutdown-summary-only/v2\0{}\0{}",
                plan.shutdown_id,
                current.revision.value()
            )
            .as_bytes(),
        );
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex::encode(
                self.authority.digest(
                    format!(
                        "shutdown-compact/v2\0{}\0{}",
                        plan.shutdown_id,
                        next_revision.value()
                    )
                    .as_bytes(),
                ),
            ))
            .map_err(|_| ApplicationQuitError::Internal {
                correlation_id: correlation("compact-identity"),
            })?,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: OperationKind::ApplicationQuit.into(),
                idempotency_key: format!("{}.compact.{}", plan.shutdown_id, next_revision.value()),
                payload_hash,
            },
            expected_heads: vec![ExpectedStreamHead {
                stream_id: stream_id.clone(),
                expected: StreamVersion::new(head).expect("nonnegative head"),
            }],
            events: vec![UncommittedDomainEvent {
                stream_id,
                event: LocalDomainEvent::Application(
                    ApplicationDomainEvent::ShutdownDetailsCompacted {
                        shutdown_id: plan.shutdown_id.clone(),
                        at_ms: now_ms(),
                    },
                ),
                occurred_at_ms: now_ms(),
            }],
            state_mutations: vec![LocalStateMutation::ShutdownDetailsCompaction(
                ShutdownDetailsCompactionMutation {
                    key: plan.clone(),
                    expected: current.revision,
                    revision: next_revision,
                },
            )],
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {}
            Err(CommitBatchError::PayloadConflict) => {
                if let Ok(page) = self.shutdown_plan_page(plan.clone(), 1, None).await {
                    if page.plan.details_state == ShutdownDetailsState::Compacted {
                        return Ok(page.plan);
                    }
                }
                return Err(ApplicationQuitError::Internal {
                    correlation_id: correlation("compact-conflict"),
                });
            }
            Err(CommitBatchError::CapacityExceeded | CommitBatchError::SequenceExhausted) => {
                return Err(ApplicationQuitError::CapacityExceeded)
            }
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                if let Ok(page) = self.shutdown_plan_page(plan.clone(), 1, None).await {
                    if page.plan.details_state == ShutdownDetailsState::Compacted {
                        return Ok(page.plan);
                    }
                }
                return Err(ApplicationQuitError::Internal {
                    correlation_id: correlation("compact-stream-conflict"),
                });
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                if let Ok(page) = self.shutdown_plan_page(plan.clone(), 1, None).await {
                    if page.plan.details_state == ShutdownDetailsState::Compacted {
                        return Ok(page.plan);
                    }
                }
                return Err(ApplicationQuitError::Internal {
                    correlation_id: correlation("compact-outcome"),
                });
            }
            Err(CommitBatchError::StorageUnavailable { failure }) => {
                return Err(ApplicationQuitError::Internal {
                    correlation_id: failure.correlation_id,
                })
            }
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                return Err(ApplicationQuitError::Internal { correlation_id })
            }
        }
        let result = self.shutdown_plan_page(plan, 1, None).await.map_err(|_| {
            ApplicationQuitError::Internal {
                correlation_id: correlation("compact-readback"),
            }
        })?;
        Ok(result.plan)
    }

    /// Returns the closed public read model after compaction. The use case
    /// maps the repository's closed domain record so transports cannot infer
    /// capabilities or collapse a storage state while presenting the result.
    pub async fn compact_shutdown_details_read_model(
        &self,
        plan: ShutdownPlanKey,
    ) -> Result<ApplicationShutdownPlanReadModel, ApplicationQuitError> {
        let compacted = self.compact_shutdown_details(plan).await?;
        self.decode_shutdown_plan_read_model(compacted)
            .map_err(|error| {
                use crate::domain::local_event::LocalEventQueryError as E;
                let correlation_id = match error {
                    E::StorageUnavailable { failure } => failure.correlation_id,
                    E::Corrupt { correlation_id }
                    | E::IncompatibleStoredEvent { correlation_id }
                    | E::Internal { correlation_id }
                    | E::ReplayRequired { correlation_id } => correlation_id,
                    other => correlation(&format!("compact-read-model-{other:?}")),
                };
                ApplicationQuitError::Internal { correlation_id }
            })
    }

    async fn application_head(&self) -> Result<i64, ()> {
        let stream = StreamId::application();
        self.repository
            .load_stream(LoadStreamRequest {
                stream_id: stream,
                after: None,
                limit: 1,
            })
            .await
            .map(|page| page.head.value())
            .map_err(|_| ())
    }

    async fn fixed_recovery_snapshot(
        &self,
    ) -> Result<Vec<crate::domain::local_event::PendingObligationView>, ApplicationQuitError> {
        let mut entries = Vec::new();
        for partition in [
            PendingPartition::ClosedSession,
            PendingPartition::ArchivedSession,
            PendingPartition::UnownedRuntime,
        ] {
            let mut cursor = None;
            loop {
                let result = self
                    .repository
                    .query(LocalEventQuery::PendingRecoveryPage {
                        limit: 200,
                        partition: Some(partition),
                        owner: None,
                        ordered_key_prefix: None,
                        shutdown_plan: None,
                        cursor,
                    })
                    .await
                    .map_err(|_| ApplicationQuitError::Internal {
                        correlation_id: correlation("recovery-snapshot"),
                    })?;
                let LocalEventQueryResult::PendingRecoveryPage(page) = result else {
                    return Err(ApplicationQuitError::Internal {
                        correlation_id: correlation("recovery-snapshot-shape"),
                    });
                };
                entries.extend(page.entries);
                if entries.len() > MAX_TARGETS {
                    return Err(ApplicationQuitError::CapacityExceeded);
                }
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
        }
        Ok(entries)
    }

    fn recovery_snapshot_id(
        &self,
        entries: &[crate::domain::local_event::PendingObligationView],
    ) -> Result<String, ApplicationQuitError> {
        fn push_lp(material: &mut Vec<u8>, value: &str) {
            material.extend_from_slice(&(value.len() as u64).to_be_bytes());
            material.extend_from_slice(value.as_bytes());
        }
        let mut material = Vec::new();
        push_lp(&mut material, "shutdown-recovery-snapshot/v2");
        material.extend_from_slice(&(entries.len() as u64).to_be_bytes());
        for entry in entries {
            push_lp(&mut material, &entry.obligation_id);
            push_lp(&mut material, &entry.ordered_key);
            push_lp(&mut material, &entry.owner);
            push_lp(&mut material, entry.partition.label());
            material.extend_from_slice(&entry.revision.value().to_be_bytes());
            material.extend_from_slice(&entry.record_sha256);
        }
        Ok(hex::encode(self.authority.digest(&material)))
    }

    pub async fn request(
        &self,
        request: ApplicationQuitRequest,
    ) -> Result<ApplicationQuitOutcome, ApplicationQuitError> {
        let ingress_instant = tokio::time::Instant::now();
        let ingress_deadlines = ShutdownDeadlines::from_ingress(ingress_instant);
        let ingress_t0_ms = now_ms();
        let principal = request.principal.clone();
        let request_id = request.request_id.clone();
        let operation_id = self.operation_id(&request.principal, &request.request_id);
        let intent = request.intent;
        let exact_command = encode_quit_attempt(intent);
        let outcome = match tokio::time::timeout_at(
            ingress_deadlines.decision_deadline,
            self.request_with_deadlines(request, ingress_t0_ms, ingress_deadlines),
        )
        .await
        {
            Ok(result) => result,
            // A request-wide guard is the final backstop for authority and
            // projection reads. Individual pre-acceptance inventory waits
            // return RejectedBeforeCommit at T0+13; this branch preserves
            // ambiguity when a durable binding or commit may already exist.
            Err(_) => Ok(ApplicationQuitOutcome::OutcomeUnknown {
                request_id: request_id.clone(),
                operation_id,
                intent,
            }),
        };
        if !matches!(&outcome, Ok(ApplicationQuitOutcome::OutcomeUnknown { .. })) {
            let accepted = matches!(&outcome, Ok(ApplicationQuitOutcome::Accepted { .. }));
            let resolution = tokio::time::timeout(
                Duration::from_secs(1),
                self.caller_attempt_journal().clear_attempt(
                    &principal,
                    OperationKind::ApplicationQuit,
                    &request_id,
                    &exact_command,
                    accepted,
                ),
            )
            .await;
            if matches!(resolution, Ok(Err(_)) | Err(_)) {
                log::warn!(
                    "application quit caller-attempt resolution requires reconciliation: {request_id}"
                );
            }
        }
        outcome
    }

    async fn request_with_deadlines(
        &self,
        request: ApplicationQuitRequest,
        ingress_t0_ms: i64,
        ingress_deadlines: ShutdownDeadlines,
    ) -> Result<ApplicationQuitOutcome, ApplicationQuitError> {
        validate_operation_identity(&request.request_id)
            .map_err(|_| ApplicationQuitError::InvalidRequest)?;
        let ingress_ticket = self
            .ingress_sequence
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |next| {
                next.checked_add(1)
            })
            .map_err(|_| ApplicationQuitError::CapacityExceeded)?;
        // Admission, activation and the bounded terminal decision are one
        // backend flight. Joiners wait here, then bind to its immutable
        // current or just-completed result without executing fixed targets.
        let _flight = match tokio::time::timeout_at(
            ingress_deadlines.decision_deadline,
            self.request_flight.lock(),
        )
        .await
        {
            Ok(flight) => flight,
            Err(_) => {
                return Ok(ApplicationQuitOutcome::OutcomeUnknown {
                    operation_id: self.operation_id(&request.principal, &request.request_id),
                    request_id: request.request_id,
                    intent: request.intent,
                });
            }
        };
        let operation_id = self.operation_id(&request.principal, &request.request_id);
        let caller_key = self.caller_key(&request.principal, &request.request_id);
        if let Some(saved_binding) = self.get_binding(&caller_key).await? {
            let binding_material =
                crate::usecase::agent_session::operation::binding::application_quit(
                    &request.principal,
                    &self.installation_id,
                    &request.request_id,
                    &saved_binding.operation_id,
                    request.intent.mode(),
                    request.intent.code(),
                );
            let binding = self.authority.mac(&binding_material);
            if !constant_time_eq_32(&saved_binding.binding_hmac, &binding) {
                return Err(ApplicationQuitError::PayloadConflict);
            }
            let saved_operation = self
                .get_operation(&saved_binding.operation_id)
                .await
                .map_err(|_| ApplicationQuitError::Internal {
                    correlation_id: correlation("bound-operation-lookup"),
                })?;
            if let Some((receipt, state, operation_binding, _)) = saved_operation {
                if matches!(state, ApplicationQuitState::Preparing) {
                    let deadlines = ShutdownDeadlines::from_receipt(&receipt);
                    return Ok(self
                        .continue_prepared(receipt, operation_binding, deadlines)
                        .await);
                }
                if matches!(state, ApplicationQuitState::Activated) {
                    let deadlines = ShutdownDeadlines::from_receipt(&receipt);
                    return Ok(self
                        .continue_activated(receipt, operation_binding, deadlines)
                        .await);
                }
                return Ok(ApplicationQuitOutcome::Accepted { receipt, state });
            }
            if saved_binding.operation_id != operation_id {
                return Err(ApplicationQuitError::Internal {
                    correlation_id: correlation("bound-operation-missing"),
                });
            }
        }

        let completed_flight = self
            .completed_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(CompletedApplicationQuitFlight {
            receipt,
            state,
            operation_binding: _operation_binding,
            join_before_ticket: _join_before_ticket,
        }) = completed_flight.filter(|completed| ingress_ticket < completed.join_before_ticket)
        {
            let join_material = crate::usecase::agent_session::operation::binding::application_quit(
                &request.principal,
                &self.installation_id,
                &request.request_id,
                &receipt.operation_id,
                request.intent.mode(),
                request.intent.code(),
            );
            let join_binding = self.authority.mac(&join_material);
            if !self
                .save_join_binding(
                    caller_key,
                    &receipt.operation_id,
                    &join_material,
                    join_binding,
                )
                .await?
            {
                return Ok(ApplicationQuitOutcome::RejectedBeforeCommit {
                    failure: SafeOperationFailure::new(
                        SessionOperationFailureKind::PersistFailure,
                        true,
                        "The completed quit join was not committed. Retry with the same request identity.",
                        correlation("completed-join-not-committed"),
                    ),
                });
            }
            return Ok(ApplicationQuitOutcome::Accepted { receipt, state });
        }

        if self
            .get_operation(&operation_id)
            .await
            .map_err(|_| ApplicationQuitError::Internal {
                correlation_id: correlation("unbound-operation-lookup"),
            })?
            .is_some()
        {
            return Err(ApplicationQuitError::Internal {
                correlation_id: correlation("unbound-operation"),
            });
        }

        let current = self
            .repository
            .query(LocalEventQuery::CurrentShutdown)
            .await
            .map_err(|_| ApplicationQuitError::Internal {
                correlation_id: correlation("current"),
            })?;
        let LocalEventQueryResult::CurrentShutdown(current) = current else {
            return Err(ApplicationQuitError::Internal {
                correlation_id: correlation("current-shape"),
            });
        };
        let mut replaces_failed_plan = None;
        if let Some(current) = current {
            let current_operation = (!current.summary.operation_id.is_empty())
                .then(|| current.summary.operation_id.clone())
                .ok_or_else(|| ApplicationQuitError::Internal {
                    correlation_id: correlation("current-operation"),
                })?;
            let Some((receipt, state, operation_binding, _)) = self
                .get_operation(&current_operation)
                .await
                .map_err(|_| ApplicationQuitError::Internal {
                    correlation_id: correlation("current-lookup"),
                })?
            else {
                return Err(ApplicationQuitError::Internal {
                    correlation_id: correlation("current-operation-reference-missing"),
                });
            };
            if matches!(state, ApplicationQuitState::FailedBeforeActivation { .. })
                && self.retry_quit_available(&current).await.map_err(|error| {
                    ApplicationQuitError::Internal {
                        correlation_id: correlation(&format!("retry-quit-eligibility-{error}")),
                    }
                })?
            {
                replaces_failed_plan = Some(current.plan.clone());
            } else {
                if matches!(state, ApplicationQuitState::ReconciliationRequired { .. }) {
                    return Err(self.previous_shutdown_reconciliation_error(current));
                }
                let join_material =
                    crate::usecase::agent_session::operation::binding::application_quit(
                        &request.principal,
                        &self.installation_id,
                        &request.request_id,
                        &current_operation,
                        request.intent.mode(),
                        request.intent.code(),
                    );
                let join_binding = self.authority.mac(&join_material);
                if !self
                    .save_join_binding(
                        caller_key.clone(),
                        &current_operation,
                        &join_material,
                        join_binding,
                    )
                    .await?
                {
                    return Ok(ApplicationQuitOutcome::RejectedBeforeCommit {
                        failure: SafeOperationFailure::new(
                            SessionOperationFailureKind::PersistFailure,
                            true,
                            "The quit join was not committed. Retry with the same request identity.",
                            correlation("join-not-committed"),
                        ),
                    });
                }
                if matches!(state, ApplicationQuitState::Preparing) {
                    let deadlines = ShutdownDeadlines::from_receipt(&receipt);
                    return Ok(self
                        .continue_prepared(receipt, operation_binding, deadlines)
                        .await);
                }
                if matches!(state, ApplicationQuitState::Activated) {
                    let deadlines = ShutdownDeadlines::from_receipt(&receipt);
                    return Ok(self
                        .continue_activated(receipt, operation_binding, deadlines)
                        .await);
                }
                return Ok(ApplicationQuitOutcome::Accepted { receipt, state });
            }
        }

        match tokio::time::timeout_at(
            ingress_deadlines.preparation_cutoff,
            self.ensure_shutdown_detail_capacity(),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                return Ok(ApplicationQuitOutcome::RejectedBeforeCommit {
                    failure: shutdown_deadline_failure("admission-deadline"),
                });
            }
        }

        let binding_material = crate::usecase::agent_session::operation::binding::application_quit(
            &request.principal,
            &self.installation_id,
            &request.request_id,
            &operation_id,
            request.intent.mode(),
            request.intent.code(),
        );
        let binding = self.authority.mac(&binding_material);

        let targets = match tokio::time::timeout_at(
            ingress_deadlines.preparation_cutoff,
            self.executor.targets(),
        )
        .await
        {
            Ok(Ok(targets)) => targets,
            Ok(Err(_)) => {
                return Err(ApplicationQuitError::Internal {
                    correlation_id: correlation("targets"),
                });
            }
            Err(_) => {
                return Ok(ApplicationQuitOutcome::RejectedBeforeCommit {
                    failure: shutdown_deadline_failure("targets-deadline"),
                });
            }
        };
        if targets.len() > MAX_TARGETS {
            return Err(ApplicationQuitError::CapacityExceeded);
        }
        let recovery_snapshot = match tokio::time::timeout_at(
            ingress_deadlines.preparation_cutoff,
            self.fixed_recovery_snapshot(),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                return Ok(ApplicationQuitOutcome::RejectedBeforeCommit {
                    failure: shutdown_deadline_failure("recovery-snapshot-deadline"),
                });
            }
        };
        let recovery_snapshot_id = if recovery_snapshot.is_empty() {
            None
        } else {
            Some(self.recovery_snapshot_id(&recovery_snapshot)?)
        };
        if 5usize
            .saturating_add(targets.len())
            .saturating_add(recovery_snapshot.len())
            > MAX_ACCEPTANCE_MUTATIONS
        {
            return Err(ApplicationQuitError::CapacityExceeded);
        }
        let stream_id = StreamId::application();
        let head = match tokio::time::timeout_at(
            ingress_deadlines.preparation_cutoff,
            self.application_head(),
        )
        .await
        {
            Ok(Ok(head)) => head,
            Ok(Err(())) => {
                return Err(ApplicationQuitError::Internal {
                    correlation_id: correlation("head"),
                });
            }
            Err(_) => {
                return Ok(ApplicationQuitOutcome::RejectedBeforeCommit {
                    failure: shutdown_deadline_failure("head-deadline"),
                });
            }
        };
        let exact_command = encode_quit_attempt(request.intent);
        let journaled = tokio::time::timeout_at(
            ingress_deadlines.preparation_cutoff,
            self.caller_attempt_journal().record_bound_attempt_scoped(
                &request.principal,
                OperationKind::ApplicationQuit,
                &request.request_id,
                &exact_command,
                Some("application"),
                crate::usecase::agent_session::operation::BoundCallerOperation {
                    operation_id: &operation_id,
                    binding_hmac: binding,
                },
            ),
        )
        .await;
        match journaled {
            Ok(Ok(_)) => {}
            Err(_)
            | Ok(Err(
                crate::usecase::agent_session::operation::CallerJournalError::OutcomeUnknown,
            )) => {
                return Ok(ApplicationQuitOutcome::OutcomeUnknown {
                    request_id: request.request_id,
                    operation_id,
                    intent: request.intent,
                });
            }
            Ok(Err(
                crate::usecase::agent_session::operation::CallerJournalError::PayloadConflict,
            )) => return Err(ApplicationQuitError::PayloadConflict),
            Ok(Err(
                crate::usecase::agent_session::operation::CallerJournalError::InvalidRequest,
            )) => return Err(ApplicationQuitError::InvalidRequest),
            Ok(Err(
                crate::usecase::agent_session::operation::CallerJournalError::RejectedBeforeCommit
                | crate::usecase::agent_session::operation::CallerJournalError::ShutdownInProgress,
            )) => {
                return Ok(ApplicationQuitOutcome::RejectedBeforeCommit {
                    failure: SafeOperationFailure::new(
                        SessionOperationFailureKind::PersistFailure,
                        true,
                        "The application quit intent was not durably recorded. Retry with the same request identity.",
                        correlation("quit-attempt-not-committed"),
                    ),
                });
            }
        }
        #[cfg(test)]
        {
            let hook = self
                .pre_acceptance_hook
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if let Some(hook) = hook {
                hook().await;
            }
        }
        let t0_ms = ingress_t0_ms;
        let receipt = ApplicationQuitReceipt {
            operation_id: operation_id.clone(),
            shutdown_id: operation_id.clone(),
            intent: request.intent,
            t0_ms,
            deadline_ms: t0_ms.saturating_add(DECISION_DEADLINE.as_millis() as i64),
        };
        let plan = ShutdownPlanKey {
            shutdown_id: receipt.shutdown_id.clone(),
        };
        let target_count =
            u64::try_from(targets.len()).map_err(|_| ApplicationQuitError::CapacityExceeded)?;
        let recovery_snapshot_count = u64::try_from(recovery_snapshot.len())
            .map_err(|_| ApplicationQuitError::CapacityExceeded)?;
        let summary = ShutdownPlanRecord {
            operation_id: operation_id.clone(),
            intent: request.intent.domain(),
            t0_ms,
            preparation_cutoff_ms: Some(t0_ms.saturating_add(13_000)),
            deadline_ms: receipt.deadline_ms,
            target_count: Some(target_count),
            prepared_count: Some(target_count),
            effect_reserved_count: Some(0),
            terminal_count: Some(0),
            completed_count: Some(0),
            unresolved_count: Some(target_count),
            recovery_snapshot_count: Some(recovery_snapshot_count),
            recovery_snapshot_id,
            process_instance_id: self.process_instance_id.clone(),
            outcome: None,
            failure: None,
            shutdown_effect_count: None,
            admission_open: None,
            retry_quit_same_boot: None,
        };
        let mut mutations = vec![
            LocalStateMutation::OperationBinding(OperationBindingMutation {
                key: caller_key,
                operation_id: operation_id.clone(),
                binding_hmac: binding,
            }),
            LocalStateMutation::OperationRecord(OperationRecordMutation {
                kind: OperationKind::ApplicationQuit,
                operation_id: operation_id.clone(),
                receipt: operation_receipt_record(&receipt, binding),
                latest_status: operation_status_record(&ApplicationQuitState::Preparing),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).expect("zero revision"),
            }),
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: plan.clone(),
                phase: ApplicationShutdownPhase::Prepared,
                summary: summary.clone(),
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).expect("zero revision"),
            }),
            LocalStateMutation::ShutdownLatestPointer(ShutdownLatestPointerMutation {
                expected: replaces_failed_plan,
                new: Some(plan.clone()),
            }),
        ];
        for (ordinal, target) in targets.iter().enumerate() {
            let kind = shutdown_target_kind_from_label(&target.kind).ok_or_else(|| {
                ApplicationQuitError::Internal {
                    correlation_id: correlation("shutdown-target-kind"),
                }
            })?;
            mutations.push(LocalStateMutation::ShutdownTarget(ShutdownTargetMutation {
                key: plan.clone(),
                ordinal: ordinal as i64,
                detail: ShutdownTargetRecord::Target {
                    target_id: target.target_id.clone(),
                    kind,
                    state: ShutdownTargetStateRecord::Prepared,
                    effect_identity: format!("shutdown-target/{operation_id}/{ordinal}"),
                    owner_operation_id: None,
                    failure: None,
                    recovery_action: None,
                },
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).expect("zero revision"),
            }));
        }
        for (ordinal, entry) in recovery_snapshot.iter().enumerate() {
            mutations.push(LocalStateMutation::ShutdownRecoverySnapshot(
                ShutdownRecoverySnapshotMutation {
                    key: plan.clone(),
                    partition: entry.partition,
                    ordinal: ordinal as i64,
                    detail: ShutdownTargetRecord::RecoverySnapshot {
                        obligation_id: entry.obligation_id.clone(),
                        ordered_key: entry.ordered_key.clone(),
                        owner: entry.owner.clone(),
                        revision: entry.revision.value() as u64,
                        record: Box::new(entry.record.clone()),
                    },
                },
            ));
        }
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex::encode(
                self.authority
                    .digest(format!("application-quit-accept/v1\0{operation_id}").as_bytes()),
            ))
            .map_err(|_| ApplicationQuitError::Internal {
                correlation_id: correlation("commit-id"),
            })?,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: OperationKind::ApplicationQuit.into(),
                idempotency_key: operation_id.clone(),
                payload_hash: self.authority.digest(&binding_material),
            },
            expected_heads: vec![ExpectedStreamHead {
                stream_id: stream_id.clone(),
                expected: StreamVersion::new(head).expect("nonnegative head"),
            }],
            events: vec![
                UncommittedDomainEvent {
                    stream_id: stream_id.clone(),
                    event: LocalDomainEvent::Application(
                        ApplicationDomainEvent::ApplicationQuitAccepted {
                            quit_operation_id: operation_id.clone(),
                            intent: request.intent.domain(),
                            at_ms: t0_ms,
                        },
                    ),
                    occurred_at_ms: t0_ms,
                },
                UncommittedDomainEvent {
                    stream_id,
                    event: LocalDomainEvent::Application(
                        ApplicationDomainEvent::ShutdownPhaseAdvanced {
                            shutdown_id: plan.shutdown_id.clone(),
                            phase: ApplicationShutdownPhase::Prepared,
                            at_ms: t0_ms,
                        },
                    ),
                    occurred_at_ms: t0_ms,
                },
            ],
            state_mutations: mutations,
        };
        let acceptance = tokio::time::timeout_at(
            ingress_deadlines.preparation_cutoff,
            self.repository.commit_batch(batch),
        )
        .await;
        match acceptance {
            Err(_) => {
                return Ok(ApplicationQuitOutcome::OutcomeUnknown {
                    request_id: request.request_id,
                    operation_id: operation_id.clone(),
                    intent: request.intent,
                });
            }
            Ok(result) => match result {
                Ok(CommitBatchResult::Committed(_)) => {}
                Ok(CommitBatchResult::Replayed(_)) => {
                    let readback = self.get_operation(&operation_id).await.map_err(|_| {
                        ApplicationQuitError::Internal {
                            correlation_id: correlation("accept-replay-readback"),
                        }
                    })?;
                    let (receipt, state, _, _) =
                        readback.ok_or_else(|| ApplicationQuitError::Internal {
                            correlation_id: correlation("accept-replay-missing"),
                        })?;
                    return Ok(ApplicationQuitOutcome::Accepted { receipt, state });
                }
                Err(CommitBatchError::PayloadConflict) => {
                    return Err(ApplicationQuitError::PayloadConflict)
                }
                Err(CommitBatchError::CapacityExceeded | CommitBatchError::SequenceExhausted) => {
                    return Err(ApplicationQuitError::CapacityExceeded)
                }
                Err(CommitBatchError::OutcomeUnknown { .. }) => {
                    return Ok(ApplicationQuitOutcome::OutcomeUnknown {
                        request_id: request.request_id,
                        operation_id,
                        intent: request.intent,
                    })
                }
                Err(CommitBatchError::StorageUnavailable { failure }) => {
                    return Ok(ApplicationQuitOutcome::RejectedBeforeCommit { failure })
                }
                Err(CommitBatchError::StreamHeadConflict { .. }) => {
                    return Err(self.current_shutdown_reconciliation_error().await);
                }
                Err(CommitBatchError::Corrupt { correlation_id }) => {
                    return Err(ApplicationQuitError::Internal { correlation_id })
                }
            },
        }

        Ok(self
            .continue_prepared(receipt, binding, ingress_deadlines)
            .await)
    }

    async fn continue_prepared(
        &self,
        receipt: ApplicationQuitReceipt,
        binding: [u8; 32],
        deadlines: ShutdownDeadlines,
    ) -> ApplicationQuitOutcome {
        let plan = ShutdownPlanKey {
            shutdown_id: receipt.shutdown_id.clone(),
        };
        let prepared =
            tokio::time::timeout_at(deadlines.preparation_cutoff, self.prepared_targets(&plan))
                .await;
        let (summary, summary_sha256, targets) = match prepared {
            Ok(Ok(value)) => value,
            Ok(Err(failure)) => {
                return self
                    .abort_before_activation(receipt, binding, failure, deadlines)
                    .await;
            }
            Err(_) => {
                return self
                    .abort_before_activation(
                        receipt,
                        binding,
                        shutdown_deadline_failure("target-read-deadline"),
                        deadlines,
                    )
                    .await;
            }
        };
        let revalidated = tokio::time::timeout_at(
            deadlines.preparation_cutoff,
            self.revalidate_pre_activation_inventory(&plan, &summary, &targets),
        )
        .await;
        match revalidated {
            Ok(Ok(())) => {}
            Ok(Err(failure)) => {
                return self
                    .abort_before_activation(receipt, binding, failure, deadlines)
                    .await;
            }
            Err(_) => {
                return self
                    .abort_before_activation(
                        receipt,
                        binding,
                        shutdown_deadline_failure("inventory-revalidation-deadline"),
                        deadlines,
                    )
                    .await;
            }
        }
        #[cfg(test)]
        {
            let hook = self
                .pre_activation_hook
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if let Some(hook) = hook {
                hook().await;
            }
        }
        let activation = tokio::time::timeout_at(
            deadlines.preparation_cutoff,
            self.activate(&receipt, binding, &summary, summary_sha256),
        )
        .await;
        match activation.unwrap_or(ActivationCommit::OutcomeUnknown) {
            ActivationCommit::Activated => {}
            ActivationCommit::RejectedBeforeCommit { failure } => {
                return self
                    .abort_before_activation(receipt, binding, failure, deadlines)
                    .await;
            }
            ActivationCommit::OutcomeUnknown => {
                return ApplicationQuitOutcome::Accepted {
                    state: ApplicationQuitState::OutcomeUnknown {
                        operation_id: receipt.operation_id.clone(),
                        shutdown_id: receipt.shutdown_id.clone(),
                        activation_commit_id: self.activation_commit_id(&receipt.operation_id),
                    },
                    receipt,
                };
            }
        }
        self.continue_activated_with_targets(receipt, binding, targets, deadlines)
            .await
    }

    async fn abort_before_activation(
        &self,
        receipt: ApplicationQuitReceipt,
        binding: [u8; 32],
        failure: SafeOperationFailure,
        deadlines: ShutdownDeadlines,
    ) -> ApplicationQuitOutcome {
        let failed = ApplicationQuitState::FailedBeforeActivation { failure };
        let persisted = tokio::time::timeout_at(
            deadlines.decision_deadline,
            self.finish(&receipt, binding, &failed),
        )
        .await
        .is_ok_and(|persisted| persisted);
        ApplicationQuitOutcome::Accepted {
            receipt,
            state: if persisted {
                failed
            } else {
                ApplicationQuitState::Preparing
            },
        }
    }

    async fn continue_activated(
        &self,
        receipt: ApplicationQuitReceipt,
        binding: [u8; 32],
        deadlines: ShutdownDeadlines,
    ) -> ApplicationQuitOutcome {
        let plan = ShutdownPlanKey {
            shutdown_id: receipt.shutdown_id.clone(),
        };
        let targets = match tokio::time::timeout_at(
            deadlines.decision_deadline,
            self.prepared_targets(&plan),
        )
        .await
        {
            Ok(Ok((_, _, targets))) => targets,
            Ok(Err(failure)) => {
                return ApplicationQuitOutcome::Accepted {
                    receipt,
                    state: ApplicationQuitState::ReconciliationRequired { failure },
                };
            }
            Err(_) => {
                return ApplicationQuitOutcome::Accepted {
                    receipt,
                    state: ApplicationQuitState::ReconciliationRequired {
                        failure: shutdown_deadline_failure("activated-target-read-deadline"),
                    },
                };
            }
        };
        self.continue_activated_with_targets(receipt, binding, targets, deadlines)
            .await
    }

    async fn continue_activated_with_targets(
        &self,
        receipt: ApplicationQuitReceipt,
        binding: [u8; 32],
        targets: Vec<StoredShutdownTarget>,
        deadlines: ShutdownDeadlines,
    ) -> ApplicationQuitOutcome {
        let plan = ShutdownPlanKey {
            shutdown_id: receipt.shutdown_id.clone(),
        };
        let mut unresolved = None;
        for stored in targets {
            if stored.state == ShutdownTargetStateRecord::Completed {
                continue;
            }
            if stored.state != ShutdownTargetStateRecord::Prepared {
                unresolved.get_or_insert_with(|| {
                    SafeOperationFailure::new(
                        SessionOperationFailureKind::OutcomeUnknown,
                        true,
                        "A prior shutdown target effect requires reconciliation.",
                        correlation("target-prior-effect"),
                    )
                });
                continue;
            }
            let Some(reserved_revision) = stored.revision.next() else {
                unresolved.get_or_insert_with(|| {
                    SafeOperationFailure::new(
                        SessionOperationFailureKind::CapacityExceeded,
                        false,
                        "A shutdown target revision was exhausted.",
                        correlation("target-revision"),
                    )
                });
                continue;
            };
            let reservation = tokio::time::timeout_at(
                deadlines.preparation_cutoff,
                self.persist_target_state(TargetStateTransition {
                    receipt: &receipt,
                    plan: &plan,
                    stored: &stored,
                    state: ShutdownTargetStateRecord::EffectReserved,
                    expected: stored.revision,
                    revision: reserved_revision,
                    failure: None,
                }),
            )
            .await;
            if !reservation.is_ok_and(|persisted| persisted) {
                unresolved.get_or_insert_with(|| {
                    SafeOperationFailure::new(
                        SessionOperationFailureKind::OutcomeUnknown,
                        true,
                        "A shutdown target reservation result is unknown.",
                        correlation("target-reserve"),
                    )
                });
                continue;
            }
            let result = match tokio::time::timeout_at(
                deadlines.preparation_cutoff,
                self.executor.execute_target(
                    &receipt.operation_id,
                    &stored.effect_identity,
                    reserved_revision,
                    &stored.target,
                ),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(shutdown_deadline_failure("target-deadline")),
            };
            let terminal_revision = reserved_revision
                .next()
                .expect("bounded shutdown target revision");
            let (target_state, failure) = match result {
                Ok(()) => (ShutdownTargetStateRecord::Completed, None),
                Err(failure) => {
                    unresolved.get_or_insert_with(|| failure.clone());
                    (
                        ShutdownTargetStateRecord::ReconciliationRequired,
                        Some(failure),
                    )
                }
            };
            let terminal = tokio::time::timeout_at(
                deadlines.decision_deadline,
                self.persist_target_state(TargetStateTransition {
                    receipt: &receipt,
                    plan: &plan,
                    stored: &stored,
                    state: target_state,
                    expected: reserved_revision,
                    revision: terminal_revision,
                    failure: failure.as_ref(),
                }),
            )
            .await;
            if !terminal.is_ok_and(|persisted| persisted) {
                unresolved.get_or_insert_with(|| {
                    SafeOperationFailure::new(
                        SessionOperationFailureKind::OutcomeUnknown,
                        true,
                        "A shutdown target result requires readback.",
                        correlation("target-finish"),
                    )
                });
            }
        }
        if unresolved.is_none() {
            match tokio::time::timeout_at(
                deadlines.preparation_cutoff,
                self.executor.shutdown_subordinates(),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(failure)) => unresolved = Some(failure),
                Err(_) => {
                    unresolved = Some(shutdown_deadline_failure("subordinate-deadline"));
                }
            }
        }
        let desired_state = match unresolved {
            None => ApplicationQuitState::Completed,
            Some(failure) => ApplicationQuitState::ReconciliationRequired { failure },
        };
        let terminal_persisted = tokio::time::timeout_at(
            deadlines.decision_deadline,
            self.finish(&receipt, binding, &desired_state),
        )
        .await
        .is_ok_and(|persisted| persisted);
        let state = if terminal_persisted {
            desired_state
        } else {
            ApplicationQuitState::Activated
        };
        ApplicationQuitOutcome::Accepted { receipt, state }
    }

    async fn persist_target_state(&self, transition: TargetStateTransition<'_>) -> bool {
        let TargetStateTransition {
            receipt,
            plan,
            stored,
            state,
            expected,
            revision,
            failure,
        } = transition;
        let Some(kind) = shutdown_target_kind_from_label(&stored.target.kind) else {
            return false;
        };
        let detail = ShutdownTargetRecord::Target {
            target_id: stored.target.target_id.clone(),
            kind,
            state,
            effect_identity: stored.effect_identity.clone(),
            owner_operation_id: Some(receipt.operation_id.clone()),
            failure: failure.cloned(),
            recovery_action: stored.recovery_action.clone(),
        };
        let mut identity = Vec::new();
        identity.extend_from_slice(b"shutdown-target-state/v2\0");
        identity.extend_from_slice(stored.target.target_id.as_bytes());
        identity.push(0);
        identity.extend_from_slice(stored.target.kind.as_bytes());
        identity.push(0);
        identity.extend_from_slice(shutdown_target_state_label(state).as_bytes());
        identity.push(0);
        identity.extend_from_slice(stored.effect_identity.as_bytes());
        identity.push(0);
        identity.extend_from_slice(receipt.operation_id.as_bytes());
        if let Some(failure) = failure {
            identity.push(1);
            identity.extend_from_slice(failure.correlation_id.as_bytes());
        } else {
            identity.push(0);
        }
        let binding = self.authority.digest(&identity);
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex::encode(
                self.authority.digest(
                    format!(
                        "shutdown-target/v1\0{}\0{}\0{}",
                        receipt.operation_id,
                        stored.ordinal,
                        revision.value()
                    )
                    .as_bytes(),
                ),
            ))
            .expect("digest commit identity"),
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: OperationKind::ApplicationQuit.into(),
                idempotency_key: format!(
                    "{}.target.{}.{}",
                    receipt.operation_id,
                    stored.ordinal,
                    revision.value()
                ),
                payload_hash: binding,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![LocalStateMutation::ShutdownTarget(ShutdownTargetMutation {
                key: plan.clone(),
                ordinal: stored.ordinal,
                detail,
                expected: RevisionGuard::Expected(expected),
                revision,
            })],
        };
        matches!(
            self.repository.commit_batch(batch).await,
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_))
        )
    }

    async fn prepared_targets(
        &self,
        plan: &ShutdownPlanKey,
    ) -> Result<(ShutdownPlanRecord, [u8; 32], Vec<StoredShutdownTarget>), SafeOperationFailure>
    {
        let mut cursor = None;
        let mut targets = Vec::new();
        let mut summary = None;
        loop {
            let result = self
                .repository
                .query(LocalEventQuery::ShutdownPlanPage {
                    plan: plan.clone(),
                    limit: 128,
                    cursor,
                })
                .await
                .map_err(|_| {
                    SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageUnavailable,
                        true,
                        "The fixed shutdown target set could not be read.",
                        correlation("target-read"),
                    )
                })?;
            let LocalEventQueryResult::ShutdownPlanPage(page) = result else {
                return Err(SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageCorrupt,
                    false,
                    "The shutdown target query returned an incompatible result.",
                    correlation("target-shape"),
                ));
            };
            summary.get_or_insert((page.plan.summary, page.plan.summary_sha256));
            for target in page.targets {
                let ShutdownTargetRecord::Target {
                    target_id,
                    kind,
                    state,
                    effect_identity,
                    owner_operation_id: _,
                    failure: _,
                    recovery_action,
                } = target.detail
                else {
                    return Err(SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageCorrupt,
                        false,
                        "A shutdown target record has the wrong closed kind.",
                        correlation("target-kind"),
                    ));
                };
                targets.push(StoredShutdownTarget {
                    ordinal: target.ordinal,
                    target: ShutdownTarget {
                        target_id,
                        kind: shutdown_target_kind_label(kind).to_string(),
                    },
                    state,
                    revision: target.revision,
                    effect_identity,
                    recovery_action,
                });
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
            if targets.len() > MAX_TARGETS {
                return Err(SafeOperationFailure::new(
                    SessionOperationFailureKind::CapacityExceeded,
                    false,
                    "The shutdown target set exceeds its fixed bound.",
                    correlation("target-capacity"),
                ));
            }
        }
        let (summary, summary_sha256) = summary.ok_or_else(|| {
            SafeOperationFailure::new(
                SessionOperationFailureKind::StorageCorrupt,
                false,
                "The shutdown plan summary is missing.",
                correlation("target-summary"),
            )
        })?;
        Ok((summary, summary_sha256, targets))
    }

    /// Re-read every source whose contents were fixed by the acceptance
    /// transaction. The current-shutdown pointer is already durable here, so
    /// fresh user mutations are closed while these reads run. A target that
    /// became terminal after the first inventory may disappear; only a
    /// currently active target absent from the fixed set is an unsafe delta.
    async fn revalidate_pre_activation_inventory(
        &self,
        plan: &ShutdownPlanKey,
        summary: &ShutdownPlanRecord,
        fixed_targets: &[StoredShutdownTarget],
    ) -> Result<(), SafeOperationFailure> {
        let fixed_target_keys = fixed_targets
            .iter()
            .map(|stored| (stored.target.kind.clone(), stored.target.target_id.clone()))
            .collect::<HashSet<_>>();
        if fixed_target_keys.len() != fixed_targets.len() {
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::StorageCorrupt,
                false,
                "The fixed shutdown target set contains a duplicate identity.",
                correlation("target-revalidation-duplicate"),
            ));
        }

        let current_targets = self.executor.targets().await?;
        if current_targets.len() > MAX_TARGETS {
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::CapacityExceeded,
                false,
                "The current shutdown target inventory exceeds its fixed bound.",
                correlation("target-revalidation-capacity"),
            ));
        }
        if current_targets.iter().any(|target| {
            !fixed_target_keys.contains(&(target.kind.clone(), target.target_id.clone()))
        }) {
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::TargetRevisionChanged,
                true,
                "The shutdown target inventory changed before activation.",
                correlation("target-revalidation-mismatch"),
            ));
        }

        let fixed_recovery = self
            .fixed_recovery_snapshot_identities(plan, summary)
            .await?;
        let current_recovery =
            self.fixed_recovery_snapshot()
                .await
                .map_err(|error| match error {
                    ApplicationQuitError::CapacityExceeded => SafeOperationFailure::new(
                        SessionOperationFailureKind::CapacityExceeded,
                        false,
                        "The current recovery inventory exceeds its fixed bound.",
                        correlation("recovery-revalidation-capacity"),
                    ),
                    _ => SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageUnavailable,
                        true,
                        "The current recovery inventory could not be revalidated.",
                        correlation("recovery-revalidation-read"),
                    ),
                })?;
        if current_recovery.iter().any(|entry| {
            !fixed_recovery.contains(&entry.obligation_id)
                && !Self::pending_obligation_owned_by_fixed_target(entry, fixed_targets)
        }) {
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::OwnerRevisionChanged,
                true,
                "The pending recovery inventory changed before activation.",
                correlation("recovery-revalidation-mismatch"),
            ));
        }
        Ok(())
    }

    async fn fixed_recovery_snapshot_identities(
        &self,
        plan: &ShutdownPlanKey,
        summary: &ShutdownPlanRecord,
    ) -> Result<HashSet<String>, SafeOperationFailure> {
        let expected_count = summary.recovery_snapshot_count.ok_or_else(|| {
            SafeOperationFailure::new(
                SessionOperationFailureKind::StorageCorrupt,
                false,
                "The fixed recovery count is missing.",
                correlation("recovery-revalidation-count"),
            )
        })?;
        let snapshot_id = summary.recovery_snapshot_id.as_deref();
        if expected_count == 0 {
            return if snapshot_id.is_none() {
                Ok(HashSet::new())
            } else {
                Err(SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageCorrupt,
                    false,
                    "An empty recovery snapshot has an unexpected identity.",
                    correlation("recovery-revalidation-empty-id"),
                ))
            };
        }
        let snapshot_id = snapshot_id.ok_or_else(|| {
            SafeOperationFailure::new(
                SessionOperationFailureKind::StorageCorrupt,
                false,
                "The fixed recovery snapshot identity is missing.",
                correlation("recovery-revalidation-snapshot-id"),
            )
        })?;
        let mut identities = HashSet::new();
        for partition in [
            PendingPartition::ClosedSession,
            PendingPartition::ArchivedSession,
            PendingPartition::UnownedRuntime,
        ] {
            let mut cursor = None;
            loop {
                let result = self
                    .repository
                    .query(LocalEventQuery::PendingRecoverySnapshotPage {
                        plan: plan.clone(),
                        snapshot_id: snapshot_id.to_string(),
                        partition,
                        limit: 200,
                        cursor,
                    })
                    .await
                    .map_err(|_| {
                        SafeOperationFailure::new(
                            SessionOperationFailureKind::StorageUnavailable,
                            true,
                            "The fixed recovery snapshot could not be read.",
                            correlation("recovery-revalidation-fixed-read"),
                        )
                    })?;
                let LocalEventQueryResult::PendingRecoverySnapshotPage(page) = result else {
                    return Err(SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageCorrupt,
                        false,
                        "The fixed recovery snapshot query returned an incompatible result.",
                        correlation("recovery-revalidation-fixed-shape"),
                    ));
                };
                for entry in page.entries {
                    let ShutdownTargetRecord::RecoverySnapshot { obligation_id, .. } = entry.detail
                    else {
                        return Err(SafeOperationFailure::new(
                            SessionOperationFailureKind::StorageCorrupt,
                            false,
                            "A fixed recovery snapshot entry has the wrong closed kind.",
                            correlation("recovery-revalidation-fixed-entry"),
                        ));
                    };
                    if !identities.insert(obligation_id) {
                        return Err(SafeOperationFailure::new(
                            SessionOperationFailureKind::StorageCorrupt,
                            false,
                            "The fixed recovery snapshot contains a duplicate obligation.",
                            correlation("recovery-revalidation-fixed-duplicate"),
                        ));
                    }
                    if identities.len() > MAX_TARGETS {
                        return Err(SafeOperationFailure::new(
                            SessionOperationFailureKind::CapacityExceeded,
                            false,
                            "The fixed recovery snapshot exceeds its bound.",
                            correlation("recovery-revalidation-fixed-capacity"),
                        ));
                    }
                }
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
        }
        if identities.len() as u64 != expected_count {
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::StorageCorrupt,
                false,
                "The fixed recovery snapshot count does not match its summary.",
                correlation("recovery-revalidation-fixed-count-mismatch"),
            ));
        }
        Ok(identities)
    }

    fn pending_obligation_owned_by_fixed_target(
        entry: &crate::domain::local_event::PendingObligationView,
        fixed_targets: &[StoredShutdownTarget],
    ) -> bool {
        fixed_targets
            .iter()
            .any(|stored| match stored.target.kind.as_str() {
                "agent_session" => entry.owner == stored.target.target_id,
                "workflow_execution" => {
                    entry.owner == "workflow-runtime"
                        && entry.obligation_id
                            == format!("workflow-execution-{}", stored.target.target_id)
                        && entry.ordered_key
                            == format!("workflow_execution:{}", stored.target.target_id)
                }
                _ => false,
            })
    }

    async fn activate(
        &self,
        receipt: &ApplicationQuitReceipt,
        binding: [u8; 32],
        summary: &ShutdownPlanRecord,
        summary_sha256: [u8; 32],
    ) -> ActivationCommit {
        let stream_id = StreamId::application();
        let Ok(head) = self.application_head().await else {
            return ActivationCommit::RejectedBeforeCommit {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "Shutdown activation could not read the durable stream head.",
                    correlation("activation-head"),
                ),
            };
        };
        let at_ms = now_ms();
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&self.activation_commit_id(&receipt.operation_id))
                .expect("digest commit identity"),
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: OperationKind::ApplicationQuit.into(),
                idempotency_key: format!("{}.activate", receipt.operation_id),
                payload_hash: summary_sha256,
            },
            expected_heads: vec![ExpectedStreamHead {
                stream_id: stream_id.clone(),
                expected: StreamVersion::new(head).expect("nonnegative head"),
            }],
            events: vec![UncommittedDomainEvent {
                stream_id,
                event: LocalDomainEvent::Application(
                    ApplicationDomainEvent::ShutdownPhaseAdvanced {
                        shutdown_id: receipt.shutdown_id.clone(),
                        phase: ApplicationShutdownPhase::Activated,
                        at_ms,
                    },
                ),
                occurred_at_ms: at_ms,
            }],
            state_mutations: vec![
                LocalStateMutation::OperationRecord(OperationRecordMutation {
                    kind: OperationKind::ApplicationQuit,
                    operation_id: receipt.operation_id.clone(),
                    receipt: operation_receipt_record(receipt, binding),
                    latest_status: operation_status_record(&ApplicationQuitState::Activated),
                    expected: RevisionGuard::Expected(Revision::new(0).expect("zero revision")),
                    revision: Revision::new(1).expect("revision one"),
                }),
                LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                    key: ShutdownPlanKey {
                        shutdown_id: receipt.shutdown_id.clone(),
                    },
                    phase: ApplicationShutdownPhase::Activated,
                    summary: summary.clone(),
                    details_state: ShutdownDetailsState::Available,
                    expected: RevisionGuard::Expected(Revision::new(0).expect("zero revision")),
                    revision: Revision::new(1).expect("revision one"),
                }),
            ],
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {
                ActivationCommit::Activated
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => ActivationCommit::OutcomeUnknown,
            Err(CommitBatchError::StorageUnavailable { failure }) => {
                ActivationCommit::RejectedBeforeCommit { failure }
            }
            Err(CommitBatchError::CapacityExceeded | CommitBatchError::SequenceExhausted) => {
                ActivationCommit::RejectedBeforeCommit {
                    failure: SafeOperationFailure::new(
                        SessionOperationFailureKind::CapacityExceeded,
                        false,
                        "Shutdown activation exceeded a durable store bound.",
                        correlation("activation-capacity"),
                    ),
                }
            }
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                ActivationCommit::RejectedBeforeCommit {
                    failure: SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageCorrupt,
                        false,
                        "Shutdown activation encountered an integrity failure.",
                        correlation_id,
                    ),
                }
            }
            Err(
                CommitBatchError::PayloadConflict | CommitBatchError::StreamHeadConflict { .. },
            ) => ActivationCommit::RejectedBeforeCommit {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::PersistFailure,
                    true,
                    "Shutdown activation was rejected before any target effect started.",
                    correlation("activation-conflict"),
                ),
            },
        }
    }

    async fn finish(
        &self,
        receipt: &ApplicationQuitReceipt,
        binding: [u8; 32],
        state: &ApplicationQuitState,
    ) -> bool {
        let phase = match state {
            ApplicationQuitState::Completed => ApplicationShutdownPhase::Completed,
            ApplicationQuitState::FailedBeforeActivation { .. } => ApplicationShutdownPhase::Failed,
            _ => ApplicationShutdownPhase::ReconciliationRequired,
        };
        let plan = ShutdownPlanKey {
            shutdown_id: receipt.shutdown_id.clone(),
        };
        let Ok(Some((_, _, _, operation_revision))) =
            self.get_operation(&receipt.operation_id).await
        else {
            return false;
        };
        let Ok(Some(current_plan)) = self.current_shutdown().await else {
            return matches!(state, ApplicationQuitState::Completed);
        };
        if current_plan.plan != plan {
            return false;
        }
        let mut summary = current_plan.summary.clone();
        let Ok((_, _, targets)) = self.prepared_targets(&plan).await else {
            return false;
        };
        let (prepared_count, effect_reserved_count, terminal_count, completed_count) =
            if matches!(state, ApplicationQuitState::FailedBeforeActivation { .. }) {
                (targets.len(), 0, 0, 0)
            } else {
                let prepared = targets
                    .iter()
                    .filter(|target| target.state == ShutdownTargetStateRecord::Prepared)
                    .count();
                let reserved = targets
                    .iter()
                    .filter(|target| target.state == ShutdownTargetStateRecord::EffectReserved)
                    .count();
                let completed = targets
                    .iter()
                    .filter(|target| target.state == ShutdownTargetStateRecord::Completed)
                    .count();
                (prepared, reserved, completed, completed)
            };
        summary.prepared_count = u64::try_from(prepared_count).ok();
        summary.effect_reserved_count = u64::try_from(effect_reserved_count).ok();
        summary.terminal_count = u64::try_from(terminal_count).ok();
        summary.completed_count = u64::try_from(completed_count).ok();
        summary.unresolved_count =
            u64::try_from(targets.len().saturating_sub(completed_count)).ok();
        if summary.prepared_count.is_none()
            || summary.effect_reserved_count.is_none()
            || summary.terminal_count.is_none()
            || summary.completed_count.is_none()
            || summary.unresolved_count.is_none()
        {
            return false;
        }
        match state {
            ApplicationQuitState::Completed => {
                summary.outcome = Some(ShutdownOutcomeRecord::Completed);
                summary.failure = None;
            }
            ApplicationQuitState::FailedBeforeActivation { failure } => {
                summary.outcome = Some(ShutdownOutcomeRecord::AbortedBeforeActivation);
                summary.shutdown_effect_count = Some(0);
                summary.admission_open = Some(true);
                summary.retry_quit_same_boot = Some(true);
                summary.failure = Some(failure.clone());
            }
            ApplicationQuitState::ReconciliationRequired { failure } => {
                summary.outcome = Some(ShutdownOutcomeRecord::ReconciliationRequired);
                summary.failure = Some(failure.clone());
            }
            _ => {}
        }
        let Some(next_operation_revision) = operation_revision.next() else {
            return false;
        };
        let Some(next_plan_revision) = current_plan.revision.next() else {
            return false;
        };
        let stream_id = StreamId::application();
        let Ok(head) = self.application_head().await else {
            return false;
        };
        let at_ms = now_ms();
        let mut state_mutations = vec![
            LocalStateMutation::OperationRecord(OperationRecordMutation {
                kind: OperationKind::ApplicationQuit,
                operation_id: receipt.operation_id.clone(),
                receipt: operation_receipt_record(receipt, binding),
                latest_status: operation_status_record(state),
                expected: RevisionGuard::Expected(operation_revision),
                revision: next_operation_revision,
            }),
            LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
                key: plan.clone(),
                phase,
                summary,
                details_state: ShutdownDetailsState::Available,
                expected: RevisionGuard::Expected(current_plan.revision),
                revision: next_plan_revision,
            }),
        ];
        if matches!(state, ApplicationQuitState::Completed) {
            state_mutations.push(LocalStateMutation::ShutdownLatestPointer(
                ShutdownLatestPointerMutation {
                    expected: Some(plan.clone()),
                    new: None,
                },
            ));
        }
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex::encode(
                self.authority.digest(
                    format!(
                        "application-quit-finish/v1\0{}\0{}",
                        receipt.operation_id,
                        next_operation_revision.value()
                    )
                    .as_bytes(),
                ),
            ))
            .expect("digest commit identity"),
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: OperationKind::ApplicationQuit.into(),
                idempotency_key: format!(
                    "{}.finish.{}",
                    receipt.operation_id,
                    next_operation_revision.value()
                ),
                payload_hash: self
                    .authority
                    .digest(&application_quit_state_identity(state)),
            },
            expected_heads: vec![ExpectedStreamHead {
                stream_id: stream_id.clone(),
                expected: StreamVersion::new(head).expect("nonnegative head"),
            }],
            events: vec![UncommittedDomainEvent {
                stream_id,
                event: LocalDomainEvent::Application(
                    ApplicationDomainEvent::ShutdownPhaseAdvanced {
                        shutdown_id: receipt.shutdown_id.clone(),
                        phase,
                        at_ms,
                    },
                ),
                occurred_at_ms: at_ms,
            }],
            state_mutations,
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {
                if matches!(state, ApplicationQuitState::Completed) {
                    *self
                        .completed_flight
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                        Some(CompletedApplicationQuitFlight {
                            receipt: receipt.clone(),
                            state: state.clone(),
                            operation_binding: binding,
                            join_before_ticket: self.ingress_sequence.load(Ordering::SeqCst),
                        });
                }
                if matches!(state, ApplicationQuitState::Completed) {
                    self.schedule_oldest_shutdown_detail_compaction();
                }
                true
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                match self.get_operation(&receipt.operation_id).await {
                    Ok(Some((_, saved, _, _))) => saved == *state,
                    Ok(None) => false,
                    Err(error) => {
                        log::warn!(
                            "shutdown terminal readback remains ambiguous after commit: {error}"
                        );
                        false
                    }
                }
            }
            Err(error) => {
                log::warn!("shutdown terminal decision remains pending: {error}");
                false
            }
        }
    }
}
