use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::domain::agent_session::aggregates::backend_recovery_attempt::{
    BackendRecoveryAttempt, BackendRecoveryCompletion, BackendRecoveryFailureClaim,
    ProviderIdentityObservation,
};
use crate::domain::agent_session::aggregates::provider_establishment::ProviderEstablishmentObservation;
use crate::domain::agent_session::aggregates::runtime_admission::accepted_effect_delivery_is_admitted;
#[cfg(test)]
use crate::domain::agent_session::aggregates::runtime_admission::provider_session_is_confirmed;
use crate::domain::agent_session::aggregates::runtime_progress::RuntimeStallDecision;
use crate::domain::agent_session::aggregates::runtime_streaming_delivery::StreamEmitFailureDecision;
use crate::domain::agent_session::aggregates::runtime_turn::{
    RuntimeFatalObservation, RuntimeTurnStartCommit,
};
use crate::domain::agent_session::aggregates::session::{TerminalApplication, TransitionOutcome};
use crate::domain::agent_session::entities::{
    AttachmentPayload, InterruptReason as DomainInterruptReason, MessagePart as DomainMessagePart,
    PermissionResponse, TurnResult,
};
#[cfg(test)]
use crate::domain::agent_session::entities::{PermissionRequestStatus, PermissionResponseDecision};
use crate::domain::agent_session::gateway::{
    AgentBackendError, AgentRuntimeEvent, AgentSessionRuntime, SessionSpec, TurnInput,
};
use crate::domain::agent_session::services::{
    accepted_effect_execution_matches, accepted_effect_has_durable_execution_identity,
    accepted_effect_is_process_owned, accepted_prompt_matches,
    accepted_queued_effect_has_durable_identity, accepted_queued_effect_identity_is_consistent,
    accepted_queued_effect_matches, accepted_queued_effect_reservation_conflicts,
    accepted_queued_effect_should_retain, accepted_worktree_matches,
    admit_backend_recovery_sensitive_operation, backend_recovery_may_be_incomplete,
    backend_selection_changes, backend_selection_is_presented_as_changeable,
    context_carry_for_established_resume, decide_accepted_queued_effect_queue,
    decide_context_restore_preparation, decide_permission_response_runtime_completion,
    decide_runtime_event_admission, decide_runtime_turn_recovery, decide_session_established_event,
    next_recovery_retry_delay, permission_request_identity_matches,
    permission_response_turn_matches, project_durable_backend_recovery,
    queue_item_identity_matches, queued_effect_remains_unstarted, require_terminal_commit_identity,
    runtime_error_message_id, runtime_event_recovery_id, runtime_permission_effect_is_owned,
    runtime_provider_session_observation_id, turn_preclaim_failure_disposition,
    validate_accepted_effect_runtime_identity, AcceptedEffectExecutionIdentity,
    AcceptedEffectIdentityRejection, AcceptedQueuedEffectIdentity,
    AcceptedQueuedEffectQueueDecision, BackendRecoveryOperationRejection,
    CanonicalQueuedEffectIdentity, ContextRestorePreparationDecision, DurableBackendRecovery,
    RuntimeEventAdmission, RuntimeEventAdmissionFacts, RuntimeTurnRecoveryDecision,
    SessionEstablishedEventDecision, TurnPreclaimFailureDisposition,
};
#[cfg(test)]
use crate::domain::agent_session::services::{
    backend_selection_change_is_admitted, should_apply_session_configuration,
    BackendSelectionChangeFacts,
};
use crate::domain::agent_session::value_objects::{
    EditorContext, ModelId, PermissionMode, SystemNotificationType as DomainSystemNotificationType,
};
use crate::domain::agent_session::{ContextSnapshot, ContextSourceKind};
use crate::domain::workflow::WorkflowError;
use crate::usecase::agent_session::backend_registry::{AgentBackendRegistry, BackendListResult};
use crate::usecase::agent_session::context::{
    BranchDiffContextPort, BuiltSystemContext, InstructionSourcePort, SystemContextEditorInput,
};
#[cfg(test)]
use crate::usecase::agent_session::event_log::BackendSessionRecoveryProjection;
#[cfg(test)]
use crate::usecase::agent_session::event_log::InterruptReason as EventInterruptReason;
#[cfg(test)]
use crate::usecase::agent_session::event_log::PromptInput;
#[cfg(test)]
use crate::usecase::agent_session::event_log::TurnEventLog;
use crate::usecase::agent_session::event_log::{
    append_part_events, finalize_turn, latest_unresolved_permission_request, AgentSessionEvent,
    BackendSessionRecoveryReason, PartEventMode, UnresolvedPermissionRequest,
};
#[cfg(test)]
use crate::usecase::agent_session::session::{
    add_message_internal, add_message_with_meta_internal, create_session_with_model_and_plan_mode,
    SessionMeta,
};
use crate::usecase::agent_session::session::{
    CanonicalQueuedSend, ChatMessage, ChatSession, ContextCarryState,
    ContextRestoreCompletionRequest, ErrorEpisodeInput, GetSessionResponse, ImageAttachment,
    InitialSessionPage, MessagePart, MessageRole, ModelInfo, OpenTabRegistry,
    PendingRecoveryMessage, PermissionRequestMsg, ProviderSessionEstablishmentOutcome,
    QueuedAgentTurn, SessionState, SessionStore, SessionSummary, INITIAL_SESSION_PAGE_LIMIT,
    RETAINED_MESSAGE_CAP,
};
use crate::usecase::agent_session::status::{
    AgentStatusCenter, AgentStatusNotifier, SessionNotice, SessionNoticeKind, SessionStatus,
    TurnPhase, TurnPhaseRepr,
};
use crate::usecase::agent_session::system_prompt::{
    build_session_system_prompt, persist_session_system_prompt_build,
    SessionSystemPromptBuildRequest,
};
use crate::usecase::workflow::ports::{
    WorkflowStallClearedNotification, WorkflowStallObservedNotification,
    WorkflowTurnCompleteNotification,
};

use super::context_restore::{
    apply_restore_prompt_prefix, context_restore_plan_for_session,
    context_restore_plan_for_session_before_turn, ContextRestorePlan,
};
#[cfg(test)]
use super::ports::AcceptedSendRecoveryWake;
use super::ports::{
    AcceptedQueuedTurnExecutionClaimOutcome, AcceptedSendExecutionClaim,
    AcceptedSendObligationDriver, AgentRuntimeProjectionGateway, AgentSessionEventNotifier,
    AgentSessionStateChangedPayload, AgentStallObservedPayload, AgentStreamingDeltaPayload,
    AgentTaskSpawner, TerminalEventProjection, TerminalProjection, WorkflowStallNotifier,
    WorkflowTurnCompleteNotifier,
};
use super::queue::QueuedTurnInput;
use super::session_state::{
    BackendSessionRecoveryState, PendingStreamDelta, RuntimeSessionMap, RuntimeSessionState,
};
use super::stale::{
    effective_stale_timeout, has_in_flight_tool_use, remaining_until_stale,
    stale_timeout_for_session, stale_watchdog_should_continue_waiting,
    startup_max_retries_for_session, startup_timeout_for_session, turn_is_stale,
};
use super::streaming::{should_persist_streaming_snapshot, StreamingFlushDecision};
#[cfg(test)]
use super::transitions::{
    force_finalize_interrupted_turn, spawn_interrupt_watchdog_task, INTERRUPT_FORCE_FINALIZE_DELAY,
};
use super::transitions::{
    SessionCommandLockGuard, SessionCommandLocks, SessionLockGuard, SessionLockMap,
    SessionTransitionCoordinator,
};

pub type SessionRuntimeLockGuard = SessionCommandLockGuard;

#[cfg(test)]
type SessionRuntimeLocks = Arc<SessionCommandLocks>;

async fn acquire_session_runtime_lock(
    session_locks: &SessionCommandLocks,
    session_id: &str,
) -> SessionRuntimeLockGuard {
    session_locks.acquire(session_id).await
}

#[cfg(test)]
const CLOSE_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(200);
#[cfg(test)]
const CLOSE_DRAIN_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

#[derive(Default)]
pub(super) struct ShutdownAdmission {
    state: std::sync::Mutex<
        crate::domain::agent_session::aggregates::runtime_admission::RuntimeAdmission,
    >,
    idle: tokio::sync::Notify,
}

impl ShutdownAdmission {
    pub(super) fn admit(self: &Arc<Self>) -> Result<ShutdownAdmissionGuard, AgentRuntimeError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.admit().is_err() {
            return Err(AgentRuntimeError::Other(
                "Agent session runtime is shutting down".to_string(),
            ));
        }
        Ok(ShutdownAdmissionGuard {
            admission: Arc::clone(self),
        })
    }

    #[cfg(test)]
    fn begin_shutdown(&self) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin_shutdown();
    }

    #[cfg(test)]
    fn cancel_shutdown(&self) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancel_shutdown();
    }

    #[cfg(test)]
    async fn wait_for_idle(&self) {
        loop {
            let notified = self.idle.notified();
            if self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_idle()
            {
                return;
            }
            notified.await;
        }
    }

    #[cfg(test)]
    fn is_shutting_down(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_shutting_down()
    }
}

pub(super) struct ShutdownAdmissionGuard {
    admission: Arc<ShutdownAdmission>,
}

impl Drop for ShutdownAdmissionGuard {
    fn drop(&mut self) {
        let became_idle = self
            .admission
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .release();
        if became_idle {
            self.admission.idle.notify_waiters();
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEditorContext {
    #[serde(default)]
    pub active_editor_path: Option<String>,
    #[serde(default)]
    pub open_editor_paths: Vec<String>,
    #[serde(default)]
    pub selection: Option<AgentEditorSelection>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEditorSelection {
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRuntimeError {
    StartupTimeout {
        retry_count: u32,
        max_retries: u32,
    },
    BackendSelectionLocked,
    BackendSessionLost {
        requested_resume_id: String,
    },
    AcceptedEffectAdmissionDeferred,
    AcceptedEffectAdmissionFailed {
        stage: &'static str,
        effect_may_be_reserved: bool,
    },
    WorkflowTurnSend(DurableWorkflowSendError),
    WorkspaceQuery(WorkflowError),
    Other(String),
}

impl std::fmt::Display for AgentRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartupTimeout {
                retry_count,
                max_retries,
            } => write!(
                f,
                "Timed out waiting for agent session startup (retry_count={retry_count}, max_retries={max_retries})"
            ),
            Self::BackendSelectionLocked => f.write_str(
                "Backend selection can only change before messages, an agent session, or an active turn exist",
            ),
            Self::BackendSessionLost {
                requested_resume_id,
            } => write!(
                f,
                "Backend session is no longer available: {requested_resume_id}"
            ),
            Self::AcceptedEffectAdmissionDeferred => {
                f.write_str("Accepted effect admission was deferred for durable redrive")
            }
            Self::AcceptedEffectAdmissionFailed {
                stage,
                effect_may_be_reserved,
            } => write!(
                f,
                "Accepted effect admission failed at {stage} (effect_may_be_reserved={effect_may_be_reserved})"
            ),
            Self::WorkflowTurnSend(error) => write!(f, "{error}"),
            Self::WorkspaceQuery(error) => write!(f, "{error}"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for AgentRuntimeError {}

fn fail_accepted_effect_preflight(
    stage: &'static str,
    error: impl std::fmt::Display,
) -> AgentRuntimeError {
    log::warn!("accepted send preflight failed before durable effect claim [{stage}]: {error}");
    AgentRuntimeError::AcceptedEffectAdmissionFailed {
        stage,
        effect_may_be_reserved: false,
    }
}

fn classify_turn_preclaim_error(
    accepted_execution: bool,
    stage: &'static str,
    error: AgentRuntimeError,
) -> AgentRuntimeError {
    match turn_preclaim_failure_disposition(accepted_execution) {
        TurnPreclaimFailureDisposition::PreserveOriginal => error,
        TurnPreclaimFailureDisposition::FailAcceptedEffect => {
            fail_accepted_effect_preflight(stage, error)
        }
    }
}

impl From<AgentBackendError> for AgentRuntimeError {
    fn from(value: AgentBackendError) -> Self {
        match value {
            AgentBackendError::StartupTimeout {
                retry_count,
                max_retries,
            } => Self::StartupTimeout {
                retry_count,
                max_retries,
            },
            AgentBackendError::BackendSessionLost {
                requested_resume_id,
            } => Self::BackendSessionLost {
                requested_resume_id,
            },
            AgentBackendError::Unavailable(message)
            | AgentBackendError::Invalid(message)
            | AgentBackendError::Other(message) => Self::Other(message),
        }
    }
}

#[async_trait::async_trait]
pub(crate) trait DurableStopDriver: Send + Sync {
    async fn stop(
        &self,
        session_id: &str,
        turn_id: u64,
        expected_session_revision: u64,
    ) -> Result<(), String>;
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct DurableWorkflowTurnRequest {
    pub operation_id: String,
    pub session_id: String,
    pub content: String,
    pub permission_mode: PermissionMode,
    pub base_system_prompt: Option<String>,
    pub workflow_instructions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableWorkflowSendError {
    SessionStore(String),
    SessionNotFound(String),
    InvalidWorkflowTarget,
    AuthorityMismatch,
    PayloadEncoding,
    Operation(crate::usecase::agent_session::operation::SendAgentMessageError),
    Admission(crate::domain::local_event::SafeOperationFailure),
    OutcomeUnknown(String),
    IncompatibleReceipt,
    DriverUnavailable,
}

impl std::fmt::Display for DurableWorkflowSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionStore(message) => write!(f, "workflow Send session store: {message}"),
            Self::SessionNotFound(session_id) => {
                write!(f, "The workflow turn session does not exist: {session_id}")
            }
            Self::InvalidWorkflowTarget => {
                f.write_str("The durable workflow Send target is not a workflow session.")
            }
            Self::AuthorityMismatch => f.write_str(
                "The durable workflow Send permission differs from the session authority.",
            ),
            Self::PayloadEncoding => {
                f.write_str("The durable workflow Send payload could not be encoded.")
            }
            Self::Operation(error) => {
                write!(f, "The durable workflow Send operation failed: {error:?}")
            }
            Self::Admission(failure) => write!(f, "{failure}"),
            Self::OutcomeUnknown(operation_id) => write!(
                f,
                "The durable workflow Send acceptance is unknown ({operation_id})."
            ),
            Self::IncompatibleReceipt => {
                f.write_str("The durable workflow Send converged on an incompatible receipt.")
            }
            Self::DriverUnavailable => {
                f.write_str("The durable workflow Send authority is unavailable.")
            }
        }
    }
}

impl std::error::Error for DurableWorkflowSendError {}

#[cfg(test)]
#[async_trait::async_trait]
pub(crate) trait DurableWorkflowSendDriver: Send + Sync {
    async fn send(
        &self,
        request: DurableWorkflowTurnRequest,
    ) -> Result<(), DurableWorkflowSendError>;
}

#[cfg(test)]
pub(crate) trait DurableWorkflowSendPayloadEncoder: Send + Sync {
    fn encode(
        &self,
        request: &DurableWorkflowTurnRequest,
        plan_mode: bool,
    ) -> Result<String, DurableWorkflowSendError>;
}

#[cfg(test)]
pub(crate) fn durable_workflow_turn_operation_id(
    node_execution_id: &str,
    turn_role: &str,
) -> String {
    crate::domain::agent_session::services::durable_workflow_turn_operation_id(
        node_execution_id,
        turn_role,
    )
}

#[derive(Debug, Clone)]
#[cfg(test)]
pub struct StartSessionOptions {
    pub permission_mode: PermissionMode,
    pub plan_mode: bool,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub struct SendAgentMessageRequest {
    pub chat_session_id: Option<String>,
    pub worktree_path: String,
    pub content: String,
    pub permission_mode: PermissionMode,
    pub plan_mode: bool,
    pub backend_id: Option<String>,
    pub model_id: Option<String>,
    pub images: Option<Vec<ImageAttachment>>,
    pub mentions: Option<Vec<crate::domain::code::MentionReference>>,
    pub editor_context: Option<AgentEditorContext>,
}

/// Provider-facing input consumed only after the durable send operation has
/// committed its immutable receipt, projections, and execution obligation.
///
/// Target resolution and provider configuration belong to the acceptance
/// manifest; keeping them out of this type prevents the runtime effect driver
/// from becoming a second send-admission authority.
#[derive(Debug, Clone)]
pub(crate) struct AcceptedRuntimeSendInput {
    pub content: String,
    pub permission_mode: PermissionMode,
    pub plan_mode: bool,
    pub images: Vec<ImageAttachment>,
    pub mentions: Vec<crate::domain::code::MentionReference>,
    pub editor_context: Option<AgentEditorContext>,
    pub base_system_prompt: Option<String>,
    pub workflow_instructions: Vec<String>,
}

pub(crate) struct AcceptedSendExecution<'a> {
    pub request: AcceptedRuntimeSendInput,
    pub operation_id: &'a str,
    pub execution_obligation_id: &'a str,
    pub session_id: &'a str,
    pub human_message_id: &'a str,
    pub assistant_message_id: Option<&'a str>,
    pub disposition: crate::domain::agent_session::events::SendDisposition,
    pub reserved_turn_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AcceptedQueueDrainOutcome {
    NoWork,
    Blocked,
    Attempted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AcceptedQueueRedriveReadiness {
    /// The exact local queue item still has a process-owned reason not to run.
    Blocked,
    /// The exact local front can be retried, including transient store errors
    /// that need the redriver's capped retry ownership.
    Ready,
    /// The retained process marker no longer has its exact local queue item.
    Missing,
}

#[derive(Debug, Clone)]
struct AcceptedTurnExecutionIdentity {
    operation_id: String,
    execution_obligation_id: String,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub struct SendMessageResponse {
    pub session: ChatSession,
    pub human_message: ChatMessage,
    pub agent_message: Option<ChatMessage>,
    pub queued_turn: Option<QueuedAgentTurn>,
    pub pending_queue: Vec<QueuedAgentTurn>,
    pub pending_queue_count: usize,
    pub sessions: Vec<SessionSummary>,
}

#[cfg(test)]
struct SendResponseProjection {
    session: ChatSession,
    sessions: Vec<SessionSummary>,
}

#[cfg(test)]
impl SendResponseProjection {
    fn into_accepted_queue_response(
        mut self,
        session_title: Option<String>,
        human_message: ChatMessage,
        persisted_meta: SessionMeta,
        queued_turn: QueuedAgentTurn,
        pending_queue: Vec<QueuedAgentTurn>,
    ) -> SendMessageResponse {
        self.session = persisted_meta.to_session(Vec::new());
        let mut persisted_summary = persisted_meta.to_summary();
        if let Some(title) = session_title {
            persisted_summary.first_message = title;
        }
        if let Some(summary) = self
            .sessions
            .iter_mut()
            .find(|summary| summary.id == self.session.id)
        {
            *summary = persisted_summary;
        }
        self.sessions.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        SendMessageResponse {
            session: self.session,
            human_message,
            agent_message: None,
            queued_turn: Some(queued_turn),
            pending_queue_count: pending_queue.len(),
            pending_queue,
            sessions: self.sessions,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelQueuedTurnResponse {
    pub session_id: String,
    pub canceled_count: usize,
    pub pending_queue: Vec<QueuedAgentTurn>,
    pub pending_queue_count: usize,
}

#[derive(Debug, Clone)]
pub struct InitSessionsResponse {
    pub sessions: Vec<SessionSummary>,
    pub active_session: Option<GetSessionResponse>,
    pub permission_mode: String,
    pub plan_mode: bool,
}

#[derive(Clone)]
pub(super) struct RuntimeContext {
    pub(super) session_store: Arc<SessionStore>,
    pub(super) registry: Arc<AgentBackendRegistry>,
    pub(super) status_center: Arc<AgentStatusCenter>,
    pub(super) status_notifier: Arc<dyn AgentStatusNotifier>,
    pub(super) notifier: Arc<dyn AgentSessionEventNotifier>,
    pub(super) projection_gateway: Arc<dyn AgentRuntimeProjectionGateway>,
    pub(super) spawner: Arc<dyn AgentTaskSpawner>,
    pub(super) branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    pub(super) instruction_source: Arc<dyn InstructionSourcePort>,
    pub(super) data_dir: Arc<PathBuf>,
    pub(super) workspace_query: Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
    pub(super) sessions: Arc<Mutex<RuntimeSessionMap>>,
    pub(super) session_locks: SessionCommandLocks,
    pub(super) runtime_event_locks: SessionLockMap,
    pub(super) transitions: SessionTransitionCoordinator,
    pub(super) shutdown_admission: Arc<ShutdownAdmission>,
    pub(super) workflow_turn_complete_notifier:
        Arc<RwLock<Option<Arc<dyn WorkflowTurnCompleteNotifier>>>>,
    pub(super) workflow_stall_notifier: Arc<RwLock<Option<Arc<dyn WorkflowStallNotifier>>>>,
    pub(super) accepted_send_obligation_driver:
        Arc<RwLock<Option<Arc<dyn AcceptedSendObligationDriver>>>>,
    #[cfg(test)]
    pub(super) durable_workflow_send_driver:
        Arc<RwLock<Option<Arc<dyn DurableWorkflowSendDriver>>>>,
    pub(super) durable_stop_driver: Arc<RwLock<Option<Arc<dyn DurableStopDriver>>>>,
    pub(super) lifecycle_repository: Arc<
        RwLock<
            Option<
                Weak<dyn crate::domain::agent_session::repository::AgentSessionLifecycleRepository>,
            >,
        >,
    >,
}

impl RuntimeContext {
    pub(super) fn lifecycle_repository(
        &self,
    ) -> Option<Arc<dyn crate::domain::agent_session::repository::AgentSessionLifecycleRepository>>
    {
        self.lifecycle_repository
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .and_then(Weak::upgrade)
    }
}

const PERSIST_MAX_ATTEMPTS: usize = 4;
const PERSIST_RETRY_BACKOFFS: [Duration; PERSIST_MAX_ATTEMPTS - 1] = [
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(200),
];

#[derive(Debug, Clone, Copy)]
enum PersistFailureKind {
    ReopenRuntime,
    QueuedTurnInterrupt,
    FinalPartsRecorded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PersistenceLogRecord {
    EventLogRecovered {
        session_id: String,
        kind: &'static str,
    },
    PersistFailure {
        session_id: String,
        kind: &'static str,
        attempts: usize,
        error: String,
    },
}

#[cfg(test)]
std::thread_local! {
    static PERSISTENCE_LOG_RECORDS: std::cell::RefCell<Vec<PersistenceLogRecord>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn emit_persistence_log_record(record: PersistenceLogRecord) {
    match &record {
        PersistenceLogRecord::EventLogRecovered { session_id, kind } => {
            log::warn!("agent_session_persist_notice session_id={session_id} kind={kind}");
        }
        PersistenceLogRecord::PersistFailure {
            session_id,
            kind,
            attempts,
            error,
        } => {
            log::error!(
                "agent_session_persist_failure session_id={session_id} kind={kind} attempts={attempts} error={error}"
            );
        }
    }
    #[cfg(test)]
    PERSISTENCE_LOG_RECORDS.with(|records| records.borrow_mut().push(record));
}

#[cfg(test)]
fn take_persistence_log_records() -> Vec<PersistenceLogRecord> {
    PERSISTENCE_LOG_RECORDS.with(|records| std::mem::take(&mut *records.borrow_mut()))
}

impl PersistFailureKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReopenRuntime => "reopen_runtime",
            Self::QueuedTurnInterrupt => "queued_turn_interrupt",
            Self::FinalPartsRecorded => "final_parts_recorded",
        }
    }

    fn notice_message(self) -> &'static str {
        match self {
            Self::ReopenRuntime => {
                "Failed to save the session error state after retrying."
            }
            Self::QueuedTurnInterrupt => {
                "Failed to save the queued turn failure after retrying."
            }
            Self::FinalPartsRecorded => {
                "Failed to save the completed response after retrying. The existing response body was preserved."
            }
        }
    }
}

#[cfg(test)]
#[derive(Clone)]
struct StalledActiveTurnTarget {
    runtime: Arc<dyn AgentSessionRuntime>,
}

pub struct AgentSessionRuntimeUsecase {
    pub(super) ctx: RuntimeContext,
}

include!("driver.rs");
include!("persistence.rs");
include!("watchdog.rs");
include!("recovery.rs");
include!("event_dispatch.rs");
include!("streaming_apply.rs");
include!("queue_driver.rs");
include!("projections.rs");
#[cfg(test)]
include!("tests.rs");
