use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::domain::agent_session::entities::{
    AttachmentPayload, InterruptReason as DomainInterruptReason, MessagePart as DomainMessagePart,
    PermissionDecision as DomainPermissionDecision, PermissionRequestStatus, PermissionResponse,
    PermissionResponseDecision, ToolResultUpdate, TurnResult,
    TurnStopReason as DomainTurnStopReason,
};
use crate::domain::agent_session::gateway::{
    AgentBackendError, AgentRuntimeEvent, AgentSessionRuntime, SessionSpec, TurnInput,
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
use crate::usecase::agent_session::event_log::PromptInput;
use crate::usecase::agent_session::event_log::{
    append_part_events, finalize_turn, latest_unresolved_permission_request, AgentSessionEvent,
    BackendSessionRecoveryProjection, BackendSessionRecoveryReason,
    InterruptReason as EventInterruptReason, PartEventMode, TurnEventLog,
    TurnStopReason as EventTurnStopReason, TurnTokenUsage, UnresolvedPermissionRequest,
    WorkflowTurnCompleteInput,
};
#[cfg(test)]
use crate::usecase::agent_session::session::{
    add_message_internal, add_message_with_meta_internal, create_session_with_model_and_plan_mode,
    SessionMeta,
};
use crate::usecase::agent_session::session::{
    apply_tool_result_update, CanonicalQueuedSend, ChatMessage, ChatSession, ContextCarryState,
    ContextRestoreCompletionRequest, ErrorEpisodeInput, GetSessionResponse, ImageAttachment,
    InitialSessionPage, MessagePart, MessageRole, ModelInfo, OpenTabRegistry,
    PendingRecoveryMessage, PermissionPartStatus, PermissionRequestMsg,
    ProviderSessionEstablishmentOutcome, QueuedAgentTurn, SessionState, SessionStore,
    SessionSummary, INITIAL_SESSION_PAGE_LIMIT, RETAINED_MESSAGE_CAP,
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
    WorkflowTurnCompleteNotification, WorkflowTurnFailureSignal, WorkflowTurnTokenUsage,
};

use super::context_restore::{
    apply_restore_prompt_prefix, context_restore_plan_for_session,
    context_restore_plan_for_session_before_turn, ContextRestorePlan,
};
use super::event_apply::{
    parts_from_domain, pending_permission_request_msg, token_usage_from_domain,
};
#[cfg(test)]
use super::ports::AcceptedSendRecoveryWake;
use super::ports::{
    AcceptedQueuedTurnExecutionClaimOutcome, AcceptedSendExecutionClaim,
    AcceptedSendObligationDriver, AgentSessionEventNotifier, AgentSessionStateChangedPayload,
    AgentStallObservedPayload, AgentStreamingDeltaPayload, AgentTaskSpawner, WorkflowStallNotifier,
    WorkflowTurnCompleteNotifier,
};
use super::queue::QueuedTurnInput;
use super::session_state::{
    BackendSessionRecoveryState, BackendSessionRecoveryTurnResume, PendingStreamDelta,
    PermissionRequestVisibility, ProviderSessionEstablishmentState, RuntimeSessionMap,
    RuntimeSessionPhase, RuntimeSessionState,
};
use super::stale::{
    effective_stale_timeout, has_in_flight_tool_use, recovery_cap_reached, remaining_until_stale,
    stale_timeout_for_session, stale_watchdog_should_continue_waiting, stall_cap_reached,
    startup_max_retries_for_session, startup_timeout_for_session, turn_is_stale,
};
use super::streaming::{
    parts_can_stream_as_append_delta, should_persist_streaming_snapshot, streaming_flush_decision,
    streaming_parts_byte_size, StreamingFlushDecision,
};
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

const CLOSE_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(200);
const CLOSE_DRAIN_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

#[derive(Default)]
struct ShutdownAdmissionState {
    shutting_down: bool,
    active_operations: usize,
}

#[derive(Default)]
pub(super) struct ShutdownAdmission {
    state: std::sync::Mutex<ShutdownAdmissionState>,
    idle: tokio::sync::Notify,
}

impl ShutdownAdmission {
    pub(super) fn admit(self: &Arc<Self>) -> Result<ShutdownAdmissionGuard, AgentRuntimeError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.shutting_down {
            return Err(AgentRuntimeError::Other(
                "Agent session runtime is shutting down".to_string(),
            ));
        }
        state.active_operations += 1;
        Ok(ShutdownAdmissionGuard {
            admission: Arc::clone(self),
        })
    }

    #[cfg(test)]
    fn begin_shutdown(&self) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .shutting_down = true;
    }

    #[cfg(test)]
    fn cancel_shutdown(&self) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .shutting_down = false;
    }

    #[cfg(test)]
    async fn wait_for_idle(&self) {
        loop {
            let notified = self.idle.notified();
            if self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .active_operations
                == 0
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
            .shutting_down
    }
}

pub(super) struct ShutdownAdmissionGuard {
    admission: Arc<ShutdownAdmission>,
}

impl Drop for ShutdownAdmissionGuard {
    fn drop(&mut self) {
        let became_idle = {
            let mut state = self
                .admission
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.active_operations -= 1;
            state.active_operations == 0
        };
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
    if accepted_execution {
        fail_accepted_effect_preflight(stage, error)
    } else {
        error
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

#[derive(Debug, Clone)]
pub(crate) struct DurableWorkflowTurnRequest {
    pub operation_id: String,
    pub session_id: String,
    pub content: String,
    pub permission_mode: PermissionMode,
    pub base_system_prompt: Option<String>,
    pub workflow_instructions: Vec<String>,
}

#[async_trait::async_trait]
pub(crate) trait DurableWorkflowSendDriver: Send + Sync {
    async fn send(&self, request: DurableWorkflowTurnRequest) -> Result<(), String>;
}

pub(crate) fn durable_workflow_turn_operation_id(
    node_execution_id: &str,
    turn_role: &str,
) -> String {
    use sha2::{Digest, Sha256};

    fn append_field(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }

    let mut identity = b"durable-workflow-turn/v1".to_vec();
    append_field(&mut identity, node_execution_id);
    append_field(&mut identity, turn_role);
    let digest = Sha256::digest(identity);
    format!("workflow-send-{}", hex::encode(digest))
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
    pub(super) spawner: Arc<dyn AgentTaskSpawner>,
    pub(super) branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    pub(super) instruction_source: Arc<dyn InstructionSourcePort>,
    pub(super) data_dir: Arc<PathBuf>,
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
    pub(super) durable_workflow_send_driver:
        Arc<RwLock<Option<Arc<dyn DurableWorkflowSendDriver>>>>,
    pub(super) durable_stop_driver: Arc<RwLock<Option<Arc<dyn DurableStopDriver>>>>,
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

impl AgentSessionRuntimeUsecase {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_store: Arc<SessionStore>,
        registry: Arc<AgentBackendRegistry>,
        status_center: Arc<AgentStatusCenter>,
        status_notifier: Arc<dyn AgentStatusNotifier>,
        notifier: Arc<dyn AgentSessionEventNotifier>,
        spawner: Arc<dyn AgentTaskSpawner>,
        branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
        instruction_source: Arc<dyn InstructionSourcePort>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            ctx: RuntimeContext {
                session_store,
                registry,
                status_center,
                status_notifier,
                notifier,
                spawner,
                branch_diff_context,
                instruction_source,
                data_dir: Arc::new(data_dir),
                sessions: Arc::new(Mutex::new(RuntimeSessionMap::new())),
                session_locks: SessionCommandLocks::default(),
                runtime_event_locks: SessionLockMap::default(),
                transitions: SessionTransitionCoordinator::default(),
                shutdown_admission: Arc::new(ShutdownAdmission::default()),
                workflow_turn_complete_notifier: Arc::new(RwLock::new(None)),
                workflow_stall_notifier: Arc::new(RwLock::new(None)),
                accepted_send_obligation_driver: Arc::new(RwLock::new(None)),
                durable_workflow_send_driver: Arc::new(RwLock::new(None)),
                durable_stop_driver: Arc::new(RwLock::new(None)),
            },
        }
    }

    pub(crate) fn report_event_log_recovered(&self, session_id: &str) {
        report_event_log_recovered(
            &self.ctx.status_center,
            &self.ctx.status_notifier,
            &self.ctx.notifier,
            session_id,
        );
    }

    pub fn set_workflow_turn_complete_notifier(
        &self,
        notifier: Arc<dyn WorkflowTurnCompleteNotifier>,
    ) {
        *self
            .ctx
            .workflow_turn_complete_notifier
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(notifier);
    }

    pub fn set_workflow_stall_notifier(&self, notifier: Arc<dyn WorkflowStallNotifier>) {
        *self
            .ctx
            .workflow_stall_notifier
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(notifier);
    }

    pub(crate) fn set_accepted_send_obligation_driver(
        &self,
        driver: Arc<dyn AcceptedSendObligationDriver>,
    ) {
        *self
            .ctx
            .accepted_send_obligation_driver
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(driver);
    }

    pub(crate) fn set_durable_workflow_send_driver(
        &self,
        driver: Arc<dyn DurableWorkflowSendDriver>,
    ) {
        *self
            .ctx
            .durable_workflow_send_driver
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(driver);
    }

    pub(crate) fn set_durable_stop_driver(&self, driver: Arc<dyn DurableStopDriver>) {
        *self
            .ctx
            .durable_stop_driver
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(driver);
    }

    pub fn list_backends(&self) -> BackendListResult {
        self.ctx.registry.list_result()
    }

    pub(crate) fn backend_registry(&self) -> &AgentBackendRegistry {
        self.ctx.registry.as_ref()
    }

    #[cfg(test)]
    pub async fn send_message(
        &self,
        req: SendAgentMessageRequest,
    ) -> Result<SendMessageResponse, AgentRuntimeError> {
        self.send_message_with_reserved_session_id(req, None).await
    }

    /// Executes an already accepted send using the session identity fixed by
    /// the durable receipt. `reserved_session_id` is used only for a new
    /// session; regular callers keep using [`Self::send_message`].
    #[cfg(test)]
    pub async fn send_message_with_reserved_session_id(
        &self,
        req: SendAgentMessageRequest,
        reserved_session_id: Option<String>,
    ) -> Result<SendMessageResponse, AgentRuntimeError> {
        let _admission_guard = self.ctx.shutdown_admission.admit()?;
        let mut session_guard = match req.chat_session_id.as_deref() {
            Some(session_id) => {
                Some(acquire_session_control_after_recovery(&self.ctx, session_id).await)
            }
            None => None,
        };
        if let Some(session_id) = req.chat_session_id.as_deref() {
            self.ensure_session_not_closing(session_id).await?;
        }
        let session = self
            .resolve_or_create_session(&req, reserved_session_id.as_deref())
            .await?;
        if session_guard.is_none() {
            session_guard =
                Some(acquire_session_control_after_recovery(&self.ctx, &session.id).await);
        }
        let images = req.images.unwrap_or_default();
        let mentions = req.mentions.unwrap_or_default();
        let session_id = session.id.clone();
        let backend_id = required_backend_id(&session)?;
        self.hydrate_runtime_session_state(&session).await?;
        self.recover_queued_turn_if_idle_without_runtime(&session_id)
            .await;
        let stalled_active_turn = if self.backend_supports_steering(&backend_id) {
            self.stalled_active_turn_target(&session_id).await?
        } else {
            None
        };
        if self.is_turn_busy(&session_id).await {
            if let Some(target) = stalled_active_turn {
                target
                    .runtime
                    .steer(TurnInput {
                        prompt: req.content.clone(),
                        images: images
                            .iter()
                            .cloned()
                            .map(|image| AttachmentPayload {
                                data: image.data,
                                media_type: image.media_type,
                            })
                            .collect(),
                        system_prompt: None,
                        permission_mode: req.permission_mode,
                        plan_mode: req.plan_mode,
                        permission_profile_id: session.permission_profile_id.clone(),
                        editor_context: req.editor_context.clone().map(EditorContext::from),
                    })
                    .await
                    .map_err(AgentRuntimeError::from)?;
                let (human_message, _) = add_human_message_internal(
                    &self.ctx.session_store,
                    &self.ctx.data_dir,
                    &session_id,
                    &req.content,
                    &images,
                    &mentions,
                )?;
                return self.send_response(
                    &session_id,
                    &session.worktree_path,
                    human_message,
                    None,
                    None,
                    self.pending_queue(&session_id).await,
                );
            }
            // Resolve fallible read-model projections before accepting the message. Once the
            // human message is persisted and queued, the command must return an accepted
            // response so the composer cannot retain and resend an already-queued input.
            let response_projection =
                self.prepare_send_response_projection(&session_id, &session.worktree_path)?;
            let session_title = self
                .ctx
                .session_store
                .session_title(&self.ctx.data_dir, &session_id)
                .map_err(AgentRuntimeError::Other)?;
            let (human_message, persisted_meta) = add_human_message_internal(
                &self.ctx.session_store,
                &self.ctx.data_dir,
                &session_id,
                &req.content,
                &images,
                &mentions,
            )?;
            let mut queued = QueuedTurnInput::new(
                req.content,
                req.permission_mode,
                req.plan_mode,
                session.permission_profile_id.clone(),
                images,
                session.worktree_path.clone(),
                mentions,
                req.editor_context,
            );
            queued.existing_human_message_id = Some(human_message.id.clone());
            let queued_view = QueuedAgentTurn::from(&queued);
            let pending_queue = {
                let mut sessions = self.ctx.sessions.lock().await;
                let state = sessions
                    .entry(session_id.clone())
                    .or_insert_with(|| RuntimeSessionState::new(backend_id));
                state.pending_queue.push_back(queued);
                pending_queue_view(state)
            };
            return Ok(response_projection.into_accepted_queue_response(
                session_title,
                human_message,
                persisted_meta,
                queued_view,
                pending_queue,
            ));
        }

        let (human_message, _) = add_human_message_internal(
            &self.ctx.session_store,
            &self.ctx.data_dir,
            &session_id,
            &req.content,
            &images,
            &mentions,
        )?;
        let agent_message = add_message_internal(
            &self.ctx.session_store,
            &self.ctx.data_dir,
            &session_id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .map_err(AgentRuntimeError::Other)?;

        self.ctx
            .notifier
            .turn_prepared(&session, &human_message, &agent_message);
        let system_prompt = self.build_turn_system_prompt(
            &session,
            None,
            &mentions,
            req.editor_context.as_ref(),
            Vec::new(),
        )?;
        self.start_turn_for_session(
            &session,
            &human_message,
            agent_message.id.clone(),
            TurnStartPayload {
                prompt: req.content,
                images,
                mentions,
                permission_mode: req.permission_mode,
                plan_mode: req.plan_mode,
                permission_profile_id: session.permission_profile_id.clone(),
                editor_context: req.editor_context,
                system_prompt,
                accepted_execution_identity: None,
            },
            None,
            None,
        )
        .await?;

        let response = self.send_response(
            &session_id,
            &session.worktree_path,
            human_message,
            Some(agent_message),
            None,
            self.pending_queue(&session_id).await,
        );
        drop(session_guard);
        response
    }

    /// Consume the identities already committed by durable send acceptance.
    /// This path never performs send admission, creates a second human
    /// message, or appends another `TurnStarted` for an immediately-started
    /// disposition.
    pub(crate) async fn execute_accepted_send(
        &self,
        execution: AcceptedSendExecution<'_>,
    ) -> Result<(), AgentRuntimeError> {
        let AcceptedSendExecution {
            request: req,
            operation_id,
            execution_obligation_id,
            session_id,
            human_message_id,
            assistant_message_id,
            disposition,
            reserved_turn_id,
        } = execution;
        let _admission_guard = self
            .ctx
            .shutdown_admission
            .admit()
            .map_err(|error| fail_accepted_effect_preflight("shutdown-admission", error))?;
        let _session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        self.ensure_session_not_closing(session_id)
            .await
            .map_err(|error| fail_accepted_effect_preflight("session-closing", error))?;
        let session = self
            .ctx
            .session_store
            .get_session_shell(&self.ctx.data_dir, session_id)
            .map_err(|error| fail_accepted_effect_preflight("session-shell", error))?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
        let backend_id = required_backend_id(&session)?;
        self.hydrate_runtime_session_state(&session)
            .await
            .map_err(|error| fail_accepted_effect_preflight("runtime-hydration", error))?;

        let base_system_prompt = req.base_system_prompt;
        let workflow_instructions = req.workflow_instructions;
        let mut accepted_input = QueuedTurnInput::new(
            req.content,
            req.permission_mode,
            req.plan_mode,
            session.permission_profile_id.clone(),
            req.images,
            session.worktree_path.clone(),
            req.mentions,
            req.editor_context,
        );
        accepted_input.existing_human_message_id = Some(human_message_id.to_string());
        accepted_input.existing_agent_message_id = assistant_message_id.map(str::to_string);
        accepted_input.reserved_turn_id = reserved_turn_id
            .map(str::parse::<u64>)
            .transpose()
            .map_err(|_| AgentRuntimeError::Other("accepted turn identity is invalid".into()))?;
        accepted_input.accepted_operation_id = Some(operation_id.to_string());
        accepted_input.execution_obligation_id = Some(execution_obligation_id.to_string());

        match disposition {
            crate::domain::agent_session::events::SendDisposition::Queued { queue_item_id } => {
                accepted_input.id = queue_item_id;
                let reserved_turn_id = accepted_input.reserved_turn_id.ok_or_else(|| {
                    AgentRuntimeError::Other(
                        "accepted queued send is missing its reserved turn identity".into(),
                    )
                })?;
                let canonical_queue = self
                    .ctx
                    .session_store
                    .canonical_pending_send_queue(session_id)
                    .map_err(|error| fail_accepted_effect_preflight("canonical-queue", error))?;
                let mut sessions = self.ctx.sessions.lock().await;
                let state = sessions
                    .entry(session_id.to_string())
                    .or_insert_with(|| RuntimeSessionState::new(backend_id));
                if state.current_turn_input.as_ref().is_some_and(|current| {
                    current.accepted_operation_id.as_deref() == Some(operation_id)
                        && current.execution_obligation_id.as_deref()
                            == Some(execution_obligation_id)
                }) {
                    return Ok(());
                }
                if state.pending_queue.iter().any(|queued| {
                    queued.id != accepted_input.id
                        && queued.reserved_turn_id == Some(reserved_turn_id)
                        && (queued.accepted_operation_id.as_deref() != Some(operation_id)
                            || queued.execution_obligation_id.as_deref()
                                != Some(execution_obligation_id))
                }) {
                    return Err(AgentRuntimeError::Other(format!(
                        "accepted queued turn identity {reserved_turn_id} is already owned"
                    )));
                }
                insert_accepted_queue_in_canonical_order(
                    &mut state.pending_queue,
                    accepted_input,
                    &canonical_queue,
                )
                .map_err(AgentRuntimeError::Other)?;
                Ok(())
            }
            crate::domain::agent_session::events::SendDisposition::StartedTurn { turn_id } => {
                let committed_turn_id = turn_id.parse::<u64>().map_err(|_| {
                    AgentRuntimeError::Other("accepted turn identity is invalid".into())
                })?;
                let human_message = queued_human_message(&accepted_input);
                let assistant_message_id = assistant_message_id.ok_or_else(|| {
                    AgentRuntimeError::Other(
                        "accepted send is missing its committed assistant identity".into(),
                    )
                })?;
                let agent_message = self
                    .ctx
                    .session_store
                    .canonical_message_projection(session_id, assistant_message_id)
                    .map_err(|error| fail_accepted_effect_preflight("assistant-projection", error))?
                    .ok_or_else(|| {
                        AgentRuntimeError::Other(
                            "accepted assistant projection is unavailable".into(),
                        )
                    })?;
                self.ctx
                    .notifier
                    .turn_prepared(&session, &human_message, &agent_message);
                let system_prompt = self
                    .build_turn_system_prompt(
                        &session,
                        base_system_prompt,
                        &accepted_input.mentions,
                        accepted_input.editor_context.as_ref(),
                        workflow_instructions,
                    )
                    .map_err(|error| fail_accepted_effect_preflight("system-prompt", error))?;
                self.start_turn_for_session(
                    &session,
                    &human_message,
                    agent_message.id,
                    TurnStartPayload {
                        prompt: accepted_input.content,
                        images: accepted_input.images,
                        mentions: accepted_input.mentions,
                        permission_mode: accepted_input.permission_mode,
                        plan_mode: accepted_input.plan_mode,
                        permission_profile_id: accepted_input.permission_profile_id,
                        editor_context: accepted_input.editor_context,
                        system_prompt,
                        accepted_execution_identity: Some(AcceptedTurnExecutionIdentity {
                            operation_id: operation_id.to_string(),
                            execution_obligation_id: execution_obligation_id.to_string(),
                        }),
                    },
                    None,
                    Some(committed_turn_id),
                )
                .await
            }
        }
    }

    /// A stored provider id is only resume input. It does not prove the
    /// current process has observed a successful create/resume handshake.
    #[cfg(test)]
    pub(crate) async fn provider_session_is_confirmed(&self, session_id: &str) -> bool {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .is_some_and(|state| state.runtime.is_some() && state.provider_session_established)
    }

    /// Read the process-local send admission state without acquiring the
    /// per-session command lock. Workflow activation already owns that lock
    /// while it commits its durable Send operation.
    pub(crate) async fn workflow_send_runtime_is_busy(&self, session_id: &str) -> bool {
        let sessions = self.ctx.sessions.lock().await;
        sessions.get(session_id).is_some_and(|state| {
            state.phase != RuntimeSessionPhase::Idle
                || state.queue_paused
                || !state.pending_queue.is_empty()
        })
    }

    /// Process-local ownership proof for hiding only the exact accepted turn
    /// currently driven by this runtime. Durable status alone cannot
    /// distinguish a live reservation from one left by a crashed process.
    pub(crate) async fn owns_accepted_turn_execution(
        &self,
        session_id: &str,
        operation_id: &str,
        obligation_id: &str,
    ) -> bool {
        let sessions = self.ctx.sessions.lock().await;
        sessions.get(session_id).is_some_and(|state| {
            state.phase != RuntimeSessionPhase::Idle
                && state.current_turn_input.as_ref().is_some_and(|input| {
                    input.accepted_operation_id.as_deref() == Some(operation_id)
                        && input.execution_obligation_id.as_deref() == Some(obligation_id)
                })
        })
    }

    /// Read-only recovery fence for workflow and other aggregate operations.
    /// This deliberately does not open or hydrate a live provider session.
    pub(crate) fn ensure_recovery_operation_allowed(
        &self,
        session_id: &str,
    ) -> Result<(), AgentRuntimeError> {
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)
    }

    #[cfg(test)]
    pub async fn start_session(
        &self,
        session_id: &str,
        opts: StartSessionOptions,
    ) -> Result<(), AgentRuntimeError> {
        let _admission_guard = self.ctx.shutdown_admission.admit()?;
        let _session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        self.ensure_session_not_closing(session_id).await?;
        let mut session = self
            .ctx
            .session_store
            .get_session_shell(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
        if session.permission_mode != opts.permission_mode.as_str() {
            self.ctx
                .session_store
                .update_permission_mode(
                    &self.ctx.data_dir,
                    session_id,
                    opts.permission_mode.as_str(),
                )
                .map_err(AgentRuntimeError::Other)?;
            session.permission_mode = opts.permission_mode.as_str().to_string();
        }
        if session.plan_mode != opts.plan_mode {
            self.ctx
                .session_store
                .update_plan_mode(&self.ctx.data_dir, session_id, opts.plan_mode)
                .map_err(AgentRuntimeError::Other)?;
            session.plan_mode = opts.plan_mode;
        }
        match self.ensure_runtime(&session, None).await {
            Ok(_) => Ok(()),
            Err(AgentRuntimeError::BackendSessionLost { .. }) => {
                recover_backend_session(
                    &self.ctx,
                    session_id,
                    BackendSessionRecoveryReason::BackendSessionLost,
                )
                .await
            }
            Err(error) => Err(error),
        }
    }

    /// Read-only admission for the durable permission-response operation.
    /// This verifies the exact pending request and live provider owner but
    /// performs no provider I/O and no persistence mutation.
    pub(crate) async fn prepare_permission_response_operation(
        &self,
        session_id: &str,
        response: &PermissionResponse,
    ) -> Result<(u64, bool), AgentRuntimeError> {
        let _session_guard = self
            .acquire_session_control_after_recovery(session_id)
            .await;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        self.ensure_session_not_closing(session_id).await?;
        let pending = self
            .pending_permission_for_response(session_id, response)
            .await?;
        let turn_id = pending.turn_id.ok_or_else(|| {
            AgentRuntimeError::Other(format!(
                "Permission response has no durable turn identity for session {session_id}"
            ))
        })?;
        let has_runtime = self
            .ctx
            .sessions
            .lock()
            .await
            .get(session_id)
            .and_then(|state| state.runtime.as_ref())
            .is_some();
        if !has_runtime {
            return Err(AgentRuntimeError::Other(format!(
                "No active agent runtime for session {session_id}"
            )));
        }
        Ok((turn_id, pending.from_runtime_state))
    }

    /// The only production provider handoff for a permission response. The
    /// durable operation has already accepted and claimed the exact payload;
    /// this method deliberately performs no reservation or completion write.
    pub(crate) async fn execute_accepted_permission_response_effect(
        &self,
        session_id: &str,
        turn_id: u64,
        response: PermissionResponse,
    ) -> Result<(), AgentRuntimeError> {
        let _session_guard = self
            .acquire_session_control_after_recovery(session_id)
            .await;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        self.ensure_session_not_closing(session_id).await?;
        let pending = self
            .pending_permission_for_response(session_id, &response)
            .await?;
        if pending.turn_id != Some(turn_id) {
            return Err(AgentRuntimeError::Other(format!(
                "Permission response turn identity changed for session {session_id}"
            )));
        }
        let runtime = {
            let sessions = self.ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .and_then(|state| state.runtime.clone())
        }
        .ok_or_else(|| {
            AgentRuntimeError::Other(format!("No active agent runtime for session {session_id}"))
        })?;
        runtime
            .respond_permission(response)
            .await
            .map_err(AgentRuntimeError::from)
    }

    /// Refresh process-local mirrors only after the operation completion
    /// batch (operation state, obligation, event and projections) is durable.
    pub(crate) async fn apply_permission_response_completion(
        &self,
        session_id: &str,
        response: &PermissionResponse,
        from_runtime_state: bool,
    ) {
        let (
            patched,
            did_resume_streaming,
            permission_wait_measurement,
            pending_permission_state_revision,
            cleared_stall,
        ) = {
            let mut sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return;
            };
            let pending_in_state_matches = state
                .pending_permission_request
                .as_ref()
                .is_some_and(|pending| pending.id == response.request_id);
            let patched = pending_in_state_matches
                .then(|| patch_permission_response_in_state(state, response))
                .flatten();
            let did_resume_streaming = (state.phase == RuntimeSessionPhase::WaitingPermission
                && pending_in_state_matches)
                || !from_runtime_state;
            let mut pending_permission_state_revision = None;
            let mut cleared_stall = false;
            if did_resume_streaming {
                state.phase = RuntimeSessionPhase::Streaming;
                pending_permission_state_revision = Some(state.clear_pending_permission_request());
                cleared_stall = state.record_progress(std::time::Instant::now());
                state.permission_wait_diagnostic_emitted = false;
            }
            let permission_wait_measurement = did_resume_streaming
                .then(|| {
                    state
                        .permission_wait_started_at
                        .take()
                        .map(|started_at| started_at.elapsed())
                })
                .flatten();
            (
                patched,
                did_resume_streaming,
                permission_wait_measurement,
                pending_permission_state_revision,
                cleared_stall,
            )
        };
        if cleared_stall {
            if let Err(error) = dispatch_stall_cleared_notifications(&self.ctx, session_id).await {
                log::warn!("workflow stall-cleared notification failed for {session_id}: {error}");
            }
        }
        if let Some(elapsed) = permission_wait_measurement {
            record_agent_turn_duration_detached(
                &self.ctx,
                session_id.to_string(),
                crate::other::telemetry::AgentTurn::PermissionWait,
                elapsed,
            );
        }
        if let Some((message_id, seq, parts, _turn_id)) = patched {
            emit_streaming_delta_or_retry(
                &self.ctx,
                session_id,
                PendingStreamDelta {
                    message_id,
                    seq,
                    snapshot: true,
                    parts,
                    message: None,
                    authoritative: true,
                },
            )
            .await;
        }
        if did_resume_streaming {
            emit_session_state_change(
                &self.ctx.session_store,
                &self.ctx.notifier,
                &self.ctx.status_center,
                &self.ctx.status_notifier,
                &self.ctx.data_dir,
                session_id,
                StateChange {
                    turn_phase: TurnPhase::Streaming,
                    queue_paused: None,
                    pending_permission_request: None,
                    pending_permission_state_revision,
                    exit_code: None,
                    completed_at: None,
                    interrupted: false,
                    session_state: Some(SessionState::Active),
                },
            );
        }
    }

    #[cfg(test)]
    pub async fn respond_permission(
        &self,
        session_id: &str,
        response: PermissionResponse,
    ) -> Result<(), AgentRuntimeError> {
        let _session_guard = self
            .acquire_session_control_after_recovery(session_id)
            .await;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        self.ensure_session_not_closing(session_id).await?;
        let pending = self
            .pending_permission_for_response(session_id, &response)
            .await?;
        let turn_id = pending.turn_id.ok_or_else(|| {
            AgentRuntimeError::Other(format!(
                "Permission response has no durable turn identity for session {session_id}"
            ))
        })?;
        let runtime = {
            let sessions = self.ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .and_then(|state| state.runtime.clone())
        }
        .ok_or_else(|| {
            AgentRuntimeError::Other(format!("No active agent runtime for session {session_id}"))
        })?;
        let projected_message = {
            let sessions = self.ctx.sessions.lock().await;
            sessions.get(session_id).and_then(|state| {
                let mut parts = state.domain_streaming_parts.clone();
                if !patch_permission_response_in_domain_parts(&mut parts, &response) {
                    return None;
                }
                state
                    .streaming_message_id
                    .clone()
                    .or_else(|| state.last_agent_message_id.clone())
                    .map(|message_id| (message_id, state.streaming_delta_seq.saturating_add(1)))
            })
        };
        let obligation_id = self
            .ctx
            .session_store
            .reserve_permission_response(
                &self.ctx.data_dir,
                session_id,
                turn_id,
                &response.request_id,
                response.clone(),
            )
            .map_err(AgentRuntimeError::Other)?;
        self.ctx
            .session_store
            .claim_permission_response_effect(session_id, &obligation_id)
            .map_err(AgentRuntimeError::Other)?;
        runtime
            .respond_permission(response.clone())
            .await
            .map_err(AgentRuntimeError::from)?;
        let resolved_event = permission_resolved_event(turn_id, &response);
        self.ctx
            .session_store
            .complete_permission_response(
                &self.ctx.data_dir,
                session_id,
                &obligation_id,
                resolved_event,
                projected_message
                    .as_ref()
                    .map(|(message_id, _)| message_id.as_str()),
                projected_message.as_ref().map(|(_, seq)| *seq),
            )
            .map_err(AgentRuntimeError::Other)?;
        let (
            patched,
            did_resume_streaming,
            permission_wait_measurement,
            pending_permission_state_revision,
            cleared_stall,
        ) = {
            let mut sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return Ok(());
            };
            let patched = patch_permission_response_in_state(state, &response);
            let pending_in_state_matches = state
                .pending_permission_request
                .as_ref()
                .is_some_and(|pending| pending.id == response.request_id);
            let did_resume_streaming = (state.phase == RuntimeSessionPhase::WaitingPermission
                && pending_in_state_matches)
                || !pending.from_runtime_state;
            let mut pending_permission_state_revision = None;
            let mut cleared_stall = false;
            if did_resume_streaming {
                state.phase = RuntimeSessionPhase::Streaming;
                pending_permission_state_revision = Some(state.clear_pending_permission_request());
                cleared_stall = state.record_progress(std::time::Instant::now());
                state.permission_wait_diagnostic_emitted = false;
            }
            let permission_wait_measurement = did_resume_streaming
                .then(|| {
                    state
                        .permission_wait_started_at
                        .take()
                        .map(|started_at| started_at.elapsed())
                })
                .flatten();
            (
                patched,
                did_resume_streaming,
                permission_wait_measurement,
                pending_permission_state_revision,
                cleared_stall,
            )
        };
        if cleared_stall {
            if let Err(error) = dispatch_stall_cleared_notifications(&self.ctx, session_id).await {
                log::warn!("workflow stall-cleared notification failed for {session_id}: {error}");
            }
        }
        if let Some(elapsed) = permission_wait_measurement {
            record_agent_turn_duration_detached(
                &self.ctx,
                session_id.to_string(),
                crate::other::telemetry::AgentTurn::PermissionWait,
                elapsed,
            );
        }
        if let Some((message_id, seq, parts, _turn_id)) = patched {
            emit_streaming_delta_or_retry(
                &self.ctx,
                session_id,
                PendingStreamDelta {
                    message_id,
                    seq,
                    snapshot: true,
                    parts,
                    message: None,
                    authoritative: true,
                },
            )
            .await;
        }
        if did_resume_streaming {
            emit_session_state_change(
                &self.ctx.session_store,
                &self.ctx.notifier,
                &self.ctx.status_center,
                &self.ctx.status_notifier,
                &self.ctx.data_dir,
                session_id,
                StateChange {
                    turn_phase: TurnPhase::Streaming,
                    queue_paused: None,
                    pending_permission_request: None,
                    pending_permission_state_revision,
                    exit_code: None,
                    completed_at: None,
                    interrupted: false,
                    session_state: Some(SessionState::Active),
                },
            );
        }
        Ok(())
    }

    pub async fn report_permission_request_observed(
        &self,
        session_id: &str,
        request_id: &str,
        visible: bool,
    ) -> Result<(), AgentRuntimeError> {
        let mut sessions = self.ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return Ok(());
        };
        if visible {
            let pending_matches = state
                .pending_permission_request
                .as_ref()
                .is_some_and(|pending| pending.id == request_id);
            if pending_matches {
                state.permission_request_visibility = Some(PermissionRequestVisibility {
                    request_id: request_id.to_string(),
                    last_seen_at: std::time::Instant::now(),
                });
            }
            return Ok(());
        }
        if state
            .permission_request_visibility
            .as_ref()
            .is_some_and(|visibility| visibility.request_id == request_id)
        {
            state.permission_request_visibility = None;
        }
        Ok(())
    }

    async fn pending_permission_for_response(
        &self,
        session_id: &str,
        response: &PermissionResponse,
    ) -> Result<PendingPermissionForResponse, AgentRuntimeError> {
        {
            let sessions = self.ctx.sessions.lock().await;
            if let Some((pending, turn_id)) = sessions.get(session_id).and_then(|state| {
                state
                    .pending_permission_request
                    .as_ref()
                    .map(|pending| (pending.clone(), state.current_turn_id))
            }) {
                if pending.id != response.request_id {
                    return Err(AgentRuntimeError::Other(format!(
                        "Permission request id mismatch: pending={}, response={}",
                        pending.id, response.request_id
                    )));
                }
                return Ok(PendingPermissionForResponse {
                    turn_id,
                    from_runtime_state: true,
                });
            }
        }

        let Some(pending) = self.unresolved_permission_request_from_event_log(session_id) else {
            return Err(AgentRuntimeError::Other(format!(
                "No pending permission request for session {session_id}"
            )));
        };
        if pending.request.id != response.request_id {
            return Err(AgentRuntimeError::Other(format!(
                "Permission request id mismatch: pending={}, response={}",
                pending.request.id, response.request_id
            )));
        }
        Ok(PendingPermissionForResponse {
            turn_id: Some(pending.turn_id),
            from_runtime_state: false,
        })
    }

    #[cfg(test)]
    async fn recover_queued_turn_if_idle_without_runtime(&self, session_id: &str) {
        let should_drain = {
            let sessions = self.ctx.sessions.lock().await;
            sessions.get(session_id).is_some_and(|state| {
                state.phase == RuntimeSessionPhase::Idle && !state.pending_queue.is_empty()
            })
        };
        if should_drain {
            start_next_queued_turn(&self.ctx, session_id).await;
        }
    }

    pub async fn set_permission_mode(
        &self,
        session_id: &str,
        mode: PermissionMode,
    ) -> Result<(), AgentRuntimeError> {
        let _admission_guard = self.ctx.shutdown_admission.admit()?;
        let _session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        self.ctx
            .session_store
            .update_permission_mode_from_user(&self.ctx.data_dir, session_id, mode.as_str())
            .map_err(AgentRuntimeError::Other)?;
        self.ctx
            .notifier
            .permission_mode_changed(session_id, mode.as_str());
        Ok(())
    }

    pub async fn set_plan_mode(
        &self,
        session_id: &str,
        plan_mode: bool,
    ) -> Result<(), AgentRuntimeError> {
        let _admission_guard = self.ctx.shutdown_admission.admit()?;
        let _session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        self.ctx
            .session_store
            .update_plan_mode_from_user(&self.ctx.data_dir, session_id, plan_mode)
            .map_err(AgentRuntimeError::Other)?;
        Ok(())
    }

    pub async fn set_model(
        &self,
        session_id: &str,
        entry_id: &str,
    ) -> Result<(), AgentRuntimeError> {
        let _admission_guard = self.ctx.shutdown_admission.admit()?;
        let _session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        let entry = self
            .ctx
            .registry
            .resolve_model_entry(entry_id)
            .map_err(AgentRuntimeError::Other)?;
        let (session, page, _) = self
            .ctx
            .session_store
            .get_session_with_latest_page(&self.ctx.data_dir, session_id, 1)
            .map_err(AgentRuntimeError::Other)?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
        let backend_changes = session.backend_id.as_deref() != Some(entry.backend.as_str());
        if backend_changes {
            let runtime_is_idle = {
                let sessions = self.ctx.sessions.lock().await;
                sessions.get(session_id).is_none_or(|state| {
                    state.phase == RuntimeSessionPhase::Idle
                        && state.pending_permission_request.is_none()
                        && state.pending_queue.is_empty()
                        && state.backend_recovery.is_none()
                })
            };
            let selection_is_unlocked = page.total_count == 0
                && session.agent_session_id.is_none()
                && runtime_is_idle
                && !matches!(session.state, SessionState::Closed | SessionState::Archived);
            if !selection_is_unlocked {
                return Err(AgentRuntimeError::BackendSelectionLocked);
            }
        }
        self.ctx
            .session_store
            .update_backend_selection_from_user(
                &self.ctx.data_dir,
                session_id,
                entry.backend.clone(),
                Some(entry.model_id.clone()),
            )
            .map_err(AgentRuntimeError::Other)?;
        if backend_changes {
            self.close_session_runtime_locked(session_id).await;
        }
        if let Ok(available_models) = self.ctx.registry.available_models(&entry.backend) {
            self.ctx
                .notifier
                .models_updated(session_id, available_models, entry.model_id.clone());
        }
        Ok(())
    }

    #[cfg(test)]
    pub async fn set_session_backend(
        &self,
        session_id: &str,
        backend_id: &str,
    ) -> Result<GetSessionResponse, AgentRuntimeError> {
        let selected_model = self
            .ctx
            .registry
            .default_model_for(backend_id)
            .map_err(AgentRuntimeError::Other)?;
        self.run_session_close(session_id, || async {
            ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
            self.ctx
                .session_store
                .update_backend_selection(
                    &self.ctx.data_dir,
                    session_id,
                    backend_id.to_string(),
                    Some(selected_model),
                )
                .map_err(AgentRuntimeError::Other)
        })
        .await?;
        self.get_session(session_id)
            .await?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))
    }

    /// Waits for an in-flight backend recovery and then closes the live runtime.
    ///
    /// This is the normal teardown entry point. It may also reconcile the durable
    /// event log before returning session control, including persisting an
    /// interrupted recovery failure and publishing its user-facing error part.
    pub async fn close_session(&self, session_id: &str) -> Result<(), AgentRuntimeError> {
        self.run_session_close(session_id, || async { Ok(()) })
            .await?;
        Ok(())
    }

    async fn run_session_close<T, F, Fut>(
        &self,
        session_id: &str,
        after_finish: F,
    ) -> Result<T, AgentRuntimeError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, AgentRuntimeError>>,
    {
        let session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        let should_drain = self.begin_session_close_locked(session_id).await?;
        drop(session_guard);
        if should_drain {
            self.drain_closing_turn(session_id).await;
        }
        let session_guard = acquire_session_runtime_lock(&self.ctx.session_locks, session_id).await;
        let workflow_notification = self.finalize_session_close_locked(session_id).await?;
        let output = match after_finish().await {
            Ok(output) => output,
            Err(error) => {
                if let Some(state) = self.ctx.sessions.lock().await.get_mut(session_id) {
                    state.closing = false;
                }
                drop(session_guard);
                if let Some(workflow_notification) = workflow_notification {
                    dispatch_workflow_turn_complete_notification(
                        &self.ctx.workflow_turn_complete_notifier,
                        workflow_notification,
                    )
                    .await;
                }
                return Err(error);
            }
        };
        self.close_session_runtime_locked(session_id).await;
        drop(session_guard);
        if let Some(workflow_notification) = workflow_notification {
            dispatch_workflow_turn_complete_notification(
                &self.ctx.workflow_turn_complete_notifier,
                workflow_notification,
            )
            .await;
        }
        Ok(output)
    }

    #[cfg(test)]
    pub async fn close_all(&self) -> Result<(), AgentRuntimeError> {
        self.ctx.shutdown_admission.begin_shutdown();
        self.ctx.shutdown_admission.wait_for_idle().await;
        let session_ids = {
            let sessions = self.ctx.sessions.lock().await;
            sessions.keys().cloned().collect::<Vec<_>>()
        };
        let results = futures_util::future::join_all(
            session_ids
                .iter()
                .map(|session_id| self.close_session(session_id)),
        )
        .await;
        let errors = session_ids
            .iter()
            .zip(results)
            .filter_map(|(session_id, result)| {
                result.err().map(|error| format!("{session_id}: {error}"))
            })
            .collect::<Vec<_>>();
        if errors.is_empty() {
            Ok(())
        } else {
            {
                let mut sessions = self.ctx.sessions.lock().await;
                for state in sessions.values_mut() {
                    state.closing = false;
                }
            }
            self.ctx.shutdown_admission.cancel_shutdown();
            Err(AgentRuntimeError::Other(format!(
                "Failed to close agent sessions: {}",
                errors.join("; ")
            )))
        }
    }

    pub(crate) fn application_shutdown_target_session_ids(
        &self,
    ) -> Result<Vec<String>, AgentRuntimeError> {
        self.ctx
            .session_store
            .application_shutdown_target_session_ids(&self.ctx.data_dir)
            .map_err(AgentRuntimeError::Other)
    }

    async fn begin_session_close_locked(
        &self,
        session_id: &str,
    ) -> Result<bool, AgentRuntimeError> {
        let should_finalize = {
            let mut sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return Ok(false);
            };
            state.closing = true;
            state.phase != RuntimeSessionPhase::Idle
        };
        if should_finalize {
            flush_streaming_update(&self.ctx, session_id, true)
                .await
                .map_err(AgentRuntimeError::Other)?;
        }
        Ok(should_finalize)
    }

    async fn finalize_session_close_locked(
        &self,
        session_id: &str,
    ) -> Result<Option<WorkflowTurnCompleteNotification>, AgentRuntimeError> {
        let should_finalize = {
            let sessions = self.ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .is_some_and(|state| state.phase != RuntimeSessionPhase::Idle)
        };
        let workflow_notification = if should_finalize {
            complete_turn(
                &self.ctx,
                session_id,
                None,
                TurnResult::Interrupted {
                    reason: DomainInterruptReason::SessionClosed,
                    error: None,
                },
            )
            .await
            .map_err(AgentRuntimeError::Other)?
        } else {
            None
        };
        Ok(workflow_notification)
    }

    async fn close_session_runtime_locked(&self, session_id: &str) {
        let runtime = {
            let sessions = self.ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .and_then(|state| state.runtime.clone())
        };
        if let Some(runtime) = runtime {
            runtime.close().await;
        }
        self.ctx.sessions.lock().await.remove(session_id);
    }

    /// Closes the live runtime immediately without waiting for backend recovery.
    ///
    /// This is reserved for lifecycle teardown paths where waiting for a provider
    /// establishment event could deadlock shutdown or node cleanup.
    pub(crate) async fn force_close_session(
        &self,
        session_id: &str,
    ) -> Result<(), AgentRuntimeError> {
        self.close_session_runtime_locked(session_id).await;
        Ok(())
    }

    async fn drain_closing_turn(&self, session_id: &str) {
        let deadline = tokio::time::Instant::now() + CLOSE_DRAIN_TIMEOUT;
        loop {
            let still_active = {
                let sessions = self.ctx.sessions.lock().await;
                sessions
                    .get(session_id)
                    .is_some_and(|state| state.phase != RuntimeSessionPhase::Idle)
            };
            if !still_active {
                return;
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                return;
            }
            tokio::time::sleep(CLOSE_DRAIN_POLL_INTERVAL.min(deadline - now)).await;
        }
    }

    pub async fn find_permission_request(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Result<Option<PermissionRequestMsg>, AgentRuntimeError> {
        {
            let sessions = self.ctx.sessions.lock().await;
            if let Some(state) = sessions.get(session_id) {
                if let Some(request) = state
                    .pending_permission_request
                    .as_ref()
                    .filter(|request| request.id == request_id)
                    .cloned()
                {
                    return Ok(Some(request));
                }
                if let Some(request) =
                    permission_request_from_parts(&state.streaming_parts, request_id)
                {
                    return Ok(Some(request));
                }
            }
        }

        let mut cursor = None;
        while let Some(page) = self
            .ctx
            .session_store
            .get_session_page(
                &self.ctx.data_dir,
                session_id,
                cursor.clone(),
                INITIAL_SESSION_PAGE_LIMIT,
            )
            .map_err(AgentRuntimeError::Other)?
        {
            if let Some(request) = page
                .messages
                .iter()
                .rev()
                .filter_map(|message| message.parts.as_deref())
                .find_map(|parts| permission_request_from_parts(parts, request_id))
            {
                return Ok(Some(request));
            }
            if !page.has_more {
                break;
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(self
            .unresolved_permission_request_from_event_log(session_id)
            .filter(|pending| pending.request.id == request_id)
            .map(|pending| pending.request))
    }

    pub async fn cancel_queued_turn(
        &self,
        _session_id: &str,
        _queued_turn_id: Option<&str>,
    ) -> Result<CancelQueuedTurnResponse, AgentRuntimeError> {
        Err(AgentRuntimeError::Other(
            "Queued turn cancellation is unavailable until it has an atomic durable queue operation"
                .to_string(),
        ))
    }

    pub async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<GetSessionResponse>, AgentRuntimeError> {
        self.get_session_with_message_limit(session_id, INITIAL_SESSION_PAGE_LIMIT, false)
            .await
    }

    pub async fn get_display_session_window(
        &self,
        session_id: &str,
        visible_message_count: Option<usize>,
    ) -> Result<Option<GetSessionResponse>, AgentRuntimeError> {
        let message_limit = visible_message_count
            .unwrap_or(INITIAL_SESSION_PAGE_LIMIT)
            .clamp(INITIAL_SESSION_PAGE_LIMIT, RETAINED_MESSAGE_CAP);
        self.get_session_with_message_limit(session_id, message_limit, true)
            .await
    }

    async fn get_session_with_message_limit(
        &self,
        session_id: &str,
        message_limit: usize,
        overlay_live_streaming: bool,
    ) -> Result<Option<GetSessionResponse>, AgentRuntimeError> {
        let _session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        let Some((mut session, page, last_turn_interruption)) = self
            .ctx
            .session_store
            .get_session_with_latest_page(&self.ctx.data_dir, session_id, message_limit)
            .map_err(AgentRuntimeError::Other)?
        else {
            return Ok(None);
        };
        let backend_id = required_backend_id(&session)?;
        let durable_queue_paused_at = self
            .ctx
            .session_store
            .load_queue_paused_at(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?;
        let (
            mut turn_phase,
            pending_queue,
            queue_paused,
            latest_token_usage,
            pending_permission_request,
            pending_permission_state_revision,
            active_turn_id,
            streaming_message,
        ) = {
            let mut sessions = self.ctx.sessions.lock().await;
            let state = sessions.entry(session_id.to_string()).or_insert_with(|| {
                RuntimeSessionState::with_queue_pause(backend_id, durable_queue_paused_at)
            });
            (
                TurnPhase::from(state.phase),
                pending_queue_view(state),
                state.queue_paused,
                state.latest_token_usage,
                (state.runtime.is_some() && state.phase == RuntimeSessionPhase::WaitingPermission)
                    .then(|| state.pending_permission_request.clone())
                    .flatten(),
                state.pending_permission_state_revision,
                (state.phase != RuntimeSessionPhase::Idle)
                    .then_some(state.current_turn_id)
                    .flatten(),
                overlay_live_streaming
                    .then(|| {
                        state.streaming_message_id.as_ref().map(|message_id| {
                            (
                                message_id.clone(),
                                state.streaming_parts.clone(),
                                state.streaming_delta_seq,
                            )
                        })
                    })
                    .flatten(),
            )
        };
        if let Some((message_id, parts, streaming_final_seq)) = streaming_message {
            if let Some(message) = session
                .messages
                .iter_mut()
                .find(|message| message.id == message_id)
            {
                message.parts = Some(parts);
                message.streaming_final_seq = message.streaming_final_seq.max(streaming_final_seq);
            }
        }
        if pending_permission_request.is_some() {
            turn_phase = TurnPhase::WaitingPermission;
        }
        let available_models = self.available_models_for_session(&session)?;
        let total_count = page.total_count;
        let can_change_backend = session.messages.is_empty()
            && session.agent_session_id.is_none()
            && turn_phase == TurnPhase::Idle;
        let session_meta = self
            .ctx
            .session_store
            .get_session_meta(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
        let response = GetSessionResponse {
            session,
            session_revision: session_meta.state_revision,
            active_turn_id,
            turn_phase,
            available_models,
            can_change_backend,
            pending_queue_count: pending_queue.len(),
            pending_queue,
            queue_paused,
            pending_permission_request,
            pending_permission_state_revision,
            initial_page: Some(InitialSessionPage {
                next_cursor: page.next_cursor,
                has_more: page.has_more,
                total_count,
            }),
            latest_token_usage: latest_token_usage.or(page.latest_token_usage),
            last_turn_interruption,
        };
        // This method still owns the per-session runtime lock acquired above. Publish the
        // bounded window before releasing it so every later runtime/state event is ordered
        // after this snapshot instead of being overwritten by a delayed command response.
        if overlay_live_streaming && !self.ctx.notifier.display_window_updated(&response) {
            return Err(AgentRuntimeError::Other(
                "failed to publish agent session display window".to_string(),
            ));
        }
        Ok(Some(response))
    }

    fn unresolved_permission_request_from_event_log(
        &self,
        session_id: &str,
    ) -> Option<UnresolvedPermissionRequest> {
        let events = match self
            .ctx
            .session_store
            .load_session_events(&self.ctx.data_dir, session_id)
        {
            Ok(events) => events,
            Err(error) => {
                log::warn!(
                    "failed to load session events for unresolved permission lookup {session_id}: {error}"
                );
                return None;
            }
        };
        latest_unresolved_permission_request(&events)
    }

    pub async fn init_sessions(
        &self,
        worktree_path: &str,
        open_tabs: &OpenTabRegistry,
    ) -> Result<InitSessionsResponse, AgentRuntimeError> {
        let sessions = self.list_sessions(worktree_path).await?;
        for session in &sessions {
            if session.is_workflow_node_session() && session.state != SessionState::Closed {
                open_tabs.add(&session.id);
            }
        }
        let active_candidate = sessions
            .iter()
            .find(|session| !session.is_workflow_node_session())
            .map(|session| session.id.clone());
        let active_mode = active_candidate.as_deref().and_then(|session_id| {
            sessions
                .iter()
                .find(|session| session.id == session_id)
                .map(|session| (session.permission_mode.clone(), session.plan_mode))
        });
        let active_session = match active_candidate.as_deref() {
            Some(session_id) => self.get_session(session_id).await?,
            None => None,
        };
        let (permission_mode, plan_mode) = active_mode
            .or_else(|| {
                active_session.as_ref().map(|session| {
                    (
                        session.session.permission_mode.clone(),
                        session.session.plan_mode,
                    )
                })
            })
            .unwrap_or_else(|| (PermissionMode::Edit.as_str().to_string(), false));
        Ok(InitSessionsResponse {
            sessions,
            active_session,
            permission_mode,
            plan_mode,
        })
    }

    pub async fn has_live_runtime(&self, session_id: &str) -> bool {
        self.live_runtime(session_id).await.is_some()
    }

    async fn ensure_session_not_closing(&self, session_id: &str) -> Result<(), AgentRuntimeError> {
        let sessions = self.ctx.sessions.lock().await;
        if sessions.get(session_id).is_some_and(|state| state.closing) {
            return Err(AgentRuntimeError::Other(format!(
                "Agent session is closing: {session_id}"
            )));
        }
        Ok(())
    }

    /// Acquires the per-session runtime lock.
    ///
    /// While the returned guard is held, callers must not acquire another session runtime lock,
    /// including the same session recursively. Backend I/O awaits such as process startup and
    /// stdin writes must be limited to the smallest range required for per-session ordering.
    /// UI and event notifications, including session state-change emits, must run after the guard
    /// is dropped.
    pub async fn acquire_session_lock(&self, session_id: &str) -> SessionCommandLockGuard {
        self.ctx.session_locks.acquire(session_id).await
    }

    /// Waits for backend recovery and acquires exclusive session control.
    ///
    /// Besides waiting, this projects the complete durable event log. If recovery
    /// was interrupted, it persists a Failed marker, moves the session to Error,
    /// and publishes the user-facing recovery Error part before returning.
    pub async fn acquire_session_control_after_recovery(
        &self,
        session_id: &str,
    ) -> SessionRuntimeLockGuard {
        acquire_session_control_after_recovery(&self.ctx, session_id).await
    }

    pub async fn list_sessions(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, AgentRuntimeError> {
        self.ctx
            .session_store
            .list_published_sessions(&self.ctx.data_dir, worktree_path)
            .map_err(AgentRuntimeError::Other)
    }

    #[cfg(test)]
    pub(crate) fn session_runtime_lock_is_held_for_test(&self, session_id: &str) -> bool {
        self.ctx.session_locks.is_held_for_test(session_id)
    }

    pub(crate) async fn start_workflow_turn_locked(
        &self,
        request: DurableWorkflowTurnRequest,
    ) -> Result<(), AgentRuntimeError> {
        let driver = self
            .ctx
            .durable_workflow_send_driver
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(driver) = driver {
            return driver.send(request).await.map_err(AgentRuntimeError::Other);
        }

        #[cfg(test)]
        return self
            .start_turn_locked(
                &request.session_id,
                request.permission_mode,
                request.content,
                request.base_system_prompt,
                request.workflow_instructions,
            )
            .await;

        #[cfg(not(test))]
        Err(AgentRuntimeError::Other(
            "The durable workflow Send authority is unavailable.".to_string(),
        ))
    }

    #[cfg(test)]
    pub async fn start_turn_locked(
        &self,
        session_id: &str,
        permission_mode: PermissionMode,
        prompt: String,
        base_system_prompt: Option<String>,
        workflow_instructions: Vec<String>,
    ) -> Result<(), AgentRuntimeError> {
        let _admission_guard = self.ctx.shutdown_admission.admit()?;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        self.ensure_session_not_closing(session_id).await?;
        let mut session = self
            .ctx
            .session_store
            .get_session_shell(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
        let queue_transition_guard = self.ctx.transitions.acquire(session_id).await;
        self.hydrate_runtime_session_state(&session).await?;
        let queue_paused = {
            let sessions = self.ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .is_some_and(|state| state.queue_paused)
        };
        if queue_paused {
            return Err(AgentRuntimeError::Other(format!(
                "Agent queue is paused for session {session_id}; resume it before starting a workflow turn"
            )));
        }
        if session.permission_mode != permission_mode.as_str() {
            self.ctx
                .session_store
                .update_permission_mode(&self.ctx.data_dir, session_id, permission_mode.as_str())
                .map_err(AgentRuntimeError::Other)?;
            session.permission_mode = permission_mode.as_str().to_string();
        }

        let human_message = add_message_internal(
            &self.ctx.session_store,
            &self.ctx.data_dir,
            session_id,
            MessageRole::Human,
            &prompt,
            None,
            None,
        )
        .map_err(AgentRuntimeError::Other)?;
        let agent_message = add_message_internal(
            &self.ctx.session_store,
            &self.ctx.data_dir,
            session_id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .map_err(AgentRuntimeError::Other)?;
        self.ctx
            .notifier
            .turn_prepared(&session, &human_message, &agent_message);

        let system_prompt = self.build_turn_system_prompt(
            &session,
            base_system_prompt,
            &[],
            None,
            workflow_instructions,
        )?;
        self.start_turn_for_session(
            &session,
            &human_message,
            agent_message.id,
            TurnStartPayload {
                prompt,
                images: Vec::new(),
                mentions: Vec::new(),
                permission_mode,
                plan_mode: session.plan_mode,
                permission_profile_id: session.permission_profile_id.clone(),
                editor_context: None,
                system_prompt,
                accepted_execution_identity: None,
            },
            Some(queue_transition_guard),
            None,
        )
        .await
    }

    pub async fn turn_phase(&self, session_id: &str) -> Option<TurnPhase> {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .map(|state| TurnPhase::from(state.phase))
    }

    pub async fn streaming_parts(&self, session_id: &str) -> Vec<MessagePart> {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .map(|state| state.streaming_parts.clone())
            .unwrap_or_default()
    }

    pub async fn build_agent_task_list_report(
        &self,
        session_id: &str,
    ) -> Result<crate::usecase::agent_session::session::AgentTaskListReport, AgentRuntimeError>
    {
        let mut parts = self
            .ctx
            .session_store
            .load_full_session_for_restore(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .map(|session| {
                session
                    .messages
                    .into_iter()
                    .filter_map(|message| message.parts)
                    .flatten()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        parts.extend(self.streaming_parts(session_id).await);
        Ok(crate::usecase::agent_session::session::build_agent_task_list_report_from_parts(&parts))
    }

    #[cfg(test)]
    pub(crate) async fn insert_runtime_state_for_test(
        &self,
        session_id: &str,
        phase: TurnPhase,
        queued: bool,
    ) {
        let mut sessions = self.ctx.sessions.lock().await;
        let state = sessions
            .entry(session_id.to_string())
            .or_insert_with(|| RuntimeSessionState::new("claude".to_string()));
        state.runtime = Some(Arc::new(TestNoopAgentRuntime));
        state.phase = match phase {
            TurnPhase::Idle => RuntimeSessionPhase::Idle,
            TurnPhase::Streaming => RuntimeSessionPhase::Streaming,
            TurnPhase::WaitingPermission => RuntimeSessionPhase::WaitingPermission,
        };
        if queued {
            state.pending_queue.push_back(QueuedTurnInput::new(
                "queued".to_string(),
                PermissionMode::Edit,
                false,
                None,
                Vec::new(),
                "/repo".to_string(),
                Vec::new(),
                None,
            ));
        }
    }

    #[cfg(test)]
    pub(crate) async fn insert_failing_runtime_state_for_test(&self, session_id: &str) {
        let mut sessions = self.ctx.sessions.lock().await;
        let state = sessions
            .entry(session_id.to_string())
            .or_insert_with(|| RuntimeSessionState::new("claude".to_string()));
        state.runtime = Some(Arc::new(TestFailingAgentRuntime));
        state.phase = RuntimeSessionPhase::Idle;
    }

    pub(crate) async fn drain_accepted_queue_if_idle(
        &self,
        session_id: &str,
    ) -> Result<AcceptedQueueDrainOutcome, AgentRuntimeError> {
        let _session_guard = self.ctx.session_locks.acquire(session_id).await;
        let (front_requires_durable_idle, runtime_ready, accepted_identity, queue_item_id) = {
            let sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get(session_id) else {
                return Ok(AcceptedQueueDrainOutcome::NoWork);
            };
            let Some(front) = state.pending_queue.front() else {
                return Ok(AcceptedQueueDrainOutcome::NoWork);
            };
            if state.closing {
                return Ok(AcceptedQueueDrainOutcome::NoWork);
            }
            (
                queued_turn_has_accepted_identity(front),
                state.phase == RuntimeSessionPhase::Idle
                    && !state.queue_paused
                    && state.backend_recovery.is_none(),
                front
                    .accepted_operation_id
                    .as_ref()
                    .zip(front.execution_obligation_id.as_ref())
                    .map(|(operation_id, obligation_id)| {
                        (operation_id.clone(), obligation_id.clone())
                    }),
                front.id.clone(),
            )
        };
        if !runtime_ready {
            let Some((operation_id, obligation_id)) = accepted_identity else {
                return Ok(AcceptedQueueDrainOutcome::Blocked);
            };
            return Ok(
                match self
                    .accepted_queue_redrive_readiness(session_id, &operation_id, &obligation_id)
                    .await
                {
                    AcceptedQueueRedriveReadiness::Blocked => AcceptedQueueDrainOutcome::Blocked,
                    // The canonical state already unblocked, but its local
                    // mirror has not caught up. Keep one bounded redriver alive
                    // across that projection-to-memory handoff.
                    AcceptedQueueRedriveReadiness::Ready => AcceptedQueueDrainOutcome::Attempted,
                    AcceptedQueueRedriveReadiness::Missing => AcceptedQueueDrainOutcome::NoWork,
                },
            );
        }
        match self
            .ctx
            .session_store
            .load_queue_paused_at(&self.ctx.data_dir, session_id)
        {
            Ok(Some(_)) => return Ok(AcceptedQueueDrainOutcome::Blocked),
            Ok(None) => {}
            Err(error) => return Err(AgentRuntimeError::Other(error)),
        }
        if let Err(failure) = self
            .ctx
            .session_store
            .ensure_no_unresolved_recovery(session_id)
            .await
        {
            if failure.kind
                == crate::domain::local_event::SessionOperationFailureKind::StorageUnavailable
            {
                return Err(AgentRuntimeError::Other(failure.to_string()));
            }
            return Ok(AcceptedQueueDrainOutcome::Blocked);
        }
        if front_requires_durable_idle {
            let readiness = self
                .ctx
                .session_store
                .accepted_queue_start_readiness(&self.ctx.data_dir, session_id)
                .map_err(AgentRuntimeError::Other)?;
            match readiness {
                Some(true) => {}
                Some(false) => return Ok(AcceptedQueueDrainOutcome::Blocked),
                None => {
                    return Err(AgentRuntimeError::Other(format!(
                        "Session not found: {session_id}"
                    )));
                }
            }
        }
        start_next_queued_turn(&self.ctx, session_id).await;
        let queue_item_remains = {
            let sessions = self.ctx.sessions.lock().await;
            sessions.get(session_id).is_some_and(|state| {
                state.phase == RuntimeSessionPhase::Idle
                    && state
                        .pending_queue
                        .front()
                        .is_some_and(|front| front.id == queue_item_id)
            })
        };
        if !queue_item_remains {
            return Ok(AcceptedQueueDrainOutcome::Attempted);
        }
        let Some((operation_id, obligation_id)) = accepted_identity else {
            return Ok(AcceptedQueueDrainOutcome::Blocked);
        };
        Ok(
            match self
                .accepted_queue_redrive_readiness(session_id, &operation_id, &obligation_id)
                .await
            {
                AcceptedQueueRedriveReadiness::Ready => AcceptedQueueDrainOutcome::Attempted,
                AcceptedQueueRedriveReadiness::Blocked => AcceptedQueueDrainOutcome::Blocked,
                AcceptedQueueRedriveReadiness::Missing => AcceptedQueueDrainOutcome::NoWork,
            },
        )
    }

    pub(crate) async fn accepted_queue_redrive_readiness(
        &self,
        session_id: &str,
        operation_id: &str,
        obligation_id: &str,
    ) -> AcceptedQueueRedriveReadiness {
        let requires_durable_idle = {
            let sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get(session_id) else {
                return AcceptedQueueRedriveReadiness::Missing;
            };
            let Some(position) = state.pending_queue.iter().position(|queued| {
                queued.accepted_operation_id.as_deref() == Some(operation_id)
                    && queued.execution_obligation_id.as_deref() == Some(obligation_id)
            }) else {
                return AcceptedQueueRedriveReadiness::Missing;
            };
            if state.closing || state.backend_recovery.is_some() || position != 0 {
                return AcceptedQueueRedriveReadiness::Blocked;
            }
            state
                .pending_queue
                .front()
                .is_some_and(queued_turn_has_accepted_identity)
        };
        if !requires_durable_idle {
            return AcceptedQueueRedriveReadiness::Ready;
        }
        match self
            .ctx
            .session_store
            .load_queue_paused_at(&self.ctx.data_dir, session_id)
        {
            Ok(Some(_)) => return AcceptedQueueRedriveReadiness::Blocked,
            Ok(None) => {}
            Err(_) => return AcceptedQueueRedriveReadiness::Ready,
        }
        match self
            .ctx
            .session_store
            .accepted_queue_start_readiness(&self.ctx.data_dir, session_id)
        {
            Ok(Some(false)) => return AcceptedQueueRedriveReadiness::Blocked,
            // A missing or temporarily unreadable projection must enter the
            // bounded redriver. It will either recover the exact local item or
            // retire/reconcile it; treating the read error as owned forever
            // would strand the accepted obligation without another signal.
            Ok(None) | Err(_) => return AcceptedQueueRedriveReadiness::Ready,
            Ok(Some(true)) => {}
        }
        if let Err(failure) = self
            .ctx
            .session_store
            .ensure_no_unresolved_recovery(session_id)
            .await
        {
            return if failure.kind
                == crate::domain::local_event::SessionOperationFailureKind::StorageUnavailable
            {
                AcceptedQueueRedriveReadiness::Ready
            } else {
                AcceptedQueueRedriveReadiness::Blocked
            };
        }
        AcceptedQueueRedriveReadiness::Ready
    }

    #[cfg(test)]
    pub(crate) async fn drain_next_queued_turn_for_test(&self, session_id: &str) {
        let _session_guard = self.ctx.session_locks.acquire(session_id).await;
        start_next_queued_turn(&self.ctx, session_id).await;
    }

    #[cfg(test)]
    pub(crate) async fn prepare_queued_runtime_reopen_for_test(&self, session_id: &str) {
        let mut sessions = self.ctx.sessions.lock().await;
        let state = sessions
            .get_mut(session_id)
            .expect("queued runtime state must exist");
        assert!(!state.pending_queue.is_empty());
        state.runtime = None;
        state.phase = RuntimeSessionPhase::Idle;
    }

    #[cfg(test)]
    pub(crate) async fn stream_emit_failure_state_for_test(
        &self,
        session_id: &str,
    ) -> Option<(u32, bool)> {
        let sessions = self.ctx.sessions.lock().await;
        sessions.get(session_id).map(|state| {
            (
                state.stream_emit_failure_count,
                state.stream_emit_suppressed,
            )
        })
    }

    pub async fn skill_catalog(
        &self,
        backend_id: Option<&str>,
        cwd: &Path,
        query: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<crate::domain::agent_session::value_objects::SkillEntry>, AgentRuntimeError>
    {
        let backend = self
            .ctx
            .registry
            .backend_for_optional_id(backend_id)
            .map_err(AgentRuntimeError::Other)?;
        backend
            .skill_catalog(cwd, query, limit)
            .await
            .map_err(AgentRuntimeError::from)
    }

    pub async fn mentionable_files(
        &self,
        backend_id: Option<&str>,
        root: &Path,
        query: &str,
        limit: usize,
    ) -> Result<Option<Vec<String>>, AgentRuntimeError> {
        let backend = self
            .ctx
            .registry
            .backend_for_optional_id(backend_id)
            .map_err(AgentRuntimeError::Other)?;
        backend
            .fuzzy_file_search(root, query, limit)
            .await
            .map_err(AgentRuntimeError::from)
    }

    #[cfg(test)]
    async fn resolve_or_create_session(
        &self,
        req: &SendAgentMessageRequest,
        reserved_session_id: Option<&str>,
    ) -> Result<ChatSession, AgentRuntimeError> {
        if let Some(session_id) = req.chat_session_id.as_deref() {
            ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
            let mut session = self
                .ctx
                .session_store
                .get_session_shell(&self.ctx.data_dir, session_id)
                .map_err(AgentRuntimeError::Other)?
                .ok_or_else(|| {
                    AgentRuntimeError::Other(format!("Session not found: {session_id}"))
                })?;
            let backend_recovery_in_progress = {
                let sessions = self.ctx.sessions.lock().await;
                sessions
                    .get(session_id)
                    .is_some_and(|state| state.backend_recovery.is_some())
            };
            if !backend_recovery_in_progress
                && session.permission_mode != req.permission_mode.as_str()
            {
                self.ctx
                    .session_store
                    .update_permission_mode(
                        &self.ctx.data_dir,
                        session_id,
                        req.permission_mode.as_str(),
                    )
                    .map_err(AgentRuntimeError::Other)?;
                session.permission_mode = req.permission_mode.as_str().to_string();
            }
            if !backend_recovery_in_progress && session.plan_mode != req.plan_mode {
                self.ctx
                    .session_store
                    .update_plan_mode(&self.ctx.data_dir, session_id, req.plan_mode)
                    .map_err(AgentRuntimeError::Other)?;
                session.plan_mode = req.plan_mode;
            }
            return Ok(session);
        }

        let resolved_model = match req.model_id.as_deref() {
            Some(model_id) => Some(
                self.ctx
                    .registry
                    .resolve_model_entry(model_id)
                    .map_err(AgentRuntimeError::Other)?,
            ),
            None => None,
        };
        let requested_backend = resolved_model
            .as_ref()
            .map(|model| model.backend.clone())
            .or(req.backend_id.clone());
        let backend_id = self
            .ctx
            .registry
            .resolve_backend_id(requested_backend)
            .map_err(AgentRuntimeError::Other)?;
        if let Some(session_id) = reserved_session_id {
            let selected_model = match resolved_model {
                Some(model) => model.model_id,
                None => self
                    .ctx
                    .registry
                    .default_model_for(&backend_id)
                    .map_err(AgentRuntimeError::Other)?,
            };
            crate::usecase::agent_session::session::create_session_with_resolved_options_and_id(
                &self.ctx.session_store,
                &self.ctx.data_dir,
                session_id.to_string(),
                &req.worktree_path,
                backend_id,
                req.permission_mode,
                selected_model,
                req.plan_mode,
            )
            .map_err(AgentRuntimeError::Other)
        } else {
            create_session_with_model_and_plan_mode(
                &self.ctx.session_store,
                &self.ctx.registry,
                &self.ctx.data_dir,
                &req.worktree_path,
                backend_id,
                req.permission_mode,
                resolved_model.map(|model| model.model_id),
                req.plan_mode,
            )
            .map_err(AgentRuntimeError::Other)
        }
    }

    async fn start_turn_for_session(
        &self,
        session: &ChatSession,
        human_message: &ChatMessage,
        agent_message_id: String,
        mut payload: TurnStartPayload,
        queue_transition_guard: Option<SessionLockGuard>,
        committed_turn_id: Option<u64>,
    ) -> Result<(), AgentRuntimeError> {
        let accepted_execution = payload.accepted_execution_identity.is_some();
        let accepted_running_identity = payload.accepted_execution_identity.clone();
        let had_runtime = self.live_runtime(&session.id).await.is_some();
        let restore_policy =
            context_restore_policy_for_turn(&self.ctx, &session.id, &agent_message_id, had_runtime)
                .map_err(|error| {
                    classify_turn_preclaim_error(
                        accepted_execution,
                        "context-restore",
                        AgentRuntimeError::Other(error),
                    )
                })?;
        let context_was_reinjected =
            matches!(&restore_policy.plan, ContextRestorePlan::Reinject { .. });
        let clear_context_carry_after_start =
            !had_runtime && matches!(&restore_policy.plan, ContextRestorePlan::NoContext);
        let recovery_restore_required = restore_policy.recovery_restore_required;
        let expected_provider_session_generation =
            restore_policy.expected_provider_session_generation;
        let restore_plan = restore_policy.plan;
        let original_prompt = payload.prompt.clone();
        payload.prompt = apply_restore_prompt_prefix(payload.prompt, &restore_plan);
        let selected_model = had_runtime
            .then(|| selected_model_for_runtime(&self.ctx, session))
            .transpose()
            .map_err(|error| {
                classify_turn_preclaim_error(
                    accepted_execution,
                    "selected-model",
                    AgentRuntimeError::from(error),
                )
            })?;
        let turn_id = match committed_turn_id {
            Some(turn_id) => turn_id,
            None => next_turn_id(&self.ctx.session_store, &self.ctx.data_dir, &session.id)
                .map_err(|error| {
                    classify_turn_preclaim_error(
                        accepted_execution,
                        "turn-identity",
                        AgentRuntimeError::Other(error),
                    )
                })?,
        };
        let backend_id = required_backend_id(session)
            .map_err(|error| classify_turn_preclaim_error(accepted_execution, "backend", error))?;
        let prompt_message = self
            .ctx
            .session_store
            .load_previous_human_message_before_agent(
                &self.ctx.data_dir,
                &session.id,
                &agent_message_id,
            )
            .map_err(|error| {
                classify_turn_preclaim_error(
                    accepted_execution,
                    "prompt-message",
                    AgentRuntimeError::Other(error),
                )
            })?
            .unwrap_or_else(|| human_message.clone());
        let queue_transition_guard = match queue_transition_guard {
            Some(guard) => guard,
            None => self.ctx.transitions.acquire(&session.id).await,
        };
        let queue_paused_at = self
            .ctx
            .session_store
            .load_queue_paused_at(&self.ctx.data_dir, &session.id)
            .map_err(|error| {
                classify_turn_preclaim_error(
                    accepted_execution,
                    "queue-pause",
                    AgentRuntimeError::Other(error),
                )
            })?;
        let queue_is_blocked = queue_paused_at.is_some()
            || self
                .ctx
                .sessions
                .lock()
                .await
                .get(&session.id)
                .is_some_and(|state| state.queue_paused);
        if queue_is_blocked && payload.accepted_execution_identity.is_some() {
            return Err(fail_accepted_effect_preflight(
                "queue-blocked-before-turn-claim",
                format!("the accepted send queue is paused for {}", session.id),
            ));
        }
        let accepted_claim = if let Some(identity) = payload.accepted_execution_identity.as_ref() {
            let driver = self
                .ctx
                .accepted_send_obligation_driver
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            match driver {
                Some(driver) => Some(
                    driver
                        .claim_immediate_turn_execution(
                            &identity.operation_id,
                            &identity.execution_obligation_id,
                        )
                        .await
                        .map_err(|()| AgentRuntimeError::AcceptedEffectAdmissionDeferred)?,
                ),
                None => {
                    #[cfg(test)]
                    {
                        Some(AcceptedSendExecutionClaim::new(|| {}))
                    }
                    #[cfg(not(test))]
                    {
                        return Err(fail_accepted_effect_preflight(
                            "turn-execution-driver",
                            "the accepted send obligation driver is unavailable",
                        ));
                    }
                }
            }
        } else {
            None
        };
        let mut current_turn_input = QueuedTurnInput::new(
            original_prompt,
            payload.permission_mode,
            payload.plan_mode,
            payload.permission_profile_id.clone(),
            payload.images.clone(),
            session.worktree_path.clone(),
            payload.mentions.clone(),
            payload.editor_context.clone(),
        );
        current_turn_input.existing_human_message_id = Some(human_message.id.clone());
        current_turn_input.existing_agent_message_id = Some(agent_message_id.clone());
        if let Some(identity) = payload.accepted_execution_identity.take() {
            current_turn_input.accepted_operation_id = Some(identity.operation_id);
            current_turn_input.execution_obligation_id = Some(identity.execution_obligation_id);
        }
        let generation = {
            let mut sessions = self.ctx.sessions.lock().await;
            let state = sessions.entry(session.id.clone()).or_insert_with(|| {
                RuntimeSessionState::with_queue_pause(backend_id.clone(), queue_paused_at)
            });
            if state.queue_paused {
                return Err(AgentRuntimeError::Other(format!(
                    "Agent queue is paused for session {}; resume it before starting a turn",
                    session.id
                )));
            }
            let generation = state.register_turn_start_intent(turn_id, agent_message_id.clone());
            state.current_turn_input = Some(current_turn_input.clone());
            generation
        };
        drop(queue_transition_guard);
        let _accepted_claim = accepted_claim;
        if committed_turn_id.is_none() {
            if let Err(error) = self
                .ctx
                .session_store
                .append_turn_started_and_project_state(
                &self.ctx.data_dir,
                &session.id,
                AgentSessionEvent::TurnStarted {
                    turn_id,
                    message_id: prompt_message.id.clone(),
                    assistant_message_id: Some(agent_message_id.clone()),
                    prompt:
                        crate::usecase::agent_session::event_log::prompt_input_from_human_message(
                            &prompt_message,
                        ),
                    at: prompt_message.timestamp,
                },
            ) {
                let rollback_guard = self.ctx.transitions.acquire(&session.id).await;
                let mut sessions = self.ctx.sessions.lock().await;
                if let Some(state) = sessions.get_mut(&session.id) {
                    if state.generation == generation
                        && state.interrupt_requested_generation != Some(generation)
                    {
                        state.rollback_started_turn();
                    }
                }
                drop(sessions);
                drop(rollback_guard);
                return Err(AgentRuntimeError::Other(error));
            }
        }
        let commit_guard = self.ctx.transitions.acquire(&session.id).await;
        let (start_committed, interrupt_was_accepted) = {
            let mut sessions = self.ctx.sessions.lock().await;
            match sessions.get_mut(&session.id) {
                Some(state)
                    if state.generation == generation
                        && state.current_turn_id == Some(turn_id)
                        && state.phase != RuntimeSessionPhase::Idle =>
                {
                    let interrupt_was_accepted =
                        state.interrupt_requested_generation == Some(generation);
                    if !interrupt_was_accepted && !state.queue_paused {
                        state.commit_turn_start(agent_message_id.clone());
                        state.current_turn_input = Some(current_turn_input);
                        (true, false)
                    } else {
                        (false, interrupt_was_accepted)
                    }
                }
                _ => (false, false),
            }
        };
        drop(commit_guard);
        if interrupt_was_accepted {
            let (notification, _) = complete_turn_with_acceptance(
                &self.ctx,
                &session.id,
                Some(generation),
                TurnResult::Interrupted {
                    reason: DomainInterruptReason::Abort,
                    error: None,
                },
            )
            .await
            .map_err(AgentRuntimeError::Other)?;
            if let Some(notification) = notification {
                dispatch_workflow_turn_complete_notification(
                    &self.ctx.workflow_turn_complete_notifier,
                    notification,
                )
                .await;
            }
            return Ok(());
        }
        if !start_committed {
            return if accepted_execution {
                Err(AgentRuntimeError::Other(format!(
                    "accepted turn lost its live start ownership for session {}",
                    session.id
                )))
            } else {
                Ok(())
            };
        }
        let runtime_result = self
            .ensure_runtime_for_turn(session, payload.system_prompt.clone(), generation)
            .await;
        let runtime_result = match runtime_result {
            Err(AgentRuntimeError::BackendSessionLost { .. }) => {
                recover_backend_session(
                    &self.ctx,
                    &session.id,
                    BackendSessionRecoveryReason::BackendSessionLost,
                )
                .await?;
                if accepted_execution {
                    let retained = {
                        let sessions = self.ctx.sessions.lock().await;
                        sessions.get(&session.id).is_some_and(|state| {
                            state.phase != RuntimeSessionPhase::Idle
                                && state.current_turn_input.as_ref().is_some_and(|input| {
                                    input.accepted_operation_id.is_some()
                                        && input.execution_obligation_id.is_some()
                                })
                        })
                    };
                    if !retained {
                        return Err(AgentRuntimeError::Other(format!(
                            "accepted backend recovery failed for session {}",
                            session.id
                        )));
                    }
                }
                return Ok(());
            }
            result => result,
        };
        let runtime = {
            let _runtime_event_guard = self.ctx.runtime_event_locks.acquire(&session.id).await;
            let runtime = match runtime_result {
                Ok(runtime) => runtime,
                Err(error) => {
                    let should_report_failure = {
                        let sessions = self.ctx.sessions.lock().await;
                        sessions.get(&session.id).is_some_and(|state| {
                            state.generation == generation
                                && state.phase != RuntimeSessionPhase::Idle
                        })
                    };
                    if !should_report_failure {
                        return if accepted_execution {
                            Err(AgentRuntimeError::Other(format!(
                                "accepted turn lost its runtime-open outcome for session {}",
                                session.id
                            )))
                        } else {
                            Ok(())
                        };
                    }
                    let message = error.to_string();
                    let (notification, interrupt_was_accepted) = complete_turn_with_acceptance(
                        &self.ctx,
                        &session.id,
                        Some(generation),
                        TurnResult::Interrupted {
                            reason: DomainInterruptReason::Crash,
                            error: Some(message),
                        },
                    )
                    .await
                    .map_err(AgentRuntimeError::Other)?;
                    if let Some(notification) = notification {
                        dispatch_workflow_turn_complete_notification(
                            &self.ctx.workflow_turn_complete_notifier,
                            notification,
                        )
                        .await;
                    }
                    return if interrupt_was_accepted {
                        Ok(())
                    } else {
                        Err(error)
                    };
                }
            };
            if !turn_owns_runtime(&self.ctx, &session.id, generation, &runtime).await {
                detach_runtime_if_current(&self.ctx, &session.id, &runtime).await;
                return if accepted_execution {
                    Err(AgentRuntimeError::Other(format!(
                        "accepted turn lost provider ownership before input for session {}",
                        session.id
                    )))
                } else {
                    Ok(())
                };
            }
            runtime
        };
        let start_result = async {
            if let Some(model) = selected_model {
                runtime.set_model(&model).await?;
            }
            runtime
                .start_turn(TurnInput {
                    prompt: payload.prompt,
                    images: payload
                        .images
                        .into_iter()
                        .map(|image| AttachmentPayload {
                            data: image.data,
                            media_type: image.media_type,
                        })
                        .collect(),
                    system_prompt: payload.system_prompt,
                    permission_mode: payload.permission_mode,
                    plan_mode: payload.plan_mode,
                    permission_profile_id: payload.permission_profile_id,
                    editor_context: payload.editor_context.map(EditorContext::from),
                })
                .await
        }
        .await;
        let _runtime_event_guard = self.ctx.runtime_event_locks.acquire(&session.id).await;
        match start_result {
            Ok(()) => {
                if !turn_owns_runtime(&self.ctx, &session.id, generation, &runtime).await {
                    return Ok(());
                }
                self.spawn_stale_watchdog(
                    session.id.clone(),
                    generation,
                    stale_timeout_for_session(session),
                );
                let runtime_epoch = {
                    let sessions = self.ctx.sessions.lock().await;
                    sessions
                        .get(&session.id)
                        .filter(|state| state.generation == generation)
                        .map(|state| state.runtime_epoch)
                };
                if let Some(identity) = accepted_running_identity {
                    mark_accepted_turn_running_or_retry(
                        &self.ctx,
                        &session.id,
                        generation,
                        identity.operation_id,
                        identity.execution_obligation_id,
                        turn_id,
                    );
                }
                drop(_runtime_event_guard);
                complete_context_restore_after_start_or_retry(
                    &self.ctx,
                    session.id.clone(),
                    runtime_epoch.unwrap_or_default(),
                    ContextRestoreCompletionRequest::after_started_turn(
                        expected_provider_session_generation,
                        turn_id,
                        context_was_reinjected,
                        clear_context_carry_after_start,
                        recovery_restore_required,
                    ),
                );
                emit_session_state_change_from_session(
                    session,
                    &self.ctx.notifier,
                    &self.ctx.status_center,
                    &self.ctx.status_notifier,
                    StateChange {
                        turn_phase: TurnPhase::Streaming,
                        queue_paused: None,
                        pending_permission_request: None,
                        pending_permission_state_revision: None,
                        exit_code: None,
                        completed_at: None,
                        interrupted: false,
                        session_state: Some(SessionState::Active),
                    },
                );
                Ok(())
            }
            Err(error) => {
                if !turn_runtime_is_current(&self.ctx, &session.id, generation, &runtime).await {
                    return if accepted_execution {
                        Err(AgentRuntimeError::Other(format!(
                            "accepted turn lost its provider failure outcome for session {}",
                            session.id
                        )))
                    } else {
                        Ok(())
                    };
                }
                let message = error.to_string();
                let (notification, interrupt_was_accepted) = complete_turn_with_acceptance(
                    &self.ctx,
                    &session.id,
                    Some(generation),
                    TurnResult::Interrupted {
                        reason: DomainInterruptReason::Crash,
                        error: Some(message),
                    },
                )
                .await
                .map_err(AgentRuntimeError::Other)?;
                if let Some(notification) = notification {
                    dispatch_workflow_turn_complete_notification(
                        &self.ctx.workflow_turn_complete_notifier,
                        notification,
                    )
                    .await;
                }
                if interrupt_was_accepted {
                    Ok(())
                } else {
                    Err(AgentRuntimeError::from(error))
                }
            }
        }
    }

    #[cfg(test)]
    async fn ensure_runtime(
        &self,
        session: &ChatSession,
        system_prompt: Option<String>,
    ) -> Result<Arc<dyn AgentSessionRuntime>, AgentRuntimeError> {
        if let Some(runtime) = self.live_runtime(&session.id).await {
            return Ok(runtime);
        }
        open_runtime_for_session(&self.ctx, session, system_prompt, None).await
    }

    async fn ensure_runtime_for_turn(
        &self,
        session: &ChatSession,
        system_prompt: Option<String>,
        generation: u64,
    ) -> Result<Arc<dyn AgentSessionRuntime>, AgentRuntimeError> {
        let runtime_open_epoch = {
            let mut sessions = self.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session.id).ok_or_else(|| {
                AgentRuntimeError::Other(format!(
                    "Runtime state disappeared before opening session {}",
                    session.id
                ))
            })?;
            if let Some(runtime) = state.runtime.clone() {
                return Ok(runtime);
            }
            if state.generation != generation || state.phase == RuntimeSessionPhase::Idle {
                return Err(AgentRuntimeError::Other(format!(
                    "Turn no longer owns runtime open for session {}",
                    session.id
                )));
            }
            state.bump_runtime_epoch()
        };
        open_runtime_for_session(&self.ctx, session, system_prompt, Some(runtime_open_epoch)).await
    }

    fn spawn_stale_watchdog(
        &self,
        session_id: String,
        generation: u64,
        timeout: std::time::Duration,
    ) {
        spawn_stale_watchdog_task(&self.ctx, session_id, generation, timeout);
    }

    async fn live_runtime(&self, session_id: &str) -> Option<Arc<dyn AgentSessionRuntime>> {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .and_then(|state| state.runtime.clone())
    }

    pub(crate) fn default_model_for_backend(&self, backend_id: &str) -> Result<String, String> {
        self.ctx.registry.default_model_for(backend_id)
    }

    #[cfg(test)]
    async fn stalled_active_turn_target(
        &self,
        session_id: &str,
    ) -> Result<Option<StalledActiveTurnTarget>, AgentRuntimeError> {
        let sessions = self.ctx.sessions.lock().await;
        let Some(state) = sessions.get(session_id) else {
            return Ok(None);
        };
        if state.phase == RuntimeSessionPhase::Idle || !state.stall_observation_active {
            return Ok(None);
        }
        let runtime = state.runtime.clone().ok_or_else(|| {
            AgentRuntimeError::Other(format!(
                "No active agent runtime for stalled session {session_id}"
            ))
        })?;
        Ok(Some(StalledActiveTurnTarget { runtime }))
    }

    #[cfg(test)]
    fn backend_supports_steering(&self, backend_id: &str) -> bool {
        self.ctx
            .registry
            .get(backend_id)
            .is_some_and(|backend| backend.capabilities().steering)
    }

    async fn hydrate_runtime_session_state(
        &self,
        session: &ChatSession,
    ) -> Result<(), AgentRuntimeError> {
        let backend_id = required_backend_id(session)?;
        let queue_paused_at = self
            .ctx
            .session_store
            .load_queue_paused_at(&self.ctx.data_dir, &session.id)
            .map_err(AgentRuntimeError::Other)?;
        let mut sessions = self.ctx.sessions.lock().await;
        sessions
            .entry(session.id.clone())
            .or_insert_with(|| RuntimeSessionState::with_queue_pause(backend_id, queue_paused_at));
        Ok(())
    }

    #[cfg(test)]
    async fn is_turn_busy(&self, session_id: &str) -> bool {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .map(|state| {
                state.phase != RuntimeSessionPhase::Idle
                    || state.queue_paused
                    || !state.pending_queue.is_empty()
            })
            .unwrap_or(false)
    }

    #[cfg(test)]
    async fn pending_queue(&self, session_id: &str) -> Vec<QueuedAgentTurn> {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .map(pending_queue_view)
            .unwrap_or_default()
    }

    #[cfg(test)]
    fn send_response(
        &self,
        session_id: &str,
        worktree_path: &str,
        human_message: ChatMessage,
        agent_message: Option<ChatMessage>,
        queued_turn: Option<QueuedAgentTurn>,
        pending_queue: Vec<QueuedAgentTurn>,
    ) -> Result<SendMessageResponse, AgentRuntimeError> {
        let projection = self.prepare_send_response_projection(session_id, worktree_path)?;
        Ok(SendMessageResponse {
            session: projection.session,
            human_message,
            agent_message,
            queued_turn,
            pending_queue_count: pending_queue.len(),
            pending_queue,
            sessions: projection.sessions,
        })
    }

    #[cfg(test)]
    fn prepare_send_response_projection(
        &self,
        session_id: &str,
        worktree_path: &str,
    ) -> Result<SendResponseProjection, AgentRuntimeError> {
        let session = self
            .ctx
            .session_store
            .get_session_shell(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
        let sessions = self
            .ctx
            .session_store
            .list_sessions(&self.ctx.data_dir, worktree_path)
            .map_err(AgentRuntimeError::Other)?;
        Ok(SendResponseProjection { session, sessions })
    }

    fn available_models_for_session(
        &self,
        session: &ChatSession,
    ) -> Result<Vec<ModelInfo>, AgentRuntimeError> {
        let backend_id = required_backend_id(session)?;
        self.ctx
            .registry
            .available_models(&backend_id)
            .map_err(AgentRuntimeError::Other)
    }

    fn build_turn_system_prompt(
        &self,
        session: &ChatSession,
        base_system_prompt: Option<String>,
        mentions: &[crate::domain::code::MentionReference],
        editor_context: Option<&AgentEditorContext>,
        workflow_instructions: Vec<String>,
    ) -> Result<Option<String>, AgentRuntimeError> {
        let backend_id = required_backend_id(session)?;
        let built = build_session_system_prompt(SessionSystemPromptBuildRequest {
            session_store: &self.ctx.session_store,
            data_dir: &self.ctx.data_dir,
            session,
            branch_diff_context: self.ctx.branch_diff_context.as_deref(),
            instruction_source: self.ctx.instruction_source.as_ref(),
            backend_id: &backend_id,
            model_id: session.selected_model.as_deref(),
            mentions,
            editor_context: editor_context.and_then(system_context_editor_input),
            workflow_instructions,
        })
        .map_err(AgentRuntimeError::Other)?;
        let prompt = compose_system_prompt(base_system_prompt, &built.system_context);
        persist_session_system_prompt_build(
            &self.ctx.session_store,
            &self.ctx.data_dir,
            &session.id,
            &built,
        )
        .map_err(AgentRuntimeError::Other)?;
        Ok(prompt)
    }
}

fn publish_session_notice(
    status_center: &Arc<AgentStatusCenter>,
    status_notifier: &Arc<dyn AgentStatusNotifier>,
    notifier: &Arc<dyn AgentSessionEventNotifier>,
    notice: SessionNotice,
) {
    status_notifier.status_changed(status_center.record_session_notice(notice.clone()));
    notifier.persist_notice(notice);
}

fn report_event_log_recovered(
    status_center: &Arc<AgentStatusCenter>,
    status_notifier: &Arc<dyn AgentStatusNotifier>,
    notifier: &Arc<dyn AgentSessionEventNotifier>,
    session_id: &str,
) {
    emit_persistence_log_record(PersistenceLogRecord::EventLogRecovered {
        session_id: session_id.to_string(),
        kind: "event_log_recovered",
    });
    publish_session_notice(
        status_center,
        status_notifier,
        notifier,
        SessionNotice {
            session_id: session_id.to_string(),
            kind: SessionNoticeKind::EventLogRecovered,
            message: "Recovered a damaged session event log. New messages can be saved again."
                .to_string(),
            created_at: crate::usecase::agent_session::session::now_timestamp(),
        },
    );
}

fn report_persist_failure(
    ctx: &RuntimeContext,
    session_id: &str,
    kind: PersistFailureKind,
    error: &str,
) {
    emit_persistence_log_record(PersistenceLogRecord::PersistFailure {
        session_id: session_id.to_string(),
        kind: kind.as_str(),
        attempts: PERSIST_MAX_ATTEMPTS,
        error: error.to_string(),
    });
    publish_session_notice(
        &ctx.status_center,
        &ctx.status_notifier,
        &ctx.notifier,
        SessionNotice {
            session_id: session_id.to_string(),
            kind: SessionNoticeKind::PersistFailure,
            message: kind.notice_message().to_string(),
            created_at: crate::usecase::agent_session::session::now_timestamp(),
        },
    );
}

fn clear_persist_failure(ctx: &RuntimeContext, session_id: &str) {
    let changes = ctx
        .status_center
        .clear_session_notice(session_id, SessionNoticeKind::PersistFailure);
    if !changes.is_empty() {
        ctx.status_notifier.status_changed(changes);
    }
}

async fn persist_with_retry<T>(
    ctx: &RuntimeContext,
    session_id: &str,
    kind: PersistFailureKind,
    mut operation: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    let mut last_error = None;
    for attempt in 1..=PERSIST_MAX_ATTEMPTS {
        match operation() {
            Ok(value) => {
                clear_persist_failure(ctx, session_id);
                return Ok(value);
            }
            Err(error) => {
                log::warn!(
                    "agent_session_persist_retry session_id={} kind={} attempt={} max_attempts={} error={}",
                    session_id,
                    kind.as_str(),
                    attempt,
                    PERSIST_MAX_ATTEMPTS,
                    error
                );
                last_error = Some(error);
                if let Some(backoff) = PERSIST_RETRY_BACKOFFS.get(attempt - 1) {
                    tokio::time::sleep(*backoff).await;
                }
            }
        }
    }
    let error = last_error.expect("persist retry must execute at least once");
    report_persist_failure(ctx, session_id, kind, &error);
    Err(error)
}

#[cfg(test)]
async fn append_session_event_and_project_state_with_retry(
    ctx: &RuntimeContext,
    session_id: &str,
    kind: PersistFailureKind,
    event: AgentSessionEvent,
) -> Result<SessionState, String> {
    let projected_state = persist_with_retry(ctx, session_id, kind, || {
        ctx.session_store
            .append_session_event_and_project(&ctx.data_dir, session_id, event.clone())
    })
    .await?;
    persist_with_retry(ctx, session_id, kind, || {
        ctx.session_store
            .set_session_state(&ctx.data_dir, session_id, projected_state.clone())
    })
    .await?;
    Ok(projected_state)
}

#[cfg(not(test))]
const PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD: std::time::Duration =
    std::time::Duration::from_secs(60);
#[cfg(test)]
const PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD: std::time::Duration =
    std::time::Duration::from_millis(50);
#[cfg(not(test))]
const PERMISSION_REQUEST_OBSERVED_TTL: std::time::Duration = std::time::Duration::from_secs(20);
#[cfg(test)]
const PERMISSION_REQUEST_OBSERVED_TTL: std::time::Duration = std::time::Duration::from_millis(100);

fn maybe_mark_permission_wait_diagnostic(
    session_id: &str,
    state: &mut RuntimeSessionState,
    now: std::time::Instant,
) -> bool {
    if state.phase != RuntimeSessionPhase::WaitingPermission
        || state.permission_wait_diagnostic_emitted
    {
        return false;
    }
    let Some(started_at) = state.permission_wait_started_at else {
        return false;
    };
    if now.duration_since(started_at) < PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD {
        return false;
    }
    if pending_permission_request_is_observed(state, now) {
        return false;
    }
    state.permission_wait_diagnostic_emitted = true;
    let request_id = state
        .pending_permission_request
        .as_ref()
        .map(|request| request.id.as_str())
        .unwrap_or("<missing>");
    log::warn!(
        "agent permission wait diagnostic: chat_session={} request_id={} elapsed_ms={} threshold_ms={} observed=false",
        session_id,
        request_id,
        now.duration_since(started_at).as_millis(),
        PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD.as_millis()
    );
    true
}

fn pending_permission_request_is_observed(
    state: &RuntimeSessionState,
    now: std::time::Instant,
) -> bool {
    let Some(pending) = state.pending_permission_request.as_ref() else {
        return false;
    };
    let Some(visibility) = state.permission_request_visibility.as_ref() else {
        return false;
    };
    if visibility.request_id != pending.id {
        return false;
    }
    now.saturating_duration_since(visibility.last_seen_at) <= PERMISSION_REQUEST_OBSERVED_TTL
}

fn spawn_stale_watchdog_task(
    ctx: &RuntimeContext,
    session_id: String,
    generation: u64,
    timeout: std::time::Duration,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        loop {
            let next = {
                let _session_guard = ctx.session_locks.acquire(&session_id).await;
                let mut sessions = ctx.sessions.lock().await;
                let Some(state) = sessions.get_mut(&session_id) else {
                    return;
                };
                maybe_mark_permission_wait_diagnostic(
                    &session_id,
                    state,
                    std::time::Instant::now(),
                );
                let effective_timeout = effective_stale_timeout(
                    timeout,
                    has_in_flight_tool_use(&state.domain_streaming_parts),
                );
                if !turn_is_stale(
                    state.phase,
                    generation,
                    state.generation,
                    state.last_progress_at,
                    effective_timeout,
                    std::time::Instant::now(),
                ) {
                    if !stale_watchdog_should_continue_waiting(
                        state.phase,
                        generation,
                        state.generation,
                    ) {
                        return;
                    }
                    if state.phase == RuntimeSessionPhase::WaitingPermission {
                        std::time::Duration::from_secs(5).min(timeout)
                    } else {
                        remaining_until_stale(
                            state.last_progress_at,
                            effective_timeout,
                            std::time::Instant::now(),
                        )
                        .unwrap_or(effective_timeout)
                        // ツール実行中に延長された timeout はツール完了（ToolResult 到着）で
                        // 基準値へ戻るため、待機は基準 timeout を上限にして再評価する。
                        .min(timeout)
                    }
                    .max(std::time::Duration::from_millis(1))
                } else {
                    std::time::Duration::ZERO
                }
            };
            if !next.is_zero() {
                tokio::time::sleep(next).await;
                continue;
            }

            let observation = {
                let _session_guard = ctx.session_locks.acquire(&session_id).await;
                let observation = {
                    let mut sessions = ctx.sessions.lock().await;
                    let Some(state) = sessions.get_mut(&session_id) else {
                        return;
                    };
                    let effective_timeout = effective_stale_timeout(
                        timeout,
                        has_in_flight_tool_use(&state.domain_streaming_parts),
                    );
                    if !turn_is_stale(
                        state.phase,
                        generation,
                        state.generation,
                        state.last_progress_at,
                        effective_timeout,
                        std::time::Instant::now(),
                    ) {
                        continue;
                    }
                    state.stall_observation_active = true;
                    if stall_cap_reached(state.stall_signal_count)
                        && (recovery_cap_reached(state.stall_recovery_attempts)
                            || state.runtime.is_none())
                    {
                        return;
                    }

                    let now = std::time::Instant::now();
                    let payload = if stall_cap_reached(state.stall_signal_count) {
                        None
                    } else {
                        state.stall_signal_count = state.stall_signal_count.saturating_add(1);
                        Some(AgentStallObservedPayload {
                            chat_session_id: session_id.clone(),
                            turn_phase: TurnPhase::from(state.phase),
                            idle_secs: state
                                .last_progress_at
                                .map(|last_progress_at| {
                                    now.duration_since(last_progress_at).as_secs()
                                })
                                .unwrap_or(0),
                            signal_count: state.stall_signal_count,
                            cap_reached: stall_cap_reached(state.stall_signal_count),
                        })
                    };
                    let runtime = if recovery_cap_reached(state.stall_recovery_attempts) {
                        None
                    } else {
                        let runtime = state.runtime.clone();
                        if runtime.is_some() {
                            state.stall_recovery_attempts =
                                state.stall_recovery_attempts.saturating_add(1);
                        }
                        runtime
                    };
                    let should_rearm = !stall_cap_reached(state.stall_signal_count)
                        || (!recovery_cap_reached(state.stall_recovery_attempts)
                            && state.runtime.is_some());
                    StallObservation {
                        payload,
                        runtime,
                        should_rearm,
                        rearm_delay: effective_timeout.min(timeout),
                    }
                };
                if let Some(payload) = observation.payload.clone() {
                    let workflow_notification = workflow_stall_observed_notification(&payload);
                    ctx.notifier.stall_observed(payload);
                    // WorkflowStallObserved dispatch intentionally completes while the
                    // per-session runtime lock is held. The event pump dispatches
                    // WorkflowStallCleared for KeepAlive/PartsMerged under the same lock, so
                    // observe and clear are serialized here; moving this await outside the lock
                    // would reintroduce the clear-overtakes-observe race fixed in 1d4105e9.
                    dispatch_workflow_stall_observed_notification(
                        &ctx.workflow_stall_notifier,
                        workflow_notification,
                    )
                    .await;
                }
                observation
            };

            if let Some(runtime) = observation.runtime {
                match runtime.reconnect().await {
                    Ok(()) => {}
                    Err(AgentBackendError::Unavailable(message)) => {
                        log::debug!(
                            "agent runtime reconnect unavailable for {session_id}: {message}"
                        );
                    }
                    Err(error) => {
                        log::warn!("agent runtime reconnect failed for {session_id}: {error}");
                    }
                }
            }
            if !observation.should_rearm {
                return;
            }
            if !observation.rearm_delay.is_zero() {
                tokio::time::sleep(observation.rearm_delay).await;
            }
        }
    }));
}

struct StallObservation {
    payload: Option<AgentStallObservedPayload>,
    runtime: Option<Arc<dyn AgentSessionRuntime>>,
    should_rearm: bool,
    rearm_delay: std::time::Duration,
}

async fn open_runtime_for_session(
    ctx: &RuntimeContext,
    session: &ChatSession,
    system_prompt: Option<String>,
    expected_runtime_epoch: Option<u64>,
) -> Result<Arc<dyn AgentSessionRuntime>, AgentRuntimeError> {
    let backend_id = required_backend_id(session)?;
    let backend = ctx.registry.get(&backend_id).ok_or_else(|| {
        AgentRuntimeError::Other(format!("Agent backend not found: {backend_id}"))
    })?;
    let model_id = match session.selected_model.as_deref() {
        Some(model) => model.to_string(),
        None => ctx
            .registry
            .default_model_for(&backend_id)
            .map_err(AgentRuntimeError::Other)?,
    };
    let base_branch = ctx.branch_diff_context.as_ref().and_then(|port| {
        match port.get_branch_diff_context(&session.worktree_path) {
            Ok(summary) => (!summary.base_branch.trim().is_empty()).then_some(summary.base_branch),
            Err(error) => {
                log::debug!(
                    "failed to resolve base branch for agent child env {}: {error}",
                    session.id
                );
                None
            }
        }
    });
    let queue_paused_at = ctx
        .session_store
        .load_queue_paused_at(&ctx.data_dir, &session.id)
        .map_err(AgentRuntimeError::Other)?;
    let extra_env = workflow_execution_env(session.workflow_node_context.as_ref());
    let mut runtime = backend
        .open_session(SessionSpec {
            session_id: session.id.clone(),
            cwd: session.worktree_path.clone(),
            permission_mode: PermissionMode::parse(&session.permission_mode)
                .map_err(|error| AgentRuntimeError::Other(error.to_string()))?,
            plan_mode: session.plan_mode,
            permission_profile_id: session.permission_profile_id.clone(),
            model: ModelId::parse(&model_id).map_err(AgentRuntimeError::Other)?,
            system_prompt,
            resume: session.agent_session_id.clone(),
            base_branch,
            startup_timeout: startup_timeout_for_session(session),
            startup_max_retries: startup_max_retries_for_session(session),
            stale_timeout: None,
            extra_env,
        })
        .await
        .map_err(AgentRuntimeError::from)?;
    let events = runtime.take_events();
    let runtime: Arc<dyn AgentSessionRuntime> = Arc::from(runtime);
    let runtime_epoch = {
        let mut sessions = ctx.sessions.lock().await;
        let state = sessions.entry(session.id.clone()).or_insert_with(|| {
            RuntimeSessionState::with_queue_pause(backend_id.clone(), queue_paused_at)
        });
        if expected_runtime_epoch.is_some_and(|epoch| {
            state.runtime_epoch != epoch
                || state.queue_paused
                || state.interrupt_requested_generation == Some(state.generation)
        }) {
            drop(sessions);
            runtime.close().await;
            return Err(AgentRuntimeError::Other(format!(
                "Runtime open was superseded for session {}",
                session.id
            )));
        }
        state.backend_id = backend_id;
        state.runtime = Some(Arc::clone(&runtime));
        expected_runtime_epoch.unwrap_or_else(|| state.bump_runtime_epoch())
    };
    spawn_event_pump_task(ctx, session.id.clone(), runtime_epoch, events);
    Ok(runtime)
}

fn workflow_execution_env(
    context: Option<&crate::usecase::agent_session::session::WorkflowNodeContextDto>,
) -> Vec<(String, String)> {
    context
        .map(|context| {
            vec![
                (
                    "RELEASH_WORKFLOW_EXECUTION_ID".to_string(),
                    context.execution_id.clone(),
                ),
                (
                    "RELEASH_NODE_EXECUTION_ID".to_string(),
                    context.node_execution_id.clone(),
                ),
            ]
        })
        .unwrap_or_default()
}

fn selected_model_for_runtime(
    ctx: &RuntimeContext,
    session: &ChatSession,
) -> Result<ModelId, AgentBackendError> {
    let model_id = match session.selected_model.as_deref() {
        Some(model_id) => model_id.to_string(),
        None => ctx
            .registry
            .default_model_for(
                &required_backend_id(session)
                    .map_err(|error| AgentBackendError::Invalid(error.to_string()))?,
            )
            .map_err(AgentBackendError::Invalid)?,
    };
    ModelId::parse(&model_id).map_err(AgentBackendError::Invalid)
}

fn spawn_event_pump_task(
    ctx: &RuntimeContext,
    session_id: String,
    runtime_epoch: u64,
    mut events: std::pin::Pin<Box<dyn futures_util::Stream<Item = AgentRuntimeEvent> + Send>>,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        while let Some(event) = events.next().await {
            let event_received_at = crate::usecase::agent_session::session::now_timestamp();
            let mut failed_attempts = 0_u64;
            loop {
                let applied = {
                    let _session_guard = ctx.session_locks.acquire(&session_id).await;
                    let _runtime_event_guard = ctx.runtime_event_locks.acquire(&session_id).await;
                    apply_runtime_event(
                        &ctx,
                        &session_id,
                        runtime_epoch,
                        event_received_at,
                        event.clone(),
                    )
                    .await
                };
                match applied {
                    Ok(actions) => {
                        run_runtime_event_post_actions(&ctx, &session_id, actions).await;
                        break;
                    }
                    Err(error) => {
                        failed_attempts = failed_attempts.saturating_add(1);
                        if failed_attempts == 1 {
                            log::error!(
                                "canonical runtime event persistence failed for {session_id}; retaining the exact event for same-identity retry: {error}"
                            );
                        } else {
                            log::debug!(
                                "canonical runtime event persistence retry {failed_attempts} remains pending for {session_id}: {error}"
                            );
                        }
                        // Release both per-session locks between attempts so Stop, close,
                        // and a winning terminal can make progress. A changed runtime epoch or
                        // durable terminal is observed at the top of `apply_runtime_event` and
                        // safely supersedes this retained event.
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                }
            }
        }
    }));
}

struct TurnContextRestorePolicy {
    plan: ContextRestorePlan,
    recovery_restore_required: bool,
    expected_provider_session_generation: u64,
}

fn context_restore_policy_for_turn(
    ctx: &RuntimeContext,
    session_id: &str,
    streaming_agent_message_id: &str,
    had_runtime: bool,
) -> Result<TurnContextRestorePolicy, String> {
    let Some(meta) = ctx
        .session_store
        .get_session_meta(&ctx.data_dir, session_id)?
    else {
        return Ok(TurnContextRestorePolicy {
            plan: ContextRestorePlan::NoContext,
            recovery_restore_required: false,
            expected_provider_session_generation: 0,
        });
    };
    let reinjection_required =
        meta.context_reinjection_generation == Some(meta.provider_session_generation);
    if had_runtime && !reinjection_required {
        return Ok(TurnContextRestorePolicy {
            plan: ContextRestorePlan::NoContext,
            recovery_restore_required: false,
            expected_provider_session_generation: meta.provider_session_generation,
        });
    }

    let mut persisted = ctx
        .session_store
        .load_full_session_for_restore(&ctx.data_dir, session_id)?;
    if reinjection_required {
        if let Some(session) = persisted.as_mut() {
            session.agent_session_id = None;
            session.context_carry = None;
        }
    }
    Ok(TurnContextRestorePolicy {
        plan: context_restore_plan_for_session_before_turn(
            persisted.as_ref(),
            streaming_agent_message_id,
        ),
        recovery_restore_required: reinjection_required,
        expected_provider_session_generation: meta.provider_session_generation,
    })
}

fn context_restore_policy_before_human_message(
    ctx: &RuntimeContext,
    session_id: &str,
    human_message_id: &str,
    had_runtime: bool,
) -> Result<TurnContextRestorePolicy, String> {
    let Some(meta) = ctx
        .session_store
        .get_session_meta(&ctx.data_dir, session_id)?
    else {
        return Ok(TurnContextRestorePolicy {
            plan: ContextRestorePlan::NoContext,
            recovery_restore_required: false,
            expected_provider_session_generation: 0,
        });
    };
    let reinjection_required =
        meta.context_reinjection_generation == Some(meta.provider_session_generation);
    if had_runtime && !reinjection_required {
        return Ok(TurnContextRestorePolicy {
            plan: ContextRestorePlan::NoContext,
            recovery_restore_required: false,
            expected_provider_session_generation: meta.provider_session_generation,
        });
    }

    let mut persisted = ctx
        .session_store
        .load_full_session_for_restore(&ctx.data_dir, session_id)?;
    if let Some(session) = persisted.as_mut() {
        let boundary = session
            .messages
            .iter()
            .position(|message| message.id == human_message_id)
            .ok_or_else(|| {
                format!(
                    "accepted queued human message is absent from restore history: {human_message_id}"
                )
            })?;
        session.messages.truncate(boundary);
        if reinjection_required {
            session.agent_session_id = None;
            session.context_carry = None;
        }
    }
    Ok(TurnContextRestorePolicy {
        plan: context_restore_plan_for_session(persisted.as_ref()),
        recovery_restore_required: reinjection_required,
        expected_provider_session_generation: meta.provider_session_generation,
    })
}

fn context_restore_plan_for_backend_recovery(
    ctx: &RuntimeContext,
    session_id: &str,
    streaming_agent_message_id: &str,
) -> Result<ContextRestorePlan, String> {
    let mut persisted = ctx
        .session_store
        .load_full_session_for_restore(&ctx.data_dir, session_id)?;
    if let Some(session) = persisted.as_mut() {
        // `begin_backend_session_recovery` deliberately clears the dead
        // provider identity and marks carry Failed. That durable marker fences
        // ordinary turns, but the already-accepted current turn must rebuild
        // the history that preceded its exact human input on the replacement
        // runtime.
        session.agent_session_id = None;
        session.context_carry = None;
    }
    Ok(context_restore_plan_for_session_before_turn(
        persisted.as_ref(),
        streaming_agent_message_id,
    ))
}

fn complete_context_restore_after_start(
    ctx: &RuntimeContext,
    session_id: &str,
    request: ContextRestoreCompletionRequest,
) -> Result<(), String> {
    if let Some(meta) = ctx
        .session_store
        .complete_context_restore_after_start_if_current(&ctx.data_dir, session_id, request)?
    {
        ctx.notifier.context_carry_updated(
            session_id,
            meta.agent_session_id,
            meta.context_carry,
            meta.updated_at,
        );
    }
    Ok(())
}

fn complete_context_restore_after_start_or_retry(
    ctx: &RuntimeContext,
    session_id: String,
    runtime_epoch: u64,
    request: ContextRestoreCompletionRequest,
) {
    if let Err(error) = complete_context_restore_after_start(ctx, &session_id, request) {
        log::warn!("context restore completion will retry for {session_id}: {error}");
        retry_context_restore_completion(ctx, session_id, runtime_epoch, request);
    }
}

fn retry_context_restore_completion(
    ctx: &RuntimeContext,
    session_id: String,
    runtime_epoch: u64,
    request: ContextRestoreCompletionRequest,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        let mut retry_delay = Duration::from_millis(25);
        loop {
            let still_current = {
                let sessions = ctx.sessions.lock().await;
                sessions
                    .get(&session_id)
                    .is_some_and(|state| state.runtime_epoch == runtime_epoch)
            };
            if !still_current {
                return;
            }
            match complete_context_restore_after_start(&ctx, &session_id, request) {
                Ok(()) => return,
                Err(error) => {
                    if matches!(
                        ctx.session_store
                            .get_session_meta(&ctx.data_dir, &session_id),
                        Ok(None)
                    ) {
                        return;
                    }
                    log::warn!(
                        "context restore completion remains pending for {session_id}: {error}"
                    );
                }
            }
            tokio::time::sleep(retry_delay).await;
            retry_delay = retry_delay.saturating_mul(2).min(Duration::from_secs(1));
        }
    }));
}

fn mark_accepted_turn_running_or_retry(
    ctx: &RuntimeContext,
    session_id: &str,
    generation: u64,
    operation_id: String,
    obligation_id: String,
    turn_id: u64,
) {
    let driver = ctx
        .accepted_send_obligation_driver
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Some(driver) = driver else {
        log::error!("accepted turn has no running-status driver [{operation_id}/{obligation_id}]");
        return;
    };
    let ctx = ctx.clone();
    let session_id = session_id.to_string();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        let mut retry_delay = Duration::from_millis(25);
        loop {
            let still_owned = {
                let sessions = ctx.sessions.lock().await;
                sessions.get(&session_id).is_some_and(|state| {
                    state.generation == generation
                        && state.current_turn_id == Some(turn_id)
                        && state.phase != RuntimeSessionPhase::Idle
                })
            };
            if !still_owned {
                return;
            }
            if driver
                .mark_turn_running(&operation_id, &obligation_id, turn_id)
                .await
                .is_ok()
            {
                return;
            }
            retry_delay = retry_delay.saturating_mul(2).min(Duration::from_secs(1));
            tokio::time::sleep(retry_delay).await;
        }
    }));
}

async fn recover_backend_session(
    ctx: &RuntimeContext,
    session_id: &str,
    reason: BackendSessionRecoveryReason,
) -> Result<(), AgentRuntimeError> {
    recover_backend_session_with_identity(ctx, session_id, reason, uuid::Uuid::new_v4().to_string())
        .await
}

async fn recover_backend_session_with_identity(
    ctx: &RuntimeContext,
    session_id: &str,
    reason: BackendSessionRecoveryReason,
    recovery_id: String,
) -> Result<(), AgentRuntimeError> {
    recover_backend_session_with_identity_lock_state(ctx, session_id, reason, recovery_id, false)
        .await
}

async fn recover_backend_session_with_identity_lock_state(
    ctx: &RuntimeContext,
    session_id: &str,
    reason: BackendSessionRecoveryReason,
    recovery_id: String,
    runtime_event_lock_held: bool,
) -> Result<(), AgentRuntimeError> {
    // Stop acceptance is the terminal owner for the active turn. Its durable
    // QueuePaused projection closes the small interval before the production
    // gate installs the matching process-generation fence; the in-memory
    // fence covers all later provider events. Reopening here would resubmit an
    // input that Stop already owns and make the old runtime's terminal stale.
    let durable_queue_paused = ctx
        .session_store
        .load_queue_paused_at(&ctx.data_dir, session_id)
        .map_err(AgentRuntimeError::Other)?
        .is_some();
    let stop_owns_current_turn = {
        let sessions = ctx.sessions.lock().await;
        sessions.get(session_id).is_some_and(|state| {
            state.phase != RuntimeSessionPhase::Idle
                && (durable_queue_paused
                    || state.interrupt_requested_generation == Some(state.generation))
        })
    };
    if stop_owns_current_turn {
        return Ok(());
    }

    let existing_recovery = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .and_then(|state| state.backend_recovery.as_ref())
            .map(|recovery| {
                (
                    recovery.recovery_id.clone(),
                    recovery.pending_failure.clone(),
                )
            })
    };
    if let Some((existing_recovery_id, pending_failure)) = existing_recovery {
        if existing_recovery_id == recovery_id {
            if let Some(error) = pending_failure {
                return if schedule_backend_session_recovery_failure(ctx, session_id, error).await? {
                    Ok(())
                } else {
                    Err(AgentRuntimeError::Other(format!(
                        "backend recovery completion is already settling for {session_id}"
                    )))
                };
            }
        }
        // A duplicate provider event must join the recovery already owning
        // the session. Only a retained terminal persistence failure above
        // needs another write attempt.
        return Ok(());
    }

    let recovery_start = ctx
        .session_store
        .begin_backend_session_recovery(&ctx.data_dir, session_id, &recovery_id, reason)
        .map_err(AgentRuntimeError::Other)?;
    let meta = match recovery_start {
        crate::usecase::agent_session::session::BackendSessionRecoveryStartOutcome::Started(
            meta,
        ) => *meta,
        crate::usecase::agent_session::session::BackendSessionRecoveryStartOutcome::SuppressedByQueuePause => {
            return Ok(());
        }
    };

    let backend_id = meta.backend_id.clone();
    let (completion, _) = tokio::sync::watch::channel(false);
    let (old_runtime, accepted_turn) = {
        let mut sessions = ctx.sessions.lock().await;
        let state = sessions
            .entry(session_id.to_string())
            .or_insert_with(|| RuntimeSessionState::new(backend_id));
        let runtime = state.runtime.take();
        let current_turn_id = state.current_turn_id;
        let current_turn = state.current_turn_input.take();
        let accepted_turn = match current_turn {
            Some(current_turn)
                if current_turn.accepted_operation_id.is_some()
                    && current_turn.execution_obligation_id.is_some()
                    && current_turn_id.is_some()
                    && current_turn.existing_agent_message_id.is_some() =>
            {
                let turn_id = current_turn_id.expect("accepted turn identity was checked");
                let agent_message_id = current_turn
                    .existing_agent_message_id
                    .clone()
                    .expect("accepted assistant identity was checked");
                // The durable TurnExecution is already claimed. Retain that
                // exact input as the current process owner and start a new
                // process generation for the replacement runtime; never route
                // it through the normal queued Pending -> EffectReserved claim
                // again.
                state.rollback_started_turn();
                let generation =
                    state.register_turn_start_intent(turn_id, agent_message_id.clone());
                state.commit_turn_start(agent_message_id);
                state.current_turn_input = Some(current_turn.clone());
                Some((current_turn, generation))
            }
            Some(mut current_turn) => {
                // Legacy turns, and any incomplete process-local accepted
                // identity, remain explicitly queued instead of disappearing
                // during recovery. The accepted queue fence will reject a
                // partial identity without provider I/O.
                current_turn.id = uuid::Uuid::new_v4().to_string();
                state.rollback_started_turn();
                state.pending_queue.push_front(current_turn);
                None
            }
            None => {
                state.rollback_started_turn();
                None
            }
        };
        state.backend_recovery = Some(BackendSessionRecoveryState {
            recovery_id: recovery_id.clone(),
            old_provider_session_generation: meta.provider_session_generation,
            reason,
            pending_failure: None,
            turn_resume: if accepted_turn.is_some() {
                BackendSessionRecoveryTurnResume::AwaitingAcceptedTurnStart
            } else {
                BackendSessionRecoveryTurnResume::NoStartedTurn
            },
            observed_backend_session_id: None,
            completion_in_flight: false,
            failure_in_flight: false,
            completion,
        });
        (runtime, accepted_turn)
    };
    if let Some(runtime) = old_runtime {
        runtime.close().await;
    }

    let session = match ctx
        .session_store
        .get_session_shell(&ctx.data_dir, session_id)
        .map_err(AgentRuntimeError::Other)
        .and_then(|session| {
            session
                .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))
        }) {
        Ok(session) => session,
        Err(error) => {
            return fail_backend_recovery_before_claimed_turn_resume(
                ctx,
                session_id,
                accepted_turn.as_ref(),
                error.to_string(),
            )
            .await;
        }
    };
    let queued = if let Some((accepted_turn, _)) = &accepted_turn {
        Some(accepted_turn.clone())
    } else {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .and_then(|state| state.pending_queue.front().cloned())
    };
    let system_prompt = match queued
        .as_ref()
        .map(|queued| {
            build_queued_system_prompt(
                &ctx.session_store,
                ctx.branch_diff_context.as_deref(),
                ctx.instruction_source.as_ref(),
                &ctx.data_dir,
                &session,
                queued,
            )
        })
        .transpose()
        .map_err(AgentRuntimeError::Other)
        .map(Option::flatten)
    {
        Ok(system_prompt) => system_prompt,
        Err(error) => {
            return fail_backend_recovery_before_claimed_turn_resume(
                ctx,
                session_id,
                accepted_turn.as_ref(),
                error.to_string(),
            )
            .await;
        }
    };

    let runtime = match open_runtime_for_session(ctx, &session, system_prompt.clone(), None).await {
        Ok(runtime) => runtime,
        Err(error) => {
            return fail_backend_recovery_before_claimed_turn_resume(
                ctx,
                session_id,
                accepted_turn.as_ref(),
                error.to_string(),
            )
            .await;
        }
    };
    if let Some((accepted_turn, generation)) = accepted_turn {
        resume_claimed_turn_during_backend_recovery(
            ctx,
            &session,
            runtime,
            accepted_turn,
            generation,
            system_prompt,
            runtime_event_lock_held,
        )
        .await?;
    }
    Ok(())
}

fn reconcile_claimed_turn_after_backend_recovery_failure(
    ctx: &RuntimeContext,
    input: &QueuedTurnInput,
) {
    let (Some(operation_id), Some(obligation_id)) = (
        input.accepted_operation_id.clone(),
        input.execution_obligation_id.clone(),
    ) else {
        return;
    };
    let driver = ctx
        .accepted_send_obligation_driver
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Some(driver) = driver else {
        log::error!(
            "accepted backend recovery lost its obligation driver [{operation_id}/{obligation_id}]"
        );
        return;
    };
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        if let Some(recovery_wake) = driver
            .reconcile_turn_execution(&operation_id, &obligation_id)
            .await
        {
            recovery_wake.publish();
        }
    }));
}

async fn fail_claimed_turn_backend_recovery(
    ctx: &RuntimeContext,
    session_id: &str,
    generation: u64,
    _input: &QueuedTurnInput,
    error: String,
) -> Result<(), AgentRuntimeError> {
    let still_current = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .is_some_and(|state| state.generation == generation)
    };
    if !still_current {
        return Ok(());
    }
    persist_backend_session_recovery_failure(ctx, session_id, error).await
}

async fn fail_backend_recovery_before_claimed_turn_resume(
    ctx: &RuntimeContext,
    session_id: &str,
    accepted_turn: Option<&(QueuedTurnInput, u64)>,
    error: String,
) -> Result<(), AgentRuntimeError> {
    if let Some((input, generation)) = accepted_turn {
        return fail_claimed_turn_backend_recovery(ctx, session_id, *generation, input, error)
            .await;
    }
    persist_backend_session_recovery_failure(ctx, session_id, error).await
}

/// Continue the exact accepted execution on a replacement provider runtime.
///
/// `TurnExecution` was already claimed before the original runtime open, so
/// this path must neither enqueue the input nor repeat the durable claim. In
/// particular it submits the first input before waiting for
/// `SessionEstablished`; Claude reports that identity only after receiving
/// the input.
async fn resume_claimed_turn_during_backend_recovery(
    ctx: &RuntimeContext,
    session: &ChatSession,
    runtime: Arc<dyn AgentSessionRuntime>,
    input: QueuedTurnInput,
    generation: u64,
    system_prompt: Option<String>,
    runtime_event_lock_held: bool,
) -> Result<(), AgentRuntimeError> {
    let agent_message_id = match input.existing_agent_message_id.as_deref() {
        Some(agent_message_id) => agent_message_id,
        None => {
            return fail_claimed_turn_backend_recovery(
                ctx,
                &session.id,
                generation,
                &input,
                "accepted backend recovery has no assistant identity".to_string(),
            )
            .await;
        }
    };
    let restore_plan =
        match context_restore_plan_for_backend_recovery(ctx, &session.id, agent_message_id) {
            Ok(plan) => plan,
            Err(error) => {
                return fail_claimed_turn_backend_recovery(
                    ctx,
                    &session.id,
                    generation,
                    &input,
                    error,
                )
                .await;
            }
        };
    let context_was_reinjected = matches!(&restore_plan, ContextRestorePlan::Reinject { .. });
    if !turn_owns_runtime(ctx, &session.id, generation, &runtime).await {
        detach_runtime_if_current(ctx, &session.id, &runtime).await;
        return Ok(());
    }
    let prompt = apply_restore_prompt_prefix(input.content.clone(), &restore_plan);
    let start_result = runtime
        .start_turn(TurnInput {
            prompt,
            images: input
                .images
                .iter()
                .cloned()
                .map(|image| AttachmentPayload {
                    data: image.data,
                    media_type: image.media_type,
                })
                .collect(),
            system_prompt,
            permission_mode: input.permission_mode,
            plan_mode: input.plan_mode,
            permission_profile_id: input.permission_profile_id.clone(),
            editor_context: input.editor_context.clone().map(EditorContext::from),
        })
        .await;
    let _runtime_event_guard = if runtime_event_lock_held {
        None
    } else {
        Some(ctx.runtime_event_locks.acquire(&session.id).await)
    };
    if let Err(error) = start_result {
        if !turn_runtime_is_current(ctx, &session.id, generation, &runtime).await {
            return Ok(());
        }
        return fail_claimed_turn_backend_recovery(
            ctx,
            &session.id,
            generation,
            &input,
            error.to_string(),
        )
        .await;
    }
    if !turn_owns_runtime(ctx, &session.id, generation, &runtime).await {
        return Ok(());
    }
    spawn_stale_watchdog_task(
        ctx,
        session.id.clone(),
        generation,
        stale_timeout_for_session(session),
    );
    let recovery_id = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(&session.id) else {
            return Ok(());
        };
        if state.generation != generation {
            return Ok(());
        }
        let Some(recovery) = state.backend_recovery.as_mut() else {
            return Ok(());
        };
        recovery.turn_resume = BackendSessionRecoveryTurnResume::AcceptedTurnStarted {
            context_was_reinjected,
        };
        recovery.recovery_id.clone()
    };
    retry_backend_session_recovery_completion(ctx, session.id.clone(), generation, recovery_id);
    let turn_id = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(&session.id)
            .filter(|state| state.generation == generation)
            .and_then(|state| state.current_turn_id)
    }
    .ok_or_else(|| {
        AgentRuntimeError::Other(format!(
            "accepted backend recovery lost its turn identity for {}",
            session.id
        ))
    })?;
    let (operation_id, obligation_id) = match (
        input.accepted_operation_id.clone(),
        input.execution_obligation_id.clone(),
    ) {
        (Some(operation_id), Some(obligation_id)) => (operation_id, obligation_id),
        _ => {
            return Err(AgentRuntimeError::Other(format!(
                "accepted backend recovery lost its operation identity for {}",
                session.id
            )));
        }
    };
    mark_accepted_turn_running_or_retry(
        ctx,
        &session.id,
        generation,
        operation_id,
        obligation_id,
        turn_id,
    );
    emit_session_state_change_from_session(
        session,
        &ctx.notifier,
        &ctx.status_center,
        &ctx.status_notifier,
        StateChange {
            turn_phase: TurnPhase::Streaming,
            queue_paused: None,
            pending_permission_request: None,
            pending_permission_state_revision: None,
            exit_code: None,
            completed_at: None,
            interrupted: false,
            session_state: Some(SessionState::Active),
        },
    );
    Ok(())
}

struct BackendSessionRecoveryCompletion {
    recovery_id: String,
    old_provider_session_generation: u64,
    reason: BackendSessionRecoveryReason,
    backend_session_id: String,
    context_was_reinjected: Option<bool>,
}

async fn claim_backend_session_recovery_completion(
    ctx: &RuntimeContext,
    session_id: &str,
    generation: u64,
    recovery_id: &str,
) -> Option<BackendSessionRecoveryCompletion> {
    let mut sessions = ctx.sessions.lock().await;
    let state = sessions
        .get_mut(session_id)
        .filter(|state| state.generation == generation)?;
    let recovery = state
        .backend_recovery
        .as_mut()
        .filter(|recovery| recovery.recovery_id == recovery_id)?;
    if recovery.pending_failure.is_some() || recovery.completion_in_flight {
        return None;
    }
    let backend_session_id = recovery.observed_backend_session_id.clone()?;
    let context_was_reinjected = match recovery.turn_resume {
        BackendSessionRecoveryTurnResume::NoStartedTurn => None,
        BackendSessionRecoveryTurnResume::AwaitingAcceptedTurnStart => return None,
        BackendSessionRecoveryTurnResume::AcceptedTurnStarted {
            context_was_reinjected,
        } => Some(context_was_reinjected),
    };
    recovery.completion_in_flight = true;
    Some(BackendSessionRecoveryCompletion {
        recovery_id: recovery.recovery_id.clone(),
        old_provider_session_generation: recovery.old_provider_session_generation,
        reason: recovery.reason,
        backend_session_id,
        context_was_reinjected,
    })
}

async fn persist_backend_session_recovery_completion(
    ctx: &RuntimeContext,
    session_id: &str,
    generation: u64,
    completion_input: &BackendSessionRecoveryCompletion,
) -> Result<bool, AgentRuntimeError> {
    let mut meta = ctx
        .session_store
        .complete_backend_session_recovery(
            &ctx.data_dir,
            session_id,
            &completion_input.recovery_id,
            completion_input.old_provider_session_generation,
            completion_input.backend_session_id.clone(),
        )
        .map_err(AgentRuntimeError::Other)?;
    if let Some(context_was_reinjected) = completion_input.context_was_reinjected {
        if let Some(updated) = ctx
            .session_store
            .complete_context_reinjection_if_required(
                &ctx.data_dir,
                session_id,
                meta.provider_session_generation,
                context_was_reinjected,
            )
            .map_err(AgentRuntimeError::Other)?
        {
            meta = updated;
        }
    }
    let completion = {
        let _runtime_event_guard = ctx.runtime_event_locks.acquire(session_id).await;
        let mut sessions = ctx.sessions.lock().await;
        let state = sessions
            .get_mut(session_id)
            .filter(|state| state.generation == generation);
        state.and_then(|state| {
            let owns_exact_recovery = state.backend_recovery.as_ref().is_some_and(|recovery| {
                recovery.recovery_id == completion_input.recovery_id
                    && recovery.completion_in_flight
                    && recovery.pending_failure.is_none()
            });
            owns_exact_recovery
                .then(|| state.backend_recovery.take())
                .flatten()
                .map(|recovery| recovery.completion)
        })
    };
    let Some(completion) = completion else {
        return Ok(false);
    };
    let _ = completion.send(true);
    let notifier = Arc::clone(&ctx.notifier);
    let notification_session_id = session_id.to_string();
    let notification_spawner = Arc::clone(&ctx.spawner);
    notification_spawner.spawn(Box::pin(async move {
        notifier.context_carry_updated(
            &notification_session_id,
            meta.agent_session_id,
            meta.context_carry,
            meta.updated_at,
        );
    }));
    log::debug!(
        "completed backend session recovery for {session_id} ({:?}, recovery_id={})",
        completion_input.reason,
        completion_input.recovery_id
    );
    reconcile_pending_recovery_message_detached(
        ctx,
        session_id.to_string(),
        "backend recovery notice",
    );
    Ok(true)
}

fn retry_backend_session_recovery_completion(
    ctx: &RuntimeContext,
    session_id: String,
    generation: u64,
    recovery_id: String,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        let Some(completion_input) =
            claim_backend_session_recovery_completion(&ctx, &session_id, generation, &recovery_id)
                .await
        else {
            return;
        };
        let mut retry_delay = Duration::from_millis(25);
        loop {
            let still_current = {
                let sessions = ctx.sessions.lock().await;
                sessions.get(&session_id).is_some_and(|state| {
                    state.generation == generation
                        && state.backend_recovery.as_ref().is_some_and(|recovery| {
                            recovery.recovery_id == recovery_id
                                && recovery.completion_in_flight
                                && recovery.pending_failure.is_none()
                        })
                })
            };
            if !still_current {
                return;
            }
            match persist_backend_session_recovery_completion(
                &ctx,
                &session_id,
                generation,
                &completion_input,
            )
            .await
            {
                Ok(true) => {
                    let _session_guard = ctx.session_locks.acquire(&session_id).await;
                    start_next_queued_turn(&ctx, &session_id).await;
                    return;
                }
                Ok(false) => return,
                Err(error) => {
                    log::warn!(
                        "backend recovery completion remains pending for {session_id}: {error}"
                    );
                }
            }
            tokio::time::sleep(retry_delay).await;
            retry_delay = retry_delay.saturating_mul(2).min(Duration::from_secs(1));
        }
    }));
}

async fn persist_backend_session_recovery_failure(
    ctx: &RuntimeContext,
    session_id: &str,
    error: String,
) -> Result<(), AgentRuntimeError> {
    if schedule_backend_session_recovery_failure(ctx, session_id, error).await? {
        Ok(())
    } else {
        Err(AgentRuntimeError::Other(format!(
            "backend recovery completion is already settling for {session_id}"
        )))
    }
}

async fn schedule_backend_session_recovery_failure(
    ctx: &RuntimeContext,
    session_id: &str,
    error: String,
) -> Result<bool, AgentRuntimeError> {
    let recovery_id = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(recovery) = sessions
            .get_mut(session_id)
            .and_then(|state| state.backend_recovery.as_mut())
        else {
            return Err(AgentRuntimeError::Other(format!(
                "cannot durably fail backend recovery without an active recovery identity for {session_id}"
            )));
        };
        if recovery.completion_in_flight {
            return Ok(false);
        }
        if recovery.failure_in_flight {
            return Ok(recovery.pending_failure.as_deref() == Some(error.as_str()));
        }
        recovery.pending_failure = Some(error.clone());
        recovery.failure_in_flight = true;
        recovery.recovery_id.clone()
    };
    retry_backend_session_recovery_failure(ctx, session_id.to_string(), recovery_id, error);
    Ok(true)
}

fn retry_backend_session_recovery_failure(
    ctx: &RuntimeContext,
    session_id: String,
    recovery_id: String,
    error: String,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        let mut retry_delay = Duration::from_millis(25);
        loop {
            let still_current = {
                let sessions = ctx.sessions.lock().await;
                sessions.get(&session_id).is_some_and(|state| {
                    state.backend_recovery.as_ref().is_some_and(|recovery| {
                        recovery.recovery_id == recovery_id
                            && recovery.failure_in_flight
                            && !recovery.completion_in_flight
                            && recovery.pending_failure.as_deref() == Some(error.as_str())
                    })
                })
            };
            if !still_current {
                return;
            }
            match ctx.session_store.fail_backend_session_recovery(
                &ctx.data_dir,
                &session_id,
                &recovery_id,
                &error,
            ) {
                Ok(_) => break,
                Err(persist_error) => {
                    log::warn!(
                        "backend recovery failure remains pending for {session_id}: {persist_error}"
                    );
                }
            }
            tokio::time::sleep(retry_delay).await;
            retry_delay = retry_delay.saturating_mul(2).min(Duration::from_secs(1));
        }

        let settled = {
            let _runtime_event_guard = ctx.runtime_event_locks.acquire(&session_id).await;
            let mut sessions = ctx.sessions.lock().await;
            sessions.get_mut(&session_id).and_then(|state| {
                let owns_exact_recovery = state.backend_recovery.as_ref().is_some_and(|recovery| {
                    recovery.recovery_id == recovery_id
                        && recovery.failure_in_flight
                        && !recovery.completion_in_flight
                        && recovery.pending_failure.as_deref() == Some(error.as_str())
                });
                if !owns_exact_recovery {
                    return None;
                }
                let accepted_turn = state.current_turn_input.clone().filter(|input| {
                    input.accepted_operation_id.is_some() && input.execution_obligation_id.is_some()
                });
                state.rollback_started_turn();
                state.bump_runtime_epoch();
                let runtime = state.runtime.take();
                let completion = state
                    .backend_recovery
                    .take()
                    .map(|recovery| recovery.completion)?;
                Some((runtime, completion, accepted_turn))
            })
        };
        let Some((runtime, completion, accepted_turn)) = settled else {
            return;
        };
        let _ = completion.send(true);
        if let Some(runtime) = runtime {
            let close_spawner = Arc::clone(&ctx.spawner);
            close_spawner.spawn(Box::pin(async move {
                runtime.close().await;
            }));
        }
        if let Some(input) = accepted_turn.as_ref() {
            reconcile_claimed_turn_after_backend_recovery_failure(&ctx, input);
        }
        emit_session_state_change(
            &ctx.session_store,
            &ctx.notifier,
            &ctx.status_center,
            &ctx.status_notifier,
            &ctx.data_dir,
            &session_id,
            StateChange {
                turn_phase: TurnPhase::Idle,
                queue_paused: None,
                pending_permission_request: None,
                pending_permission_state_revision: None,
                exit_code: Some(1),
                completed_at: Some(crate::usecase::agent_session::session::now_timestamp()),
                interrupted: true,
                session_state: Some(SessionState::Error),
            },
        );
        reconcile_pending_recovery_message_detached(&ctx, session_id, "backend recovery error");
    }));
}

async fn wait_for_backend_session_recovery(ctx: &RuntimeContext, session_id: &str) {
    let receiver = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .and_then(|state| state.backend_recovery.as_ref())
            .map(|recovery| recovery.completion.subscribe())
    };
    let Some(mut receiver) = receiver else {
        return;
    };
    while !*receiver.borrow() {
        if receiver.changed().await.is_err() {
            break;
        }
    }
}

pub(super) async fn acquire_session_control_after_recovery(
    ctx: &RuntimeContext,
    session_id: &str,
) -> SessionRuntimeLockGuard {
    loop {
        wait_for_backend_session_recovery(ctx, session_id).await;
        let guard = ctx.session_locks.acquire(session_id).await;
        let recovery_started = {
            let sessions = ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .is_some_and(|state| state.backend_recovery.is_some())
        };
        if !recovery_started {
            reconcile_incomplete_backend_recovery(ctx, session_id).await;
            return guard;
        }
        drop(guard);
    }
}

fn backend_recovery_projection(
    ctx: &RuntimeContext,
    session_id: &str,
) -> Result<Option<BackendSessionRecoveryProjection>, AgentRuntimeError> {
    let events = ctx
        .session_store
        .load_session_events(&ctx.data_dir, session_id)
        .map_err(AgentRuntimeError::Other)?;
    Ok(TurnEventLog::from_events(events).project().backend_recovery)
}

pub(super) fn ensure_backend_recovery_operation_allowed(
    ctx: &RuntimeContext,
    session_id: &str,
) -> Result<(), AgentRuntimeError> {
    let meta = ctx
        .session_store
        .get_session_meta(&ctx.data_dir, session_id)
        .map_err(AgentRuntimeError::Other)?;
    if let Some(pending) = meta
        .as_ref()
        .and_then(|meta| meta.pending_recovery_message.as_ref())
    {
        let recovery_id = match pending {
            PendingRecoveryMessage::Notice { recovery_id, .. }
            | PendingRecoveryMessage::Error { recovery_id, .. } => recovery_id,
        };
        return Err(AgentRuntimeError::Other(format!(
            "backend session recovery publication {recovery_id} is still pending"
        )));
    }
    if !meta.is_some_and(|meta| {
        meta.agent_session_id.is_none()
            && meta.context_carry == Some(ContextCarryState::Failed)
            && meta.context_reinjection_generation.is_none()
    }) {
        return Ok(());
    }
    match backend_recovery_projection(ctx, session_id)? {
        Some(BackendSessionRecoveryProjection::Recovering { recovery_id, .. }) => {
            Err(AgentRuntimeError::Other(format!(
                "backend session recovery {recovery_id} is still in progress"
            )))
        }
        Some(BackendSessionRecoveryProjection::ReconciliationRequired { recovery_id, error }) => {
            Err(AgentRuntimeError::Other(format!(
                "backend session recovery {recovery_id} requires reconciliation: {error}"
            )))
        }
        None => Ok(()),
    }
}

fn backend_recovery_may_be_incomplete(
    ctx: &RuntimeContext,
    session_id: &str,
) -> Result<bool, AgentRuntimeError> {
    Ok(ctx
        .session_store
        .get_session_meta(&ctx.data_dir, session_id)
        .map_err(AgentRuntimeError::Other)?
        .is_some_and(|meta| {
            meta.agent_session_id.is_none()
                && meta.context_carry == Some(ContextCarryState::Failed)
                && meta.context_reinjection_generation.is_none()
        }))
}

async fn reconcile_incomplete_backend_recovery(ctx: &RuntimeContext, session_id: &str) {
    if let Err(error) = reconcile_pending_recovery_message(ctx, session_id).await {
        log::warn!(
            "failed to reconcile pending backend recovery message for {session_id}: {error}"
        );
    }
    let recovery_may_be_incomplete = match backend_recovery_may_be_incomplete(ctx, session_id) {
        Ok(recovery_may_be_incomplete) => recovery_may_be_incomplete,
        Err(error) => {
            log::warn!("failed to load backend recovery metadata for {session_id}: {error}");
            return;
        }
    };
    if !recovery_may_be_incomplete {
        return;
    }
    let projection = match backend_recovery_projection(ctx, session_id) {
        Ok(projection) => projection,
        Err(error) => {
            log::warn!("failed to restore backend recovery state for {session_id}: {error}");
            return;
        }
    };
    let Some(BackendSessionRecoveryProjection::Recovering { recovery_id, .. }) = projection else {
        return;
    };
    let error = "backend session recovery was interrupted before completion";
    if let Err(persist_error) = ctx.session_store.fail_backend_session_recovery(
        &ctx.data_dir,
        session_id,
        &recovery_id,
        error,
    ) {
        log::warn!(
            "failed to persist interrupted backend recovery for {session_id}: {persist_error}"
        );
        return;
    }
    if let Err(persist_error) = reconcile_pending_recovery_message(ctx, session_id).await {
        log::warn!(
            "failed to publish interrupted backend recovery for {session_id}: {persist_error}"
        );
    }
}

struct TurnStartPayload {
    prompt: String,
    images: Vec<ImageAttachment>,
    mentions: Vec<crate::domain::code::MentionReference>,
    permission_mode: PermissionMode,
    plan_mode: bool,
    permission_profile_id: Option<String>,
    editor_context: Option<AgentEditorContext>,
    system_prompt: Option<String>,
    accepted_execution_identity: Option<AcceptedTurnExecutionIdentity>,
}

#[derive(Default)]
pub(super) struct RuntimeEventPostActions {
    workflow_notification: Option<WorkflowTurnCompleteNotification>,
    runtime_shutdowns: Vec<RuntimeShutdown>,
    drain_next_queued_turn: bool,
}

enum RuntimeShutdown {
    Close(Arc<dyn AgentSessionRuntime>),
}

impl RuntimeEventPostActions {
    fn workflow(notification: Option<WorkflowTurnCompleteNotification>) -> Self {
        Self {
            workflow_notification: notification,
            ..Self::default()
        }
    }

    fn drain(&mut self) {
        self.drain_next_queued_turn = true;
    }

    pub(super) fn close_runtime(&mut self, runtime: Option<Arc<dyn AgentSessionRuntime>>) {
        if let Some(runtime) = runtime {
            self.runtime_shutdowns.push(RuntimeShutdown::Close(runtime));
        }
    }
}

pub(super) async fn run_runtime_event_post_actions(
    ctx: &RuntimeContext,
    session_id: &str,
    actions: RuntimeEventPostActions,
) {
    if let Some(notification) = actions.workflow_notification {
        dispatch_workflow_turn_complete_notification(
            &ctx.workflow_turn_complete_notifier,
            notification,
        )
        .await;
    }
    for shutdown in actions.runtime_shutdowns {
        match shutdown {
            RuntimeShutdown::Close(runtime) => {
                runtime.close().await;
            }
        }
    }
    if actions.drain_next_queued_turn {
        let _session_guard = ctx.session_locks.acquire(session_id).await;
        start_next_queued_turn(ctx, session_id).await;
    }
}

pub(super) async fn turn_completion_post_actions(
    ctx: &RuntimeContext,
    session_id: &str,
    workflow_notification: Option<WorkflowTurnCompleteNotification>,
) -> RuntimeEventPostActions {
    let queue_paused = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .is_some_and(|state| state.queue_paused)
    };
    let mut actions = RuntimeEventPostActions::workflow(workflow_notification);
    if !queue_paused {
        actions.drain();
    }
    actions
}

#[cfg(test)]
pub(super) async fn append_session_events_blocking(
    ctx: &RuntimeContext,
    session_id: &str,
    events: Vec<AgentSessionEvent>,
) -> Result<(), String> {
    let session_store = Arc::clone(&ctx.session_store);
    let data_dir = Arc::clone(&ctx.data_dir);
    let session_id = session_id.to_string();
    tokio::task::spawn_blocking(move || {
        session_store.append_session_events(&data_dir, &session_id, &events)
    })
    .await
    .map_err(|error| format!("Failed to join session event append task: {error}"))?
}

pub(super) async fn append_user_session_events_blocking(
    ctx: &RuntimeContext,
    session_id: &str,
    events: Vec<AgentSessionEvent>,
) -> Result<(), String> {
    let session_store = Arc::clone(&ctx.session_store);
    let data_dir = Arc::clone(&ctx.data_dir);
    let session_id = session_id.to_string();
    tokio::task::spawn_blocking(move || {
        session_store.append_session_events_from_user(&data_dir, &session_id, &events)
    })
    .await
    .map_err(|error| format!("Failed to join user session event append task: {error}"))?
}

async fn dispatch_workflow_turn_complete_notification(
    workflow_turn_complete_notifier: &Arc<RwLock<Option<Arc<dyn WorkflowTurnCompleteNotifier>>>>,
    notification: WorkflowTurnCompleteNotification,
) {
    let workflow_notifier = workflow_turn_complete_notifier
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(workflow_notifier) = workflow_notifier {
        workflow_notifier.turn_completed(notification).await;
    }
}

async fn dispatch_workflow_stall_observed_notification(
    workflow_stall_notifier: &Arc<RwLock<Option<Arc<dyn WorkflowStallNotifier>>>>,
    notification: WorkflowStallObservedNotification,
) {
    let workflow_notifier = workflow_stall_notifier
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(workflow_notifier) = workflow_notifier {
        workflow_notifier.stall_observed(notification).await;
    }
}

async fn dispatch_workflow_stall_cleared_notification(
    workflow_stall_notifier: &Arc<RwLock<Option<Arc<dyn WorkflowStallNotifier>>>>,
    notification: WorkflowStallClearedNotification,
) -> Result<(), WorkflowError> {
    let workflow_notifier = workflow_stall_notifier
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(workflow_notifier) = workflow_notifier {
        workflow_notifier.stall_cleared(notification).await?;
    }
    Ok(())
}

async fn dispatch_stall_cleared_notifications(
    ctx: &RuntimeContext,
    session_id: &str,
) -> Result<(), WorkflowError> {
    dispatch_workflow_stall_cleared_notification(
        &ctx.workflow_stall_notifier,
        workflow_stall_cleared_notification(session_id),
    )
    .await?;
    let cleared_stall = {
        let mut sessions = ctx.sessions.lock().await;
        sessions
            .get_mut(session_id)
            .map(|state| state.mark_progress(std::time::Instant::now()))
            .unwrap_or(false)
    };
    if cleared_stall {
        ctx.notifier.stall_cleared(session_id);
    }
    Ok(())
}

async fn record_first_backend_event_if_needed(ctx: &RuntimeContext, session_id: &str) {
    let elapsed = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        if state.current_turn_id.is_none() || state.first_backend_event_recorded {
            return;
        }
        let Some(started_at) = state.turn_started_at else {
            return;
        };
        state.first_backend_event_recorded = true;
        Some(started_at.elapsed())
    };
    if let Some(elapsed) = elapsed {
        record_agent_turn_duration_detached(
            ctx,
            session_id.to_string(),
            crate::other::telemetry::AgentTurn::FirstBackendEvent,
            elapsed,
        );
    }
}

fn record_agent_turn_duration_detached(
    ctx: &RuntimeContext,
    session_id: String,
    metric: crate::other::telemetry::AgentTurn,
    elapsed: Duration,
) {
    let session_store = Arc::clone(&ctx.session_store);
    let data_dir = ctx.data_dir.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        let Some(dims) = session_telemetry_dimensions(&session_store, &data_dir, &session_id)
        else {
            return;
        };
        crate::other::telemetry::record_agent_turn_duration(metric, &dims, elapsed);
    }));
}

fn runtime_event_kind(event: &AgentRuntimeEvent) -> &'static str {
    match event {
        AgentRuntimeEvent::SessionEstablished { .. } => "SessionEstablished",
        AgentRuntimeEvent::BackendSessionCleared => "BackendSessionCleared",
        AgentRuntimeEvent::PartsMerged(_) => "PartsMerged",
        AgentRuntimeEvent::PermissionRequested(_) => "PermissionRequested",
        AgentRuntimeEvent::PermissionModeChanged(_) => "PermissionModeChanged",
        AgentRuntimeEvent::SlashCommandsUpdated(_) => "SlashCommandsUpdated",
        AgentRuntimeEvent::TokenUsageUpdated(_) => "TokenUsageUpdated",
        AgentRuntimeEvent::KeepAlive => "KeepAlive",
        AgentRuntimeEvent::TurnCompleted(_) => "TurnCompleted",
        AgentRuntimeEvent::Fatal { .. } => "Fatal",
    }
}

async fn reconcile_pending_recovery_message(
    ctx: &RuntimeContext,
    session_id: &str,
) -> Result<(), String> {
    let Some(meta) = ctx
        .session_store
        .get_session_meta(&ctx.data_dir, session_id)?
    else {
        return Ok(());
    };
    let Some(pending) = meta.pending_recovery_message else {
        return Ok(());
    };
    match &pending {
        PendingRecoveryMessage::Notice { message_id, .. } => {
            persist_and_publish_recovery_notice(ctx, session_id, &pending, message_id)?;
        }
        PendingRecoveryMessage::Error {
            message_id, error, ..
        } => {
            persist_and_publish_recovery_error(ctx, session_id, &pending, message_id, error)
                .await?;
        }
    }
    Ok(())
}

fn reconcile_pending_recovery_message_detached(
    ctx: &RuntimeContext,
    session_id: String,
    publication_kind: &'static str,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        if let Err(error) = reconcile_pending_recovery_message(&ctx, &session_id).await {
            log::warn!(
                "failed to persist {publication_kind} for {session_id}; it remains pending: {error}"
            );
        }
    }));
}

async fn clear_provider_session_establishment_if_current(
    ctx: &RuntimeContext,
    session_id: &str,
    runtime_epoch: u64,
    observation_id: &str,
) {
    let _runtime_event_guard = ctx.runtime_event_locks.acquire(session_id).await;
    let mut sessions = ctx.sessions.lock().await;
    if let Some(state) = sessions.get_mut(session_id) {
        let owns_exact_observation = state.runtime_epoch == runtime_epoch
            && state
                .provider_session_establishment
                .as_ref()
                .is_some_and(|establishment| establishment.observation_id == observation_id);
        if owns_exact_observation {
            state.provider_session_establishment = None;
        }
    }
}

fn retry_provider_session_establishment(
    ctx: &RuntimeContext,
    session_id: String,
    runtime_epoch: u64,
    observation_id: String,
    backend_session_id: String,
    context_carry: Option<ContextCarryState>,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        let mut retry_delay = Duration::from_millis(25);
        loop {
            let still_current = {
                let sessions = ctx.sessions.lock().await;
                sessions.get(&session_id).is_some_and(|state| {
                    state.runtime_epoch == runtime_epoch
                        && state.provider_session_establishment.as_ref().is_some_and(
                            |establishment| {
                                establishment.observation_id == observation_id
                            },
                        )
                })
            };
            if !still_current {
                return;
            }

            let expected_provider_session_generation = match ctx
                .session_store
                .get_session_meta(&ctx.data_dir, &session_id)
            {
                Ok(Some(meta)) => meta.provider_session_generation,
                Ok(None) => {
                    clear_provider_session_establishment_if_current(
                        &ctx,
                        &session_id,
                        runtime_epoch,
                        &observation_id,
                    )
                    .await;
                    return;
                }
                Err(error) => {
                    log::warn!(
                        "provider establishment generation read remains pending for {session_id}: {error}"
                    );
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = retry_delay
                        .saturating_mul(2)
                        .min(Duration::from_secs(1));
                    continue;
                }
            };

            match ctx.session_store.record_backend_session_established(
                &ctx.data_dir,
                &session_id,
                expected_provider_session_generation,
                &observation_id,
                backend_session_id.clone(),
                context_carry.clone(),
            ) {
                Ok(ProviderSessionEstablishmentOutcome::Settled(meta)) => {
                    let settled = {
                        let _runtime_event_guard =
                            ctx.runtime_event_locks.acquire(&session_id).await;
                        let mut sessions = ctx.sessions.lock().await;
                        sessions.get_mut(&session_id).is_some_and(|state| {
                            let owns_exact_observation = state.runtime_epoch == runtime_epoch
                                && state.provider_session_establishment.as_ref().is_some_and(
                                    |establishment| {
                                        establishment.observation_id == observation_id
                                    },
                                );
                            if owns_exact_observation {
                                state.provider_session_establishment = None;
                                state.provider_session_established = true;
                            }
                            owns_exact_observation
                        })
                    };
                    if !settled {
                        return;
                    }
                    let notifier = Arc::clone(&ctx.notifier);
                    let notification_session_id = session_id.clone();
                    let notification_spawner = Arc::clone(&ctx.spawner);
                    notification_spawner.spawn(Box::pin(async move {
                        notifier.context_carry_updated(
                            &notification_session_id,
                            meta.agent_session_id,
                            meta.context_carry,
                            meta.updated_at,
                        );
                    }));
                    return;
                }
                Ok(
                    ProviderSessionEstablishmentOutcome::Missing
                    | ProviderSessionEstablishmentOutcome::Fenced,
                ) => {
                    clear_provider_session_establishment_if_current(
                        &ctx,
                        &session_id,
                        runtime_epoch,
                        &observation_id,
                    )
                    .await;
                    return;
                }
                Err(error) => {
                    log::warn!(
                        "provider establishment observation remains pending for {session_id}: {error}"
                    );
                }
            }
            tokio::time::sleep(retry_delay).await;
            retry_delay = retry_delay
                .saturating_mul(2)
                .min(Duration::from_secs(1));
        }
    }));
}

fn persist_and_publish_recovery_notice(
    ctx: &RuntimeContext,
    session_id: &str,
    pending: &PendingRecoveryMessage,
    message_id: &str,
) -> Result<(), String> {
    let parts = parts_from_domain(vec![DomainMessagePart::SystemNotification {
        notification_type: DomainSystemNotificationType::SessionRecovery,
        status: "recovered".to_string(),
        label: "backend セッションを作り直したため文脈は引き継がれません".to_string(),
        detail: None,
        hook_id: None,
    }]);
    let message = ChatMessage {
        id: message_id.to_string(),
        role: MessageRole::Agent,
        content: String::new(),
        thinking: None,
        activities: None,
        parts: Some(parts),
        streaming_final_seq: 0,
        timestamp: crate::usecase::agent_session::session::now_timestamp(),
        mentions: None,
    };
    let inserted = ctx.session_store.publish_pending_recovery_message(
        &ctx.data_dir,
        session_id,
        pending,
        message.clone(),
    )?;
    if inserted {
        ctx.notifier
            .pending_message_consumed(session_id, None, None, message);
    }
    Ok(())
}

async fn persist_and_publish_recovery_error(
    ctx: &RuntimeContext,
    session_id: &str,
    pending: &PendingRecoveryMessage,
    message_id: &str,
    error: &str,
) -> Result<(), String> {
    let content = format!("backend session recovery failed: {error}");
    let persisted = ctx
        .session_store
        .canonical_message_projection(session_id, message_id)?;
    if let Some(mut message) = persisted {
        let mut parts = message.parts.clone().unwrap_or_default();
        let error_part = MessagePart::Error {
            content,
            parent_tool_use_id: None,
        };
        if !parts.contains(&error_part) {
            merge_persisted_message_part(&mut parts, error_part);
            message.streaming_final_seq = message.streaming_final_seq.saturating_add(1);
            message.timestamp = crate::usecase::agent_session::session::now_timestamp();
            message.parts = Some(parts.clone());
            ctx.session_store.publish_pending_recovery_message(
                &ctx.data_dir,
                session_id,
                pending,
                message.clone(),
            )?;
            let _ = ctx.notifier.streaming_delta(AgentStreamingDeltaPayload {
                chat_session_id: session_id.to_string(),
                message_id: message.id,
                seq: message.streaming_final_seq,
                snapshot: true,
                parts,
                message: None,
            });
        } else {
            ctx.session_store.publish_pending_recovery_message(
                &ctx.data_dir,
                session_id,
                pending,
                message,
            )?;
        }
        return Ok(());
    }

    let message = ChatMessage {
        id: message_id.to_string(),
        role: MessageRole::Agent,
        content: String::new(),
        thinking: None,
        activities: None,
        parts: Some(vec![MessagePart::Error {
            content,
            parent_tool_use_id: None,
        }]),
        streaming_final_seq: 0,
        timestamp: crate::usecase::agent_session::session::now_timestamp(),
        mentions: None,
    };
    let inserted = ctx.session_store.publish_pending_recovery_message(
        &ctx.data_dir,
        session_id,
        pending,
        message.clone(),
    )?;
    if inserted {
        ctx.notifier
            .pending_message_consumed(session_id, None, None, message);
    }
    Ok(())
}

async fn apply_runtime_event(
    ctx: &RuntimeContext,
    session_id: &str,
    runtime_epoch: u64,
    event_received_at: f64,
    event: AgentRuntimeEvent,
) -> Result<RuntimeEventPostActions, String> {
    let (
        is_current_runtime,
        terminal_committed,
        provider_establishment_in_flight,
        recovery_completion_in_flight,
        recovery_failure_in_flight,
    ) = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .map_or((false, false, false, false, false), |state| {
                (
                    state.runtime_epoch == runtime_epoch,
                    state.phase != RuntimeSessionPhase::Idle
                        && state
                            .current_turn_id
                            .or(state.last_turn_id)
                            .is_some_and(|turn_id| state.terminal_turn_id == Some(turn_id)),
                    state.provider_session_establishment.is_some(),
                    state
                        .backend_recovery
                        .as_ref()
                        .is_some_and(|recovery| recovery.completion_in_flight),
                    state
                        .backend_recovery
                        .as_ref()
                        .is_some_and(|recovery| recovery.failure_in_flight),
                )
            })
    };
    if !is_current_runtime {
        log::debug!(
            "dropping {} from stale runtime epoch {runtime_epoch} for {session_id}",
            runtime_event_kind(&event)
        );
        return Ok(RuntimeEventPostActions::default());
    }
    if terminal_committed && runtime_event_targets_current_turn(&event) {
        log::debug!(
            "dropping {} after durable terminal commit for {session_id}",
            runtime_event_kind(&event)
        );
        return Ok(RuntimeEventPostActions::default());
    }
    if recovery_failure_in_flight {
        return Err("backend recovery failure is still settling".to_string());
    }
    let event_must_follow_provider_identity = matches!(
        &event,
        AgentRuntimeEvent::TurnCompleted(_)
            | AgentRuntimeEvent::Fatal { .. }
            | AgentRuntimeEvent::BackendSessionCleared
            | AgentRuntimeEvent::SessionEstablished {
                resume: crate::domain::agent_session::gateway::ResumeOutcome::Mismatch { .. },
                ..
            }
    );
    if provider_establishment_in_flight && event_must_follow_provider_identity {
        return Err("provider establishment observation is still settling".to_string());
    }
    let event_must_follow_recovery_completion = event_must_follow_provider_identity;
    if recovery_completion_in_flight && event_must_follow_recovery_completion {
        return Err("backend recovery completion is still settling".to_string());
    }
    record_first_backend_event_if_needed(ctx, session_id).await;
    ctx.notifier.runtime_event_debug(session_id, &event);
    match event {
        AgentRuntimeEvent::SessionEstablished {
            backend_session_id,
            resume,
        } => {
            if matches!(
                resume,
                crate::domain::agent_session::gateway::ResumeOutcome::Mismatch { .. }
            ) {
                let recovery_id = runtime_event_recovery_id(
                    session_id,
                    runtime_epoch,
                    event_received_at,
                    BackendSessionRecoveryReason::ResumeMismatch,
                    &backend_session_id,
                );
                recover_backend_session_with_identity_lock_state(
                    ctx,
                    session_id,
                    BackendSessionRecoveryReason::ResumeMismatch,
                    recovery_id,
                    true,
                )
                .await
                .map_err(|error| {
                    format!(
                        "resume-mismatch recovery trigger could not be durably handled: {error}"
                    )
                })?;
                return Ok(RuntimeEventPostActions::default());
            }
            let (already_established, recovery_active) = {
                let sessions = ctx.sessions.lock().await;
                sessions
                    .get(session_id)
                    .map(|state| {
                        (
                            state.provider_session_established,
                            state.backend_recovery.is_some(),
                        )
                    })
                    .unwrap_or((false, false))
            };
            if already_established && !recovery_active {
                return Ok(RuntimeEventPostActions::default());
            }
            let recovery_identity = {
                let mut sessions = ctx.sessions.lock().await;
                let Some(state) = sessions.get_mut(session_id) else {
                    return Ok(RuntimeEventPostActions::default());
                };
                if state.runtime_epoch != runtime_epoch {
                    return Ok(RuntimeEventPostActions::default());
                }
                match state.backend_recovery.as_mut() {
                    Some(recovery) => {
                        if recovery
                            .observed_backend_session_id
                            .as_deref()
                            .is_some_and(|observed| observed != backend_session_id)
                        {
                            return Err(
                                "backend recovery observed conflicting provider identities"
                                    .to_string(),
                            );
                        }
                        recovery.observed_backend_session_id = Some(backend_session_id.clone());
                        state.provider_session_established = true;
                        Some((state.generation, recovery.recovery_id.clone()))
                    }
                    None => None,
                }
            };
            if let Some((generation, recovery_id)) = recovery_identity {
                retry_backend_session_recovery_completion(
                    ctx,
                    session_id.to_string(),
                    generation,
                    recovery_id,
                );
                return Ok(RuntimeEventPostActions::default());
            }
            let context_carry = match resume {
                crate::domain::agent_session::gateway::ResumeOutcome::Resumed => {
                    Some(ContextCarryState::Resumed)
                }
                crate::domain::agent_session::gateway::ResumeOutcome::NotRequested => None,
                crate::domain::agent_session::gateway::ResumeOutcome::Mismatch { .. } => {
                    unreachable!("resume mismatch is handled before metadata update")
                }
            };
            let observation_id = runtime_provider_session_observation_id(
                session_id,
                runtime_epoch,
                event_received_at,
                &backend_session_id,
                context_carry.as_ref(),
            );
            {
                let mut sessions = ctx.sessions.lock().await;
                let Some(state) = sessions.get_mut(session_id) else {
                    return Ok(RuntimeEventPostActions::default());
                };
                if state.runtime_epoch != runtime_epoch {
                    return Ok(RuntimeEventPostActions::default());
                }
                if state.provider_session_established {
                    return Ok(RuntimeEventPostActions::default());
                }
                if let Some(establishment) = state.provider_session_establishment.as_ref() {
                    if establishment.observation_id == observation_id {
                        return Ok(RuntimeEventPostActions::default());
                    }
                    return Err(
                        "runtime observed conflicting provider establishment identities"
                            .to_string(),
                    );
                }
                state.provider_session_establishment = Some(ProviderSessionEstablishmentState {
                    observation_id: observation_id.clone(),
                });
            }
            retry_provider_session_establishment(
                ctx,
                session_id.to_string(),
                runtime_epoch,
                observation_id,
                backend_session_id,
                context_carry,
            );
        }
        AgentRuntimeEvent::BackendSessionCleared => {
            let recovery_id = runtime_event_recovery_id(
                session_id,
                runtime_epoch,
                event_received_at,
                BackendSessionRecoveryReason::BackendSessionLost,
                "backend-session-cleared",
            );
            recover_backend_session_with_identity_lock_state(
                ctx,
                session_id,
                BackendSessionRecoveryReason::BackendSessionLost,
                recovery_id,
                true,
            )
            .await
            .map_err(|error| {
                format!("backend-session-cleared recovery trigger could not be durably handled: {error}")
            })?;
        }
        AgentRuntimeEvent::PartsMerged(parts) => {
            apply_parts(ctx, session_id, parts, StreamingApplyMode::Coalesced)
                .await
                .map_err(|error| format!("streaming parts commit failed: {error}"))?;
        }
        AgentRuntimeEvent::PermissionRequested(request) => {
            let pending = pending_permission_request_msg(&request);
            let persisted = apply_parts(
                ctx,
                session_id,
                vec![DomainMessagePart::permission(request)],
                StreamingApplyMode::Immediate,
            )
            .await
            .map_err(|error| format!("permission request commit failed: {error}"))?;
            if persisted {
                if let Some(pending) = pending {
                    let pending_permission_state_revision = {
                        let mut sessions = ctx.sessions.lock().await;
                        sessions.get_mut(session_id).map(|state| {
                            state.phase = RuntimeSessionPhase::WaitingPermission;
                            let revision = state.set_pending_permission_request(pending.clone());
                            state.permission_wait_started_at = Some(std::time::Instant::now());
                            state.permission_wait_diagnostic_emitted = false;
                            revision
                        })
                    };
                    emit_session_state_change(
                        &ctx.session_store,
                        &ctx.notifier,
                        &ctx.status_center,
                        &ctx.status_notifier,
                        &ctx.data_dir,
                        session_id,
                        StateChange {
                            turn_phase: TurnPhase::WaitingPermission,
                            queue_paused: None,
                            pending_permission_request: Some(pending),
                            pending_permission_state_revision,
                            exit_code: None,
                            completed_at: None,
                            interrupted: false,
                            session_state: Some(SessionState::Active),
                        },
                    );
                }
            }
        }
        AgentRuntimeEvent::PermissionModeChanged(mode) => {
            if let Some(saved_mode) =
                resync_permission_mode(&ctx.session_store, &ctx.data_dir, session_id, mode)
            {
                ctx.notifier
                    .permission_mode_changed(session_id, saved_mode.as_str());
            }
        }
        AgentRuntimeEvent::SlashCommandsUpdated(commands) => {
            ctx.notifier
                .supported_commands_updated(session_id, commands);
        }
        AgentRuntimeEvent::TokenUsageUpdated(usage) => {
            let usage = token_usage_from_domain(usage);
            {
                let mut sessions = ctx.sessions.lock().await;
                if let Some(state) = sessions.get_mut(session_id) {
                    state.latest_token_usage = Some(usage);
                }
            }
            ctx.notifier.token_usage_updated(session_id, usage);
        }
        AgentRuntimeEvent::KeepAlive => {
            let cleared_stall = {
                let mut sessions = ctx.sessions.lock().await;
                if let Some(state) = sessions.get_mut(session_id) {
                    if state.phase != RuntimeSessionPhase::Idle {
                        state.record_progress(std::time::Instant::now())
                    } else {
                        false
                    }
                } else {
                    false
                }
            };
            if cleared_stall {
                if let Err(error) = dispatch_stall_cleared_notifications(ctx, session_id).await {
                    log::warn!(
                        "workflow stall-cleared notification failed for {session_id}: {error}"
                    );
                }
            }
        }
        AgentRuntimeEvent::TurnCompleted(result) => {
            let trailing_fatal_message = match &result {
                TurnResult::Interrupted {
                    reason: DomainInterruptReason::Crash,
                    error: Some(message),
                } => Some(message.clone()),
                _ => None,
            };
            let wait_for_trailing_fatal = if trailing_fatal_message.is_some() {
                let sessions = ctx.sessions.lock().await;
                sessions
                    .get(session_id)
                    .is_some_and(|state| state.phase != RuntimeSessionPhase::Idle)
            } else {
                false
            };
            let workflow_notification = match complete_turn(ctx, session_id, None, result).await {
                Ok(notification) => notification,
                Err(error) => {
                    return Err(format!("terminal commit failed: {error}"));
                }
            };
            if wait_for_trailing_fatal {
                let mut sessions = ctx.sessions.lock().await;
                if let Some(state) = sessions.get_mut(session_id) {
                    state.pending_trailing_fatal_message = trailing_fatal_message;
                }
                return Ok(RuntimeEventPostActions::workflow(workflow_notification));
            }
            return Ok(turn_completion_post_actions(ctx, session_id, workflow_notification).await);
        }
        AgentRuntimeEvent::Fatal { message } => {
            log::warn!("agent runtime fatal for {session_id}: {message}");
            let recovery_in_progress = {
                let sessions = ctx.sessions.lock().await;
                sessions
                    .get(session_id)
                    .is_some_and(|state| state.backend_recovery.is_some())
            };
            if recovery_in_progress {
                let failure_owned =
                    schedule_backend_session_recovery_failure(ctx, session_id, message.clone())
                        .await
                        .map_err(|error| {
                            format!(
                            "backend recovery fatal observation could not be handed off: {error}"
                        )
                        })?;
                return if failure_owned {
                    Ok(RuntimeEventPostActions::default())
                } else {
                    Err("backend recovery completion is still settling".to_string())
                };
            }
            let (should_complete_crash, trailing_completed_crash) = {
                let mut sessions = ctx.sessions.lock().await;
                sessions
                    .get_mut(session_id)
                    .map_or((false, false), |state| {
                        if state.phase != RuntimeSessionPhase::Idle {
                            state.pending_trailing_fatal_message = None;
                            (true, false)
                        } else {
                            let trailing = state.pending_trailing_fatal_message.as_deref()
                                == Some(message.as_str());
                            state.pending_trailing_fatal_message = None;
                            (false, trailing)
                        }
                    })
            };
            let mut actions = RuntimeEventPostActions::default();
            if should_complete_crash {
                match complete_turn(
                    ctx,
                    session_id,
                    None,
                    TurnResult::Interrupted {
                        reason: DomainInterruptReason::Crash,
                        error: Some(message.clone()),
                    },
                )
                .await
                {
                    Ok(notification) => actions.workflow_notification = notification,
                    Err(error) => {
                        return Err(format!("fatal terminal commit failed: {error}"));
                    }
                }
            }
            let runtime = {
                let mut sessions = ctx.sessions.lock().await;
                sessions
                    .get_mut(session_id)
                    .and_then(|state| state.runtime.take())
            };
            actions.close_runtime(runtime);
            {
                let mut sessions = ctx.sessions.lock().await;
                if let Some(state) = sessions.get_mut(session_id) {
                    state.phase = RuntimeSessionPhase::Idle;
                    state.stall_observation_active = false;
                }
            }
            if !should_complete_crash && !trailing_completed_crash {
                let completed_at = event_received_at;
                let message_id = runtime_error_message_id(
                    session_id,
                    runtime_epoch,
                    event_received_at,
                    &message,
                );
                let projected_message = ctx
                    .session_store
                    .append_error_episode_and_pause_queue(
                        &ctx.data_dir,
                        session_id,
                        ErrorEpisodeInput {
                            message_id: message_id.clone(),
                            reason: message.clone(),
                            at: completed_at,
                        },
                    )
                    .map(|(_, projected_message)| projected_message);
                match projected_message {
                    Ok(projected_message) => {
                        let parts = projected_message.parts.clone().unwrap_or_default();
                        {
                            let mut sessions = ctx.sessions.lock().await;
                            if let Some(state) = sessions.get_mut(session_id) {
                                state.last_agent_message_id = Some(message_id.clone());
                                state.streaming_delta_seq = 1;
                                state.queue_paused = true;
                                state.queue_paused_at = Some(completed_at);
                            }
                        }
                        emit_streaming_delta_or_retry(
                            ctx,
                            session_id,
                            PendingStreamDelta {
                                message_id,
                                seq: 1,
                                snapshot: true,
                                parts,
                                message: Some(projected_message),
                                authoritative: true,
                            },
                        )
                        .await;
                        emit_session_state_change(
                            &ctx.session_store,
                            &ctx.notifier,
                            &ctx.status_center,
                            &ctx.status_notifier,
                            &ctx.data_dir,
                            session_id,
                            StateChange {
                                turn_phase: TurnPhase::Idle,
                                queue_paused: Some(true),
                                pending_permission_request: None,
                                pending_permission_state_revision: None,
                                exit_code: Some(1),
                                // Idle-Fatal creates a standalone message that already carries its
                                // backend timestamp. It must not finalize an older agent turn.
                                completed_at: None,
                                interrupted: true,
                                session_state: Some(SessionState::Error),
                            },
                        );
                    }
                    Err(error) => {
                        if let Some(RuntimeShutdown::Close(runtime)) =
                            actions.runtime_shutdowns.pop()
                        {
                            if let Some(state) = ctx.sessions.lock().await.get_mut(session_id) {
                                state.runtime = Some(runtime);
                            }
                        }
                        return Err(format!("idle fatal projection commit failed: {error}"));
                    }
                }
            } else if trailing_completed_crash {
                log::debug!(
                    "suppressed trailing fatal projection for completed crash in {session_id}"
                );
            }
            return Ok(actions);
        }
    };
    Ok(RuntimeEventPostActions::default())
}

fn runtime_error_message_id(
    session_id: &str,
    runtime_epoch: u64,
    event_received_at: f64,
    message: &str,
) -> String {
    use sha2::{Digest, Sha256};

    let mut exact = Vec::with_capacity(session_id.len() + message.len() + 48);
    exact.extend_from_slice(b"runtime_error_message_v1");
    exact.extend_from_slice(&(session_id.len() as u64).to_be_bytes());
    exact.extend_from_slice(session_id.as_bytes());
    exact.extend_from_slice(&runtime_epoch.to_be_bytes());
    exact.extend_from_slice(&event_received_at.to_bits().to_be_bytes());
    exact.extend_from_slice(&(message.len() as u64).to_be_bytes());
    exact.extend_from_slice(message.as_bytes());
    let digest = Sha256::digest(exact);
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

fn runtime_event_recovery_id(
    session_id: &str,
    runtime_epoch: u64,
    event_received_at: f64,
    reason: BackendSessionRecoveryReason,
    event_identity: &str,
) -> String {
    use sha2::{Digest, Sha256};

    let reason_tag = match reason {
        BackendSessionRecoveryReason::ResumeMismatch => b"resume_mismatch".as_slice(),
        BackendSessionRecoveryReason::BackendSessionLost => b"backend_session_lost".as_slice(),
    };
    let mut exact = Vec::with_capacity(session_id.len() + event_identity.len() + 80);
    exact.extend_from_slice(b"runtime_event_recovery_v1");
    exact.extend_from_slice(&(session_id.len() as u64).to_be_bytes());
    exact.extend_from_slice(session_id.as_bytes());
    exact.extend_from_slice(&runtime_epoch.to_be_bytes());
    exact.extend_from_slice(&event_received_at.to_bits().to_be_bytes());
    exact.extend_from_slice(&(reason_tag.len() as u64).to_be_bytes());
    exact.extend_from_slice(reason_tag);
    exact.extend_from_slice(&(event_identity.len() as u64).to_be_bytes());
    exact.extend_from_slice(event_identity.as_bytes());
    let digest = Sha256::digest(exact);
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

fn runtime_provider_session_observation_id(
    session_id: &str,
    runtime_epoch: u64,
    event_received_at: f64,
    backend_session_id: &str,
    context_carry: Option<&ContextCarryState>,
) -> String {
    use sha2::{Digest, Sha256};

    let context_carry_tag = match context_carry {
        None => b"not_requested".as_slice(),
        Some(ContextCarryState::Resumed) => b"resumed".as_slice(),
        Some(ContextCarryState::Reinjected) => b"reinjected".as_slice(),
        Some(ContextCarryState::Failed) => b"failed".as_slice(),
    };
    let mut exact = Vec::with_capacity(session_id.len() + backend_session_id.len() + 80);
    exact.extend_from_slice(b"runtime_provider_session_observation_v1");
    exact.extend_from_slice(&(session_id.len() as u64).to_be_bytes());
    exact.extend_from_slice(session_id.as_bytes());
    exact.extend_from_slice(&runtime_epoch.to_be_bytes());
    exact.extend_from_slice(&event_received_at.to_bits().to_be_bytes());
    exact.extend_from_slice(&(backend_session_id.len() as u64).to_be_bytes());
    exact.extend_from_slice(backend_session_id.as_bytes());
    exact.extend_from_slice(&(context_carry_tag.len() as u64).to_be_bytes());
    exact.extend_from_slice(context_carry_tag);
    let digest = Sha256::digest(exact);
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

fn runtime_event_targets_current_turn(event: &AgentRuntimeEvent) -> bool {
    matches!(
        event,
        AgentRuntimeEvent::PartsMerged(_)
            | AgentRuntimeEvent::PermissionRequested(_)
            | AgentRuntimeEvent::TokenUsageUpdated(_)
            | AgentRuntimeEvent::KeepAlive
            | AgentRuntimeEvent::TurnCompleted(_)
            | AgentRuntimeEvent::Fatal { .. }
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamingApplyMode {
    Coalesced,
    Immediate,
}

async fn apply_parts(
    ctx: &RuntimeContext,
    session_id: &str,
    parts: Vec<DomainMessagePart>,
    mode: StreamingApplyMode,
) -> Result<bool, String> {
    let domain_parts = parts;
    let delta_parts = parts_from_domain(domain_parts.clone());
    if delta_parts.is_empty() {
        return Ok(false);
    }
    let (
        turn_id,
        message_id,
        candidate_domain_parts,
        candidate_parts,
        next_streaming_seq,
        requires_snapshot,
    ) = {
        let sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get(session_id) else {
            return Ok(false);
        };
        if state.phase == RuntimeSessionPhase::Idle {
            log::debug!("dropping late message parts after terminal commit for {session_id}");
            return Ok(false);
        }
        let message_id = state
            .streaming_message_id
            .clone()
            .or_else(|| state.last_agent_message_id.clone());
        let Some(message_id) = message_id else {
            return Ok(false);
        };
        let Some(turn_id) = state.current_turn_id else {
            return Ok(false);
        };
        let mut candidate_domain_parts = state.domain_streaming_parts.clone();
        for part in &domain_parts {
            crate::domain::agent_session::entities::merge_part(
                &mut candidate_domain_parts,
                part.clone(),
            );
        }
        let can_append_delta = parts_can_stream_as_append_delta(&delta_parts);
        let requires_snapshot = mode == StreamingApplyMode::Immediate
            || state.streaming_delta_seq == 0
            || state.streaming_parts.is_empty()
            || !can_append_delta;
        (
            turn_id,
            message_id,
            candidate_domain_parts.clone(),
            parts_from_domain(candidate_domain_parts),
            state.streaming_delta_seq.saturating_add(1),
            requires_snapshot,
        )
    };
    let durable_events = durable_part_events(
        &ctx.session_store,
        &ctx.data_dir,
        session_id,
        turn_id,
        &message_id,
        &delta_parts,
    )?;
    let persisted_parts = ctx.session_store.persist_streaming_parts_with_events(
        &ctx.data_dir,
        session_id,
        &durable_events,
        &message_id,
        &candidate_parts,
        next_streaming_seq,
    )?;
    let persisted_at = std::time::Instant::now();
    let (emit_now, schedule_delay, cleared_stall) = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return Ok(false);
        };
        let current_message_id = state
            .streaming_message_id
            .as_ref()
            .or(state.last_agent_message_id.as_ref());
        if state.phase == RuntimeSessionPhase::Idle
            || state.current_turn_id != Some(turn_id)
            || current_message_id.map(String::as_str) != Some(message_id.as_str())
        {
            return Ok(false);
        }
        state.domain_streaming_parts = candidate_domain_parts;
        state.streaming_parts = persisted_parts;
        state.last_stream_persist_at = Some(persisted_at);
        let cleared_stall = state.record_progress(persisted_at);
        if requires_snapshot {
            state.pending_stream_snapshot = true;
            state.pending_stream_parts.clear();
            state.pending_stream_bytes = 0;
        } else if !state.pending_stream_snapshot {
            state.pending_stream_bytes = state
                .pending_stream_bytes
                .saturating_add(streaming_parts_byte_size(&delta_parts));
            state.pending_stream_parts.extend(delta_parts.clone());
        }
        let has_pending = state.pending_stream_snapshot || !state.pending_stream_parts.is_empty();
        let pending_part_count = if state.pending_stream_snapshot {
            state.streaming_parts.len()
        } else {
            state.pending_stream_parts.len()
        };
        let pending_byte_size = if state.pending_stream_snapshot {
            streaming_parts_byte_size(&state.streaming_parts)
        } else {
            state.pending_stream_bytes
        };
        let decision = if mode == StreamingApplyMode::Immediate {
            StreamingFlushDecision::Now
        } else {
            streaming_flush_decision(
                has_pending,
                state.retry_stream_delta.is_some(),
                pending_part_count,
                pending_byte_size,
                state.last_stream_emit_at,
                std::time::Instant::now(),
            )
        };
        let mut schedule_delay = None;
        let emit_now = match decision {
            StreamingFlushDecision::Now => true,
            StreamingFlushDecision::Later(delay) => {
                if !state.stream_flush_scheduled {
                    state.stream_flush_scheduled = true;
                    schedule_delay = Some(delay);
                }
                false
            }
            StreamingFlushDecision::NotNeeded => false,
        };
        (emit_now, schedule_delay, cleared_stall)
    };
    if cleared_stall {
        if let Err(error) = dispatch_stall_cleared_notifications(ctx, session_id).await {
            log::warn!("workflow stall-cleared notification failed for {session_id}: {error}");
        }
    }
    if emit_now {
        if let Err(error) = flush_streaming_update(ctx, session_id, false).await {
            log::warn!("failed to persist coalesced streaming parts for {session_id}: {error}");
        }
    } else if let Some(delay) = schedule_delay {
        spawn_delayed_stream_flush(ctx, session_id.to_string(), delay);
    }
    Ok(true)
}

fn spawn_delayed_stream_flush(
    ctx: &RuntimeContext,
    session_id: String,
    delay: std::time::Duration,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        tokio::time::sleep(delay).await;
        let _session_guard = acquire_session_runtime_lock(&ctx.session_locks, &session_id).await;
        if let Err(error) = flush_streaming_update(&ctx, &session_id, false).await {
            log::warn!("failed to persist delayed streaming parts for {session_id}: {error}");
        }
    }));
}

async fn flush_streaming_update(
    ctx: &RuntimeContext,
    session_id: &str,
    force_persist: bool,
) -> Result<(), String> {
    let now = std::time::Instant::now();
    let (payload, persist_snapshot, emit_suppressed) = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return Ok(());
        };
        state.stream_flush_scheduled = false;
        let message_id = state
            .streaming_message_id
            .clone()
            .or_else(|| state.last_agent_message_id.clone());
        let retry = state.retry_stream_delta.take();
        let payload = if let Some(retry) = retry {
            Some(retry)
        } else if state.pending_stream_snapshot || !state.pending_stream_parts.is_empty() {
            let Some(message_id) = message_id.clone() else {
                return Ok(());
            };
            let snapshot = state.pending_stream_snapshot || state.streaming_delta_seq == 0;
            let parts = if snapshot {
                state.streaming_parts.clone()
            } else {
                std::mem::take(&mut state.pending_stream_parts)
            };
            state.pending_stream_bytes = 0;
            state.pending_stream_snapshot = false;
            Some(PendingStreamDelta {
                message_id,
                seq: state.streaming_delta_seq.saturating_add(1),
                snapshot,
                parts,
                message: None,
                authoritative: false,
            })
        } else {
            None
        };
        let persist = message_id.and_then(|message_id| {
            should_persist_streaming_snapshot(state.last_stream_persist_at, now, force_persist)
                .then(|| {
                    let seq = payload
                        .as_ref()
                        .map(|payload| payload.seq)
                        .unwrap_or_else(|| state.streaming_delta_seq.saturating_add(1));
                    (message_id, seq, state.streaming_parts.clone())
                })
        });
        (payload, persist, state.stream_emit_suppressed)
    };

    let persist_result = if let Some((message_id, seq, parts)) = persist_snapshot {
        match ctx.session_store.persist_message_parts(
            &ctx.data_dir,
            session_id,
            &message_id,
            &parts,
            seq,
            None,
        ) {
            Ok(_) => {
                if let Some(state) = ctx.sessions.lock().await.get_mut(session_id) {
                    state.last_stream_persist_at = Some(now);
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    } else {
        Ok(())
    };

    if let Err(error) = persist_result {
        if payload.is_some() {
            let mut sessions = ctx.sessions.lock().await;
            if let Some(state) = sessions.get_mut(session_id) {
                // The attempted delta was removed from the pending fields above. Quarantine it
                // as a full snapshot and force the next flush to cross persistence again before
                // anything derived from it can become live.
                state.pending_stream_snapshot = true;
                state.pending_stream_parts.clear();
                state.pending_stream_bytes = 0;
                state.retry_stream_delta = None;
                state.last_stream_persist_at = None;
            }
        }
        return Err(error);
    }

    let Some(payload) = payload else {
        return Ok(());
    };

    if emit_suppressed {
        return Ok(());
    }

    let emitted = ctx
        .notifier
        .streaming_delta(payload.to_delta_payload(session_id));

    let mut retry_delay = None;
    {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return Ok(());
        };
        if emitted {
            state.streaming_delta_seq = state.streaming_delta_seq.max(payload.seq);
            state.last_stream_emit_at = Some(now);
            state.stream_emit_failure_count = 0;
        } else {
            retry_delay = on_stream_emit_failure(state, session_id, &payload);
        }
    };
    if let Some(delay) = retry_delay {
        spawn_delayed_stream_flush(ctx, session_id.to_string(), delay);
    }
    Ok(())
}

async fn emit_streaming_delta_or_retry(
    ctx: &RuntimeContext,
    session_id: &str,
    payload: PendingStreamDelta,
) {
    if payload.authoritative {
        emit_authoritative_streaming_delta_or_retry(ctx, session_id, payload).await;
        return;
    }
    {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        if state.stream_emit_suppressed {
            return;
        }
    }
    let now = std::time::Instant::now();
    let emitted = ctx
        .notifier
        .streaming_delta(payload.to_delta_payload(session_id));
    let retry_delay = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        if emitted {
            state.last_stream_emit_at = Some(now);
            state.stream_emit_failure_count = 0;
            return;
        }
        on_stream_emit_failure(state, session_id, &payload)
    };
    if let Some(delay) = retry_delay {
        spawn_delayed_stream_flush(ctx, session_id.to_string(), delay);
    }
}

async fn emit_authoritative_streaming_delta_or_retry(
    ctx: &RuntimeContext,
    session_id: &str,
    payload: PendingStreamDelta,
) {
    let retry_delay = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        prepare_authoritative_stream_emit(state, &payload.message_id);
        if state.authoritative_stream_retries.is_empty() {
            None
        } else {
            upsert_authoritative_stream_retry(state, payload.clone());
            if state.authoritative_stream_flush_scheduled {
                return;
            }
            state.authoritative_stream_flush_scheduled = true;
            Some(super::streaming::STREAMING_EMIT_INTERVAL)
        }
    };
    if let Some(delay) = retry_delay {
        spawn_delayed_authoritative_stream_flush(ctx, session_id.to_string(), delay);
        return;
    }
    let now = std::time::Instant::now();
    let emitted = ctx
        .notifier
        .streaming_delta(payload.to_delta_payload(session_id));
    let retry_delay = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        if emitted {
            state.last_stream_emit_at = Some(now);
            state.authoritative_stream_emit_failure_count = 0;
            None
        } else {
            on_authoritative_stream_emit_failure(state, session_id, &payload)
        }
    };
    if let Some(delay) = retry_delay {
        spawn_delayed_authoritative_stream_flush(ctx, session_id.to_string(), delay);
    }
}

fn prepare_authoritative_stream_emit(state: &mut RuntimeSessionState, message_id: &str) {
    // A backend-owned snapshot supersedes any older coalesced retry before the notifier call.
    // Delayed flushes are serialized by the session runtime lock and therefore observe this
    // updated state after the authoritative attempt completes.
    state.retry_stream_delta = None;
    state.stream_flush_scheduled = false;
    state.stream_emit_failure_count = 0;
    state.stream_emit_suppressed = false;
    state
        .authoritative_stream_retries
        .retain(|retry| retry.message_id != message_id);
}

fn upsert_authoritative_stream_retry(state: &mut RuntimeSessionState, payload: PendingStreamDelta) {
    if let Some(retry) = state
        .authoritative_stream_retries
        .iter_mut()
        .find(|retry| retry.message_id == payload.message_id)
    {
        *retry = payload;
    } else {
        state.authoritative_stream_retries.push_back(payload);
    }
}

fn spawn_delayed_authoritative_stream_flush(
    ctx: &RuntimeContext,
    session_id: String,
    delay: std::time::Duration,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        tokio::time::sleep(delay).await;
        let _session_guard = acquire_session_runtime_lock(&ctx.session_locks, &session_id).await;
        flush_authoritative_stream_retry(&ctx, &session_id).await;
    }));
}

async fn flush_authoritative_stream_retry(ctx: &RuntimeContext, session_id: &str) {
    {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        state.authoritative_stream_flush_scheduled = false;
    }
    loop {
        let payload = {
            let sessions = ctx.sessions.lock().await;
            let Some(state) = sessions.get(session_id) else {
                return;
            };
            let Some(payload) = state.authoritative_stream_retries.front().cloned() else {
                return;
            };
            payload
        };
        let emitted = ctx
            .notifier
            .streaming_delta(payload.to_delta_payload(session_id));
        let retry_delay = {
            let mut sessions = ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return;
            };
            if emitted {
                if state
                    .authoritative_stream_retries
                    .front()
                    .is_some_and(|retry| {
                        retry.message_id == payload.message_id && retry.seq == payload.seq
                    })
                {
                    state.authoritative_stream_retries.pop_front();
                }
                state.authoritative_stream_emit_failure_count = 0;
                None
            } else {
                on_authoritative_stream_emit_failure(state, session_id, &payload)
            }
        };
        if let Some(delay) = retry_delay {
            spawn_delayed_authoritative_stream_flush(ctx, session_id.to_string(), delay);
            return;
        }
    }
}

const STREAM_EMIT_FAILURE_FALLBACK_LIMIT: u32 = 5;
const STREAM_EMIT_FAILURE_STOP_LIMIT: u32 = STREAM_EMIT_FAILURE_FALLBACK_LIMIT * 2;

fn on_stream_emit_failure(
    state: &mut RuntimeSessionState,
    session_id: &str,
    payload: &PendingStreamDelta,
) -> Option<std::time::Duration> {
    state.stream_emit_failure_count = state.stream_emit_failure_count.saturating_add(1);
    let failures = state.stream_emit_failure_count;
    log::warn!(
        "agent-streaming-delta emit failure: chat_session={} message_id={} seq={} snapshot={} part_count={} consecutive_failures={}",
        session_id,
        payload.message_id,
        payload.seq,
        payload.snapshot,
        payload.parts.len(),
        failures
    );
    if failures >= STREAM_EMIT_FAILURE_STOP_LIMIT {
        log::error!(
            "agent-streaming-delta emit failed {failures} consecutive times for chat_session={session_id}; stopping streaming emit until turn end"
        );
        state.stream_emit_suppressed = true;
        state.retry_stream_delta = None;
        state.pending_stream_snapshot = false;
        state.pending_stream_parts.clear();
        state.pending_stream_bytes = 0;
        return None;
    }
    if failures >= STREAM_EMIT_FAILURE_FALLBACK_LIMIT {
        if failures == STREAM_EMIT_FAILURE_FALLBACK_LIMIT {
            log::warn!(
                "agent-streaming-delta emit failed {failures} consecutive times for chat_session={session_id}; falling back to full snapshot resync"
            );
        }
        state.retry_stream_delta = None;
        state.pending_stream_snapshot = true;
        state.pending_stream_parts.clear();
        state.pending_stream_bytes = 0;
    } else if state.retry_stream_delta.is_none() {
        state.retry_stream_delta = Some(PendingStreamDelta {
            snapshot: true,
            parts: state.streaming_parts.clone(),
            ..payload.clone()
        });
    }
    if state.stream_flush_scheduled {
        None
    } else {
        state.stream_flush_scheduled = true;
        Some(super::streaming::STREAMING_EMIT_INTERVAL)
    }
}

fn on_authoritative_stream_emit_failure(
    state: &mut RuntimeSessionState,
    session_id: &str,
    payload: &PendingStreamDelta,
) -> Option<std::time::Duration> {
    state.authoritative_stream_emit_failure_count = state
        .authoritative_stream_emit_failure_count
        .saturating_add(1);
    let failures = state.authoritative_stream_emit_failure_count;
    log::warn!(
        "authoritative agent-streaming-delta emit failure: chat_session={} message_id={} seq={} part_count={} consecutive_failures={}",
        session_id,
        payload.message_id,
        payload.seq,
        payload.parts.len(),
        failures
    );
    if failures >= STREAM_EMIT_FAILURE_STOP_LIMIT {
        log::error!(
            "authoritative agent-streaming-delta emit failed {failures} consecutive times for chat_session={session_id}; stopping delivery retry"
        );
        state.authoritative_stream_retries.clear();
        state.authoritative_stream_flush_scheduled = false;
        return None;
    }
    upsert_authoritative_stream_retry(state, payload.clone());
    if state.authoritative_stream_flush_scheduled {
        None
    } else {
        state.authoritative_stream_flush_scheduled = true;
        Some(super::streaming::STREAMING_EMIT_INTERVAL)
    }
}

fn merge_persisted_message_part(parts: &mut Vec<MessagePart>, incoming: MessagePart) {
    match incoming {
        MessagePart::Text {
            content,
            parent_tool_use_id,
        } => {
            if let Some(MessagePart::Text {
                content: existing,
                parent_tool_use_id: existing_parent,
            }) = parts.last_mut()
            {
                if existing_parent == &parent_tool_use_id {
                    existing.push_str(&content);
                    return;
                }
            }
            parts.push(MessagePart::Text {
                content,
                parent_tool_use_id,
            });
        }
        MessagePart::Thinking {
            content,
            parent_tool_use_id,
        } => {
            if let Some(MessagePart::Thinking {
                content: existing,
                parent_tool_use_id: existing_parent,
            }) = parts.last_mut()
            {
                if existing_parent == &parent_tool_use_id {
                    existing.push_str(&content);
                    return;
                }
            }
            parts.push(MessagePart::Thinking {
                content,
                parent_tool_use_id,
            });
        }
        MessagePart::ToolUse { ref id, .. } => {
            if let Some(existing) = parts.iter_mut().find(|part| {
                matches!(part, MessagePart::ToolUse { id: existing_id, .. } if existing_id == id)
            }) {
                *existing = incoming;
            } else {
                parts.push(incoming);
            }
        }
        MessagePart::ToolResult {
            content,
            is_error,
            tool_use_id,
            parent_tool_use_id,
            content_ref,
            summary,
        } => {
            apply_tool_result_update(
                parts,
                ToolResultUpdate {
                    content,
                    is_error,
                    tool_use_id,
                    parent_tool_use_id,
                    content_ref,
                    summary,
                },
            );
        }
        MessagePart::Permission { ref request, .. } => {
            if let Some(existing) = parts.iter_mut().find(|part| {
                matches!(
                    part,
                    MessagePart::Permission {
                        request: existing_request,
                        ..
                    } if existing_request.id == request.id
                )
            }) {
                *existing = incoming;
            } else {
                parts.push(incoming);
            }
        }
        MessagePart::TaskStatus {
            ref task_tool_use_id,
            ..
        } => {
            if let Some(existing) = parts.iter_mut().find(|part| {
                matches!(
                    part,
                    MessagePart::TaskStatus {
                        task_tool_use_id: existing_id,
                        ..
                    } if existing_id == task_tool_use_id
                )
            }) {
                *existing = incoming;
            } else {
                parts.push(incoming);
            }
        }
        MessagePart::TodoListSnapshot { .. } => {
            if let Some(existing) = parts
                .iter_mut()
                .find(|part| matches!(part, MessagePart::TodoListSnapshot { .. }))
            {
                *existing = incoming;
            } else {
                parts.push(incoming);
            }
        }
        MessagePart::SystemNotification {
            ref notification_type,
            ..
        } => {
            if let Some(existing) = parts.iter_mut().find(|part| {
                matches!(
                    part,
                    MessagePart::SystemNotification {
                        notification_type: existing_type,
                        status,
                        ..
                    } if existing_type == notification_type && status == "in_progress"
                )
            }) {
                *existing = incoming;
            } else {
                parts.push(incoming);
            }
        }
        MessagePart::Error {
            ref content,
            ref parent_tool_use_id,
        } => {
            let duplicate = parts.iter().any(|part| {
                matches!(
                    part,
                    MessagePart::Error {
                        content: existing_content,
                        parent_tool_use_id: existing_parent,
                    } if existing_content == content && existing_parent == parent_tool_use_id
                )
            });
            if !duplicate {
                parts.push(incoming);
            }
        }
        MessagePart::Image { .. } | MessagePart::ImageRef { .. } => parts.push(incoming),
    }
}

pub(super) async fn complete_turn(
    ctx: &RuntimeContext,
    session_id: &str,
    expected_generation: Option<u64>,
    result: crate::domain::agent_session::entities::TurnResult,
) -> Result<Option<WorkflowTurnCompleteNotification>, String> {
    complete_turn_with_acceptance_and_persist_kind(
        ctx,
        session_id,
        expected_generation,
        result,
        PersistFailureKind::FinalPartsRecorded,
    )
    .await
    .map(|(notification, _)| notification)
}

async fn complete_turn_with_acceptance(
    ctx: &RuntimeContext,
    session_id: &str,
    expected_generation: Option<u64>,
    result: crate::domain::agent_session::entities::TurnResult,
) -> Result<(Option<WorkflowTurnCompleteNotification>, bool), String> {
    complete_turn_with_acceptance_and_persist_kind(
        ctx,
        session_id,
        expected_generation,
        result,
        PersistFailureKind::FinalPartsRecorded,
    )
    .await
}

async fn complete_turn_with_acceptance_and_persist_kind(
    ctx: &RuntimeContext,
    session_id: &str,
    expected_generation: Option<u64>,
    result: crate::domain::agent_session::entities::TurnResult,
    persist_kind: PersistFailureKind,
) -> Result<(Option<WorkflowTurnCompleteNotification>, bool), String> {
    let _queue_transition_guard = ctx.transitions.acquire(session_id).await;
    let interrupt_was_accepted = {
        let sessions = ctx.sessions.lock().await;
        sessions.get(session_id).is_some_and(|state| {
            state.interrupt_requested_generation == Some(state.generation)
                && expected_generation.is_none_or(|generation| state.generation == generation)
        })
    };
    let emit_crash_snapshot = matches!(
        &result,
        TurnResult::Interrupted {
            reason: DomainInterruptReason::Crash,
            ..
        }
    );
    let should_complete = {
        let sessions = ctx.sessions.lock().await;
        sessions.get(session_id).is_some_and(|state| {
            state.phase != RuntimeSessionPhase::Idle
                && expected_generation.is_none_or(|generation| state.generation == generation)
        })
    };
    if !should_complete {
        log::debug!(
            "skipping turn completion for {session_id}: turn already completed or generation mismatch (expected={expected_generation:?})"
        );
        return Ok((None, false));
    }
    flush_streaming_update(ctx, session_id, true).await?;
    let completed_at = crate::usecase::agent_session::session::now_timestamp();
    let terminal = terminal_projection(&result);
    let (message_id, parts, seq, turn_id, started_at, queue_was_paused_at) = {
        let sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get(session_id) else {
            return Ok((None, false));
        };
        if state.phase == RuntimeSessionPhase::Idle
            || expected_generation.is_some_and(|generation| state.generation != generation)
        {
            return Ok((None, false));
        }
        (
            state.streaming_message_id.clone(),
            state.streaming_parts.clone(),
            state.streaming_delta_seq,
            state.current_turn_id,
            state.turn_started_at,
            state.queue_paused_at,
        )
    };
    if turn_id.is_none() || message_id.is_none() {
        return Err(format!(
            "cannot commit a terminal result without the durable turn and assistant-message identity for {session_id}"
        ));
    }
    let mut projected = None;
    let mut crash_snapshot = None;
    if let (Some(turn_id), Some(message_id)) = (turn_id, message_id.clone()) {
        let final_seq = if emit_crash_snapshot {
            seq.saturating_add(1)
        } else {
            seq
        };
        let events = final_turn_events(
            ctx,
            session_id,
            turn_id,
            &message_id,
            &parts,
            &terminal,
            completed_at,
        )?;
        let (model, persisted_parts) = persist_with_retry(ctx, session_id, persist_kind, || {
            ctx.session_store.append_terminal_events_and_materialize(
                &ctx.data_dir,
                session_id,
                &events,
                &message_id,
                final_seq,
                completed_at,
                &result,
            )
        })
        .await?;
        {
            let mut sessions = ctx.sessions.lock().await;
            if let Some(state) = sessions.get_mut(session_id) {
                if state.current_turn_id == Some(turn_id) {
                    state.terminal_turn_id = Some(turn_id);
                }
            }
        }
        projected = Some(model);
        if emit_crash_snapshot {
            crash_snapshot = Some(PendingStreamDelta {
                message_id,
                seq: final_seq,
                snapshot: true,
                parts: persisted_parts,
                message: None,
                authoritative: true,
            });
        }
    }
    if let Some(snapshot) = crash_snapshot {
        emit_streaming_delta_or_retry(ctx, session_id, snapshot).await;
    }
    let session_state = projected
        .as_ref()
        .map(|model| model.status.session_state.clone())
        .or_else(|| (!emit_crash_snapshot).then(|| terminal.session_state.clone()));
    let queue_paused_at = projected
        .as_ref()
        .and_then(|model| model.queue_paused_at)
        .or(queue_was_paused_at)
        .or_else(|| terminal.pause_queue.then_some(completed_at));
    let queue_paused = queue_paused_at.is_some();
    let pending_permission_state_revision = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return Ok((None, false));
        };
        if state.phase == RuntimeSessionPhase::Idle
            || expected_generation.is_some_and(|generation| state.generation != generation)
        {
            return Ok((None, false));
        }
        state.phase = RuntimeSessionPhase::Idle;
        state.queue_paused = queue_paused;
        state.queue_paused_at = queue_paused_at;
        let pending_permission_state_revision = state.clear_pending_permission_request();
        state.permission_wait_started_at = None;
        state.permission_wait_diagnostic_emitted = false;
        state.stall_observation_active = false;
        state.last_agent_message_id = message_id;
        let usage = match &result {
            crate::domain::agent_session::entities::TurnResult::Completed {
                token_usage, ..
            }
            | crate::domain::agent_session::entities::TurnResult::Failed { token_usage, .. } => {
                token_usage.map(token_usage_from_domain)
            }
            crate::domain::agent_session::entities::TurnResult::Interrupted { .. } => None,
        };
        if let Some(usage) = usage {
            state.latest_token_usage = Some(usage);
        }
        state.turn_started_at = None;
        state.streaming_message_id = None;
        state.current_turn_id = None;
        state.current_turn_input = None;
        state.interrupt_requested_generation = None;
        state.domain_streaming_parts.clear();
        state.streaming_parts.clear();
        state.streaming_delta_seq = 0;
        state.stream_emit_failure_count = 0;
        state.stream_emit_suppressed = false;
        pending_permission_state_revision
    };
    if let Some(started_at) = started_at {
        record_agent_turn_duration_detached(
            ctx,
            session_id.to_string(),
            crate::other::telemetry::AgentTurn::Complete,
            started_at.elapsed(),
        );
    }
    let workflow_notification = projected
        .as_ref()
        .and_then(|model| model.workflow_turn_complete.as_ref())
        .map(|input| workflow_turn_complete_notification(session_id, input));
    emit_session_state_change(
        &ctx.session_store,
        &ctx.notifier,
        &ctx.status_center,
        &ctx.status_notifier,
        &ctx.data_dir,
        session_id,
        StateChange {
            turn_phase: TurnPhase::Idle,
            queue_paused: Some(queue_paused),
            pending_permission_request: None,
            pending_permission_state_revision: Some(pending_permission_state_revision),
            exit_code: Some(terminal.exit_code),
            completed_at: Some(completed_at),
            interrupted: terminal.interrupted,
            session_state,
        },
    );
    Ok((workflow_notification, interrupt_was_accepted))
}

async fn turn_owns_runtime(
    ctx: &RuntimeContext,
    session_id: &str,
    generation: u64,
    runtime: &Arc<dyn AgentSessionRuntime>,
) -> bool {
    let sessions = ctx.sessions.lock().await;
    sessions.get(session_id).is_some_and(|state| {
        state.generation == generation
            && state.phase != RuntimeSessionPhase::Idle
            && !state.queue_paused
            && state.interrupt_requested_generation != Some(generation)
            && state
                .runtime
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, runtime))
    })
}

async fn turn_runtime_is_current(
    ctx: &RuntimeContext,
    session_id: &str,
    generation: u64,
    runtime: &Arc<dyn AgentSessionRuntime>,
) -> bool {
    let sessions = ctx.sessions.lock().await;
    sessions.get(session_id).is_some_and(|state| {
        state.generation == generation
            && state.phase != RuntimeSessionPhase::Idle
            && state
                .runtime
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, runtime))
    })
}

async fn detach_runtime_if_current(
    ctx: &RuntimeContext,
    session_id: &str,
    runtime: &Arc<dyn AgentSessionRuntime>,
) {
    let detached = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        if !state
            .runtime
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, runtime))
        {
            return;
        }
        let detached = state.runtime.take();
        state.bump_runtime_epoch();
        detached
    };
    if let Some(runtime) = detached {
        let spawner = Arc::clone(&ctx.spawner);
        spawner.spawn(Box::pin(async move {
            runtime.close().await;
        }));
    }
}

fn queued_turn_has_accepted_identity(queued: &QueuedTurnInput) -> bool {
    // Treat a partial identity as durable too; the obligation match below will
    // reject it, but it must never bypass the canonical idle fence first.
    queued.accepted_operation_id.is_some() || queued.execution_obligation_id.is_some()
}

fn queued_input_matches_canonical_entry(
    queued: &QueuedTurnInput,
    canonical: &CanonicalQueuedSend,
) -> bool {
    queued.id == canonical.queue_item_id
        && queued.existing_human_message_id.as_deref() == Some(canonical.human_message_id.as_str())
        && queued.reserved_turn_id == canonical.reserved_turn_id.parse::<u64>().ok()
}

fn insert_accepted_queue_in_canonical_order(
    pending_queue: &mut std::collections::VecDeque<QueuedTurnInput>,
    accepted_input: QueuedTurnInput,
    canonical_queue: &[CanonicalQueuedSend],
) -> Result<(), String> {
    let canonical_rank = canonical_queue
        .iter()
        .position(|entry| queued_input_matches_canonical_entry(&accepted_input, entry))
        .ok_or_else(|| {
            "accepted queued send is absent from the canonical queue projection".to_string()
        })?;

    if let Some(existing_index) = pending_queue
        .iter()
        .position(|queued| queued.id == accepted_input.id)
    {
        let existing = &pending_queue[existing_index];
        if existing.accepted_operation_id != accepted_input.accepted_operation_id
            || existing.execution_obligation_id != accepted_input.execution_obligation_id
            || existing.existing_human_message_id != accepted_input.existing_human_message_id
            || existing.reserved_turn_id != accepted_input.reserved_turn_id
        {
            return Err("accepted queue identity changed during restoration".to_string());
        }
        pending_queue.remove(existing_index);
    }

    pending_queue.retain(|queued| {
        !queued_turn_has_accepted_identity(queued)
            || canonical_queue
                .iter()
                .any(|entry| queued_input_matches_canonical_entry(queued, entry))
    });
    let insertion_index = pending_queue
        .iter()
        .position(|queued| {
            queued_turn_has_accepted_identity(queued)
                && canonical_queue
                    .iter()
                    .position(|entry| queued_input_matches_canonical_entry(queued, entry))
                    .is_some_and(|rank| rank > canonical_rank)
        })
        .unwrap_or(pending_queue.len());
    pending_queue.insert(insertion_index, accepted_input);
    Ok(())
}

async fn remove_local_queue_front_if_matches(
    ctx: &RuntimeContext,
    session_id: &str,
    queue_item_id: &str,
) {
    let mut sessions = ctx.sessions.lock().await;
    let Some(state) = sessions.get_mut(session_id) else {
        return;
    };
    if state.pending_queue.front().map(|front| front.id.as_str()) == Some(queue_item_id) {
        state.pending_queue.pop_front();
    }
}

async fn arm_accepted_send_recovery_after_claim_release(
    driver: &dyn AcceptedSendObligationDriver,
    operation_id: &str,
    obligation_id: &str,
    accepted_claim: &mut Option<AcceptedSendExecutionClaim>,
) {
    let Some(recovery_wake) = driver
        .reconcile_turn_execution(operation_id, obligation_id)
        .await
    else {
        return;
    };
    match accepted_claim.take() {
        Some(claim) => {
            *accepted_claim = Some(claim.wake_after_release(recovery_wake));
        }
        None => recovery_wake.publish(),
    }
}

pub(super) async fn start_next_queued_turn(ctx: &RuntimeContext, session_id: &str) {
    if let Err(failure) = ctx
        .session_store
        .ensure_no_unresolved_recovery(session_id)
        .await
    {
        log::warn!(
            "queued turn drain blocked by unresolved recovery {} for {session_id}: {failure}",
            failure.correlation_id
        );
        return;
    }
    let (queued, runtime) = {
        let sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get(session_id) else {
            return;
        };
        if state.closing
            || state.backend_recovery.is_some()
            || state.phase != RuntimeSessionPhase::Idle
            || state.queue_paused
        {
            return;
        }
        let Some(queued) = state.pending_queue.front().cloned() else {
            return;
        };
        let runtime = state.runtime.clone();
        (queued, runtime)
    };

    if queued_turn_has_accepted_identity(&queued) {
        let canonical_queue = match ctx.session_store.canonical_pending_send_queue(session_id) {
            Ok(queue) => queue,
            Err(error) => {
                log::warn!("accepted queue authority is unavailable for {session_id}: {error}");
                return;
            }
        };
        let is_canonical_front = canonical_queue
            .first()
            .is_some_and(|front| queued_input_matches_canonical_entry(&queued, front));
        if !is_canonical_front {
            let remains_canonically_pending = canonical_queue
                .iter()
                .any(|entry| queued_input_matches_canonical_entry(&queued, entry));
            if !remains_canonically_pending {
                remove_local_queue_front_if_matches(ctx, session_id, &queued.id).await;
            }
            return;
        }
    }

    let Some(session) = (match ctx
        .session_store
        .get_session_shell(&ctx.data_dir, session_id)
    {
        Ok(session) => session,
        Err(error) => {
            log::warn!("failed to load queued turn session {session_id}: {error}");
            return;
        }
    }) else {
        log::warn!("queued turn session not found: {session_id}");
        return;
    };
    if queued_turn_has_accepted_identity(&queued) {
        match ctx
            .session_store
            .accepted_queue_start_readiness(&ctx.data_dir, session_id)
        {
            Ok(Some(true)) => {}
            Ok(Some(false)) => return,
            Ok(None) => {
                log::warn!("accepted queue session projection not found: {session_id}");
                return;
            }
            Err(error) => {
                log::warn!("accepted queue readiness is unavailable for {session_id}: {error}");
                return;
            }
        }
    }
    let mut accepted_claim;
    let accepted_obligation = match (
        queued.accepted_operation_id.as_deref(),
        queued.execution_obligation_id.as_deref(),
    ) {
        (Some(operation_id), Some(obligation_id)) => {
            let driver = ctx
                .accepted_send_obligation_driver
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let Some(driver) = driver else {
                log::warn!("accepted queued send driver is unavailable [{operation_id}]");
                return;
            };
            accepted_claim = None;
            Some((operation_id.to_string(), obligation_id.to_string(), driver))
        }
        (None, None) => {
            #[cfg(test)]
            {
                accepted_claim = None;
                None
            }
            #[cfg(not(test))]
            {
                log::error!(
                    "queued turn has no durable accepted operation identity for {session_id}"
                );
                return;
            }
        }
        _ => {
            log::error!("accepted queued send has incomplete obligation identity");
            return;
        }
    };
    if queued.worktree_path != session.worktree_path {
        if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
            arm_accepted_send_recovery_after_claim_release(
                driver.as_ref(),
                operation_id,
                obligation_id,
                &mut accepted_claim,
            )
            .await;
        }
        log::error!(
            "queued turn worktree mismatch for {session_id}: queued={}, session={}",
            queued.worktree_path,
            session.worktree_path
        );
        return;
    }
    let had_runtime = runtime.is_some();
    let system_prompt = match build_queued_system_prompt(
        &ctx.session_store,
        ctx.branch_diff_context.as_deref(),
        ctx.instruction_source.as_ref(),
        &ctx.data_dir,
        &session,
        &queued,
    ) {
        Ok(system_prompt) => system_prompt,
        Err(error) => {
            log::warn!(
                "queued turn system prompt preflight remains pending for {session_id}: {error}"
            );
            return;
        }
    };
    #[cfg(test)]
    let mut runtime = runtime;
    #[cfg(not(test))]
    let runtime = runtime;
    #[cfg(test)]
    if accepted_obligation.is_none() && runtime.is_none() {
        // Legacy direct-send queues exist only in unit tests. Preserve their
        // historical pre-TurnStarted reopen boundary so the fault-injection
        // tests continue to exercise a retryable queue item. Production
        // accepted queues must cross the combined durable claim first.
        let runtime_open_epoch = {
            let mut sessions = ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return;
            };
            state.bump_runtime_epoch()
        };
        runtime = match open_runtime_for_session(
            ctx,
            &session,
            system_prompt.clone(),
            Some(runtime_open_epoch),
        )
        .await
        {
            Ok(runtime) => Some(runtime),
            Err(AgentRuntimeError::BackendSessionLost { .. }) => {
                if let Err(error) = recover_backend_session(
                    ctx,
                    session_id,
                    BackendSessionRecoveryReason::BackendSessionLost,
                )
                .await
                {
                    log::warn!(
                        "failed to recover backend session for queued turn {session_id}: {error}"
                    );
                }
                return;
            }
            Err(error) => {
                log::warn!("failed to reopen runtime for queued turn {session_id}: {error}");
                if let Err(persist_error) =
                    persist_with_retry(ctx, session_id, PersistFailureKind::ReopenRuntime, || {
                        ctx.session_store.set_session_state(
                            &ctx.data_dir,
                            session_id,
                            SessionState::Error,
                        )
                    })
                    .await
                {
                    log::error!(
                        "failed to persist queued runtime reopen error for {session_id}: {persist_error}"
                    );
                    return;
                }
                emit_session_state_change(
                    &ctx.session_store,
                    &ctx.notifier,
                    &ctx.status_center,
                    &ctx.status_notifier,
                    &ctx.data_dir,
                    session_id,
                    StateChange {
                        turn_phase: TurnPhase::Idle,
                        queue_paused: None,
                        pending_permission_request: None,
                        pending_permission_state_revision: None,
                        exit_code: Some(1),
                        completed_at: Some(crate::usecase::agent_session::session::now_timestamp()),
                        interrupted: true,
                        session_state: Some(SessionState::Error),
                    },
                );
                return;
            }
        };
    }
    let turn_id = match (queued.reserved_turn_id, accepted_obligation.is_some()) {
        (Some(turn_id), _) => turn_id,
        (None, true) => {
            if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
                arm_accepted_send_recovery_after_claim_release(
                    driver.as_ref(),
                    operation_id,
                    obligation_id,
                    &mut accepted_claim,
                )
                .await;
            }
            log::error!("accepted queued send has no reserved turn identity");
            return;
        }
        (None, false) => match next_turn_id(&ctx.session_store, &ctx.data_dir, session_id) {
            Ok(turn_id) => turn_id,
            Err(error) => {
                log::warn!("failed to allocate queued turn id for {session_id}: {error}");
                return;
            }
        },
    };
    let queue_item_is_current = {
        let sessions = ctx.sessions.lock().await;
        sessions.get(session_id).is_some_and(|state| {
            state.pending_queue.front().map(|front| front.id.as_str()) == Some(queued.id.as_str())
        })
    };
    if !queue_item_is_current {
        log::warn!("accepted queued send preflight lost its exact in-memory queue identity");
        return;
    }

    let human_message_id = queued
        .existing_human_message_id
        .as_deref()
        .unwrap_or(queued.id.as_str());
    let human_message = match committed_queued_message(
        &ctx.session_store,
        &ctx.data_dir,
        session_id,
        human_message_id,
        MessageRole::Human,
    ) {
        Ok(Some(message)) => message,
        Ok(None) => {
            #[cfg(test)]
            if accepted_obligation.is_none() {
                queued_human_message(&queued)
            } else {
                if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
                    arm_accepted_send_recovery_after_claim_release(
                        driver.as_ref(),
                        operation_id,
                        obligation_id,
                        &mut accepted_claim,
                    )
                    .await;
                }
                log::error!(
                    "accepted queued send has no committed human projection [{human_message_id}]"
                );
                return;
            }
            #[cfg(not(test))]
            {
                if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
                    arm_accepted_send_recovery_after_claim_release(
                        driver.as_ref(),
                        operation_id,
                        obligation_id,
                        &mut accepted_claim,
                    )
                    .await;
                }
                log::error!(
                    "accepted queued send has no committed human projection [{human_message_id}]"
                );
                return;
            }
        }
        Err(error) => {
            log::warn!(
                "accepted queued human projection remains unreadable for {session_id}: {error}"
            );
            return;
        }
    };
    let durably_accepted = accepted_obligation.is_some();
    let committed_prompt =
        crate::usecase::agent_session::event_log::prompt_input_from_human_message(&human_message);
    let accepted_payload_mismatch = durably_accepted
        && committed_prompt
            != crate::domain::agent_session::events::PromptInput {
                content: queued.content.clone(),
                mentions: queued.mentions.clone(),
                attachment_refs: Vec::new(),
                parts: queued
                    .images
                    .iter()
                    .map(|image| MessagePart::Image {
                        data: image.data.clone(),
                        media_type: image.media_type.clone(),
                    })
                    .collect(),
            };
    if accepted_payload_mismatch {
        if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
            arm_accepted_send_recovery_after_claim_release(
                driver.as_ref(),
                operation_id,
                obligation_id,
                &mut accepted_claim,
            )
            .await;
        }
        log::error!("accepted queued human projection does not match its canonical payload");
        return;
    }
    let (agent_message_id, legacy_agent_message) = if durably_accepted {
        let Some(message_id) = queued.existing_agent_message_id.clone() else {
            if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
                arm_accepted_send_recovery_after_claim_release(
                    driver.as_ref(),
                    operation_id,
                    obligation_id,
                    &mut accepted_claim,
                )
                .await;
            }
            log::error!("accepted queued send has no reserved assistant identity");
            return;
        };
        (message_id, None)
    } else {
        #[cfg(test)]
        {
            // Legacy direct-send queues exist only in unit tests. Keep their
            // assistant append as a separate fault boundary for retry oracles.
            let agent_message = match queued_agent_message(
                &ctx.session_store,
                &ctx.data_dir,
                session_id,
                &queued,
            ) {
                Ok(message) => message,
                Err(error) => {
                    log::warn!("failed to append queued agent message for {session_id}: {error}");
                    return;
                }
            };
            (agent_message.id.clone(), Some(agent_message))
        }
        #[cfg(not(test))]
        {
            log::error!("queued turn reached the legacy test path in production");
            return;
        }
    };
    let agent_message = if durably_accepted {
        queued_agent_projection(agent_message_id.clone(), human_message.timestamp)
    } else {
        legacy_agent_message.expect("legacy test queue path must materialize its assistant")
    };
    let restore_policy = match context_restore_policy_before_human_message(
        ctx,
        session_id,
        &human_message.id,
        had_runtime,
    ) {
        Ok(policy) => policy,
        Err(error) => {
            log::warn!(
                "queued turn restore preflight remains pending for {session_id}; provider start was not claimed: {error}"
            );
            return;
        }
    };
    let context_was_reinjected =
        matches!(&restore_policy.plan, ContextRestorePlan::Reinject { .. });
    let clear_context_carry_after_start =
        !had_runtime && matches!(&restore_policy.plan, ContextRestorePlan::NoContext);
    let recovery_restore_required = restore_policy.recovery_restore_required;
    let expected_provider_session_generation = restore_policy.expected_provider_session_generation;
    let restore_plan = restore_policy.plan;
    let prompt = apply_restore_prompt_prefix(queued.content.clone(), &restore_plan);
    let selected_model = match had_runtime
        .then(|| selected_model_for_runtime(ctx, &session))
        .transpose()
    {
        Ok(model) => model,
        Err(error) => {
            log::warn!(
                "queued turn model preflight remains pending for {session_id}; provider start was not claimed: {error}"
            );
            return;
        }
    };
    let mut queued_for_turn = queued.clone();
    queued_for_turn.existing_human_message_id = Some(human_message.id.clone());
    queued_for_turn.existing_agent_message_id = Some(agent_message_id.clone());
    if !durably_accepted {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        let Some(front) = state.pending_queue.front_mut() else {
            return;
        };
        if front.id != queued.id {
            return;
        }
        front.existing_human_message_id = queued_for_turn.existing_human_message_id.clone();
        front.existing_agent_message_id = queued_for_turn.existing_agent_message_id.clone();
    }
    let mut generation = None;
    if durably_accepted {
        let (operation_id, obligation_id, driver) = accepted_obligation
            .as_ref()
            .expect("durably accepted queue has an obligation driver");
        let turn_started = AgentSessionEvent::TurnStarted {
            turn_id,
            message_id: human_message.id.clone(),
            assistant_message_id: Some(agent_message_id.clone()),
            prompt: committed_prompt.clone(),
            at: human_message.timestamp,
        };
        match driver
            .claim_queued_turn_execution(
                operation_id,
                obligation_id,
                session_id,
                &queued.id,
                turn_started,
            )
            .await
        {
            Ok(AcceptedQueuedTurnExecutionClaimOutcome::Claimed(claim)) => {
                accepted_claim = Some(claim);
            }
            Ok(AcceptedQueuedTurnExecutionClaimOutcome::Blocked) => {
                log::debug!(
                    "accepted queued turn remains blocked by canonical lifecycle state for {session_id}"
                );
                return;
            }
            Err(()) => {
                log::warn!(
                    "accepted queued turn atomic claim remains pending for {session_id}; recovery was notified"
                );
                return;
            }
        }
        generation = {
            let mut sessions = ctx.sessions.lock().await;
            sessions.get_mut(session_id).and_then(|state| {
                (state.pending_queue.front().map(|front| front.id.as_str())
                    == Some(queued.id.as_str()))
                .then(|| {
                    state.pending_queue.pop_front();
                    state.reset_for_turn(turn_id, agent_message_id.clone());
                    state.current_turn_input = Some(queued_for_turn.clone());
                    state.generation
                })
            })
        };
        if generation.is_none() {
            if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
                arm_accepted_send_recovery_after_claim_release(
                    driver.as_ref(),
                    operation_id,
                    obligation_id,
                    &mut accepted_claim,
                )
                .await;
            }
            log::error!(
                "accepted queued send committed TurnStarted but lost its in-memory queue identity"
            );
            return;
        }
    }
    if !durably_accepted {
        #[cfg(test)]
        {
            // Legacy tests retain the historical split boundary and keep the
            // queue visible until the provider accepts start_turn below.
            if let Err(error) = ctx.session_store.append_turn_started_and_project_state(
                &ctx.data_dir,
                session_id,
                AgentSessionEvent::TurnStarted {
                    turn_id,
                    message_id: human_message.id.clone(),
                    assistant_message_id: Some(agent_message_id.clone()),
                    prompt: committed_prompt,
                    at: human_message.timestamp,
                },
            ) {
                log::warn!("failed to append queued TurnStarted for {session_id}: {error}");
                return;
            }
            generation = {
                let mut sessions = ctx.sessions.lock().await;
                sessions.get_mut(session_id).and_then(|state| {
                    (state.pending_queue.front().map(|front| front.id.as_str())
                        == Some(queued.id.as_str()))
                    .then(|| {
                        state.reset_for_turn(turn_id, agent_message_id.clone());
                        state.current_turn_input = Some(queued_for_turn.clone());
                        state.generation
                    })
                })
            };
        }
        #[cfg(not(test))]
        unreachable!("production queues must have a durable accepted operation identity");
    }
    let Some(generation) = generation else {
        return;
    };
    let runtime = match runtime {
        Some(runtime) => runtime,
        None => {
            let runtime_open_epoch = {
                let mut sessions = ctx.sessions.lock().await;
                let Some(state) = sessions.get_mut(session_id) else {
                    return;
                };
                if state.generation != generation {
                    return;
                }
                state.bump_runtime_epoch()
            };
            match open_runtime_for_session(
                ctx,
                &session,
                system_prompt.clone(),
                Some(runtime_open_epoch),
            )
            .await
            {
                Ok(runtime) => runtime,
                Err(AgentRuntimeError::BackendSessionLost { .. }) => {
                    if let Err(error) = recover_backend_session(
                        ctx,
                        session_id,
                        BackendSessionRecoveryReason::BackendSessionLost,
                    )
                    .await
                    {
                        if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
                            arm_accepted_send_recovery_after_claim_release(
                                driver.as_ref(),
                                operation_id,
                                obligation_id,
                                &mut accepted_claim,
                            )
                            .await;
                        }
                        log::warn!(
                            "failed to recover backend session for queued turn {session_id}: {error}"
                        );
                    }
                    return;
                }
                Err(error) => {
                    log::warn!("failed to reopen runtime for queued turn {session_id}: {error}");
                    let terminal = complete_turn_with_acceptance_and_persist_kind(
                        ctx,
                        session_id,
                        Some(generation),
                        TurnResult::Interrupted {
                            reason: DomainInterruptReason::Crash,
                            error: Some(error.to_string()),
                        },
                        PersistFailureKind::ReopenRuntime,
                    )
                    .await;
                    match terminal {
                        Ok((Some(notification), _)) => {
                            dispatch_workflow_turn_complete_notification(
                                &ctx.workflow_turn_complete_notifier,
                                notification,
                            )
                            .await;
                        }
                        Ok((None, _)) => {}
                        Err(persist_error) => {
                            if let Some((operation_id, obligation_id, driver)) =
                                &accepted_obligation
                            {
                                arm_accepted_send_recovery_after_claim_release(
                                    driver.as_ref(),
                                    operation_id,
                                    obligation_id,
                                    &mut accepted_claim,
                                )
                                .await;
                            }
                            log::error!(
                                "failed to persist queued runtime reopen error for {session_id}: {persist_error}"
                            );
                        }
                    }
                    return;
                }
            }
        }
    };
    if !turn_owns_runtime(ctx, session_id, generation, &runtime).await {
        if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
            arm_accepted_send_recovery_after_claim_release(
                driver.as_ref(),
                operation_id,
                obligation_id,
                &mut accepted_claim,
            )
            .await;
        }
        detach_runtime_if_current(ctx, session_id, &runtime).await;
        return;
    }
    let start_result = async {
        if let Some(model) = selected_model {
            runtime.set_model(&model).await?;
        }
        runtime
            .start_turn(TurnInput {
                prompt,
                images: queued
                    .images
                    .iter()
                    .cloned()
                    .map(|image| AttachmentPayload {
                        data: image.data,
                        media_type: image.media_type,
                    })
                    .collect(),
                system_prompt,
                permission_mode: queued.permission_mode,
                plan_mode: queued.plan_mode,
                permission_profile_id: queued.permission_profile_id.clone(),
                editor_context: queued.editor_context.clone().map(EditorContext::from),
            })
            .await
    }
    .await;
    let _runtime_event_guard = ctx.runtime_event_locks.acquire(session_id).await;
    if let Err(error) = start_result {
        if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
            arm_accepted_send_recovery_after_claim_release(
                driver.as_ref(),
                operation_id,
                obligation_id,
                &mut accepted_claim,
            )
            .await;
        }
        if !turn_runtime_is_current(ctx, session_id, generation, &runtime).await {
            return;
        }
        log::warn!("failed to start queued turn for {session_id}: {error}");
        match complete_turn_with_acceptance_and_persist_kind(
            ctx,
            session_id,
            Some(generation),
            TurnResult::Interrupted {
                reason: DomainInterruptReason::Crash,
                error: Some(error.to_string()),
            },
            PersistFailureKind::QueuedTurnInterrupt,
        )
        .await
        {
            Ok((Some(notification), _)) => {
                dispatch_workflow_turn_complete_notification(
                    &ctx.workflow_turn_complete_notifier,
                    notification,
                )
                .await;
            }
            Ok((None, _)) => {}
            Err(persist_error) => {
                log::warn!(
                    "failed to persist queued turn interruption for {session_id}: {persist_error}"
                );
            }
        }
    } else {
        spawn_stale_watchdog_task(
            ctx,
            session_id.to_string(),
            generation,
            stale_timeout_for_session(&session),
        );
        let runtime_epoch = {
            let sessions = ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .filter(|state| state.generation == generation)
                .map(|state| state.runtime_epoch)
                .unwrap_or_default()
        };
        if let Some((operation_id, obligation_id, _)) = &accepted_obligation {
            mark_accepted_turn_running_or_retry(
                ctx,
                session_id,
                generation,
                operation_id.clone(),
                obligation_id.clone(),
                turn_id,
            );
        }
        if !turn_owns_runtime(ctx, session_id, generation, &runtime).await {
            return;
        }
        #[cfg(test)]
        if !durably_accepted {
            let mut sessions = ctx.sessions.lock().await;
            if let Some(state) = sessions.get_mut(session_id) {
                if state.pending_queue.front().map(|front| front.id.as_str())
                    == Some(queued.id.as_str())
                {
                    state.pending_queue.pop_front();
                }
            }
        }
        ctx.notifier.pending_message_consumed(
            session_id,
            Some(queued.id.clone()),
            Some(human_message.clone()),
            agent_message.clone(),
        );
        ctx.notifier
            .turn_prepared(&session, &human_message, &agent_message);
        drop(_runtime_event_guard);
        complete_context_restore_after_start_or_retry(
            ctx,
            session_id.to_string(),
            runtime_epoch,
            ContextRestoreCompletionRequest::after_started_turn(
                expected_provider_session_generation,
                turn_id,
                context_was_reinjected,
                clear_context_carry_after_start,
                recovery_restore_required,
            ),
        );
        emit_session_state_change_from_session(
            &session,
            &ctx.notifier,
            &ctx.status_center,
            &ctx.status_notifier,
            StateChange {
                turn_phase: TurnPhase::Streaming,
                queue_paused: None,
                pending_permission_request: None,
                pending_permission_state_revision: None,
                exit_code: None,
                completed_at: None,
                interrupted: false,
                session_state: Some(SessionState::Active),
            },
        );
    }
}

fn queued_human_message(queued: &QueuedTurnInput) -> ChatMessage {
    ChatMessage {
        id: queued
            .existing_human_message_id
            .clone()
            .unwrap_or_else(|| queued.id.clone()),
        role: MessageRole::Human,
        content: queued.content.clone(),
        thinking: None,
        activities: None,
        parts: (!human_parts(&queued.content, &queued.images).is_empty())
            .then(|| human_parts(&queued.content, &queued.images)),
        streaming_final_seq: 0,
        timestamp: queued.created_at,
        mentions: (!queued.mentions.is_empty()).then(|| {
            queued
                .mentions
                .iter()
                .cloned()
                .map(crate::usecase::agent_session::session::MessageMention::from_domain)
                .collect()
        }),
    }
}

fn queued_agent_projection(message_id: String, timestamp: f64) -> ChatMessage {
    ChatMessage {
        id: message_id,
        role: MessageRole::Agent,
        content: String::new(),
        thinking: None,
        activities: None,
        parts: None,
        streaming_final_seq: 0,
        timestamp,
        mentions: None,
    }
}

#[cfg(test)]
fn queued_agent_message(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
    queued: &QueuedTurnInput,
) -> Result<ChatMessage, String> {
    if let Some(message_id) = queued.existing_agent_message_id.as_deref() {
        if let Some(message) = session_store
            .load_full_session_for_restore(data_dir, session_id)?
            .and_then(|session| {
                session
                    .messages
                    .into_iter()
                    .find(|message| message.id == message_id)
            })
        {
            return Ok(message);
        }
    }
    add_message_internal(
        session_store,
        data_dir,
        session_id,
        MessageRole::Agent,
        "",
        None,
        None,
    )
}

fn committed_queued_message(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
    message_id: &str,
    expected_role: MessageRole,
) -> Result<Option<ChatMessage>, String> {
    if let Some(message) = session_store.canonical_message_projection(session_id, message_id)? {
        return (message.role == expected_role)
            .then_some(message)
            .ok_or_else(|| "committed queued message role is incompatible".to_string())
            .map(Some);
    }
    let message = session_store
        .load_full_session_for_restore(data_dir, session_id)?
        .and_then(|session| {
            session
                .messages
                .into_iter()
                .find(|message| message.id == message_id)
        });
    match message {
        Some(message) if message.role == expected_role => Ok(Some(message)),
        Some(_) => Err("committed queued message role is incompatible".to_string()),
        None => Ok(None),
    }
}

fn build_queued_system_prompt(
    session_store: &Arc<SessionStore>,
    branch_diff_context: Option<&dyn BranchDiffContextPort>,
    instruction_source: &dyn InstructionSourcePort,
    data_dir: &Path,
    session: &ChatSession,
    queued: &QueuedTurnInput,
) -> Result<Option<String>, String> {
    let backend_id = session
        .backend_id
        .as_deref()
        .ok_or_else(|| format!("Session {} is missing backend id", session.id))?;
    let built = build_session_system_prompt(SessionSystemPromptBuildRequest {
        session_store,
        data_dir,
        session,
        branch_diff_context,
        instruction_source,
        backend_id,
        model_id: session.selected_model.as_deref(),
        mentions: &queued.mentions,
        editor_context: queued
            .editor_context
            .as_ref()
            .and_then(system_context_editor_input),
        workflow_instructions: Vec::new(),
    })?;
    let prompt = compose_system_prompt(None, &built.system_context);
    persist_session_system_prompt_build(session_store, data_dir, &session.id, &built)?;
    Ok(prompt)
}

fn permission_request_from_parts(
    parts: &[MessagePart],
    request_id: &str,
) -> Option<PermissionRequestMsg> {
    parts.iter().rev().find_map(|part| match part {
        MessagePart::Permission { request, .. } if request.id == request_id => {
            Some(super::event_apply::permission_request_msg(request))
        }
        _ => None,
    })
}

fn pending_queue_view(state: &RuntimeSessionState) -> Vec<QueuedAgentTurn> {
    state
        .pending_queue
        .iter()
        .map(QueuedAgentTurn::from)
        .collect()
}

#[cfg(test)]
fn add_human_message_internal(
    session_store: &SessionStore,
    data_dir: &Path,
    session_id: &str,
    content: &str,
    images: &[ImageAttachment],
    mentions: &[crate::domain::code::MentionReference],
) -> Result<(ChatMessage, SessionMeta), AgentRuntimeError> {
    let parts = human_parts(content, images);
    add_message_with_meta_internal(
        session_store,
        data_dir,
        session_id,
        MessageRole::Human,
        content,
        (!parts.is_empty()).then_some(parts),
        (!mentions.is_empty()).then_some(mentions.to_vec()),
    )
    .map_err(AgentRuntimeError::Other)
}

fn human_parts(content: &str, images: &[ImageAttachment]) -> Vec<MessagePart> {
    if images.is_empty() {
        return Vec::new();
    }
    let mut parts = Vec::new();
    if !content.is_empty() {
        parts.push(MessagePart::Text {
            content: content.to_string(),
            parent_tool_use_id: None,
        });
    }
    parts.extend(images.iter().map(|image| MessagePart::Image {
        data: image.data.clone(),
        media_type: image.media_type.clone(),
    }));
    parts
}

pub(super) fn required_backend_id(session: &ChatSession) -> Result<String, AgentRuntimeError> {
    session.backend_id.clone().ok_or_else(|| {
        AgentRuntimeError::Other(format!("Session {} is missing backend id", session.id))
    })
}

fn system_context_editor_input(context: &AgentEditorContext) -> Option<SystemContextEditorInput> {
    Some(SystemContextEditorInput {
        active_editor_path: context.active_editor_path.clone(),
        open_editor_paths: context.open_editor_paths.clone(),
        selection_file_path: context
            .selection
            .as_ref()
            .map(|selection| selection.file_path.clone()),
        payload: serde_json::to_string(context).ok(),
    })
}

fn compose_system_prompt(
    system_prompt: Option<String>,
    context: &BuiltSystemContext,
) -> Option<String> {
    let context_blocks = context
        .snapshots
        .iter()
        .filter_map(system_context_block)
        .collect::<Vec<_>>();
    let context_prompt = (!context_blocks.is_empty()).then(|| context_blocks.join("\n\n"));
    let system_prompt = system_prompt.filter(|prompt| !prompt.trim().is_empty());

    match (system_prompt, context_prompt) {
        (Some(prompt), Some(context_prompt)) => Some(format!("{prompt}\n\n{context_prompt}")),
        (None, Some(context_prompt)) => Some(context_prompt),
        (Some(prompt), _) => Some(prompt),
        (None, None) => None,
    }
}

fn system_context_block(snapshot: &ContextSnapshot) -> Option<String> {
    let payload = snapshot.payload.trim();
    if payload.is_empty() {
        return None;
    }
    let tag = match snapshot.kind {
        ContextSourceKind::RepoSummary => "releash_repo_summary",
        ContextSourceKind::DiffReviewSnapshot => "releash_diff_review_snapshot",
        ContextSourceKind::OpenEditorSelection => "releash_open_editor_selection",
        ContextSourceKind::Mentions => "releash_mentions",
        ContextSourceKind::TerminalLogSummary => "releash_terminal_log_summary",
        ContextSourceKind::WorkflowContext => "releash_workflow_state",
        ContextSourceKind::ProjectInstructions => "releash_project_instructions",
        ContextSourceKind::BackendModelIdentity => "releash_backend_model_identity",
    };
    Some(format!("<{tag}>\n{payload}\n</{tag}>"))
}

impl From<AgentEditorContext> for EditorContext {
    fn from(value: AgentEditorContext) -> Self {
        Self {
            active_editor_path: value.active_editor_path,
            open_editor_paths: value.open_editor_paths,
            selection: value.selection.map(|selection| {
                crate::domain::agent_session::value_objects::EditorSelection {
                    file_path: selection.file_path,
                    start_line: selection.start_line,
                    end_line: selection.end_line,
                }
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct StateChange {
    pub(super) turn_phase: TurnPhase,
    pub(super) queue_paused: Option<bool>,
    pub(super) pending_permission_request: Option<PermissionRequestMsg>,
    pub(super) pending_permission_state_revision: Option<u64>,
    pub(super) exit_code: Option<i64>,
    pub(super) completed_at: Option<f64>,
    pub(super) interrupted: bool,
    pub(super) session_state: Option<SessionState>,
}

#[derive(Debug, Clone)]
struct TerminalProjection {
    exit_code: i64,
    interrupted: bool,
    pause_queue: bool,
    session_state: SessionState,
    event: TerminalEventProjection,
}

#[derive(Debug, Clone)]
enum TerminalEventProjection {
    Completed {
        stop_reason: Option<EventTurnStopReason>,
        token_usage: Option<TurnTokenUsage>,
    },
    Interrupted {
        reason: EventInterruptReason,
        error: Option<String>,
    },
}

fn next_turn_id(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
) -> Result<u64, String> {
    session_store
        .next_turn_id(data_dir, session_id)
        .map_err(|error| error.to_string())
}

struct PendingPermissionForResponse {
    turn_id: Option<u64>,
    from_runtime_state: bool,
}

fn durable_part_events(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
    turn_id: u64,
    message_id: &str,
    parts: &[MessagePart],
) -> Result<Vec<AgentSessionEvent>, String> {
    if !parts.iter().any(part_records_durable_event) {
        return Ok(Vec::new());
    }
    let mut events = if parts.iter().any(part_needs_event_history) {
        session_store.load_session_events(data_dir, session_id)?
    } else {
        Vec::new()
    };
    let before = events.len();
    append_part_events(
        &mut events,
        turn_id,
        message_id,
        parts,
        PartEventMode::DurableOnly,
    );
    Ok(events.into_iter().skip(before).collect())
}

fn part_records_durable_event(part: &MessagePart) -> bool {
    matches!(
        part,
        MessagePart::ToolUse { .. }
            | MessagePart::ToolResult { .. }
            | MessagePart::Permission { .. }
            | MessagePart::TaskStatus { .. }
            | MessagePart::TodoListSnapshot { .. }
            | MessagePart::SystemNotification { .. }
            | MessagePart::Image { .. }
            | MessagePart::ImageRef { .. }
    )
}

fn part_needs_event_history(part: &MessagePart) -> bool {
    matches!(
        part,
        MessagePart::ToolUse { tool, .. } if tool != "Edit"
    )
}

fn patch_permission_response_in_state(
    state: &mut RuntimeSessionState,
    response: &PermissionResponse,
) -> Option<(String, u64, Vec<MessagePart>, u64)> {
    if !patch_permission_response_in_domain_parts(&mut state.domain_streaming_parts, response) {
        return None;
    }
    state.streaming_parts = parts_from_domain(state.domain_streaming_parts.clone());
    state.streaming_delta_seq = state.streaming_delta_seq.saturating_add(1);
    let message_id = state
        .streaming_message_id
        .clone()
        .or_else(|| state.last_agent_message_id.clone())?;
    let turn_id = state.current_turn_id?;
    Some((
        message_id,
        state.streaming_delta_seq,
        state.streaming_parts.clone(),
        turn_id,
    ))
}

fn patch_permission_response_in_domain_parts(
    parts: &mut [DomainMessagePart],
    response: &PermissionResponse,
) -> bool {
    let decision = permission_decision_from_response(response);
    let answers = permission_answers_from_response(response);
    let mut patched = false;
    for part in parts {
        let DomainMessagePart::Permission {
            request,
            status,
            answers: part_answers,
            parent_tool_use_id,
        } = part
        else {
            continue;
        };
        if request.id != response.request_id {
            continue;
        }
        request.status = PermissionRequestStatus::Resolved {
            decision,
            answers: answers.clone(),
        };
        *status = match decision {
            DomainPermissionDecision::Allowed => PermissionPartStatus::Allowed,
            DomainPermissionDecision::Denied => PermissionPartStatus::Denied,
            DomainPermissionDecision::Cancelled => PermissionPartStatus::Cancelled,
        };
        *part_answers = answers.clone();
        request.parent_tool_use_id = parent_tool_use_id.clone();
        patched = true;
    }
    patched
}

fn permission_decision_from_response(response: &PermissionResponse) -> DomainPermissionDecision {
    match &response.decision {
        PermissionResponseDecision::Allow { .. } => DomainPermissionDecision::Allowed,
        PermissionResponseDecision::Deny { .. } => DomainPermissionDecision::Denied,
    }
}

fn permission_answers_from_response(
    response: &PermissionResponse,
) -> Option<crate::domain::agent_session::value_objects::JsonPayload> {
    match &response.decision {
        PermissionResponseDecision::Allow { answers, .. } => answers.clone(),
        PermissionResponseDecision::Deny { .. } => None,
    }
}

#[cfg(test)]
fn permission_resolved_event(turn_id: u64, response: &PermissionResponse) -> AgentSessionEvent {
    let decision = match &response.decision {
        PermissionResponseDecision::Allow { .. } => {
            crate::usecase::agent_session::event_log::PermissionDecision::Allowed
        }
        PermissionResponseDecision::Deny { .. } => {
            crate::usecase::agent_session::event_log::PermissionDecision::Denied
        }
    };
    let answers = permission_answers_from_response(response);
    AgentSessionEvent::PermissionResolved {
        turn_id,
        tool_use_id: None,
        request_id: Some(response.request_id.clone()),
        decision,
        answers,
    }
}

fn resync_permission_mode(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
    reported_mode: PermissionMode,
) -> Option<PermissionMode> {
    let meta = match session_store.get_session_meta(data_dir, session_id) {
        Ok(meta) => meta,
        Err(error) => {
            log::warn!("failed to load permission mode for {session_id}: {error}");
            return None;
        }
    }?;
    let saved_mode = match PermissionMode::parse(&meta.permission_mode) {
        Ok(mode) => mode,
        Err(error) => {
            log::warn!("stored permission mode is invalid for {session_id}: {error}");
            return None;
        }
    };
    if saved_mode == reported_mode {
        return None;
    }
    Some(saved_mode)
}

fn final_turn_events(
    ctx: &RuntimeContext,
    session_id: &str,
    turn_id: u64,
    message_id: &str,
    parts: &[MessagePart],
    terminal: &TerminalProjection,
    completed_at: f64,
) -> Result<Vec<AgentSessionEvent>, String> {
    let existing_events = ctx
        .session_store
        .load_current_reducer_events(&ctx.data_dir, session_id)?;
    if existing_events.iter().any(|event| {
        matches!(
            event,
            AgentSessionEvent::TurnCompleted { turn_id: id, .. }
                | AgentSessionEvent::TurnInterrupted { turn_id: id, .. } if *id == turn_id
        )
    }) {
        return Ok(Vec::new());
    }
    let queue_was_paused = TurnEventLog::from_events(existing_events.clone())
        .project()
        .queue_paused_at
        .is_some();
    let mut appended = vec![AgentSessionEvent::FinalPartsRecorded {
        turn_id,
        message_id: message_id.to_string(),
        parts: parts.to_vec(),
    }];
    match &terminal.event {
        TerminalEventProjection::Completed {
            stop_reason,
            token_usage,
        } => {
            appended.push(AgentSessionEvent::TurnCompleted {
                turn_id,
                exit_code: terminal.exit_code,
                stop_reason: *stop_reason,
                token_usage: *token_usage,
            });
        }
        TerminalEventProjection::Interrupted { reason, error } => {
            let mut events = existing_events;
            events.extend(appended.iter().cloned());
            let before = events.len();
            finalize_turn(
                &mut events,
                turn_id,
                *reason,
                error.clone(),
                terminal.exit_code,
            );
            appended.extend(events.into_iter().skip(before));
        }
    }
    if terminal.pause_queue && !queue_was_paused {
        appended.push(AgentSessionEvent::QueuePaused { at: completed_at });
    }
    Ok(appended)
}

fn terminal_projection(result: &TurnResult) -> TerminalProjection {
    match result {
        TurnResult::Completed {
            stop_reason,
            token_usage,
        } => TerminalProjection {
            exit_code: 0,
            interrupted: false,
            pause_queue: false,
            session_state: SessionState::Done,
            event: TerminalEventProjection::Completed {
                stop_reason: stop_reason.map(map_turn_stop_reason),
                token_usage: token_usage.map(turn_token_usage_from_domain),
            },
        },
        TurnResult::Failed { token_usage, .. } => TerminalProjection {
            exit_code: 1,
            interrupted: false,
            pause_queue: true,
            session_state: SessionState::Error,
            event: TerminalEventProjection::Completed {
                stop_reason: None,
                token_usage: token_usage.map(turn_token_usage_from_domain),
            },
        },
        TurnResult::Interrupted { reason, error } => {
            let (exit_code, session_state, event_reason) = match reason {
                DomainInterruptReason::Abort => {
                    (0, SessionState::Idle, EventInterruptReason::Abort)
                }
                DomainInterruptReason::Timeout => {
                    (124, SessionState::Error, EventInterruptReason::Timeout)
                }
                DomainInterruptReason::Crash => {
                    (1, SessionState::Error, EventInterruptReason::Crash)
                }
                DomainInterruptReason::SessionClosed => {
                    (0, SessionState::Idle, EventInterruptReason::SessionClosed)
                }
            };
            TerminalProjection {
                exit_code,
                interrupted: true,
                pause_queue: true,
                session_state,
                event: TerminalEventProjection::Interrupted {
                    reason: event_reason,
                    error: error.clone(),
                },
            }
        }
    }
}

fn map_turn_stop_reason(reason: DomainTurnStopReason) -> EventTurnStopReason {
    match reason {
        DomainTurnStopReason::Refusal => EventTurnStopReason::Refusal,
    }
}

fn turn_token_usage_from_domain(
    usage: crate::domain::agent_session::entities::TokenUsage,
) -> TurnTokenUsage {
    TurnTokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

fn workflow_turn_complete_notification(
    session_id: &str,
    input: &WorkflowTurnCompleteInput,
) -> WorkflowTurnCompleteNotification {
    WorkflowTurnCompleteNotification {
        chat_session_id: session_id.to_string(),
        exit_code: input.exit_code,
        final_text_parts: input.final_text_parts.clone(),
        failure_signal: input.failure_signal.map(|signal| match signal {
            crate::usecase::agent_session::event_log::AgentTurnFailureSignal::ModelRefusal => {
                WorkflowTurnFailureSignal::ModelRefusal
            }
        }),
        token_usage: input.token_usage.map(|usage| WorkflowTurnTokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
        }),
        interrupted: input.interrupted,
    }
}

fn workflow_stall_observed_notification(
    payload: &AgentStallObservedPayload,
) -> WorkflowStallObservedNotification {
    WorkflowStallObservedNotification {
        chat_session_id: payload.chat_session_id.clone(),
        turn_phase: match payload.turn_phase {
            TurnPhase::Idle => "idle",
            TurnPhase::Streaming => "streaming",
            TurnPhase::WaitingPermission => "waiting_permission",
        }
        .to_string(),
        idle_secs: payload.idle_secs,
        signal_count: payload.signal_count,
        cap_reached: payload.cap_reached,
    }
}

fn workflow_stall_cleared_notification(session_id: &str) -> WorkflowStallClearedNotification {
    WorkflowStallClearedNotification {
        chat_session_id: session_id.to_string(),
    }
}

pub(super) fn emit_session_state_change(
    session_store: &Arc<SessionStore>,
    notifier: &Arc<dyn AgentSessionEventNotifier>,
    status_center: &Arc<AgentStatusCenter>,
    status_notifier: &Arc<dyn AgentStatusNotifier>,
    data_dir: &Path,
    session_id: &str,
    change: StateChange,
) {
    notifier.session_state_changed(AgentSessionStateChangedPayload {
        chat_session_id: session_id.to_string(),
        turn_phase: change.turn_phase,
        exit_code: change.exit_code,
        completed_at: change.completed_at,
        interrupted: change.interrupted,
        session_state: change.session_state.clone(),
        queue_paused: change.queue_paused,
        pending_permission_request: change.pending_permission_request.clone(),
        pending_permission_state_revision: change.pending_permission_state_revision,
    });
    publish_status_change(
        session_store,
        status_center,
        status_notifier,
        data_dir,
        session_id,
        change,
    );
}

fn emit_session_state_change_from_session(
    session: &ChatSession,
    notifier: &Arc<dyn AgentSessionEventNotifier>,
    status_center: &Arc<AgentStatusCenter>,
    status_notifier: &Arc<dyn AgentStatusNotifier>,
    change: StateChange,
) {
    notifier.session_state_changed(AgentSessionStateChangedPayload {
        chat_session_id: session.id.clone(),
        turn_phase: change.turn_phase,
        exit_code: change.exit_code,
        completed_at: change.completed_at,
        interrupted: change.interrupted,
        session_state: change.session_state.clone(),
        queue_paused: change.queue_paused,
        pending_permission_request: change.pending_permission_request.clone(),
        pending_permission_state_revision: change.pending_permission_state_revision,
    });
    publish_status_change_from_session(session, status_center, status_notifier, change);
}

fn publish_status_change(
    session_store: &Arc<SessionStore>,
    status_center: &Arc<AgentStatusCenter>,
    status_notifier: &Arc<dyn AgentStatusNotifier>,
    data_dir: &Path,
    session_id: &str,
    change: StateChange,
) {
    let session = match session_store.get_session_shell(data_dir, session_id) {
        Ok(Some(session)) => session,
        Ok(None) => return,
        Err(error) => {
            log::warn!("failed to load session for status update {session_id}: {error}");
            return;
        }
    };
    publish_status_change_from_session(&session, status_center, status_notifier, change);
}

fn publish_status_change_from_session(
    session: &ChatSession,
    status_center: &Arc<AgentStatusCenter>,
    status_notifier: &Arc<dyn AgentStatusNotifier>,
    change: StateChange,
) {
    let session_state = change
        .session_state
        .unwrap_or_else(|| session.state.clone());
    let worktree_path = session.worktree_path.clone();
    let workflow_context = session.workflow_node_context.clone();
    let workflow_execution_status = match change.turn_phase {
        TurnPhase::Streaming | TurnPhase::WaitingPermission => {
            workflow_context.as_ref().map(|_| "running".to_string())
        }
        TurnPhase::Idle => None,
    };
    let status = SessionStatus {
        chat_session_id: session.id.clone(),
        worktree_id: worktree_path.clone(),
        worktree_path,
        pty_id: None,
        agent_state: AgentStatusCenter::derive_agent_state(
            change.turn_phase,
            session_state.clone(),
        ),
        turn_phase: TurnPhaseRepr::from(change.turn_phase),
        session_state,
        pending_permission: matches!(change.turn_phase, TurnPhase::WaitingPermission),
        pending_permission_request: change.pending_permission_request,
        last_activity_at: crate::usecase::agent_session::session::now_timestamp(),
        workflow_node: workflow_context
            .as_ref()
            .map(|context| context.node_name.clone()),
        workflow_execution_status,
        workflow_execution_id: workflow_context
            .as_ref()
            .map(|context| context.execution_id.clone()),
        node_execution_id: workflow_context
            .as_ref()
            .map(|context| context.node_execution_id.clone()),
        workflow_attempt: workflow_context.as_ref().map(|context| context.attempt),
        notice: None,
        workflow_node_progress: None,
    };
    status_notifier.status_changed(status_center.update_session(status));
}

fn session_telemetry_dimensions(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
) -> Option<crate::other::telemetry::AgentTurnDimensions> {
    let session = session_store
        .get_session_shell(data_dir, session_id)
        .ok()
        .flatten()?;
    Some(crate::other::telemetry::AgentTurnDimensions {
        resume: session.agent_session_id.is_some(),
        has_session: true,
        permission_mode: crate::other::telemetry::PermissionModeDim::normalize(
            &session.permission_mode,
        ),
        model: crate::other::telemetry::ModelFamily::normalize(session.selected_model.as_deref()),
        context: crate::other::telemetry::TurnContext::from_workflow_node(
            session.is_workflow_node_session(),
        ),
        channel: crate::other::telemetry::Payload::TauriEvent,
        warm_path: crate::other::telemetry::WarmPath::QueryDirect,
    })
}

#[cfg(test)]
struct TestNoopAgentRuntime;

#[cfg(test)]
#[async_trait::async_trait]
impl AgentSessionRuntime for TestNoopAgentRuntime {
    fn take_events(
        &mut self,
    ) -> std::pin::Pin<Box<dyn futures_util::Stream<Item = AgentRuntimeEvent> + Send>> {
        Box::pin(futures_util::stream::empty())
    }

    async fn start_turn(&self, _input: TurnInput) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn interrupt(&self) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn respond_permission(
        &self,
        _response: PermissionResponse,
    ) -> Result<(), AgentBackendError> {
        Err(AgentBackendError::Other(
            "injected test permission failure".to_string(),
        ))
    }

    async fn set_model(&self, _model: &ModelId) -> Result<(), AgentBackendError> {
        Err(AgentBackendError::Other(
            "injected test model failure".to_string(),
        ))
    }

    async fn close(&self) {}
}

#[cfg(test)]
struct TestFailingAgentRuntime;

#[cfg(test)]
#[async_trait::async_trait]
impl AgentSessionRuntime for TestFailingAgentRuntime {
    fn take_events(
        &mut self,
    ) -> std::pin::Pin<Box<dyn futures_util::Stream<Item = AgentRuntimeEvent> + Send>> {
        Box::pin(futures_util::stream::empty())
    }

    async fn start_turn(&self, _input: TurnInput) -> Result<(), AgentBackendError> {
        Err(AgentBackendError::Other(
            "injected test start failure".to_string(),
        ))
    }

    async fn interrupt(&self) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn respond_permission(
        &self,
        _response: PermissionResponse,
    ) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn set_model(&self, _model: &ModelId) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn close(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::agent_session::session_storage::{
        AgentSessionProjectionCodecV1, FileSessionStorage,
    };
    use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
    use crate::adaptor::gateway::workflow::StoredWorkspaceSessionGateway;
    use crate::domain::agent_session::gateway::{AgentBackend, AgentSessionRuntime};
    use crate::domain::agent_session::value_objects::{
        BackendCapabilities, ModelDescriptor, SkillEntry,
    };
    use crate::domain::local_event::{
        CommitIdentity, CommitOperationKind, IdempotencyBinding, LocalAtomicBatch,
        LocalEventTransactionRepository, LocalStateMutation, ObligationMutation, PendingIndexEntry,
        PendingPartition, Revision, RevisionGuard,
    };
    use crate::domain::workflow::WorkflowNodeContext;
    use crate::test_support::{
        build_agent_runtime_usecase_with_controller,
        build_agent_runtime_usecase_with_controller_and_notifiers,
        build_agent_runtime_usecase_with_controller_and_spawner, build_session_store,
        TestRuntimeCallKind,
    };
    use crate::usecase::agent_session::runtime::ports::{
        AgentSessionEventNotifier, AgentSessionStateChangedPayload, AgentStallObservedPayload,
        AgentStreamingDeltaPayload, WorkflowStallNotifier,
    };
    use crate::usecase::agent_session::session::{
        create_session_internal_with_attributes, ChatMessage, MessagePart, PermissionPartStatus,
        PermissionRequestKindMsg, PermissionRequestMsg, SessionCreationAttributes,
        SystemNotificationType,
    };
    use crate::usecase::agent_session::status::{
        AgentStatusChanges, AgentStatusNotifier, TurnPhaseRepr,
    };
    use crate::usecase::workflow::ports::{
        WorkflowStallClearedNotification, WorkflowStallObservedNotification,
    };
    use crate::usecase::workflow::{WorkspaceSessionGateway, WorkspaceSessionState};
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Condvar, Mutex};
    use std::time::Duration;
    use tokio::sync::Notify;

    struct TokioSpawner;

    impl AgentTaskSpawner for TokioSpawner {
        fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
            tokio::spawn(future);
        }
    }

    struct DroppingSpawner;

    impl AgentTaskSpawner for DroppingSpawner {
        fn spawn(&self, _future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {}
    }

    struct EmptyInstructionSource;

    impl InstructionSourcePort for EmptyInstructionSource {
        fn read_instruction_file(
            &self,
            _path: &Path,
            _worktree_root: &Path,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn instruction_cache_key(&self, _worktree_root: &Path) -> Option<String> {
            None
        }
    }

    fn test_session_runtime_locks() -> SessionRuntimeLocks {
        Arc::new(SessionCommandLocks::default())
    }

    fn accepted_queued_input(
        queue_item_id: &str,
        human_message_id: &str,
        turn_id: u64,
    ) -> QueuedTurnInput {
        let mut queued = QueuedTurnInput::new(
            queue_item_id.to_string(),
            PermissionMode::Edit,
            false,
            None,
            Vec::new(),
            "/repo".to_string(),
            Vec::new(),
            None,
        );
        queued.id = queue_item_id.to_string();
        queued.existing_human_message_id = Some(human_message_id.to_string());
        queued.existing_agent_message_id = Some(format!("{human_message_id}:agent"));
        queued.reserved_turn_id = Some(turn_id);
        queued.accepted_operation_id = Some(format!("operation-{turn_id}"));
        queued.execution_obligation_id = Some(format!("operation-{turn_id}.exec"));
        queued
    }

    #[test]
    fn accepted_queue_restoration_uses_canonical_order_not_arrival_order() {
        let canonical = vec![
            CanonicalQueuedSend {
                queue_item_id: "queue-2".to_string(),
                human_message_id: "human-2".to_string(),
                reserved_turn_id: "2".to_string(),
                input_ref: "input-2".to_string(),
            },
            CanonicalQueuedSend {
                queue_item_id: "queue-3".to_string(),
                human_message_id: "human-3".to_string(),
                reserved_turn_id: "3".to_string(),
                input_ref: "input-3".to_string(),
            },
        ];
        let mut pending = std::collections::VecDeque::new();
        let later = accepted_queued_input("queue-3", "human-3", 3);
        insert_accepted_queue_in_canonical_order(&mut pending, later.clone(), &canonical).unwrap();
        assert!(
            !queued_input_matches_canonical_entry(
                pending.front().unwrap(),
                canonical.first().unwrap()
            ),
            "a later item restored alone must not satisfy the canonical front fence"
        );

        insert_accepted_queue_in_canonical_order(
            &mut pending,
            accepted_queued_input("queue-2", "human-2", 2),
            &canonical,
        )
        .unwrap();
        assert_eq!(
            pending
                .iter()
                .map(|queued| queued.id.as_str())
                .collect::<Vec<_>>(),
            vec!["queue-2", "queue-3"]
        );

        insert_accepted_queue_in_canonical_order(&mut pending, later, &canonical).unwrap();
        assert_eq!(
            pending
                .iter()
                .map(|queued| queued.id.as_str())
                .collect::<Vec<_>>(),
            vec!["queue-2", "queue-3"],
            "same-effect restoration must remain idempotent"
        );
        assert!(insert_accepted_queue_in_canonical_order(
            &mut pending,
            accepted_queued_input("queue-4", "human-4", 4),
            &canonical,
        )
        .is_err());
    }

    #[tokio::test]
    async fn shutdown_admission_notifies_all_registered_idle_waiters() {
        let admission = Arc::new(ShutdownAdmission::default());
        let guard = admission.admit().unwrap();
        let first_waiter = admission.idle.notified();
        let second_waiter = admission.idle.notified();
        tokio::pin!(first_waiter);
        tokio::pin!(second_waiter);
        first_waiter.as_mut().enable();
        second_waiter.as_mut().enable();

        drop(guard);

        tokio::time::timeout(Duration::from_secs(1), async {
            first_waiter.await;
            second_waiter.await;
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn released_session_runtime_lock_is_pruned_on_the_next_acquire() {
        let locks = test_session_runtime_locks();
        let released = acquire_session_runtime_lock(&locks, "released").await;
        assert!(locks.contains_for_test("released").await);

        drop(released);
        let active = acquire_session_runtime_lock(&locks, "active").await;

        assert!(!locks.contains_for_test("released").await);
        assert!(locks.contains_for_test("active").await);
        drop(active);
    }

    #[test]
    fn dropping_session_runtime_lock_without_a_runtime_still_schedules_prune() {
        let locks = test_session_runtime_locks();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let released = runtime.block_on(acquire_session_runtime_lock(&locks, "released"));

        drop(runtime);
        drop(released);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let active = acquire_session_runtime_lock(&locks, "active").await;
            assert!(!locks.contains_for_test("released").await);
            assert!(locks.contains_for_test("active").await);
            drop(active);
        });
    }

    #[tokio::test]
    async fn session_runtime_locks_serialize_one_session_and_keep_sessions_independent() {
        let locks = test_session_runtime_locks();
        let first = acquire_session_runtime_lock(&locks, "session-a").await;
        let waiter_locks = Arc::clone(&locks);
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let guard = acquire_session_runtime_lock(&waiter_locks, "session-a").await;
                acquired_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                drop(guard);
            });
        });

        assert!(acquired_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(first);
        acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let other = acquire_session_runtime_lock(&locks, "session-b").await;
        assert!(
            locks.is_held_for_test("session-a"),
            "an actively held session lock must remain in the registry"
        );
        assert!(locks.contains_for_test("session-b").await);
        drop(other);

        release_tx.send(()).unwrap();
        waiter.join().unwrap();

        let final_guard = acquire_session_runtime_lock(&locks, "final").await;
        assert!(!locks.contains_for_test("session-a").await);
        drop(final_guard);
    }

    #[tokio::test]
    async fn repeated_session_runtime_locks_do_not_accumulate_registry_entries() {
        let locks = test_session_runtime_locks();

        for index in 0..100 {
            let guard = acquire_session_runtime_lock(&locks, &format!("session-{index}")).await;
            assert_eq!(locks.len_for_test().await, 1);
            drop(guard);
        }

        let final_guard = acquire_session_runtime_lock(&locks, "final").await;
        assert_eq!(locks.len_for_test().await, 1);
        assert!(locks.contains_for_test("final").await);
        drop(final_guard);
    }

    #[tokio::test]
    #[should_panic(expected = "session runtime lock re-entry is forbidden")]
    async fn session_runtime_lock_reentry_is_detected_in_tests() {
        let locks = test_session_runtime_locks();
        let _first = acquire_session_runtime_lock(&locks, "session-a").await;
        let _second = acquire_session_runtime_lock(&locks, "session-b").await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn session_runtime_lock_reentry_is_detected_on_a_multi_thread_runtime() {
        let locks = test_session_runtime_locks();
        let task = tokio::spawn(async move {
            let _first = acquire_session_runtime_lock(&locks, "session-a").await;
            tokio::task::yield_now().await;
            let _second = acquire_session_runtime_lock(&locks, "session-b").await;
        });

        let error = task.await.expect_err("re-entry must panic");
        assert!(error.is_panic());
    }

    #[tokio::test]
    async fn concurrently_polled_session_runtime_lock_acquires_detect_reentry() {
        let locks = test_session_runtime_locks();
        let holder_a_locks = Arc::clone(&locks);
        let holder_b_locks = Arc::clone(&locks);
        let (holder_a_ready_tx, holder_a_ready_rx) = tokio::sync::oneshot::channel();
        let (holder_b_ready_tx, holder_b_ready_rx) = tokio::sync::oneshot::channel();
        let (release_holder_a_tx, release_holder_a_rx) = tokio::sync::oneshot::channel();
        let (release_holder_b_tx, release_holder_b_rx) = tokio::sync::oneshot::channel();
        let holder_a = tokio::spawn(async move {
            let guard = acquire_session_runtime_lock(&holder_a_locks, "session-a").await;
            holder_a_ready_tx.send(()).unwrap();
            release_holder_a_rx.await.unwrap();
            drop(guard);
        });
        let holder_b = tokio::spawn(async move {
            let guard = acquire_session_runtime_lock(&holder_b_locks, "session-b").await;
            holder_b_ready_tx.send(()).unwrap();
            release_holder_b_rx.await.unwrap();
            drop(guard);
        });
        holder_a_ready_rx.await.unwrap();
        holder_b_ready_rx.await.unwrap();

        let reentry_locks = Arc::clone(&locks);
        let reentry = tokio::spawn(async move {
            let acquire_a = acquire_session_runtime_lock(&reentry_locks, "session-a");
            let acquire_b = acquire_session_runtime_lock(&reentry_locks, "session-b");
            tokio::join!(acquire_a, acquire_b)
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !reentry.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("parallel re-entry must be detected before either session lock is released");

        release_holder_a_tx.send(()).unwrap();
        holder_a.await.unwrap();
        release_holder_b_tx.send(()).unwrap();
        holder_b.await.unwrap();

        let error = match reentry.await {
            Ok(_) => panic!("parallel re-entry must panic"),
            Err(error) => error,
        };
        assert!(error.is_panic());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn session_runtime_lock_drop_removes_task_ownership_from_another_thread() {
        let locks = test_session_runtime_locks();
        let task = tokio::spawn(async move {
            let first = acquire_session_runtime_lock(&locks, "session-a").await;
            tokio::task::spawn_blocking(move || drop(first))
                .await
                .unwrap();

            let second = acquire_session_runtime_lock(&locks, "session-b").await;
            drop(second);
        });

        task.await.unwrap();
    }

    #[tokio::test]
    #[should_panic(expected = "session runtime lock re-entry is forbidden")]
    async fn transferred_session_runtime_lock_detects_reentry_in_the_receiving_flow() {
        let locks = test_session_runtime_locks();
        let task_locks = Arc::clone(&locks);
        let mut first =
            tokio::spawn(
                async move { acquire_session_runtime_lock(&task_locks, "session-a").await },
            )
            .await
            .unwrap();
        first.adopt_for_current_test_flow();

        let _second = acquire_session_runtime_lock(&locks, "session-b").await;
    }

    #[tokio::test]
    async fn sequential_session_runtime_lock_acquires_are_not_reentry() {
        let locks = test_session_runtime_locks();
        let first = acquire_session_runtime_lock(&locks, "session-a").await;
        drop(first);

        let second = acquire_session_runtime_lock(&locks, "session-b").await;
        drop(second);
    }

    #[test]
    fn authoritative_snapshot_retry_preserves_backend_message_and_parts() {
        let message = crate::usecase::agent_session::event_log::session_error_message(
            "fatal-message".to_string(),
            "app server stopped".to_string(),
            42.0,
        );
        let parts = message.parts.clone().unwrap();
        let payload = PendingStreamDelta {
            message_id: message.id.clone(),
            seq: 1,
            snapshot: true,
            parts: parts.clone(),
            message: Some(message),
            authoritative: true,
        };
        let mut state = RuntimeSessionState::new("codex".to_string());

        assert!(on_authoritative_stream_emit_failure(&mut state, "session-1", &payload).is_some());

        let retry = state
            .authoritative_stream_retries
            .front()
            .expect("retry snapshot");
        assert_eq!(retry.parts, parts);
        let retry_message = retry.message.as_ref().expect("backend message metadata");
        assert_eq!(retry_message.id, "fatal-message");
        assert_eq!(retry_message.role, MessageRole::Agent);
        assert_eq!(retry_message.timestamp, 42.0);
    }

    #[test]
    fn authoritative_snapshot_supersedes_older_retry() {
        let mut state = RuntimeSessionState::new("codex".to_string());
        state.retry_stream_delta = Some(PendingStreamDelta {
            message_id: "streaming-message".to_string(),
            seq: 1,
            snapshot: true,
            parts: vec![MessagePart::Text {
                content: "partial output".to_string(),
                parent_tool_use_id: None,
            }],
            message: None,
            authoritative: false,
        });
        state.stream_flush_scheduled = true;
        let message = crate::usecase::agent_session::event_log::session_error_message(
            "fatal-message".to_string(),
            "app server stopped".to_string(),
            42.0,
        );
        let payload = PendingStreamDelta {
            message_id: message.id.clone(),
            seq: 1,
            snapshot: true,
            parts: message.parts.clone().unwrap(),
            message: Some(message),
            authoritative: true,
        };

        prepare_authoritative_stream_emit(&mut state, &payload.message_id);
        assert!(state.retry_stream_delta.is_none());
        assert!(!state.stream_flush_scheduled);
        assert!(on_authoritative_stream_emit_failure(&mut state, "session-1", &payload).is_some());

        let retry = state
            .authoritative_stream_retries
            .front()
            .expect("latest retry snapshot");
        assert_eq!(retry.message_id, "fatal-message");
        assert!(retry.message.is_some());
        assert!(retry.parts.iter().any(
            |part| matches!(part, MessagePart::Error { content, .. } if content == "app server stopped")
        ));
    }

    #[test]
    fn authoritative_snapshot_retry_coalesces_only_the_same_message_id() {
        let mut state = RuntimeSessionState::new("codex".to_string());
        let older = PendingStreamDelta {
            message_id: "fatal-message".to_string(),
            seq: 1,
            snapshot: true,
            parts: vec![MessagePart::Text {
                content: "older".to_string(),
                parent_tool_use_id: None,
            }],
            message: None,
            authoritative: true,
        };
        let newer = PendingStreamDelta {
            seq: 2,
            parts: vec![MessagePart::Text {
                content: "newer".to_string(),
                parent_tool_use_id: None,
            }],
            ..older.clone()
        };

        assert!(on_authoritative_stream_emit_failure(&mut state, "session-1", &older).is_some());
        prepare_authoritative_stream_emit(&mut state, &newer.message_id);
        state.authoritative_stream_flush_scheduled = false;
        assert!(on_authoritative_stream_emit_failure(&mut state, "session-1", &newer).is_some());

        assert_eq!(state.authoritative_stream_retries.len(), 1);
        let retry = state.authoritative_stream_retries.front().unwrap();
        assert_eq!(retry.seq, 2);
        assert!(matches!(
            retry.parts.as_slice(),
            [MessagePart::Text { content, .. }] if content == "newer"
        ));
    }

    #[test]
    fn workflow_execution_env_includes_run_and_node_execution_ids() {
        let context = crate::usecase::agent_session::session::workflow_node_context_mapper::to_dto(
            workflow_node_context(None, None, None),
        );

        assert_eq!(
            workflow_execution_env(Some(&context)),
            vec![
                (
                    "RELEASH_WORKFLOW_EXECUTION_ID".to_string(),
                    "run-1".to_string(),
                ),
                (
                    "RELEASH_NODE_EXECUTION_ID".to_string(),
                    "node-execution-1".to_string(),
                ),
            ]
        );
        assert!(workflow_execution_env(None).is_empty());
    }

    struct DispatchBackend {
        id: &'static str,
        model: &'static str,
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl AgentBackend for DispatchBackend {
        fn id(&self) -> &str {
            self.id
        }

        fn name(&self) -> &str {
            self.id
        }

        fn available_models(&self) -> Vec<ModelDescriptor> {
            vec![ModelDescriptor {
                id: ModelId::parse(self.model).unwrap(),
                display_name: self.model.to_string(),
            }]
        }

        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities { steering: false }
        }

        async fn open_session(
            &self,
            _spec: SessionSpec,
        ) -> Result<Box<dyn AgentSessionRuntime>, AgentBackendError> {
            Err(AgentBackendError::Other("not used".to_string()))
        }

        async fn skill_catalog(
            &self,
            _cwd: &Path,
            _query: Option<&str>,
            _limit: Option<usize>,
        ) -> Result<Vec<SkillEntry>, AgentBackendError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:skills", self.id));
            Ok(vec![SkillEntry {
                name: format!("{}-skill", self.id),
                description: "skill".to_string(),
                scope: self.id.to_string(),
            }])
        }

        async fn fuzzy_file_search(
            &self,
            _root: &Path,
            _query: &str,
            _limit: usize,
        ) -> Result<Option<Vec<String>>, AgentBackendError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:files", self.id));
            Ok(Some(vec![format!("{}-file", self.id)]))
        }
    }

    fn dispatch_test_usecase(
        data_dir: PathBuf,
        calls: Arc<Mutex<Vec<String>>>,
        default_id: &str,
    ) -> AgentSessionRuntimeUsecase {
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(DispatchBackend {
            id: "claude",
            model: "claude-opus-4-8",
            calls: Arc::clone(&calls),
        }));
        registry.register(Arc::new(DispatchBackend {
            id: "codex",
            model: "gpt-5.6-sol",
            calls,
        }));
        registry.set_default(Some(default_id.to_string()));
        AgentSessionRuntimeUsecase::new(
            Arc::new(build_session_store()),
            Arc::new(registry),
            Arc::new(AgentStatusCenter::new()),
            Arc::new(RecordingStatusNotifier::default()),
            Arc::new(RecordingAgentNotifier::default()),
            Arc::new(TokioSpawner),
            None,
            Arc::new(EmptyInstructionSource),
            data_dir,
        )
    }

    #[derive(Default)]
    struct RecordingAgentNotifier {
        notices: Mutex<Vec<SessionNotice>>,
        state_changes: Mutex<Vec<AgentSessionStateChangedPayload>>,
        stall_observations: Mutex<Vec<AgentStallObservedPayload>>,
        stall_clears: Mutex<Vec<String>>,
        streaming_deltas: Mutex<Vec<AgentStreamingDeltaPayload>>,
        delivered_streaming_deltas: Mutex<Vec<AgentStreamingDeltaPayload>>,
        permission_modes: Mutex<Vec<(String, String)>>,
        model_updates: Mutex<Vec<(String, Vec<ModelInfo>, String)>>,
        display_windows: Mutex<Vec<GetSessionResponse>>,
        display_window_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
        streaming_delta_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
        fail_streaming_delta: Mutex<bool>,
        streaming_delta_outcomes: Mutex<std::collections::VecDeque<bool>>,
        event_order: Mutex<Vec<&'static str>>,
    }

    impl RecordingAgentNotifier {
        fn notices(&self) -> Vec<SessionNotice> {
            self.notices.lock().unwrap().clone()
        }

        fn state_changes(&self) -> Vec<AgentSessionStateChangedPayload> {
            self.state_changes.lock().unwrap().clone()
        }

        fn stall_observations(&self) -> Vec<AgentStallObservedPayload> {
            self.stall_observations.lock().unwrap().clone()
        }

        fn stall_clears(&self) -> Vec<String> {
            self.stall_clears.lock().unwrap().clone()
        }

        fn streaming_deltas(&self) -> Vec<AgentStreamingDeltaPayload> {
            self.streaming_deltas.lock().unwrap().clone()
        }

        fn delivered_streaming_deltas(&self) -> Vec<AgentStreamingDeltaPayload> {
            self.delivered_streaming_deltas.lock().unwrap().clone()
        }

        fn permission_modes(&self) -> Vec<(String, String)> {
            self.permission_modes.lock().unwrap().clone()
        }

        fn model_updates(&self) -> Vec<(String, Vec<ModelInfo>, String)> {
            self.model_updates.lock().unwrap().clone()
        }

        fn display_windows(&self) -> Vec<GetSessionResponse> {
            self.display_windows.lock().unwrap().clone()
        }

        fn set_display_window_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
            *self.display_window_hook.lock().unwrap() = Some(hook);
        }

        fn set_streaming_delta_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
            *self.streaming_delta_hook.lock().unwrap() = Some(hook);
        }

        fn set_streaming_delta_failure(&self, fail: bool) {
            *self.fail_streaming_delta.lock().unwrap() = fail;
        }

        fn set_streaming_delta_outcomes(&self, outcomes: impl IntoIterator<Item = bool>) {
            *self.streaming_delta_outcomes.lock().unwrap() = outcomes.into_iter().collect();
        }

        fn event_order(&self) -> Vec<&'static str> {
            self.event_order.lock().unwrap().clone()
        }
    }

    impl AgentSessionEventNotifier for RecordingAgentNotifier {
        fn persist_notice(&self, notice: SessionNotice) {
            self.notices.lock().unwrap().push(notice);
        }

        fn display_window_updated(&self, response: &GetSessionResponse) -> bool {
            if let Some(hook) = self.display_window_hook.lock().unwrap().clone() {
                hook();
            }
            self.display_windows.lock().unwrap().push(response.clone());
            self.event_order.lock().unwrap().push("display_window");
            true
        }

        fn session_state_changed(&self, payload: AgentSessionStateChangedPayload) {
            self.event_order.lock().unwrap().push("state_change");
            self.state_changes.lock().unwrap().push(payload);
        }

        fn stall_observed(&self, payload: AgentStallObservedPayload) {
            self.stall_observations.lock().unwrap().push(payload);
        }

        fn stall_cleared(&self, session_id: &str) {
            self.stall_clears
                .lock()
                .unwrap()
                .push(session_id.to_string());
        }

        fn streaming_delta(&self, payload: AgentStreamingDeltaPayload) -> bool {
            if let Some(hook) = self.streaming_delta_hook.lock().unwrap().clone() {
                hook();
            }
            let delivered = self
                .streaming_delta_outcomes
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| !*self.fail_streaming_delta.lock().unwrap());
            self.streaming_deltas.lock().unwrap().push(payload.clone());
            if delivered {
                self.delivered_streaming_deltas
                    .lock()
                    .unwrap()
                    .push(payload);
            }
            self.event_order.lock().unwrap().push("streaming_delta");
            delivered
        }

        fn supported_commands_updated(
            &self,
            _session_id: &str,
            _commands: Vec<crate::domain::agent_session::value_objects::SlashCommand>,
        ) {
        }

        fn token_usage_updated(
            &self,
            _session_id: &str,
            _token_usage: crate::usecase::agent_session::session::TokenUsage,
        ) {
        }

        fn permission_mode_changed(&self, session_id: &str, permission_mode: &str) {
            self.permission_modes
                .lock()
                .unwrap()
                .push((session_id.to_string(), permission_mode.to_string()));
        }

        fn models_updated(
            &self,
            session_id: &str,
            available_models: Vec<ModelInfo>,
            selected_model: String,
        ) {
            self.model_updates.lock().unwrap().push((
                session_id.to_string(),
                available_models,
                selected_model,
            ));
        }

        fn context_carry_updated(
            &self,
            _session_id: &str,
            _agent_session_id: Option<String>,
            _context_carry: Option<crate::usecase::agent_session::session::ContextCarryState>,
            _updated_at: f64,
        ) {
        }

        fn pending_message_consumed(
            &self,
            _session_id: &str,
            _queued_turn_id: Option<String>,
            _human_message: Option<ChatMessage>,
            _agent_message: ChatMessage,
        ) {
        }

        fn turn_prepared(
            &self,
            _session: &ChatSession,
            _human_message: &ChatMessage,
            _agent_message: &ChatMessage,
        ) {
        }
    }

    #[derive(Default)]
    struct RecordingStatusNotifier {
        changes: Mutex<Vec<AgentStatusChanges>>,
    }

    impl RecordingStatusNotifier {
        fn changes(&self) -> Vec<AgentStatusChanges> {
            self.changes.lock().unwrap().clone()
        }
    }

    impl AgentStatusNotifier for RecordingStatusNotifier {
        fn status_changed(&self, changes: AgentStatusChanges) {
            self.changes.lock().unwrap().push(changes);
        }
    }

    struct ReentrantWorkflowNotifier {
        usecase: Arc<AgentSessionRuntimeUsecase>,
        session_id: String,
        worktree_path: String,
        done: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl WorkflowTurnCompleteNotifier for ReentrantWorkflowNotifier {
        async fn turn_completed(&self, _notification: WorkflowTurnCompleteNotification) {
            let _ = self
                .usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(self.session_id.clone()),
                    worktree_path: self.worktree_path.clone(),
                    content: "repair".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("claude".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await;
            self.done.notify_waiters();
        }
    }

    #[derive(Default)]
    struct RecordingWorkflowStallNotifier {
        notifications: Mutex<Vec<WorkflowStallObservedNotification>>,
        cleared_notifications: Mutex<Vec<WorkflowStallClearedNotification>>,
        stall_cleared_failures: Mutex<usize>,
        event_order: Mutex<Vec<&'static str>>,
        stall_observed_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
        stall_observed_record_delay: Mutex<Option<Duration>>,
    }

    impl RecordingWorkflowStallNotifier {
        fn notifications(&self) -> Vec<WorkflowStallObservedNotification> {
            self.notifications.lock().unwrap().clone()
        }

        fn cleared_notifications(&self) -> Vec<WorkflowStallClearedNotification> {
            self.cleared_notifications.lock().unwrap().clone()
        }

        fn fail_next_stall_cleared(&self) {
            *self.stall_cleared_failures.lock().unwrap() += 1;
        }

        fn event_order(&self) -> Vec<&'static str> {
            self.event_order.lock().unwrap().clone()
        }

        fn set_stall_observed_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
            *self.stall_observed_hook.lock().unwrap() = Some(hook);
        }

        fn set_stall_observed_record_delay(&self, delay: Duration) {
            *self.stall_observed_record_delay.lock().unwrap() = Some(delay);
        }
    }

    #[async_trait::async_trait]
    impl WorkflowStallNotifier for RecordingWorkflowStallNotifier {
        async fn stall_observed(&self, notification: WorkflowStallObservedNotification) {
            let hook = self.stall_observed_hook.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook();
            }
            let delay = *self.stall_observed_record_delay.lock().unwrap();
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
            }
            self.event_order.lock().unwrap().push("observed");
            self.notifications.lock().unwrap().push(notification);
        }

        async fn stall_cleared(
            &self,
            notification: WorkflowStallClearedNotification,
        ) -> Result<(), WorkflowError> {
            {
                let mut failures = self.stall_cleared_failures.lock().unwrap();
                if *failures > 0 {
                    *failures -= 1;
                    return Err(WorkflowError::external("injected workflow clear failure"));
                }
            }
            self.event_order.lock().unwrap().push("cleared");
            self.cleared_notifications
                .lock()
                .unwrap()
                .push(notification);
            Ok(())
        }
    }

    fn send_request(worktree_path: String) -> SendAgentMessageRequest {
        SendAgentMessageRequest {
            chat_session_id: None,
            worktree_path,
            content: "hello".to_string(),
            permission_mode: PermissionMode::Edit,
            plan_mode: false,
            backend_id: Some("claude".to_string()),
            model_id: None,
            images: None,
            mentions: None,
            editor_context: None,
        }
    }

    fn workflow_node_context(
        startup_timeout_secs: Option<u64>,
        startup_max_retries: Option<u32>,
        stale_timeout_secs: Option<u64>,
    ) -> WorkflowNodeContext {
        WorkflowNodeContext {
            execution_id: "run-1".to_string(),
            node_execution_id: "node-execution-1".to_string(),
            workflow_name: "workflow".to_string(),
            node_name: "step".to_string(),
            attempt: 0,
            parent_node_name: None,
            parent_attempt: None,
            order: 1,
            startup_timeout_secs,
            startup_max_retries,
            stale_timeout_secs,
        }
    }

    fn permission_request(id: &str) -> crate::domain::agent_session::entities::PermissionRequest {
        crate::domain::agent_session::entities::PermissionRequest {
            id: id.to_string(),
            tool_use_id: Some("toolu-1".to_string()),
            parent_tool_use_id: None,
            tool_name: "Bash".to_string(),
            body: crate::domain::agent_session::entities::PermissionRequestBody::ToolApproval {
                input: crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                    r#"{"command":"echo hi"}"#.to_string(),
                ),
            },
            title: None,
            display_name: None,
            description: None,
            decision_reason: None,
            status: PermissionRequestStatus::Pending,
        }
    }

    fn permission_request_msg(id: &str) -> PermissionRequestMsg {
        pending_permission_request_msg(&permission_request(id)).unwrap()
    }

    #[test]
    fn terminal_projection_maps_session_closed_to_idle_interruption() {
        let projection = terminal_projection(&TurnResult::Interrupted {
            reason: DomainInterruptReason::SessionClosed,
            error: None,
        });

        assert_eq!(projection.exit_code, 0);
        assert!(projection.interrupted);
        assert_eq!(projection.session_state, SessionState::Idle);
        assert!(matches!(
            projection.event,
            TerminalEventProjection::Interrupted {
                reason: EventInterruptReason::SessionClosed,
                error: None,
            }
        ));
    }

    #[tokio::test]
    async fn close_session_finalizes_streaming_turn_and_persists_terminal_projection() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        let agent_message_id = response.agent_message.unwrap().id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "persisted prefix".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_persisted_text(
            &session_store,
            tmp.path(),
            &session_id,
            &agent_message_id,
            "persisted prefix",
        )
        .await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![
                    DomainMessagePart::Text {
                        content: "close tail".to_string(),
                        parent_tool_use_id: None,
                    },
                    DomainMessagePart::ToolUse {
                        id: "toolu-1".to_string(),
                        tool: "Task".to_string(),
                        input:
                            crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                r#"{"run_in_background":true}"#.to_string(),
                            ),
                        parent_tool_use_id: None,
                    },
                    DomainMessagePart::ToolResult {
                        content: "background task launched".to_string(),
                        is_error: false,
                        tool_use_id: Some("toolu-1".to_string()),
                        parent_tool_use_id: None,
                        content_ref: None,
                        summary: None,
                    },
                    DomainMessagePart::ToolUse {
                        id: "toolu-2".to_string(),
                        tool: "Read".to_string(),
                        input:
                            crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                "{}".to_string(),
                            ),
                        parent_tool_use_id: None,
                    },
                ]),
            )
            .unwrap();
        wait_for_streaming_text(&usecase, &session_id, "close tail").await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-close")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;
        let before_close_parts =
            persisted_message_parts(&session_store, tmp.path(), &session_id, &agent_message_id);
        assert!(before_close_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content.contains("persisted prefix")
        )));
        assert!(before_close_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content.contains("close tail")
        )));
        assert!(before_close_parts.iter().any(|part| matches!(
            part,
            MessagePart::Permission {
                status: PermissionPartStatus::Pending,
                ..
            }
        )));

        usecase.close_session(&session_id).await.unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::SessionClosed,
                exit_code: 0,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::ToolCallFailed { tool_use_id, .. } if tool_use_id == "toolu-2"
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TaskStatusChanged {
                task_tool_use_id,
                ..
            } if task_tool_use_id == "toolu-1"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::PermissionResolved {
                request_id: Some(request_id),
                decision: crate::usecase::agent_session::event_log::PermissionDecision::Cancelled,
                ..
            } if request_id == "perm-close"
        )));
        assert!(latest_unresolved_permission_request(&events).is_none());
        let projected = TurnEventLog::from_events(events).project();
        assert_eq!(projected.status.session_state, SessionState::Idle);
        assert_eq!(projected.status.turn_phase, TurnPhase::Idle);

        let reopened = usecase
            .get_session(&session_id)
            .await
            .unwrap()
            .expect("reopened session");
        assert_eq!(reopened.turn_phase, TurnPhase::Idle);
        assert!(reopened.pending_permission_request.is_none());
        assert_eq!(
            reopened.last_turn_interruption,
            Some(crate::usecase::agent_session::session::TurnInterruption {
                message_id: agent_message_id.clone(),
                reason:
                    crate::usecase::agent_session::session::TurnInterruptionReason::SessionClosed,
            })
        );
        let parts = reopened
            .session
            .messages
            .iter()
            .find(|message| message.id == agent_message_id)
            .and_then(|message| message.parts.as_ref())
            .expect("persisted agent parts");
        assert!(parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content.contains("close tail")
        )));
        assert!(parts.iter().any(|part| matches!(
            part,
            MessagePart::ToolResult {
                tool_use_id: Some(tool_use_id),
                is_error: true,
                ..
            } if tool_use_id == "toolu-2"
        )));
        assert!(parts.iter().any(|part| matches!(
            part,
            MessagePart::ToolResult {
                tool_use_id: Some(tool_use_id),
                is_error: false,
                ..
            } if tool_use_id == "toolu-1"
        )));
        assert!(!parts.iter().any(|part| matches!(
            part,
            MessagePart::TaskStatus {
                task_tool_use_id,
                ..
            } if task_tool_use_id == "toolu-1"
        )));
        assert!(parts.iter().any(|part| matches!(
            part,
            MessagePart::Permission {
                status: PermissionPartStatus::Cancelled,
                ..
            }
        )));
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
    }

    #[tokio::test]
    async fn close_session_without_active_turn_does_not_create_interruption() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        let events_before = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();

        usecase.close_session(&session.id).await.unwrap();

        assert_eq!(
            session_store
                .load_session_events(tmp.path(), &session.id)
                .unwrap(),
            events_before
        );
        assert!(controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Close));
    }

    #[tokio::test]
    async fn close_session_keeps_runtime_state_when_force_flush_fails_and_can_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let fail_once = Arc::new(std::sync::atomic::AtomicBool::new(true));
        session_store.set_persist_parts_hook_for_test({
            let fail_once = Arc::clone(&fail_once);
            Arc::new(move |_, _, _| {
                if fail_once.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    Err("injected close flush failure".to_string())
                } else {
                    Ok(())
                }
            })
        });

        let error = usecase.close_session(&session_id).await.unwrap_err();

        assert!(error.to_string().contains("injected close flush failure"));
        assert!(usecase.has_live_runtime(&session_id).await);
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
        assert!(!controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
        assert!(!session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. })));

        usecase.close_session(&session_id).await.unwrap();
        assert!(!usecase.has_live_runtime(&session_id).await);
    }

    #[tokio::test]
    async fn close_all_failure_reopens_admission_and_session_transition() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let failed_session_id = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap()
            .session
            .id;
        let successful_session_id = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap()
            .session
            .id;
        let fail_once = Arc::new(AtomicBool::new(true));
        session_store.set_persist_parts_hook_for_test({
            let fail_once = Arc::clone(&fail_once);
            let failed_session_id = failed_session_id.clone();
            Arc::new(move |session_id, _, _| {
                if session_id == failed_session_id && fail_once.swap(false, Ordering::SeqCst) {
                    Err("injected application close flush failure".to_string())
                } else {
                    Ok(())
                }
            })
        });

        let error = usecase.close_all().await.unwrap_err();

        assert!(error
            .to_string()
            .contains("injected application close flush failure"));
        assert!(error.to_string().contains(&failed_session_id));
        assert!(!usecase.has_live_runtime(&successful_session_id).await);
        assert!(controller
            .call_kinds_for(&successful_session_id)
            .contains(&TestRuntimeCallKind::Close));
        assert_eq!(
            session_store
                .load_session_events(tmp.path(), &successful_session_id)
                .unwrap()
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::TurnInterrupted {
                        reason: EventInterruptReason::SessionClosed,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert!(usecase.has_live_runtime(&failed_session_id).await);
        assert!(!session_store
            .load_session_events(tmp.path(), &failed_session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. })));
        assert!(!usecase.ctx.shutdown_admission.is_shutting_down());
        assert!(
            !usecase
                .ctx
                .sessions
                .lock()
                .await
                .get(&failed_session_id)
                .expect("failed session remains")
                .closing
        );
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(failed_session_id),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "accepted after failed application shutdown".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn close_all_finalizes_every_active_session_once() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session_ids = Vec::new();
        for _ in 0..2 {
            session_ids.push(
                usecase
                    .send_message(send_request(tmp.path().to_string_lossy().to_string()))
                    .await
                    .unwrap()
                    .session
                    .id,
            );
        }

        usecase.close_all().await.unwrap();

        for session_id in session_ids {
            assert!(!usecase.has_live_runtime(&session_id).await);
            assert!(controller
                .call_kinds_for(&session_id)
                .contains(&TestRuntimeCallKind::Close));
            assert_eq!(
                session_store
                    .load_session_events(tmp.path(), &session_id)
                    .unwrap()
                    .iter()
                    .filter(|event| matches!(
                        event,
                        AgentSessionEvent::TurnInterrupted {
                            reason: EventInterruptReason::SessionClosed,
                            ..
                        }
                    ))
                    .count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn close_session_appends_terminal_batch_atomically_and_can_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let terminal_failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_append_event_hook_for_test({
            let terminal_failures = Arc::clone(&terminal_failures);
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::TurnInterrupted { .. })
                    && terminal_failures.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                        < PERSIST_MAX_ATTEMPTS
                {
                    Err("injected terminal append failure".to_string())
                } else {
                    Ok(())
                }
            })
        });

        let error = usecase.close_session(&session_id).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("injected terminal append failure"));
        assert!(usecase.has_live_runtime(&session_id).await);
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
        assert!(!controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
        let failed_events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(!failed_events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::FinalPartsRecorded { .. }
                | AgentSessionEvent::TurnInterrupted { .. }
        )));

        usecase.close_session(&session_id).await.unwrap();
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn close_session_rolls_back_terminal_when_message_persist_fails_and_can_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let agent_message_id = response.agent_message.unwrap().id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "text committed before terminal".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_streaming_text(&usecase, &session_id, "text committed before terminal").await;
        let persist_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_persist_parts_hook_for_test({
            let persist_count = Arc::clone(&persist_count);
            Arc::new(move |_, _, _| {
                let call = persist_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if (2..2 + PERSIST_MAX_ATTEMPTS).contains(&call) {
                    Err("injected final message persist failure".to_string())
                } else {
                    Ok(())
                }
            })
        });

        let error = usecase.close_session(&session_id).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("injected final message persist failure"));
        assert!(usecase.has_live_runtime(&session_id).await);
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
        assert!(!controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
        let events_after_terminal = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events_after_terminal
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. }))
                .count(),
            0
        );

        usecase.close_session(&session_id).await.unwrap();
        assert!(!usecase.has_live_runtime(&session_id).await);
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. }))
                .count(),
            1
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::PermissionRequested { request, .. }
                if request.id == "late-permission"
        )));
        let persisted_parts =
            persisted_message_parts(&session_store, tmp.path(), &session_id, &agent_message_id);
        assert!(persisted_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content.contains("text committed before terminal")
        )));
    }

    #[tokio::test]
    async fn set_session_backend_finalizes_active_turn_before_runtime_close() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;

        let switched = usecase
            .set_session_backend(&session_id, "codex")
            .await
            .unwrap();

        assert_eq!(switched.session.backend_id.as_deref(), Some("codex"));
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::SessionClosed,
                ..
            }
        )));
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
    }

    #[tokio::test]
    async fn set_session_backend_serializes_competing_send_with_runtime_transition() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let switch_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session_id.clone();
            async move { usecase.set_session_backend(&session_id, "codex").await }
        });
        wait_for_session_closing(&usecase, &session_id).await;

        let during_transition = session_store
            .get_session_meta(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(during_transition.backend_id, "claude");
        let send_error = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "competing backend send".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap_err();
        assert!(send_error.to_string().contains("Agent session is closing"));

        let switched = switch_task.await.unwrap().unwrap();
        assert_eq!(switched.session.backend_id.as_deref(), Some("codex"));
        assert!(!usecase.has_live_runtime(&session_id).await);
        let resumed = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "send after backend switch".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        assert_eq!(resumed.session.backend_id.as_deref(), Some("codex"));
        let sessions = usecase.ctx.sessions.lock().await;
        assert_eq!(
            sessions
                .get(&session_id)
                .map(|state| state.backend_id.as_str()),
            Some("codex")
        );
    }

    #[tokio::test]
    async fn close_all_finalizes_active_turn_for_fresh_runtime_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let agent_message_id = response.agent_message.unwrap().id;
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "persisted prefix".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_persisted_text(
            &session_store,
            tmp.path(),
            &session_id,
            &agent_message_id,
            "persisted prefix",
        )
        .await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![
                    DomainMessagePart::Text {
                        content: "shutdown tail".to_string(),
                        parent_tool_use_id: None,
                    },
                    DomainMessagePart::ToolUse {
                        id: "toolu-shutdown".to_string(),
                        tool: "Task".to_string(),
                        input:
                            crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                r#"{"run_in_background":true}"#.to_string(),
                            ),
                        parent_tool_use_id: None,
                    },
                    DomainMessagePart::ToolResult {
                        content: "background task launched".to_string(),
                        is_error: false,
                        tool_use_id: Some("toolu-shutdown".to_string()),
                        parent_tool_use_id: None,
                        content_ref: None,
                        summary: None,
                    },
                ]),
            )
            .unwrap();
        wait_for_streaming_text(&usecase, &session_id, "shutdown tail").await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-shutdown")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;
        let before_shutdown_parts =
            persisted_message_parts(&session_store, tmp.path(), &session_id, &agent_message_id);
        assert!(before_shutdown_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content.contains("persisted prefix")
        )));
        assert!(before_shutdown_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content.contains("shutdown tail")
        )));
        assert!(before_shutdown_parts.iter().any(|part| matches!(
            part,
            MessagePart::Permission {
                status: PermissionPartStatus::Pending,
                ..
            }
        )));

        usecase.close_all().await.unwrap();
        drop(usecase);
        let restarted =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());

        let reopened = restarted
            .get_session(&session_id)
            .await
            .unwrap()
            .expect("restored session");
        assert_eq!(reopened.turn_phase, TurnPhase::Idle);
        assert_eq!(
            reopened.last_turn_interruption,
            Some(crate::usecase::agent_session::session::TurnInterruption {
                message_id: agent_message_id.clone(),
                reason:
                    crate::usecase::agent_session::session::TurnInterruptionReason::SessionClosed,
            })
        );
        assert!(reopened.pending_permission_request.is_none());
        assert!(reopened.session.messages.iter().any(|message| {
            message.parts.as_ref().is_some_and(|parts| {
                parts.iter().any(|part| {
                    matches!(
                        part,
                        MessagePart::Text { content, .. } if content.contains("shutdown tail")
                    )
                })
            })
        }));
        let parts = reopened
            .session
            .messages
            .iter()
            .find(|message| message.id == agent_message_id)
            .and_then(|message| message.parts.as_ref())
            .expect("persisted shutdown parts");
        assert!(!parts.iter().any(|part| matches!(
            part,
            MessagePart::TaskStatus {
                task_tool_use_id,
                ..
            } if task_tool_use_id == "toolu-shutdown"
        )));
        assert!(parts.iter().any(|part| matches!(
            part,
            MessagePart::Permission {
                status: PermissionPartStatus::Cancelled,
                ..
            }
        )));
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::SessionClosed,
                ..
            }
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TaskStatusChanged {
                task_tool_use_id,
                ..
            } if task_tool_use_id == "toolu-shutdown"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::PermissionResolved {
                request_id: Some(request_id),
                decision: crate::usecase::agent_session::event_log::PermissionDecision::Cancelled,
                ..
            } if request_id == "perm-shutdown"
        )));
        assert!(latest_unresolved_permission_request(&events).is_none());
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
    }

    #[tokio::test]
    async fn close_session_drains_competing_backend_completion_without_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let close_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session_id.clone();
            async move { usecase.close_session(&session_id).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!close_task.is_finished());

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        close_task.await.unwrap().unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnCompleted { .. })));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::SessionClosed,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn close_session_waits_for_competing_send_message_before_finalizing() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        controller.pause_start_turn();
        let send_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            let worktree_path = tmp.path().to_string_lossy().to_string();
            async move {
                usecase
                    .send_message(SendAgentMessageRequest {
                        chat_session_id: Some(session_id),
                        worktree_path,
                        content: "competing send".to_string(),
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                        backend_id: Some("claude".to_string()),
                        model_id: None,
                        images: None,
                        mentions: None,
                        editor_context: None,
                    })
                    .await
            }
        });
        wait_for_call(&controller, &session.id, TestRuntimeCallKind::StartTurn).await;

        let close_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            async move { usecase.close_session(&session_id).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!close_task.is_finished());

        controller.release_start_turn();
        send_task.await.unwrap().unwrap();
        close_task.await.unwrap().unwrap();

        assert!(!usecase.has_live_runtime(&session.id).await);
        assert!(session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::TurnInterrupted {
                    reason: EventInterruptReason::SessionClosed,
                    ..
                }
            )));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocked_turn_start_does_not_block_other_session_and_remains_terminalizable() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Arc::new(FileSessionStorage::default());
        let session_store = Arc::new(SessionStore::new(storage.clone()));
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let unrelated_session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase
            .start_session(
                &unrelated_session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        storage.reset_event_read_count();
        let hook_entered = Arc::new(Barrier::new(2));
        let release_hook = Arc::new(Barrier::new(2));
        let global_registry_was_available = Arc::new(AtomicBool::new(false));
        session_store.set_appended_event_hook_for_test({
            let sessions = Arc::clone(&usecase.ctx.sessions);
            let session_id = session.id.clone();
            let hook_entered = Arc::clone(&hook_entered);
            let release_hook = Arc::clone(&release_hook);
            let global_registry_was_available = Arc::clone(&global_registry_was_available);
            Arc::new(move |event_session_id, event| {
                if event_session_id == session_id
                    && matches!(event, AgentSessionEvent::TurnStarted { .. })
                {
                    global_registry_was_available
                        .store(sessions.try_lock().is_ok(), Ordering::SeqCst);
                    hook_entered.wait();
                    release_hook.wait();
                }
            })
        });
        controller.pause_start_turn();
        let start_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            async move {
                let _session_guard = usecase.acquire_session_lock(&session_id).await;
                usecase
                    .start_turn_locked(
                        &session_id,
                        PermissionMode::Edit,
                        "cancel during durable start".to_string(),
                        None,
                        Vec::new(),
                    )
                    .await
            }
        });

        hook_entered.wait();
        assert_eq!(storage.event_read_count(), 0);
        tokio::time::timeout(
            Duration::from_secs(1),
            usecase.close_session(&unrelated_session.id),
        )
        .await
        .expect("unrelated session close must not wait for another session's append")
        .unwrap();
        let global_registry_guard = usecase.ctx.sessions.lock().await;
        start_task.abort();
        release_hook.wait();
        assert!(start_task.await.unwrap_err().is_cancelled());
        drop(global_registry_guard);
        assert!(global_registry_was_available.load(Ordering::SeqCst));

        usecase.close_all().await.unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::TurnInterrupted {
                        reason: EventInterruptReason::SessionClosed,
                        ..
                    }
                ))
                .count(),
            1
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn close_all_waits_for_admitted_send_before_snapshotting_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let hook_entered = Arc::new(Barrier::new(2));
        let release_hook = Arc::new(Barrier::new(2));
        let blocked = Arc::new(AtomicBool::new(false));
        session_store.set_append_event_hook_for_test({
            let session_id = session.id.clone();
            let hook_entered = Arc::clone(&hook_entered);
            let release_hook = Arc::clone(&release_hook);
            let blocked = Arc::clone(&blocked);
            Arc::new(move |event_session_id, event| {
                if event_session_id == session_id
                    && matches!(event, AgentSessionEvent::TurnStarted { .. })
                    && !blocked.swap(true, Ordering::SeqCst)
                {
                    hook_entered.wait();
                    release_hook.wait();
                }
                Ok(())
            })
        });
        controller.pause_start_turn();
        let send_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            let worktree_path = tmp.path().to_string_lossy().to_string();
            async move {
                usecase
                    .send_message(SendAgentMessageRequest {
                        chat_session_id: Some(session_id),
                        worktree_path,
                        content: "admitted before shutdown".to_string(),
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                        backend_id: Some("claude".to_string()),
                        model_id: None,
                        images: None,
                        mentions: None,
                        editor_context: None,
                    })
                    .await
            }
        });

        hook_entered.wait();
        let close_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            async move { usecase.close_all().await }
        });
        let shutdown_deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !usecase.ctx.shutdown_admission.is_shutting_down() {
            assert!(std::time::Instant::now() < shutdown_deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!close_task.is_finished());
        release_hook.wait();

        let send_error = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "rejected after shutdown".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap_err();
        assert!(send_error.to_string().contains("runtime is shutting down"));
        let start_error = usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap_err();
        assert!(start_error.to_string().contains("runtime is shutting down"));

        wait_for_call(&controller, &session.id, TestRuntimeCallKind::StartTurn).await;
        assert!(!close_task.is_finished());
        controller.release_start_turn();
        send_task.await.unwrap().unwrap();
        close_task.await.unwrap().unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::SessionClosed,
                ..
            }
        )));
        assert!(controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Close));
    }

    #[tokio::test]
    async fn close_session_waits_for_competing_permission_response_before_finalizing() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-close-race")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;
        controller.pause_respond_permission();
        let response_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session_id.clone();
            async move {
                usecase
                    .respond_permission(
                        &session_id,
                        PermissionResponse {
                            request_id: "perm-close-race".to_string(),
                            decision: PermissionResponseDecision::Allow {
                                updated_input: None,
                                answers: None,
                            },
                        },
                    )
                    .await
            }
        });
        wait_for_call(
            &controller,
            &session_id,
            TestRuntimeCallKind::RespondPermission {
                request_id: "perm-close-race".to_string(),
            },
        )
        .await;

        let close_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session_id.clone();
            async move { usecase.close_session(&session_id).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!close_task.is_finished());

        controller.release_respond_permission();
        response_task.await.unwrap().unwrap();
        close_task.await.unwrap().unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::PermissionResolved {
                request_id: Some(request_id),
                decision: crate::usecase::agent_session::event_log::PermissionDecision::Allowed,
                ..
            } if request_id == "perm-close-race"
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::PermissionResolved {
                request_id: Some(request_id),
                decision: crate::usecase::agent_session::event_log::PermissionDecision::Cancelled,
                ..
            } if request_id == "perm-close-race"
        )));
    }

    #[tokio::test]
    async fn close_session_rejects_new_send_while_event_drain_is_active() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            build_agent_runtime_usecase_with_controller(session_store, tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let close_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session_id.clone();
            async move { usecase.close_session(&session_id).await }
        });
        wait_for_session_closing(&usecase, &session_id).await;

        let error = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "too late".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap_err();

        assert!(error.to_string().contains("Agent session is closing"));
        close_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn get_session_reads_interruption_projection_without_loading_long_event_log() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Arc::new(FileSessionStorage::default());
        let session_store = Arc::new(SessionStore::new(storage.clone()));
        let (usecase, _controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session_store
            .append_session_event_and_project_state(
                tmp.path(),
                &session.id,
                AgentSessionEvent::TurnStarted {
                    turn_id: 1,
                    message_id: "human-long".to_string(),
                    assistant_message_id: Some("agent-long".to_string()),
                    prompt: PromptInput::default(),
                    at: 1.0,
                },
            )
            .unwrap();
        for index in 0..500 {
            session_store
                .append_session_event_without_projection(
                    tmp.path(),
                    &session.id,
                    AgentSessionEvent::TextRecorded {
                        turn_id: 1,
                        message_id: "agent-long".to_string(),
                        content: format!("chunk-{index}"),
                        parent_tool_use_id: None,
                    },
                )
                .unwrap();
        }
        session_store
            .append_session_event_and_project_state(
                tmp.path(),
                &session.id,
                AgentSessionEvent::TurnInterrupted {
                    turn_id: 1,
                    reason: EventInterruptReason::SessionClosed,
                    exit_code: 0,
                    error: None,
                },
            )
            .unwrap();
        storage.reset_event_read_count();

        let response = usecase
            .get_session(&session.id)
            .await
            .unwrap()
            .expect("session response");

        assert_eq!(
            response.last_turn_interruption,
            Some(crate::usecase::agent_session::session::TurnInterruption {
                message_id: "agent-long".to_string(),
                reason:
                    crate::usecase::agent_session::session::TurnInterruptionReason::SessionClosed,
            })
        );
        assert_eq!(storage.event_read_count(), 0);
    }

    #[tokio::test]
    async fn display_session_window_is_bounded_by_the_backend_retention_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        for index in 0..210 {
            add_message_internal(
                &session_store,
                tmp.path(),
                &session.id,
                MessageRole::Human,
                &format!("message-{index}"),
                None,
                None,
            )
            .unwrap();
        }

        let response = usecase
            .get_display_session_window(&session.id, Some(usize::MAX))
            .await
            .unwrap()
            .expect("display window");

        assert_eq!(response.session.messages.len(), RETAINED_MESSAGE_CAP);
        assert_eq!(response.session.messages[0].content, "message-10");
        assert_eq!(
            response.session.messages.last().unwrap().content,
            "message-209"
        );
        assert_eq!(
            response.initial_page.unwrap().total_count,
            210,
            "the bounded body must retain full-history page accounting"
        );
    }

    #[tokio::test]
    async fn display_session_window_is_published_inside_the_runtime_event_ordering_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, _controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let session_locks = usecase.ctx.session_locks.clone();
        let session_id = session.id.clone();
        event_notifier.set_display_window_hook(Arc::new(move || {
            assert!(
                session_locks.is_held_for_test(&session_id),
                "the bounded read must publish before a later runtime event can acquire the session"
            );
        }));

        let response = usecase
            .get_display_session_window(&session.id, None)
            .await
            .unwrap()
            .expect("display window");

        let published = event_notifier.display_windows();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].session.id, response.session.id);
        assert_eq!(event_notifier.event_order(), vec!["display_window"]);
    }

    #[tokio::test]
    async fn display_session_window_overlays_the_latest_runtime_stream_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            build_agent_runtime_usecase_with_controller(session_store, tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let agent_message_id = response.agent_message.unwrap().id;
        let live_parts = vec![MessagePart::Text {
            content: "latest live snapshot".to_string(),
            parent_tool_use_id: None,
        }];
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session_id).expect("live runtime state");
            assert_eq!(
                state.streaming_message_id.as_deref(),
                Some(agent_message_id.as_str())
            );
            state.streaming_parts = live_parts.clone();
            state.streaming_delta_seq = 7;
        }

        let window = usecase
            .get_display_session_window(&session_id, None)
            .await
            .unwrap()
            .expect("display window");
        let displayed = window
            .session
            .messages
            .iter()
            .find(|message| message.id == agent_message_id)
            .expect("streaming message");

        assert_eq!(displayed.parts.as_ref(), Some(&live_parts));
        assert_eq!(displayed.streaming_final_seq, 7);
    }

    #[tokio::test]
    async fn get_session_returns_in_memory_pending_permission_request() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-1")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;

        let loaded = usecase
            .get_session(&session_id)
            .await
            .unwrap()
            .expect("session");

        assert_eq!(loaded.turn_phase, TurnPhase::WaitingPermission);
        assert_eq!(
            loaded
                .pending_permission_request
                .as_ref()
                .map(|r| r.id.as_str()),
            Some("perm-1")
        );
        assert!(loaded.pending_permission_state_revision > 0);
    }

    #[tokio::test]
    async fn get_session_ignores_event_log_pending_when_runtime_state_is_clear() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::TurnStarted {
                    turn_id: 1,
                    message_id: "human-1".to_string(),
                    assistant_message_id: Some("agent-1".to_string()),
                    prompt: PromptInput {
                        content: "run".to_string(),
                        mentions: Vec::new(),
                        attachment_refs: Vec::new(),
                        parts: Vec::new(),
                    },
                    at: 1.0,
                },
            )
            .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::PermissionRequested {
                    turn_id: 1,
                    tool_use_id: Some("toolu-1".to_string()),
                    request: crate::usecase::agent_session::runtime::event_apply::pending_permission_request_from_msg(
                        &permission_request_msg("perm-from-log"),
                    )
                    .unwrap(),
                },
            )
            .unwrap();
        usecase
            .insert_runtime_state_for_test(&session.id, TurnPhase::Idle, false)
            .await;

        let loaded = usecase
            .get_session(&session.id)
            .await
            .unwrap()
            .expect("session");

        assert_eq!(loaded.turn_phase, TurnPhase::Idle);
        assert!(loaded.pending_permission_request.is_none());
        let presented = usecase
            .find_permission_request(&session.id, "perm-from-log")
            .await
            .unwrap()
            .expect("permission request");
        assert_eq!(presented.id, "perm-from-log");
        let sessions = usecase.ctx.sessions.lock().await;
        let state = sessions.get(&session.id).expect("runtime state");
        assert_eq!(state.phase, RuntimeSessionPhase::Idle);
        assert!(state.pending_permission_request.is_none());
    }

    #[tokio::test]
    async fn get_session_does_not_publish_event_log_permission_without_live_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::TurnStarted {
                    turn_id: 1,
                    message_id: "human-1".to_string(),
                    assistant_message_id: Some("agent-1".to_string()),
                    prompt: PromptInput {
                        content: "run".to_string(),
                        mentions: Vec::new(),
                        attachment_refs: Vec::new(),
                        parts: Vec::new(),
                    },
                    at: 1.0,
                },
            )
            .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::PermissionRequested {
                    turn_id: 1,
                    tool_use_id: Some("toolu-1".to_string()),
                    request: crate::usecase::agent_session::runtime::event_apply::pending_permission_request_from_msg(
                        &permission_request_msg("perm-from-log"),
                    )
                    .unwrap(),
                },
            )
            .unwrap();

        let loaded = usecase
            .get_session(&session.id)
            .await
            .unwrap()
            .expect("session");

        assert_eq!(loaded.turn_phase, TurnPhase::Idle);
        assert!(loaded.pending_permission_request.is_none());
    }

    #[tokio::test]
    async fn respond_permission_resolves_event_log_only_pending_permission() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::TurnStarted {
                    turn_id: 1,
                    message_id: "human-1".to_string(),
                    assistant_message_id: Some("agent-1".to_string()),
                    prompt: PromptInput {
                        content: "run".to_string(),
                        mentions: Vec::new(),
                        attachment_refs: Vec::new(),
                        parts: Vec::new(),
                    },
                    at: 1.0,
                },
            )
            .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::PermissionRequested {
                    turn_id: 1,
                    tool_use_id: Some("toolu-1".to_string()),
                    request: crate::usecase::agent_session::runtime::event_apply::pending_permission_request_from_msg(
                        &permission_request_msg("perm-from-log"),
                    )
                    .unwrap(),
                },
            )
            .unwrap();
        usecase
            .insert_failing_runtime_state_for_test(&session.id)
            .await;

        usecase
            .respond_permission(
                &session.id,
                PermissionResponse {
                    request_id: "perm-from-log".to_string(),
                    decision: PermissionResponseDecision::Allow {
                        updated_input: None,
                        answers: None,
                    },
                },
            )
            .await
            .unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::PermissionResolved {
                turn_id: 1,
                request_id: Some(request_id),
                ..
            } if request_id == "perm-from-log"
        )));
        assert!(latest_unresolved_permission_request(&events).is_none());
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );

        let loaded = usecase
            .get_session(&session.id)
            .await
            .unwrap()
            .expect("session");
        assert_eq!(loaded.turn_phase, TurnPhase::Streaming);
        assert!(loaded.pending_permission_request.is_none());
        assert!(loaded.pending_permission_state_revision > 0);
    }

    #[tokio::test]
    async fn permission_requested_emits_pending_state_change_when_stream_emit_is_suppressed() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            sessions
                .get_mut(&session_id)
                .expect("runtime state")
                .stream_emit_suppressed = true;
        }

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-suppressed")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;

        assert!(event_notifier.state_changes().iter().any(|change| {
            change.chat_session_id == session_id
                && change.turn_phase == TurnPhase::WaitingPermission
                && change
                    .pending_permission_request
                    .as_ref()
                    .is_some_and(|request| request.id == "perm-suppressed")
                && change.pending_permission_state_revision.is_some()
        }));
    }

    #[test]
    fn permission_wait_diagnostic_is_marked_once_after_threshold() {
        let mut state = RuntimeSessionState::new("claude".to_string());
        let now = std::time::Instant::now();
        state.phase = RuntimeSessionPhase::WaitingPermission;
        state.pending_permission_request = Some(permission_request_msg("perm-diag"));
        state.permission_wait_started_at =
            Some(now - PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD - Duration::from_millis(1));

        assert!(maybe_mark_permission_wait_diagnostic("s1", &mut state, now));
        assert!(state.permission_wait_diagnostic_emitted);
        assert!(!maybe_mark_permission_wait_diagnostic(
            "s1", &mut state, now
        ));
    }

    #[test]
    fn permission_wait_diagnostic_skips_fresh_observed_request() {
        let mut state = RuntimeSessionState::new("claude".to_string());
        let now = std::time::Instant::now();
        state.phase = RuntimeSessionPhase::WaitingPermission;
        state.pending_permission_request = Some(permission_request_msg("perm-visible"));
        state.permission_wait_started_at =
            Some(now - PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD - Duration::from_millis(1));
        state.permission_request_visibility = Some(PermissionRequestVisibility {
            request_id: "perm-visible".to_string(),
            last_seen_at: now,
        });

        assert!(!maybe_mark_permission_wait_diagnostic(
            "s1", &mut state, now
        ));
        assert!(!state.permission_wait_diagnostic_emitted);

        state.permission_request_visibility = Some(PermissionRequestVisibility {
            request_id: "perm-visible".to_string(),
            last_seen_at: now - PERMISSION_REQUEST_OBSERVED_TTL - Duration::from_millis(1),
        });
        assert!(maybe_mark_permission_wait_diagnostic("s1", &mut state, now));
        assert!(state.permission_wait_diagnostic_emitted);
    }

    #[test]
    fn permission_wait_diagnostic_treats_mismatched_observation_as_unobserved() {
        let mut state = RuntimeSessionState::new("claude".to_string());
        let now = std::time::Instant::now();
        state.phase = RuntimeSessionPhase::WaitingPermission;
        state.pending_permission_request = Some(permission_request_msg("perm-pending"));
        state.permission_wait_started_at =
            Some(now - PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD - Duration::from_millis(1));
        state.permission_request_visibility = Some(PermissionRequestVisibility {
            request_id: "perm-other".to_string(),
            last_seen_at: now,
        });

        assert!(maybe_mark_permission_wait_diagnostic("s1", &mut state, now));
        assert!(state.permission_wait_diagnostic_emitted);
    }

    #[tokio::test]
    async fn report_permission_request_observed_tracks_matching_pending_request() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        usecase
            .insert_runtime_state_for_test("s1", TurnPhase::WaitingPermission, false)
            .await;
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut("s1").expect("runtime state");
            state.pending_permission_request = Some(permission_request_msg("perm-visible"));
        }

        usecase
            .report_permission_request_observed("s1", "perm-visible", true)
            .await
            .unwrap();
        {
            let sessions = usecase.ctx.sessions.lock().await;
            let visibility = sessions
                .get("s1")
                .and_then(|state| state.permission_request_visibility.as_ref())
                .expect("visibility");
            assert_eq!(visibility.request_id, "perm-visible");
        }

        usecase
            .report_permission_request_observed("s1", "perm-other", false)
            .await
            .unwrap();
        {
            let sessions = usecase.ctx.sessions.lock().await;
            assert!(sessions
                .get("s1")
                .and_then(|state| state.permission_request_visibility.as_ref())
                .is_some());
        }

        usecase
            .report_permission_request_observed("s1", "perm-visible", false)
            .await
            .unwrap();
        {
            let sessions = usecase.ctx.sessions.lock().await;
            assert!(sessions
                .get("s1")
                .and_then(|state| state.permission_request_visibility.as_ref())
                .is_none());
        }

        usecase
            .report_permission_request_observed("s1", "perm-other", true)
            .await
            .unwrap();
        {
            let sessions = usecase.ctx.sessions.lock().await;
            assert!(sessions
                .get("s1")
                .and_then(|state| state.permission_request_visibility.as_ref())
                .is_none());
        }
    }

    #[tokio::test]
    async fn skill_catalog_and_mentionable_files_dispatch_to_selected_backend_only() {
        let tmp = tempfile::tempdir().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let usecase = dispatch_test_usecase(tmp.path().to_path_buf(), Arc::clone(&calls), "codex");

        let codex_skills = usecase
            .skill_catalog(Some("codex"), tmp.path(), Some("skill"), Some(5))
            .await
            .unwrap();
        let claude_files = usecase
            .mentionable_files(Some("claude"), tmp.path(), "src", 10)
            .await
            .unwrap()
            .unwrap();
        let default_skills = usecase
            .skill_catalog(None, tmp.path(), None, None)
            .await
            .unwrap();

        assert_eq!(codex_skills[0].scope, "codex");
        assert_eq!(claude_files, vec!["claude-file".to_string()]);
        assert_eq!(default_skills[0].scope, "codex");
        assert_eq!(
            calls.lock().unwrap().clone(),
            vec![
                "codex:skills".to_string(),
                "claude:files".to_string(),
                "codex:skills".to_string()
            ]
        );
    }

    async fn wait_for_call(
        controller: &crate::test_support::TestAgentRuntimeController,
        session_id: &str,
        expected: TestRuntimeCallKind,
    ) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if controller
                    .call_kinds_for(session_id)
                    .iter()
                    .any(|kind| kind == &expected)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_call_count(
        controller: &crate::test_support::TestAgentRuntimeController,
        session_id: &str,
        expected: TestRuntimeCallKind,
        expected_count: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let count = controller
                    .call_kinds_for(session_id)
                    .iter()
                    .filter(|kind| *kind == &expected)
                    .count();
                if count >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_stream_delta_count(notifier: &RecordingAgentNotifier, expected_count: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.streaming_deltas().len() >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_error_state_change(notifier: &RecordingAgentNotifier, session_id: &str) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.state_changes().iter().any(|change| {
                    change.chat_session_id == session_id
                        && change.turn_phase == TurnPhase::Idle
                        && change.session_state == Some(SessionState::Error)
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_stall_observation_count(
        notifier: &RecordingAgentNotifier,
        expected_count: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.stall_observations().len() >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_stall_clear_count(notifier: &RecordingAgentNotifier, expected_count: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.stall_clears().len() >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_workflow_stall_notification_count(
        notifier: &RecordingWorkflowStallNotifier,
        expected_count: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.notifications().len() >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_workflow_stall_cleared_count(
        notifier: &RecordingWorkflowStallNotifier,
        expected_count: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.cleared_notifications().len() >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_stream_emit_failure_state(
        usecase: &AgentSessionRuntimeUsecase,
        session_id: &str,
        predicate: impl Fn(u32, bool) -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some((failures, suppressed)) =
                    usecase.stream_emit_failure_state_for_test(session_id).await
                {
                    if predicate(failures, suppressed) {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_last_stream_delta(
        notifier: &RecordingAgentNotifier,
        predicate: impl Fn(&AgentStreamingDeltaPayload) -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.streaming_deltas().last().is_some_and(&predicate) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_start_prompt_count(
        controller: &crate::test_support::TestAgentRuntimeController,
        session_id: &str,
        expected_count: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let count = controller
                    .call_kinds_for(session_id)
                    .iter()
                    .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                    .count();
                if count >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_open_count(
        controller: &crate::test_support::TestAgentRuntimeController,
        session_id: &str,
        expected_count: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let count = controller
                    .call_kinds_for(session_id)
                    .iter()
                    .filter(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
                    .count();
                if count >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    fn provider_establish_test_session(
        session_store: &SessionStore,
        data_dir: &Path,
        resume_id: Option<&str>,
    ) -> ChatSession {
        let session = create_session_internal_with_attributes(
            session_store,
            data_dir,
            data_dir.to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        if let Some(resume_id) = resume_id {
            session_store
                .record_backend_session_established(
                    data_dir,
                    &session.id,
                    0,
                    "provider-establish-test-observation",
                    resume_id.to_string(),
                    Some(ContextCarryState::Resumed),
                )
                .unwrap();
        }
        session
    }

    async fn wait_for_turn_phase(
        usecase: &AgentSessionRuntimeUsecase,
        session_id: &str,
        phase: TurnPhase,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if usecase.turn_phase(session_id).await == Some(phase) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    fn persisted_message_parts(
        session_store: &SessionStore,
        data_dir: &Path,
        session_id: &str,
        message_id: &str,
    ) -> Vec<MessagePart> {
        session_store
            .load_full_session_for_restore(data_dir, session_id)
            .unwrap()
            .expect("persisted session")
            .messages
            .into_iter()
            .find(|message| message.id == message_id)
            .and_then(|message| message.parts)
            .unwrap_or_default()
    }

    async fn wait_for_persisted_text(
        session_store: &SessionStore,
        data_dir: &Path,
        session_id: &str,
        message_id: &str,
        expected: &str,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if persisted_message_parts(session_store, data_dir, session_id, message_id)
                    .iter()
                    .any(|part| {
                        matches!(part, MessagePart::Text { content, .. } if content.contains(expected))
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_streaming_text(
        usecase: &AgentSessionRuntimeUsecase,
        session_id: &str,
        expected: &str,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if usecase.streaming_parts(session_id).await.iter().any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content.contains(expected))
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_session_closing(usecase: &AgentSessionRuntimeUsecase, session_id: &str) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let closing = {
                    let sessions = usecase.ctx.sessions.lock().await;
                    sessions.get(session_id).is_some_and(|state| state.closing)
                };
                if closing {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn mark_stall_observation_active_for_test(
        usecase: &AgentSessionRuntimeUsecase,
        session_id: &str,
    ) {
        let mut sessions = usecase.ctx.sessions.lock().await;
        let state = sessions.get_mut(session_id).unwrap();
        state.stall_signal_count = 1;
        state.stall_observation_active = true;
    }

    #[test]
    fn test_human_parts_画像のみの場合は_image_partを返す() {
        // Given: human input contains no text and one image.
        let parts = human_parts(
            "",
            &[ImageAttachment {
                data: "abc".to_string(),
                media_type: "image/png".to_string(),
            }],
        );

        // Then: the generated human parts preserve the image.
        assert!(matches!(parts[0], MessagePart::Image { .. }));
    }

    #[tokio::test]
    async fn test_send_message_開始成功後に_streaming状態とstatusを通知する() {
        // Given: an agent runtime usecase with recording event/status notifiers.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier.clone(),
        );

        // When: a user sends a message and the backend accepts the turn.
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();

        // Then: the live phase and both notifier surfaces move to Streaming.
        assert_eq!(
            usecase.turn_phase(&response.session.id).await,
            Some(TurnPhase::Streaming)
        );
        assert!(event_notifier.state_changes().iter().any(|change| {
            change.chat_session_id == response.session.id
                && change.turn_phase == TurnPhase::Streaming
                && change.session_state == Some(SessionState::Active)
        }));
        assert!(status_notifier.changes().iter().any(|change| {
            change.session.as_ref().is_some_and(|session| {
                session.chat_session_id == response.session.id
                    && session.turn_phase == TurnPhaseRepr::Streaming
            })
        }));
        assert!(controller
            .call_kinds_for(&response.session.id)
            .contains(&TestRuntimeCallKind::StartTurn));
    }

    #[tokio::test]
    async fn test_send_message_並行送信2本目は二重turnを開始せずqueueへ入る() {
        // Given: an existing session and a runtime whose first start_turn is blocked.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        controller.pause_start_turn();

        // When: two sends race for the same session.
        let first_usecase = Arc::clone(&usecase);
        let first_session_id = session.id.clone();
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let first = tokio::spawn(async move {
            first_usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(first_session_id),
                    worktree_path,
                    content: "first".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("claude".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await
                .unwrap()
        });
        wait_for_start_prompt_count(&controller, &session.id, 1).await;

        let second_usecase = Arc::clone(&usecase);
        let second_session_id = session.id.clone();
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let second = tokio::spawn(async move {
            second_usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(second_session_id),
                    worktree_path,
                    content: "second".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("claude".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await
                .unwrap()
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1
        );
        controller.release_start_turn();
        let first = first.await.unwrap();
        let second = second.await.unwrap();

        // Then: only the first send starts a backend turn; the second is queued.
        assert!(first.agent_message.is_some());
        assert!(first.queued_turn.is_none());
        assert!(second.agent_message.is_none());
        assert!(second.queued_turn.is_some());
        assert_eq!(second.pending_queue_count, 1);
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn queued_turn_cancel_is_rejected_without_live_or_restart_visible_mutation() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let first = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = first.session.id;
        wait_for_start_prompt_count(&controller, &session_id, 1).await;
        let queued = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "must remain queued".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        let queued_id = queued.queued_turn.unwrap().id;
        let live_before = usecase.pending_queue(&session_id).await;
        let persisted_before = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap()
            .messages
            .into_iter()
            .map(|message| (message.id, message.content))
            .collect::<Vec<_>>();
        let events_before = format!(
            "{:?}",
            session_store
                .load_session_events(tmp.path(), &session_id)
                .unwrap()
        );

        let error = usecase
            .cancel_queued_turn(&session_id, Some(&queued_id))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("atomic durable queue operation"));
        assert_eq!(usecase.pending_queue(&session_id).await, live_before);
        assert_eq!(
            format!(
                "{:?}",
                session_store
                    .load_session_events(tmp.path(), &session_id)
                    .unwrap()
            ),
            events_before
        );
        let restarted =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());
        let persisted_after_restart = restarted
            .get_session(&session_id)
            .await
            .unwrap()
            .unwrap()
            .session
            .messages
            .into_iter()
            .map(|message| (message.id, message.content))
            .collect::<Vec<_>>();
        assert_eq!(persisted_after_restart, persisted_before);
    }

    #[tokio::test]
    async fn test_send_message_queue受理後のprojection障害でも成功応答を返す() {
        // Given: an active turn whose message index does not yet include an orphan chunk, and a
        // projection store that becomes unreadable while the queued human message is persisted.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let first = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = first.session.id;
        wait_for_start_prompt_count(&controller, &session_id, 1).await;
        let orphan = ChatMessage {
            id: "orphan-agent-message".to_string(),
            role: MessageRole::Agent,
            content: "recovered orphan".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            streaming_final_seq: 0,
            timestamp: first.session.updated_at,
            mentions: None,
        };
        let orphan_path = tmp
            .path()
            .join("sessions")
            .join(&session_id)
            .join("messages")
            .join("3.json");
        std::fs::write(
            orphan_path,
            crate::adaptor::gateway::agent_session::session_storage::encode_chat_message_v1(
                &orphan,
            )
            .expect("orphan message must serialize through legacy V1 DTO"),
        )
        .unwrap();
        let titles_path = tmp.path().join("session_titles.json");
        session_store.set_append_message_hook_for_test(Arc::new(move |_, _| {
            std::fs::write(&titles_path, "{").map_err(|error| error.to_string())
        }));

        // When: the follow-up is accepted into the pending queue.
        let response = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "queue exactly once".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .expect("accepted queue input must not fail during response projection");

        // Then: the accepted response uses the append's repaired post-write meta even though a
        // fresh all-session projection now fails, and the queued message exists exactly once.
        assert!(response.queued_turn.is_some());
        assert_eq!(response.pending_queue_count, 1);
        let response_message_count = response
            .sessions
            .iter()
            .find(|summary| summary.id == session_id)
            .map(|summary| summary.message_count)
            .expect("accepted session summary must be present");
        assert!(session_store
            .list_sessions(tmp.path(), tmp.path().to_string_lossy().as_ref())
            .is_err());
        let stored = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(response_message_count, 4);
        assert_eq!(response_message_count, stored.messages.len());
        assert!(stored
            .messages
            .iter()
            .any(|message| message.id == orphan.id));
        assert_eq!(
            stored
                .messages
                .iter()
                .filter(|message| message.content == "queue exactly once")
                .count(),
            1
        );
        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
    }

    #[tokio::test]
    async fn test_send_message_queue受理応答のsummaryにcustom_titleを再適用する() {
        // Given: a busy session with an observed stall and a custom title.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let first = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = first.session.id;
        wait_for_start_prompt_count(&controller, &session_id, 1).await;
        mark_stall_observation_active_for_test(&usecase, &session_id).await;
        let custom_title = "Investigate queued follow-up";
        session_store
            .set_session_title(tmp.path(), &session_id, Some(custom_title))
            .unwrap();

        // When: the follow-up is accepted into the pending queue.
        let response = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "queue this".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        // Then: replacing the summary with post-write meta does not discard the custom title.
        assert!(response.queued_turn.is_some());
        assert_eq!(
            response
                .sessions
                .iter()
                .find(|summary| summary.id == session_id)
                .map(|summary| summary.first_message.as_str()),
            Some(custom_title)
        );
    }

    #[tokio::test]
    async fn failed_terminal_pauses_queued_work_until_explicit_resume() {
        // Given: a session whose turn ends as Failed (e.g. Codex remote compact failure).
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let compact_error =
            "Error running remote compact task: stream disconnected before completion".to_string();
        let first = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = first.session.id.clone();
        wait_for_start_prompt_count(&controller, &session_id, 1).await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Error {
                    content: compact_error.clone(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Failed {
                    error: compact_error,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

        assert!(
            session_store
                .load_queue_paused_at(tmp.path(), &session_id)
                .unwrap()
                .is_some(),
            "a provider failure must durably pause the queue"
        );
        assert!(
            usecase
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused,
            "the live read model must agree with the durable pause"
        );

        // When: the user sends the next message to the same session.
        let second = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "continue".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        // Then: the follow-up remains queued until the user explicitly resumes it.
        assert!(second.agent_message.is_none());
        assert!(second.queued_turn.is_some());
        assert_eq!(second.pending_queue_count, 1);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .into_iter()
                .filter(|kind| *kind == TestRuntimeCallKind::StartTurn)
                .count(),
            1,
            "failure must not hand the queued input to the provider"
        );

        usecase.resume_queue(&session_id).await.unwrap();
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
    }

    async fn enqueue_second_turn_for_test(
        usecase: &Arc<AgentSessionRuntimeUsecase>,
        controller: &crate::test_support::TestAgentRuntimeController,
        worktree_path: String,
    ) -> String {
        let first = usecase
            .send_message(send_request(worktree_path.clone()))
            .await
            .unwrap();
        let session_id = first.session.id.clone();
        wait_for_start_prompt_count(controller, &session_id, 1).await;
        let second = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path,
                content: "queued".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        assert_eq!(second.pending_queue_count, 1);
        session_id
    }

    #[tokio::test]
    async fn damaged_event_log_is_recovered_and_next_message_send_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let first = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = first.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        let event_log_path = tmp
            .path()
            .join("sessions")
            .join(&session_id)
            .join("events.json");
        let content = std::fs::read_to_string(&event_log_path).unwrap();
        let closing_pos = content.rfind(']').expect("event log closing bracket");
        std::fs::write(&event_log_path, &content[..closing_pos]).unwrap();
        take_persistence_log_records();

        let second = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "continue after recovery".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        assert!(second.agent_message.is_some());
        assert!(event_notifier.notices().iter().any(|notice| {
            notice.session_id == session_id && notice.kind == SessionNoticeKind::EventLogRecovered
        }));
        let repaired_events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(repaired_events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnStarted { turn_id: 2, .. })));
        let records = take_persistence_log_records();
        assert!(records.iter().any(|record| matches!(
            record,
            PersistenceLogRecord::EventLogRecovered {
                session_id: logged_session_id,
                kind: "event_log_recovered",
            } if logged_session_id == &session_id
        )));
    }

    #[tokio::test]
    async fn batch_append_reports_event_log_recovery_through_blocking_path() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, _) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session = crate::usecase::agent_session::session::create_session_internal(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
        )
        .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::QueuePaused { at: 1.0 },
            )
            .unwrap();
        let event_log_path = tmp
            .path()
            .join("sessions")
            .join(&session.id)
            .join("events.json");
        let content = std::fs::read_to_string(&event_log_path).unwrap();
        let closing_pos = content.rfind(']').expect("event log closing bracket");
        std::fs::write(&event_log_path, &content[..closing_pos]).unwrap();

        append_session_events_blocking(
            &usecase.ctx,
            &session.id,
            vec![AgentSessionEvent::QueuePaused { at: 2.0 }],
        )
        .await
        .unwrap();

        assert!(event_notifier.notices().iter().any(|notice| {
            notice.session_id == session.id && notice.kind == SessionNoticeKind::EventLogRecovered
        }));
        assert_eq!(
            session_store
                .load_session_events(tmp.path(), &session.id)
                .unwrap()
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::QueuePaused { .. }))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn reopen_runtime_persist_failure_retries_reports_and_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, _controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_state_hook_for_test({
            let attempts = attempts.clone();
            Arc::new(move |_, _| {
                attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err("injected session state failure".to_string())
            })
        });
        take_persistence_log_records();

        let result = persist_with_retry(
            &usecase.ctx,
            &session_id,
            PersistFailureKind::ReopenRuntime,
            || session_store.set_session_state(tmp.path(), &session_id, SessionState::Error),
        )
        .await;

        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            PERSIST_MAX_ATTEMPTS
        );
        assert_eq!(result.unwrap_err(), "injected session state failure");
        assert!(event_notifier.notices().iter().any(|notice| {
            notice.session_id == session_id && notice.kind == SessionNoticeKind::PersistFailure
        }));
        assert_eq!(
            usecase
                .ctx
                .status_center
                .get_session(&session_id)
                .and_then(|status| status.notice)
                .map(|notice| notice.kind),
            Some(SessionNoticeKind::PersistFailure)
        );
        let records = take_persistence_log_records();
        assert!(records.iter().any(|record| matches!(
            record,
            PersistenceLogRecord::PersistFailure {
                session_id: logged_session_id,
                kind: "reopen_runtime",
                attempts: PERSIST_MAX_ATTEMPTS,
                error,
            } if logged_session_id == &session_id && error == "injected session state failure"
        )));
    }

    #[tokio::test]
    async fn queued_runtime_reopen_failure_retries_and_stays_visible() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier.clone(),
        );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        usecase
            .prepare_queued_runtime_reopen_for_test(&session_id)
            .await;
        controller.fail_next_open_session();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_state_hook_for_test({
            let attempts = attempts.clone();
            Arc::new(move |_, state| {
                if *state == SessionState::Error {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    return Err("injected queued reopen state failure".to_string());
                }
                Ok(())
            })
        });
        take_persistence_log_records();

        usecase.drain_next_queued_turn_for_test(&session_id).await;

        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            PERSIST_MAX_ATTEMPTS
        );
        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        assert_eq!(usecase.turn_phase(&session_id).await, Some(TurnPhase::Idle));
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2
        );
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1
        );
        assert!(event_notifier.notices().iter().any(|notice| {
            notice.session_id == session_id && notice.kind == SessionNoticeKind::PersistFailure
        }));
        assert!(!event_notifier.state_changes().iter().any(|change| {
            change.chat_session_id == session_id
                && change.turn_phase == TurnPhase::Idle
                && change.session_state == Some(SessionState::Error)
        }));
        let snapshot = usecase
            .ctx
            .status_center
            .get_session(&session_id)
            .expect("status snapshot");
        assert_eq!(snapshot.session_state, SessionState::Active);
        assert_eq!(
            snapshot.notice.map(|notice| notice.kind),
            Some(SessionNoticeKind::PersistFailure)
        );
        assert!(status_notifier.changes().iter().any(|changes| {
            changes.session.as_ref().is_some_and(|status| {
                status.chat_session_id == session_id
                    && status
                        .notice
                        .as_ref()
                        .is_some_and(|notice| notice.kind == SessionNoticeKind::PersistFailure)
            })
        }));
        assert_eq!(
            session_store
                .get_session_shell(tmp.path(), &session_id)
                .unwrap()
                .expect("durable session")
                .state,
            SessionState::Active
        );
        let records = take_persistence_log_records();
        assert!(records.iter().any(|record| matches!(
            record,
            PersistenceLogRecord::PersistFailure {
                session_id: logged_session_id,
                kind: "reopen_runtime",
                attempts: PERSIST_MAX_ATTEMPTS,
                error,
            } if logged_session_id == &session_id
                && error == "injected queued reopen state failure"
        )));
    }

    #[tokio::test]
    async fn transient_persist_failure_recovers_without_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, _controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_state_hook_for_test({
            let attempts = attempts.clone();
            Arc::new(move |_, _| {
                if attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    Err("transient session state failure".to_string())
                } else {
                    Ok(())
                }
            })
        });

        persist_with_retry(
            &usecase.ctx,
            &session_id,
            PersistFailureKind::ReopenRuntime,
            || session_store.set_session_state(tmp.path(), &session_id, SessionState::Error),
        )
        .await
        .unwrap();

        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(event_notifier.notices().is_empty());
    }

    #[tokio::test]
    async fn successful_persist_clears_previous_failure_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, _controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier,
            status_notifier.clone(),
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        session_store.set_state_hook_for_test(Arc::new(|_, _| {
            Err("injected session state failure".to_string())
        }));
        persist_with_retry(
            &usecase.ctx,
            &session_id,
            PersistFailureKind::ReopenRuntime,
            || session_store.set_session_state(tmp.path(), &session_id, SessionState::Error),
        )
        .await
        .unwrap_err();
        assert!(usecase
            .ctx
            .status_center
            .get_session(&session_id)
            .and_then(|status| status.notice)
            .is_some());

        session_store.set_state_hook_for_test(Arc::new(|_, _| Ok(())));
        persist_with_retry(
            &usecase.ctx,
            &session_id,
            PersistFailureKind::ReopenRuntime,
            || session_store.set_session_state(tmp.path(), &session_id, SessionState::Error),
        )
        .await
        .unwrap();

        assert!(usecase
            .ctx
            .status_center
            .get_session(&session_id)
            .and_then(|status| status.notice)
            .is_none());
        assert!(status_notifier.changes().iter().any(|changes| {
            changes.session.as_ref().is_some_and(|status| {
                status.chat_session_id == session_id && status.notice.is_none()
            })
        }));
    }

    #[tokio::test]
    async fn projection_retry_does_not_append_event_twice_after_partial_success() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, _controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier,
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let state_attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_state_hook_for_test({
            let state_attempts = state_attempts.clone();
            Arc::new(move |_, _| {
                state_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err("injected post-append projection failure".to_string())
            })
        });
        let event = AgentSessionEvent::FinalPartsRecorded {
            turn_id: 1,
            message_id: "agent-message".to_string(),
            parts: vec![MessagePart::Text {
                content: "durable once".to_string(),
                parent_tool_use_id: None,
            }],
        };

        let result = append_session_event_and_project_state_with_retry(
            &usecase.ctx,
            &session_id,
            PersistFailureKind::FinalPartsRecorded,
            event.clone(),
        )
        .await;

        assert_eq!(
            result.unwrap_err(),
            "injected post-append projection failure"
        );
        assert_eq!(
            state_attempts.load(std::sync::atomic::Ordering::SeqCst),
            PERSIST_MAX_ATTEMPTS
        );
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|candidate| **candidate == event)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn b024_normal_completion_commits_one_terminal_then_drains_the_next_queue_item() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();

        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert!(usecase.pending_queue(&session_id).await.is_empty());
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::TurnCompleted { turn_id: 1, .. }
                ))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnStarted { .. }))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn b040_unresolved_recovery_blocks_automatic_queue_drain_without_provider_effect() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        let recovery_id = "recovery-blocks-queue-drain";
        local_store
            .commit_batch(LocalAtomicBatch {
                commit_id: CommitIdentity::parse("b040-blocker-commit").unwrap(),
                idempotency: IdempotencyBinding {
                    installation_id: local_store.installation_id().to_string(),
                    operation_kind: CommitOperationKind::Recovery,
                    idempotency_key: "b040-blocker".to_string(),
                    payload_hash: [40; 32],
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: vec![LocalStateMutation::Obligation(ObligationMutation {
                    obligation_id: recovery_id.to_string(),
					record: crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
						session_id: session_id.clone(),
						recovery_id: recovery_id.to_string(),
						detail: crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
							old_provider_session_generation: 0,
							reason: crate::domain::agent_session::events::BackendSessionRecoveryReason::BackendSessionLost,
							reserved_at_bits: 0,
						},
						state: crate::domain::local_event::ObligationStateRecord::ReconciliationRequired,
					},
                    pending: Some(PendingIndexEntry {
                        ordered_key: format!("{recovery_id}:0001"),
                        owner: session_id.clone(),
                        partition: PendingPartition::Owner,
                        shutdown_plan: None,
                    }),
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).unwrap(),
                })],
            })
            .await
            .unwrap();

        let failure = session_store
            .ensure_no_unresolved_recovery(&session_id)
            .await
            .unwrap_err();
        assert_eq!(failure.correlation_id, recovery_id);
        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1,
            "the unresolved recovery fence must run before queue/provider dispatch"
        );
    }

    #[tokio::test]
    async fn queued_turn_append_message_failure_preserves_queue_and_retries() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        let fail_once = Arc::new(std::sync::atomic::AtomicBool::new(true));
        session_store.set_append_message_hook_for_test({
            let fail_once = Arc::clone(&fail_once);
            Arc::new(move |_, message| {
                if message.role == MessageRole::Agent
                    && fail_once.swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    return Err("injected append message failure".to_string());
                }
                Ok(())
            })
        });

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1
        );
        usecase.drain_next_queued_turn_for_test(&session_id).await;
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert!(usecase.pending_queue(&session_id).await.is_empty());
    }

    #[tokio::test]
    async fn queued_turn_started_event_failure_preserves_queue_and_retries() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        let fail_once = Arc::new(std::sync::atomic::AtomicBool::new(true));
        session_store.set_append_event_hook_for_test({
            let fail_once = Arc::clone(&fail_once);
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::TurnStarted { .. })
                    && fail_once.swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    return Err("injected turn started failure".to_string());
                }
                Ok(())
            })
        });

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        usecase.drain_next_queued_turn_for_test(&session_id).await;
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert!(usecase.pending_queue(&session_id).await.is_empty());
    }

    #[tokio::test]
    async fn queued_turn_start_turn_failure_preserves_queue_and_retries() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        controller.fail_next_start_turn();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_some());
        usecase.resume_queue(&session_id).await.unwrap();
        wait_for_start_prompt_count(&controller, &session_id, 3).await;
        assert!(usecase.pending_queue(&session_id).await.is_empty());
    }

    #[tokio::test]
    async fn queued_turn_interrupt_append_retries_then_reports() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_append_event_hook_for_test({
            let attempts = attempts.clone();
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::TurnInterrupted { .. }) {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    return Err("injected turn interruption failure".to_string());
                }
                Ok(())
            })
        });
        controller.fail_next_start_turn();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if attempts.load(std::sync::atomic::Ordering::SeqCst) == PERSIST_MAX_ATTEMPTS
                    && event_notifier.notices().iter().any(|notice| {
                        notice.session_id == session_id
                            && notice.kind == SessionNoticeKind::PersistFailure
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("queued interruption persistence should exhaust retries");

        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
    }

    #[tokio::test]
    async fn turn_completed_append_failure_is_retained_until_the_terminal_commit_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier.clone(),
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_append_event_hook_for_test({
            let attempts = attempts.clone();
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::TurnCompleted { .. }) {
                    let attempt = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if attempt < PERSIST_MAX_ATTEMPTS {
                        return Err("injected turn completed failure".to_string());
                    }
                }
                Ok(())
            })
        });

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if usecase.turn_phase(&session_id).await == Some(TurnPhase::Idle) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the retained terminal event should commit after storage recovers");
        assert_eq!(usecase.turn_phase(&session_id).await, Some(TurnPhase::Idle));

        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            PERSIST_MAX_ATTEMPTS + 1
        );
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::FinalPartsRecorded { .. })));
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnCompleted { .. })));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnCompleted { .. }))
                .count(),
            1
        );
        assert!(event_notifier.notices().iter().any(|notice| {
            notice.session_id == session_id && notice.kind == SessionNoticeKind::PersistFailure
        }));
        assert!(status_notifier.changes().iter().any(|changes| {
            changes.session.as_ref().is_some_and(|status| {
                status.chat_session_id == session_id
                    && status
                        .notice
                        .as_ref()
                        .is_some_and(|notice| notice.kind == SessionNoticeKind::PersistFailure)
            })
        }));
    }

    #[tokio::test]
    async fn final_parts_append_failure_keeps_body_not_tool_only() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![
                    DomainMessagePart::Text {
                        content: "persisted response body".to_string(),
                        parent_tool_use_id: None,
                    },
                    DomainMessagePart::ToolUse {
                        id: "tool-1".to_string(),
                        tool: "Bash".to_string(),
                        input:
                            crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                "{}".to_string(),
                            ),
                        parent_tool_use_id: None,
                    },
                ]),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let parts = usecase.streaming_parts(&session_id).await;
                if parts
                    .iter()
                    .any(|part| matches!(part, MessagePart::Text { .. }))
                    && parts
                        .iter()
                        .any(|part| matches!(part, MessagePart::ToolUse { .. }))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("streaming body and tool part should be applied");
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_append_event_hook_for_test({
            let attempts = attempts.clone();
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::FinalPartsRecorded { .. }) {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    return Err("injected final parts failure".to_string());
                }
                Ok(())
            })
        });

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if attempts.load(std::sync::atomic::Ordering::SeqCst) == PERSIST_MAX_ATTEMPTS
                    && event_notifier.notices().iter().any(|notice| {
                        notice.session_id == session_id
                            && notice.kind == SessionNoticeKind::PersistFailure
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("final parts persistence should exhaust retries");
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );

        let fresh_store = build_session_store();
        let reloaded = fresh_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("reloaded session");
        let agent_parts = reloaded
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::Agent)
            .and_then(|message| message.parts.as_ref())
            .expect("agent message parts");
        assert!(agent_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content == "persisted response body"
        )));
        assert!(agent_parts
            .iter()
            .any(|part| matches!(part, MessagePart::ToolUse { .. })));
    }

    #[tokio::test]
    async fn interrupt_durably_pauses_queue_and_resume_explicitly_starts_it() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        let appended = Arc::new(Mutex::new(Vec::new()));
        let appended_for_hook = Arc::clone(&appended);
        let controller_for_hook = controller.clone();
        session_store.set_append_event_hook_for_test(Arc::new(move |session_id, event| {
            if matches!(
                event,
                AgentSessionEvent::TurnInterruptRequested { .. }
                    | AgentSessionEvent::QueuePaused { .. }
            ) {
                assert!(!controller_for_hook
                    .call_kinds_for(session_id)
                    .contains(&TestRuntimeCallKind::Interrupt));
                appended_for_hook.lock().unwrap().push(event.clone());
            }
            Ok(())
        }));

        usecase.interrupt(&session_id).await.unwrap();

        {
            let appended = appended.lock().unwrap();
            assert_eq!(appended.len(), 2);
            assert!(matches!(
                &appended[0],
                AgentSessionEvent::TurnInterruptRequested { turn_id: 1, .. }
            ));
            assert!(matches!(
                &appended[1],
                AgentSessionEvent::QueuePaused { .. }
            ));
        }
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Interrupt));
        assert!(
            usecase
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Abort,
                    error: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurn))
                .count(),
            1
        );

        usecase.resume_queue(&session_id).await.unwrap();

        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if usecase.pending_queue(&session_id).await.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            !usecase
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
    }

    #[tokio::test]
    async fn resume_append_failure_keeps_the_pending_queue_durably_paused() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        usecase.interrupt(&session_id).await.unwrap();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Abort,
                    error: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        let start_count_before_resume = controller
            .call_kinds_for(&session_id)
            .iter()
            .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurn))
            .count();
        let state_change_count_before_resume = event_notifier.state_changes().len();
        session_store.set_append_event_hook_for_test(Arc::new(|_, event| {
            if matches!(event, AgentSessionEvent::QueueResumed { .. }) {
                return Err("injected QueueResumed append failure".to_string());
            }
            Ok(())
        }));

        let error = usecase.resume_queue(&session_id).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("injected QueueResumed append failure"));
        assert!(
            usecase
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
        assert_eq!(
            event_notifier.state_changes().len(),
            state_change_count_before_resume
        );
        assert!(!event_notifier
            .state_changes()
            .iter()
            .skip(state_change_count_before_resume)
            .any(|change| change.queue_paused == Some(false)));
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurn))
                .count(),
            start_count_before_resume
        );
        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_some());

        let restarted =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());
        assert!(
            restarted
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
    }

    #[tokio::test]
    async fn shutdown_admission_rejects_queue_resume_without_clearing_the_durable_pause() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        usecase.interrupt(&session_id).await.unwrap();
        usecase.ctx.shutdown_admission.begin_shutdown();

        let error = usecase.resume_queue(&session_id).await.unwrap_err();

        assert!(error.to_string().contains("shutting down"));
        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_some());
        assert!(!session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::QueueResumed { .. })));
    }

    #[tokio::test]
    async fn interrupt_after_active_turn_resume_reestablishes_the_durable_pause() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;

        usecase.interrupt(&session_id).await.unwrap();
        usecase.resume_queue(&session_id).await.unwrap();
        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_none());
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );

        usecase.interrupt(&session_id).await.unwrap();

        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_some());
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Abort,
                    error: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurn))
                .count(),
            1
        );
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::QueuePaused { .. }))
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::QueueResumed { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn concurrent_resume_is_persisted_after_the_inflight_pause() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let gate = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let hook_gate = Arc::clone(&gate);
        session_store.set_append_event_hook_for_test(Arc::new(move |_, event| {
            if matches!(event, AgentSessionEvent::QueuePaused { .. }) {
                let (lock, condvar) = &*hook_gate;
                let mut state = lock.lock().unwrap();
                state.0 = true;
                condvar.notify_all();
                while !state.1 {
                    state = condvar.wait(state).unwrap();
                }
            }
            Ok(())
        }));

        let interrupt_usecase = Arc::clone(&usecase);
        let interrupt_session_id = session_id.clone();
        let interrupt = tokio::spawn(async move {
            interrupt_usecase
                .interrupt(&interrupt_session_id)
                .await
                .unwrap();
        });
        loop {
            if gate.0.lock().unwrap().0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        let resume_usecase = Arc::clone(&usecase);
        let resume_session_id = session_id.clone();
        let resume = tokio::spawn(async move {
            resume_usecase
                .resume_queue(&resume_session_id)
                .await
                .unwrap();
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!resume.is_finished());

        {
            let (lock, condvar) = &*gate;
            let mut state = lock.lock().unwrap();
            state.1 = true;
            condvar.notify_all();
        }
        interrupt.await.unwrap();
        resume.await.unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        let pause_index = events
            .iter()
            .position(|event| matches!(event, AgentSessionEvent::QueuePaused { .. }))
            .unwrap();
        let resume_index = events
            .iter()
            .position(|event| matches!(event, AgentSessionEvent::QueueResumed { .. }))
            .unwrap();
        assert!(pause_index < resume_index);
        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_none());
        assert!(
            !usecase
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Interrupt));
    }

    #[tokio::test]
    async fn interrupt_watchdog_force_finalizes_an_unresponsive_backend_as_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let generation = {
            let sessions = usecase.ctx.sessions.lock().await;
            sessions.get(&session_id).unwrap().generation
        };

        usecase.interrupt(&session_id).await.unwrap();
        spawn_interrupt_watchdog_task(
            &usecase.ctx,
            session_id.clone(),
            generation,
            Duration::from_millis(10),
        );
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Timeout,
                ..
            }
        )));
        let loaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(loaded.session.state, SessionState::Error);
        assert!(loaded.queue_paused);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::Interrupt))
                .count(),
            1
        );
    }

    #[tokio::test(start_paused = true)]
    async fn production_interrupt_watchdog_finalizes_at_the_ten_second_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let mut request = send_request(tmp.path().to_string_lossy().to_string());
        request.backend_id = Some("codex".to_string());
        let response = usecase.send_message(request).await.unwrap();
        let session_id = response.session.id;
        let queued = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "queued input stays intact".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: Some(vec![ImageAttachment {
                    data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".to_string(),
                    media_type: "image/png".to_string(),
                }]),
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        assert_eq!(queued.pending_queue_count, 1);
        assert_eq!(
            queued.pending_queue[0].content_preview,
            "queued input stays intact"
        );
        assert_eq!(queued.pending_queue[0].image_count, 1);

        usecase.interrupt(&session_id).await.unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(9)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );

        tokio::time::advance(Duration::from_secs(1)).await;
        for _ in 0..20 {
            if usecase.turn_phase(&session_id).await == Some(TurnPhase::Idle) {
                break;
            }
            tokio::task::yield_now().await;
        }

        assert_eq!(usecase.turn_phase(&session_id).await, Some(TurnPhase::Idle));
        let preserved_queue = usecase.pending_queue(&session_id).await;
        assert_eq!(preserved_queue.len(), 1);
        assert_eq!(
            preserved_queue[0].content_preview,
            "queued input stays intact"
        );
        assert_eq!(preserved_queue[0].image_count, 1);
        assert!(
            usecase
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
        assert!(session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::TurnInterrupted {
                    reason: EventInterruptReason::Timeout,
                    ..
                }
            )));
    }

    #[tokio::test(start_paused = true)]
    async fn production_interrupt_from_waiting_permission_clears_permission_and_stays_paused() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-stop")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;

        usecase.interrupt(&session_id).await.unwrap();

        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_some());
        tokio::task::yield_now().await;
        tokio::time::advance(INTERRUPT_FORCE_FINALIZE_DELAY).await;
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        let loaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert!(loaded.queue_paused);
        assert!(loaded.pending_permission_request.is_none());
        assert!(session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::TurnInterrupted {
                    reason: EventInterruptReason::Timeout,
                    exit_code: 124,
                    ..
                }
            )));
    }

    #[tokio::test(start_paused = true)]
    async fn backend_interrupt_failure_keeps_the_accepted_stop_until_timeout_terminal() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller.fail_next_interrupt();

        usecase.interrupt(&session_id).await.unwrap();

        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_some());
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::Interrupt))
                .count(),
            1
        );
        tokio::task::yield_now().await;
        tokio::time::advance(INTERRUPT_FORCE_FINALIZE_DELAY).await;
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        let loaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert!(loaded.queue_paused);
        assert!(session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::TurnInterrupted {
                    reason: EventInterruptReason::Timeout,
                    exit_code: 124,
                    ..
                }
            )));
    }

    #[tokio::test(start_paused = true)]
    async fn claude_synthetic_timeout_wins_the_timer_race_without_changing_the_terminal_reason() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let synthetic_controller = controller.clone();
        let synthetic_session_id = session_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(INTERRUPT_FORCE_FINALIZE_DELAY).await;
            synthetic_controller
                .emit(
                    &synthetic_session_id,
                    AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                        reason: DomainInterruptReason::Timeout,
                        error: None,
                    }),
                )
                .unwrap();
        });
        tokio::task::yield_now().await;

        usecase.interrupt(&session_id).await.unwrap();
        tokio::time::advance(INTERRUPT_FORCE_FINALIZE_DELAY).await;
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        tokio::task::yield_now().await;

        let terminal_events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .into_iter()
            .filter(|event| {
                matches!(
                    event,
                    AgentSessionEvent::TurnCompleted { .. }
                        | AgentSessionEvent::TurnInterrupted { .. }
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(terminal_events.len(), 1);
        assert!(matches!(
            terminal_events[0],
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Timeout,
                exit_code: 124,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn provider_terminal_results_after_interrupt_are_preserved() {
        for result in [
            TurnResult::Completed {
                stop_reason: None,
                token_usage: None,
            },
            TurnResult::Failed {
                error: "late start failure".to_string(),
                token_usage: None,
            },
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let session_store = Arc::new(build_session_store());
            let (usecase, controller) =
                crate::test_support::build_agent_runtime_usecase_with_controller(
                    session_store.clone(),
                    tmp.path(),
                );
            let response = usecase
                .send_message(send_request(tmp.path().to_string_lossy().to_string()))
                .await
                .unwrap();
            let session_id = response.session.id;

            usecase.interrupt(&session_id).await.unwrap();
            controller
                .emit(&session_id, AgentRuntimeEvent::TurnCompleted(result))
                .unwrap();
            wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

            let events = session_store
                .load_session_events(tmp.path(), &session_id)
                .unwrap();
            assert!(events
                .iter()
                .any(|event| matches!(event, AgentSessionEvent::TurnCompleted { .. })));
            assert!(!events.iter().any(|event| matches!(
                event,
                AgentSessionEvent::TurnInterrupted {
                    reason: EventInterruptReason::Abort,
                    ..
                }
            )));
        }
    }

    #[tokio::test]
    async fn queue_pause_and_explicit_resume_survive_runtime_state_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        usecase.interrupt(&session_id).await.unwrap();

        let restarted =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());
        assert!(
            restarted
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );

        restarted.resume_queue(&session_id).await.unwrap();
        let restarted_again =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());
        assert!(
            !restarted_again
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
        assert!(session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::QueueResumed { .. })));
    }

    #[tokio::test]
    async fn queue_resume_after_restart_is_fenced_by_unfinished_backend_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let original_store = Arc::new(build_session_store());
        let session = create_session_internal_with_attributes(
            &original_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        original_store
            .begin_backend_session_recovery(
                tmp.path(),
                &session.id,
                "resume-fence-recovery",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        original_store
            .append_session_events(
                tmp.path(),
                &session.id,
                &[AgentSessionEvent::QueuePaused { at: 8.0 }],
            )
            .unwrap();
        drop(original_store);
        let reopened_store = Arc::new(build_session_store());
        let (restarted, controller) =
            build_agent_runtime_usecase_with_controller(reopened_store.clone(), tmp.path());

        let error = restarted.resume_queue(&session.id).await.unwrap_err();

        assert!(
            error.to_string().contains("requires reconciliation"),
            "unexpected recovery fence error: {error}"
        );
        assert!(reopened_store
            .load_queue_paused_at(tmp.path(), &session.id)
            .unwrap()
            .is_some());
        assert!(!reopened_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::QueueResumed { .. })));
        assert!(!controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));
    }

    #[tokio::test]
    async fn durable_pause_is_hydrated_before_direct_send_after_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let mut first_request = send_request(tmp.path().to_string_lossy().to_string());
        first_request.backend_id = Some("codex".to_string());
        let response = usecase.send_message(first_request).await.unwrap();
        let session_id = response.session.id;
        usecase.interrupt(&session_id).await.unwrap();

        let (restarted, restarted_controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        let queued = restarted
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "must remain queued until explicit resume".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        assert_eq!(queued.pending_queue_count, 1);
        assert!(!restarted_controller
            .call_kinds_for(&session_id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));

        restarted.resume_queue(&session_id).await.unwrap();
        wait_for_start_prompt_count(&restarted_controller, &session_id, 1).await;
    }

    #[tokio::test]
    async fn interrupt_while_runtime_open_is_pending_prevents_provider_turn_start() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &usecase.ctx.session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        controller.pause_open_session();
        let send_usecase = Arc::clone(&usecase);
        let session_id = session.id.clone();
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let send = tokio::spawn(async move {
            send_usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(session_id),
                    worktree_path,
                    content: "stop during runtime open".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("codex".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await
        });
        for _ in 0..100 {
            if controller
                .call_kinds_for(&session.id)
                .iter()
                .any(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. })));
        let generation = {
            let sessions = usecase.ctx.sessions.lock().await;
            sessions.get(&session.id).unwrap().generation
        };

        usecase.interrupt(&session.id).await.unwrap();
        controller.release_open_session();
        send.await.unwrap().unwrap();

        assert!(!controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));
        force_finalize_interrupted_turn(&usecase.ctx, &session.id, generation).await;
        assert_eq!(usecase.turn_phase(&session.id).await, Some(TurnPhase::Idle));
    }

    #[tokio::test(start_paused = true)]
    async fn runtime_open_failure_after_interrupt_timeout_does_not_replace_the_terminal_result() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session = create_session_internal_with_attributes(
            &usecase.ctx.session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        controller.pause_open_session();
        controller.fail_next_open_session();
        let send_usecase = Arc::clone(&usecase);
        let session_id = session.id.clone();
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let send = tokio::spawn(async move {
            send_usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(session_id),
                    worktree_path,
                    content: "runtime open eventually fails".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("codex".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await
        });
        for _ in 0..100 {
            if controller
                .call_kinds_for(&session.id)
                .iter()
                .any(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. })));

        usecase.interrupt(&session.id).await.unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(INTERRUPT_FORCE_FINALIZE_DELAY).await;
        for _ in 0..20 {
            if usecase.turn_phase(&session.id).await == Some(TurnPhase::Idle) {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(usecase.turn_phase(&session.id).await, Some(TurnPhase::Idle));
        let terminal_notification_count = event_notifier.state_changes().len();

        controller.release_open_session();
        send.await
            .unwrap()
            .expect("timeout terminal result owns the late runtime open failure");
        tokio::task::yield_now().await;

        assert_eq!(
            event_notifier.state_changes().len(),
            terminal_notification_count,
            "late runtime open failure must not publish a second error state"
        );
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::TurnCompleted { .. }
                        | AgentSessionEvent::TurnInterrupted { .. }
                ))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::TurnInterrupted {
                        reason: EventInterruptReason::Timeout,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Crash,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn runtime_state_hydration_rejects_a_missing_backend_without_claude_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let usecase = crate::test_support::build_agent_runtime_usecase(session_store, tmp.path());
        let mut session = create_session_internal_with_attributes(
            &usecase.ctx.session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes::default(),
        )
        .unwrap();
        session.backend_id = None;

        let error = usecase
            .hydrate_runtime_session_state(&session)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("missing backend id"));
        assert!(!usecase.ctx.sessions.lock().await.contains_key(&session.id));
    }

    #[tokio::test]
    async fn b027_past_turn_late_streaming_and_terminal_events_leave_new_turn_unchanged_live_and_reload(
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        let generation = {
            let sessions = usecase.ctx.sessions.lock().await;
            sessions.get(&session_id).unwrap().generation
        };
        usecase.interrupt(&session_id).await.unwrap();
        force_finalize_interrupted_turn(&usecase.ctx, &session_id, generation).await;

        usecase.resume_queue(&session_id).await.unwrap();
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2
        );
        let before_session = usecase.get_session(&session_id).await.unwrap().unwrap();
        let before_events = usecase
            .ctx
            .session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        let project_messages = |messages: &[ChatMessage]| {
            messages
                .iter()
                .map(|message| {
                    (
                        message.id.clone(),
                        message.role.clone(),
                        message.content.clone(),
                        message.parts.clone(),
                        message.streaming_final_seq,
                    )
                })
                .collect::<Vec<_>>()
        };
        let before_messages = project_messages(&before_session.session.messages);
        let before_state = before_session.session.state;
        controller
            .emit_for_runtime(
                &session_id,
                0,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "late t-1 output".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        controller
            .emit_for_runtime(
                &session_id,
                0,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
        let after_live = usecase.get_session(&session_id).await.unwrap().unwrap();
        let after_reload = usecase
            .ctx
            .session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        let after_events = usecase
            .ctx
            .session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            project_messages(&after_live.session.messages),
            before_messages
        );
        assert_eq!(project_messages(&after_reload.messages), before_messages);
        assert_eq!(after_live.session.state, before_state);
        assert_eq!(after_reload.state, before_state);
        assert_eq!(after_events, before_events);
        assert!(!after_live.session.messages.iter().any(|message| {
            message.parts.as_deref().is_some_and(|parts| {
                parts.iter().any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content == "late t-1 output")
                })
            })
        }));
    }

    #[tokio::test]
    async fn timeout_while_start_turn_is_pending_does_not_publish_late_streaming_state() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session = create_session_internal_with_attributes(
            &usecase.ctx.session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        controller.pause_start_turn();
        let send_usecase = Arc::clone(&usecase);
        let session_id = session.id.clone();
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let send = tokio::spawn(async move {
            send_usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(session_id),
                    worktree_path,
                    content: "start then stop".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("codex".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await
        });
        wait_for_start_prompt_count(&controller, &session.id, 1).await;
        let generation = {
            let sessions = usecase.ctx.sessions.lock().await;
            sessions.get(&session.id).unwrap().generation
        };

        usecase.interrupt(&session.id).await.unwrap();
        force_finalize_interrupted_turn(&usecase.ctx, &session.id, generation).await;
        controller.release_start_turn();
        send.await.unwrap().unwrap();
        tokio::task::yield_now().await;

        assert_eq!(usecase.turn_phase(&session.id).await, Some(TurnPhase::Idle));
        let changes = event_notifier.state_changes();
        let terminal_index = changes
            .iter()
            .rposition(|change| {
                change.chat_session_id == session.id && change.turn_phase == TurnPhase::Idle
            })
            .expect("timeout terminal notification");
        assert!(!changes.iter().skip(terminal_index + 1).any(|change| {
            change.chat_session_id == session.id && change.turn_phase == TurnPhase::Streaming
        }));
    }

    #[tokio::test(start_paused = true)]
    async fn interrupt_timeout_releases_command_waiters_while_old_start_turn_remains_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &usecase.ctx.session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let first_start_gate = controller.pause_next_start_turn();
        let first = {
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            let worktree_path = tmp.path().to_string_lossy().to_string();
            tokio::spawn(async move {
                usecase
                    .send_message(SendAgentMessageRequest {
                        chat_session_id: Some(session_id),
                        worktree_path,
                        content: "provider start remains pending".to_string(),
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                        backend_id: Some("codex".to_string()),
                        model_id: None,
                        images: None,
                        mentions: None,
                        editor_context: None,
                    })
                    .await
            })
        };
        for _ in 0..100 {
            if controller
                .call_kinds_for(&session.id)
                .iter()
                .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(!first.is_finished());

        let waiting_send = {
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            let worktree_path = tmp.path().to_string_lossy().to_string();
            tokio::spawn(async move {
                usecase
                    .send_message(SendAgentMessageRequest {
                        chat_session_id: Some(session_id),
                        worktree_path,
                        content: "queue after timeout".to_string(),
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                        backend_id: Some("codex".to_string()),
                        model_id: None,
                        images: None,
                        mentions: None,
                        editor_context: None,
                    })
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert!(!waiting_send.is_finished());

        usecase.interrupt(&session.id).await.unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(INTERRUPT_FORCE_FINALIZE_DELAY).await;
        for _ in 0..100 {
            if usecase.turn_phase(&session.id).await == Some(TurnPhase::Idle)
                && waiting_send.is_finished()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(usecase.turn_phase(&session.id).await, Some(TurnPhase::Idle));
        let queued = waiting_send.await.unwrap().unwrap();
        assert_eq!(queued.pending_queue_count, 1);
        assert!(!first.is_finished());

        usecase.resume_queue(&session.id).await.unwrap();
        for _ in 0..100 {
            if controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurn))
                .count()
                == 2
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2
        );
        assert!(!first.is_finished());

        first_start_gate.notify_waiters();
        first.await.unwrap().unwrap();
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn interrupt_timeout_waits_for_in_flight_permission_event_before_terminal_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let generation = {
            let sessions = usecase.ctx.sessions.lock().await;
            sessions.get(&session_id).unwrap().generation
        };
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let block_once = Arc::new(std::sync::atomic::AtomicBool::new(true));
        event_notifier.set_streaming_delta_hook({
            let release_rx = Arc::clone(&release_rx);
            let block_once = Arc::clone(&block_once);
            Arc::new(move || {
                if block_once.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    entered_tx.send(()).unwrap();
                    release_rx.lock().unwrap().recv().unwrap();
                }
            })
        });

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-racing-timeout")),
            )
            .unwrap();
        tokio::task::spawn_blocking(move || {
            entered_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("permission event reached its commit path")
        })
        .await
        .unwrap();
        let force_ctx = usecase.ctx.clone();
        let force_session_id = session_id.clone();
        let force = tokio::spawn(async move {
            force_finalize_interrupted_turn(&force_ctx, &force_session_id, generation).await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !force.is_finished(),
            "timeout must serialize with the in-flight runtime event"
        );

        release_tx.send(()).unwrap();
        force.await.unwrap();

        let loaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(loaded.turn_phase, TurnPhase::Idle);
        assert!(loaded.pending_permission_request.is_none());
        let changes = event_notifier.state_changes();
        let terminal_index = changes
            .iter()
            .rposition(|change| {
                change.chat_session_id == session_id && change.turn_phase == TurnPhase::Idle
            })
            .expect("timeout terminal notification");
        assert!(!changes.iter().skip(terminal_index + 1).any(|change| {
            change.chat_session_id == session_id
                && (change.turn_phase != TurnPhase::Idle
                    || change.pending_permission_request.is_some())
        }));
    }

    #[tokio::test]
    async fn interrupt_watchdog_is_a_noop_for_a_different_turn_generation() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let stale_generation = {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session_id).unwrap();
            let stale_generation = state.generation;
            state.generation += 1;
            stale_generation
        };

        force_finalize_interrupted_turn(&usecase.ctx, &session_id, stale_generation).await;

        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Timeout,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn interrupt_fails_before_backend_io_when_durable_append_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        session_store.set_append_event_hook_for_test(Arc::new(|_, event| {
            if matches!(event, AgentSessionEvent::TurnInterruptRequested { .. }) {
                return Err("injected interrupt acceptance append failure".to_string());
            }
            Ok(())
        }));

        let error = usecase.interrupt(&session_id).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("injected interrupt acceptance append failure"));
        let loaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert!(!loaded.queue_paused);
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::Interrupt))
                .count(),
            0
        );
        assert!(!event_notifier
            .state_changes()
            .iter()
            .any(|change| change.queue_paused == Some(true)));
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterruptRequested { .. }
                | AgentSessionEvent::QueuePaused { .. }
        )));

        let restarted =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());
        assert!(
            !restarted
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
    }

    #[tokio::test]
    async fn interrupt_durable_io_does_not_hold_the_runtime_state_mutex() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let first = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let second = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let blocked_session_id = first.session.id.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        session_store.set_append_event_hook_for_test({
            let release_rx = Arc::clone(&release_rx);
            Arc::new(move |session_id, event| {
                if session_id == blocked_session_id
                    && matches!(event, AgentSessionEvent::TurnInterruptRequested { .. })
                {
                    entered_tx.send(()).unwrap();
                    release_rx.lock().unwrap().recv().unwrap();
                }
                Ok(())
            })
        });
        let interrupt_usecase = Arc::clone(&usecase);
        let first_session_id = first.session.id.clone();
        let interrupt =
            tokio::spawn(async move { interrupt_usecase.interrupt(&first_session_id).await });
        tokio::task::spawn_blocking(move || {
            entered_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("durable append hook")
        })
        .await
        .unwrap();

        let other_session = tokio::time::timeout(
            Duration::from_millis(200),
            usecase.get_session(&second.session.id),
        )
        .await
        .expect("another session must remain readable during interrupt commit")
        .unwrap()
        .unwrap();
        assert_eq!(other_session.turn_phase, TurnPhase::Streaming);
        assert_eq!(
            usecase.turn_phase(&first.session.id).await,
            Some(TurnPhase::Streaming)
        );

        release_tx.send(()).unwrap();
        interrupt.await.unwrap().unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn interrupt_acceptance_and_start_failure_preserve_the_crash_terminal_winner() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let start_gate = controller.pause_next_start_turn();
        controller.fail_next_start_turn();
        let send = {
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            let worktree_path = tmp.path().to_string_lossy().to_string();
            tokio::spawn(async move {
                usecase
                    .send_message(SendAgentMessageRequest {
                        chat_session_id: Some(session_id),
                        worktree_path,
                        content: "start fails during stop commit".to_string(),
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                        backend_id: Some("codex".to_string()),
                        model_id: None,
                        images: None,
                        mentions: None,
                        editor_context: None,
                    })
                    .await
            })
        };
        wait_for_start_prompt_count(&controller, &session.id, 1).await;

        let (append_entered_tx, append_entered_rx) = std::sync::mpsc::channel();
        let (release_append_tx, release_append_rx) = std::sync::mpsc::channel();
        let release_append_rx = Arc::new(Mutex::new(release_append_rx));
        let block_once = Arc::new(std::sync::atomic::AtomicBool::new(true));
        session_store.set_append_event_hook_for_test({
            let release_append_rx = Arc::clone(&release_append_rx);
            let block_once = Arc::clone(&block_once);
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::TurnInterruptRequested { .. })
                    && block_once.swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    append_entered_tx.send(()).unwrap();
                    release_append_rx.lock().unwrap().recv().unwrap();
                }
                Ok(())
            })
        });
        let interrupt = {
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            tokio::spawn(async move { usecase.interrupt(&session_id).await })
        };
        tokio::task::spawn_blocking(move || {
            append_entered_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("interrupt append should be blocked")
        })
        .await
        .unwrap();

        start_gate.notify_waiters();
        tokio::task::yield_now().await;
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming),
            "start failure must wait behind the durable Stop transition"
        );
        release_append_tx.send(()).unwrap();
        interrupt.await.unwrap().unwrap();
        send.await
            .unwrap()
            .expect("durably accepted Stop owns the concurrent start failure");

        assert_eq!(usecase.turn_phase(&session.id).await, Some(TurnPhase::Idle));
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert!(loaded.queue_paused);
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Abort,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Crash,
                ..
            }
        )));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. }))
                .count(),
            1,
            "the shared terminal arbiter must commit exactly one winner",
        );
        assert!(event_notifier.state_changes().iter().any(|change| {
            change.chat_session_id == session.id
                && change.turn_phase == TurnPhase::Idle
                && change.queue_paused == Some(true)
        }));
    }

    #[tokio::test]
    async fn repeated_interrupt_force_finalizes_immediately_and_remains_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;

        usecase.interrupt(&session_id).await.unwrap();
        usecase.interrupt(&session_id).await.unwrap();

        assert_eq!(usecase.turn_phase(&session_id).await, Some(TurnPhase::Idle));
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterruptRequested { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::QueuePaused { .. }))
                .count(),
            1
        );
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Timeout,
                ..
            }
        )));
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::Interrupt))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn crash_emits_projected_error_snapshot_before_state_change_and_matches_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![
                    DomainMessagePart::Text {
                        content: "partial output".to_string(),
                        parent_tool_use_id: None,
                    },
                    DomainMessagePart::ToolUse {
                        id: "tool-1".to_string(),
                        tool: "Bash".to_string(),
                        input:
                            crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                "{}".to_string(),
                            ),
                        parent_tool_use_id: None,
                    },
                ]),
            )
            .unwrap();
        wait_for_stream_delta_count(&event_notifier, 1).await;
        let order_start = event_notifier.event_order().len();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "CLI process exited".to_string(),
                },
            )
            .unwrap();

        wait_for_last_stream_delta(&event_notifier, |delta| {
            delta.snapshot
                && delta.parts.iter().any(|part| {
                    matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                })
        })
        .await;
        wait_for_error_state_change(&event_notifier, &session_id).await;
        let live = event_notifier
            .streaming_deltas()
            .into_iter()
            .rev()
            .find(|delta| {
                delta.snapshot
                    && delta.parts.iter().any(|part| {
                        matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                    })
            })
            .unwrap();
        assert!(live.parts.iter().any(|part| {
            matches!(part, MessagePart::Text { content, .. } if content == "partial output")
        }));
        assert!(live.parts.iter().any(|part| {
            matches!(
                part,
                MessagePart::ToolResult {
                    tool_use_id: Some(tool_use_id),
                    is_error: true,
                    ..
                } if tool_use_id == "tool-1"
            )
        }));
        assert_eq!(
            &event_notifier.event_order()[order_start..],
            &["streaming_delta", "state_change"]
        );

        let reloaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(
            reloaded.session.error_reason.as_deref(),
            Some("CLI process exited")
        );
        let persisted = reloaded
            .session
            .messages
            .iter()
            .find(|message| message.id == live.message_id)
            .and_then(|message| message.parts.clone())
            .unwrap();
        assert_eq!(live.parts, persisted);
        let summary = session_store
            .list_sessions(tmp.path(), &reloaded.session.worktree_path)
            .unwrap()
            .into_iter()
            .find(|summary| summary.id == session_id)
            .unwrap();
        assert_eq!(summary.error_reason.as_deref(), Some("CLI process exited"));
    }

    #[tokio::test]
    async fn turn_completed_crash_followed_by_fatal_is_recorded_once() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Crash,
                    error: Some("CLI process exited".to_string()),
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "CLI process exited".to_string(),
                },
            )
            .unwrap();
        wait_for_call_count(&controller, &session_id, TestRuntimeCallKind::Close, 1).await;

        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. }))
                .count(),
            1
        );
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::SessionErrored { .. })));
        let reloaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        let error_contents = reloaded
            .session
            .messages
            .iter()
            .flat_map(|message| message.parts.iter().flatten())
            .filter_map(|part| match part {
                MessagePart::Error { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(error_contents, vec!["CLI process exited"]);
        assert_eq!(
            event_notifier
                .streaming_deltas()
                .iter()
                .filter(|delta| delta.parts.iter().any(|part| {
                    matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn crash_snapshot_supersedes_older_retry_and_lands_after_notifier_recovers() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        event_notifier.set_streaming_delta_failure(true);
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "partial output".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |failures, _| failures >= 1)
            .await;

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Crash,
                    error: Some("CLI process exited".to_string()),
                }),
            )
            .unwrap();
        wait_for_error_state_change(&event_notifier, &session_id).await;
        event_notifier.set_streaming_delta_failure(false);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if event_notifier
                    .delivered_streaming_deltas()
                    .iter()
                    .any(|delta| {
                        delta.snapshot
                            && delta.parts.iter().any(|part| {
                                matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                            })
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let delivered = event_notifier.delivered_streaming_deltas();
        let terminal = delivered
            .iter()
            .find(|delta| {
                delta.parts.iter().any(|part| {
                    matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                })
            })
            .unwrap();
        assert!(terminal.parts.iter().any(|part| {
            matches!(part, MessagePart::Text { content, .. } if content == "partial output")
        }));
    }

    #[tokio::test]
    async fn successful_crash_snapshot_cancels_pre_final_retry_before_delayed_flush() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let partial_parts = vec![MessagePart::Text {
            content: "partial output".to_string(),
            parent_tool_use_id: None,
        }];
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session_id).unwrap();
            state.streaming_parts = partial_parts.clone();
            state.pending_stream_snapshot = true;
        }
        // The pre-final flush fails, then the authoritative crash snapshot succeeds.
        event_notifier.set_streaming_delta_outcomes([false, true]);

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Crash,
                    error: Some("CLI process exited".to_string()),
                }),
            )
            .unwrap();
        wait_for_error_state_change(&event_notifier, &session_id).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let attempted = event_notifier.streaming_deltas();
        assert_eq!(attempted.len(), 2);
        let delivered = event_notifier.delivered_streaming_deltas();
        assert_eq!(delivered.len(), 1);
        let terminal = delivered.last().unwrap();
        assert_eq!(terminal.parts.first(), partial_parts.first());
        assert!(terminal.parts.iter().any(|part| {
            matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
        }));
    }

    #[tokio::test]
    async fn crash_snapshot_retry_survives_queued_turn_reset_and_lands_after_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        event_notifier.set_streaming_delta_failure(true);

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "CLI process exited".to_string(),
                },
            )
            .unwrap();
        wait_for_error_state_change(&event_notifier, &session_id).await;
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .into_iter()
                .filter(|kind| *kind == TestRuntimeCallKind::StartTurn)
                .count(),
            1,
            "Fatal must keep the queued turn paused until explicit resume"
        );
        assert_eq!(usecase.turn_phase(&session_id).await, Some(TurnPhase::Idle));

        usecase.resume_queue(&session_id).await.unwrap();
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );

        event_notifier.set_streaming_delta_failure(false);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if event_notifier
                    .delivered_streaming_deltas()
                    .iter()
                    .any(|delta| {
                        delta.parts.iter().any(|part| {
                            matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                        })
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn idle_fatal_is_durable_live_and_survives_later_projection() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if event_notifier.state_changes().iter().any(|change| {
                    change.chat_session_id == session_id
                        && change.session_state == Some(SessionState::Done)
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let order_start = event_notifier.event_order().len();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "app server stopped".to_string(),
                },
            )
            .unwrap();

        wait_for_last_stream_delta(&event_notifier, |delta| {
            delta.snapshot
                && delta.message.as_ref().is_some_and(|message| {
                    message.parts.as_deref()
                        == Some(
                            [MessagePart::Error {
                                content: "app server stopped".to_string(),
                                parent_tool_use_id: None,
                            }]
                            .as_slice(),
                        )
                })
        })
        .await;
        wait_for_error_state_change(&event_notifier, &session_id).await;
        assert!(event_notifier
            .state_changes()
            .iter()
            .rev()
            .find(|change| change.session_state == Some(SessionState::Error))
            .is_some_and(|change| change.completed_at.is_none()));
        assert_eq!(
            &event_notifier.event_order()[order_start..],
            &["streaming_delta", "state_change"]
        );
        let live = event_notifier
            .streaming_deltas()
            .into_iter()
            .find(|delta| delta.message.is_some())
            .unwrap();
        assert_eq!(
            live.parts,
            vec![MessagePart::Error {
                content: "app server stopped".to_string(),
                parent_tool_use_id: None,
            }]
        );
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::SessionErrored { reason, .. } if reason == "app server stopped"
        )));

        let reloaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(reloaded.session.state, SessionState::Error);
        assert_eq!(
            reloaded.session.error_reason.as_deref(),
            Some("app server stopped")
        );
        let persisted = reloaded
            .session
            .messages
            .iter()
            .find(|message| message.id == live.message_id)
            .and_then(|message| message.parts.clone())
            .unwrap();
        assert_eq!(live.parts, persisted);
        let live_timestamp = live.message.as_ref().unwrap().timestamp;
        let reloaded_timestamp = reloaded
            .session
            .messages
            .iter()
            .find(|message| message.id == live.message_id)
            .unwrap()
            .timestamp;
        assert!((live_timestamp - reloaded_timestamp).abs() < 1e-6);
        let summary = session_store
            .list_sessions(tmp.path(), &reloaded.session.worktree_path)
            .unwrap()
            .into_iter()
            .find(|summary| summary.id == session_id)
            .unwrap();
        assert_eq!(summary.error_reason.as_deref(), Some("app server stopped"));

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "app server stopped again".to_string(),
                },
            )
            .unwrap();
        wait_for_last_stream_delta(&event_notifier, |delta| {
            delta.message.as_ref().is_some_and(|message| {
                message.parts.as_deref()
                    == Some(
                        [MessagePart::Error {
                            content: "app server stopped again".to_string(),
                            parent_tool_use_id: None,
                        }]
                        .as_slice(),
                    )
            })
        })
        .await;
        let second_live = event_notifier.streaming_deltas().last().cloned().unwrap();
        assert_ne!(live.message_id, second_live.message_id);

        let after_second_fatal = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(
            after_second_fatal.session.error_reason.as_deref(),
            Some("app server stopped again")
        );
        let persisted_error_ids = after_second_fatal
            .session
            .messages
            .iter()
            .filter(|message| {
                message.parts.as_ref().is_some_and(|parts| {
                    parts
                        .iter()
                        .any(|part| matches!(part, MessagePart::Error { .. }))
                })
            })
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            persisted_error_ids,
            vec![live.message_id.as_str(), second_live.message_id.as_str()]
        );

        session_store
            .append_session_event_and_project_state(
                tmp.path(),
                &session_id,
                AgentSessionEvent::ToolCallRetried {
                    turn_id: 99,
                    tool_use_id: "unrelated".to_string(),
                    attempt: 1,
                },
            )
            .unwrap();
        let after_reprojection = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(after_reprojection.session.state, SessionState::Error);
        assert_eq!(
            after_reprojection.session.error_reason.as_deref(),
            Some("app server stopped again")
        );
    }

    #[tokio::test]
    async fn distinct_idle_fatal_retries_land_in_message_order_after_notifier_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if event_notifier.state_changes().iter().any(|change| {
                    change.chat_session_id == session_id
                        && change.session_state == Some(SessionState::Done)
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let delivered_before = event_notifier.delivered_streaming_deltas().len();
        event_notifier.set_streaming_delta_failure(true);

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "first fatal".to_string(),
                },
            )
            .unwrap();
        wait_for_last_stream_delta(&event_notifier, |delta| {
            delta.message.as_ref().is_some_and(|message| {
                message.parts.as_deref()
                    == Some(
                        [MessagePart::Error {
                            content: "first fatal".to_string(),
                            parent_tool_use_id: None,
                        }]
                        .as_slice(),
                    )
            })
        })
        .await;

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "second fatal".to_string(),
                },
            )
            .unwrap();
        let persisted_error_ids = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let reloaded = usecase.get_session(&session_id).await.unwrap().unwrap();
                if reloaded.session.error_reason.as_deref() == Some("second fatal") {
                    break reloaded
                        .session
                        .messages
                        .iter()
                        .filter(|message| {
                            message.parts.as_ref().is_some_and(|parts| {
                                parts
                                    .iter()
                                    .any(|part| matches!(part, MessagePart::Error { .. }))
                            })
                        })
                        .map(|message| message.id.clone())
                        .collect::<Vec<_>>();
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(persisted_error_ids.len(), 2);
        assert_eq!(
            event_notifier.delivered_streaming_deltas().len(),
            delivered_before
        );

        event_notifier.set_streaming_delta_failure(false);
        let delivered_ids = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let delivered = event_notifier.delivered_streaming_deltas();
                if delivered.len() >= delivered_before + 2 {
                    break delivered[delivered_before..]
                        .iter()
                        .map(|delta| delta.message_id.clone())
                        .collect::<Vec<_>>();
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(delivered_ids, persisted_error_ids);
    }

    #[derive(Clone, Copy)]
    enum IdleFatalPersistenceFailure {
        AppendEvent,
        AppendMessage,
        ProjectMeta,
    }

    async fn assert_idle_fatal_persistence_failure_retries_exact_episode(
        failure: IdleFatalPersistenceFailure,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if event_notifier.state_changes().iter().any(|change| {
                    change.chat_session_id == session_id
                        && change.session_state == Some(SessionState::Done)
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let failed = Arc::new(AtomicBool::new(false));
        match failure {
            IdleFatalPersistenceFailure::AppendEvent => {
                let failed = Arc::clone(&failed);
                session_store.set_append_event_hook_for_test(Arc::new(move |_, event| {
                    if matches!(event, AgentSessionEvent::SessionErrored { .. })
                        && !failed.swap(true, Ordering::SeqCst)
                    {
                        Err("injected session error event failure".to_string())
                    } else {
                        Ok(())
                    }
                }));
            }
            IdleFatalPersistenceFailure::AppendMessage => {
                let failed = Arc::clone(&failed);
                session_store.set_append_message_hook_for_test(Arc::new(move |_, message| {
                    if message.parts.as_ref().is_some_and(|parts| {
                        parts
                            .iter()
                            .any(|part| matches!(part, MessagePart::Error { .. }))
                    }) && !failed.swap(true, Ordering::SeqCst)
                    {
                        Err("injected session error message failure".to_string())
                    } else {
                        Ok(())
                    }
                }));
            }
            IdleFatalPersistenceFailure::ProjectMeta => {
                let failed = Arc::clone(&failed);
                session_store.set_projection_hook_for_test(Arc::new(move |_, state, _| {
                    if state == &SessionState::Error && !failed.swap(true, Ordering::SeqCst) {
                        Err("injected session error projection failure".to_string())
                    } else {
                        Ok(())
                    }
                }));
            }
        }
        let delta_start = event_notifier.streaming_deltas().len();
        let state_start = event_notifier.state_changes().len();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "app server stopped".to_string(),
                },
            )
            .unwrap();
        wait_for_call(&controller, &session_id, TestRuntimeCallKind::Close).await;

        let reloaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert!(failed.load(Ordering::SeqCst));
        assert_eq!(reloaded.session.state, SessionState::Error);
        assert!(reloaded.session.messages.iter().any(|message| {
            message.parts.as_ref().is_some_and(|parts| {
                parts.iter().any(|part| {
                    matches!(part, MessagePart::Error { content, .. } if content == "app server stopped")
                })
            })
        }));
        assert_eq!(
            session_store
                .load_session_events(tmp.path(), &session_id)
                .unwrap()
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::SessionErrored { .. }))
                .count(),
            1
        );
        assert!(event_notifier.streaming_deltas()[delta_start..]
            .iter()
            .any(|delta| delta.parts.iter().any(|part| {
                matches!(part, MessagePart::Error { content, .. } if content == "app server stopped")
            })));
        assert!(event_notifier.state_changes()[state_start..]
            .iter()
            .any(|change| change.session_state == Some(SessionState::Error)));
    }

    #[tokio::test]
    async fn idle_fatal_append_event_failure_retries_one_error_episode() {
        assert_idle_fatal_persistence_failure_retries_exact_episode(
            IdleFatalPersistenceFailure::AppendEvent,
        )
        .await;
    }

    #[tokio::test]
    async fn idle_fatal_append_message_failure_retries_one_error_episode() {
        assert_idle_fatal_persistence_failure_retries_exact_episode(
            IdleFatalPersistenceFailure::AppendMessage,
        )
        .await;
    }

    #[tokio::test]
    async fn idle_fatal_meta_projection_failure_retries_one_error_episode() {
        assert_idle_fatal_persistence_failure_retries_exact_episode(
            IdleFatalPersistenceFailure::ProjectMeta,
        )
        .await;
    }

    #[derive(Clone, Copy)]
    enum CrashPersistenceFailure {
        AppendEvent,
        PersistParts,
    }

    async fn assert_crash_persistence_failure_retries_exact_terminal(
        failure: CrashPersistenceFailure,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        match failure {
            CrashPersistenceFailure::AppendEvent => {
                let attempts = Arc::clone(&attempts);
                session_store.set_append_event_hook_for_test(Arc::new(move |_, event| {
                    if matches!(event, AgentSessionEvent::FinalPartsRecorded { .. }) {
                        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                        if attempt < PERSIST_MAX_ATTEMPTS {
                            return Err("injected final event failure".to_string());
                        }
                    }
                    Ok(())
                }));
            }
            CrashPersistenceFailure::PersistParts => {
                let attempts = Arc::clone(&attempts);
                session_store.set_persist_parts_hook_for_test(Arc::new(move |_, _, _| {
                    let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                    if attempt < PERSIST_MAX_ATTEMPTS {
                        return Err("injected final parts failure".to_string());
                    }
                    Ok(())
                }));
            }
        }
        let delta_start = event_notifier.streaming_deltas().len();
        let state_start = event_notifier.state_changes().len();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "CLI process exited".to_string(),
                },
            )
            .unwrap();
        wait_for_call(&controller, &session_id, TestRuntimeCallKind::Close).await;

        let reloaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(reloaded.session.state, SessionState::Error);
        assert!(
            attempts.load(Ordering::SeqCst) > PERSIST_MAX_ATTEMPTS,
            "the retained event must be retried after the bounded terminal helper exhausts its attempts"
        );
        assert!(reloaded.session.messages.iter().any(|message| {
            message.parts.as_ref().is_some_and(|parts| {
                parts.iter().any(|part| {
                    matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                })
            })
        }));
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. })));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. }))
                .count(),
            1
        );
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::FinalPartsRecorded { .. })));
        assert!(event_notifier.streaming_deltas()[delta_start..]
            .iter()
            .any(|delta| delta.parts.iter().any(|part| {
                matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
            })));
        assert!(event_notifier.state_changes()[state_start..]
            .iter()
            .any(|change| change.session_state == Some(SessionState::Error)));
    }

    #[tokio::test]
    async fn crash_append_event_failure_retries_one_terminal_without_loss() {
        assert_crash_persistence_failure_retries_exact_terminal(
            CrashPersistenceFailure::AppendEvent,
        )
        .await;
    }

    #[tokio::test]
    async fn crash_persist_parts_failure_retries_one_terminal_without_loss() {
        assert_crash_persistence_failure_retries_exact_terminal(
            CrashPersistenceFailure::PersistParts,
        )
        .await;
    }

    #[tokio::test]
    async fn fatal_closes_runtime_and_pauses_queued_turn_until_explicit_resume() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let first = usecase
            .send_message(send_request(worktree_path.clone()))
            .await
            .unwrap();
        let session_id = first.session.id.clone();
        wait_for_start_prompt_count(&controller, &session_id, 1).await;
        let second = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path,
                content: "queued after fatal".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        assert_eq!(second.pending_queue_count, 1);
        controller.pause_start_turn();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "fatal test".to_string(),
                },
            )
            .unwrap();

        wait_for_call(&controller, &session_id, TestRuntimeCallKind::Close).await;
        assert!(event_notifier.state_changes().iter().any(|change| {
            change.chat_session_id == session_id
                && change.turn_phase == TurnPhase::Idle
                && change.queue_paused == Some(true)
                && change.session_state == Some(SessionState::Error)
        }));
        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::StartTurn))
                .count(),
            1,
            "fatal must not automatically submit the queued turn"
        );

        controller.release_start_turn();
        usecase.resume_queue(&session_id).await.unwrap();
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if usecase.pending_queue(&session_id).await.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
    }

    #[tokio::test]
    async fn test_workflow_turn_complete通知は_session_lock外でdispatchされる() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let response = usecase
            .send_message(send_request(worktree_path.clone()))
            .await
            .unwrap();
        wait_for_start_prompt_count(&controller, &response.session.id, 1).await;

        let done = Arc::new(Notify::new());
        usecase.set_workflow_turn_complete_notifier(Arc::new(ReentrantWorkflowNotifier {
            usecase: Arc::clone(&usecase),
            session_id: response.session.id.clone(),
            worktree_path,
            done: Arc::clone(&done),
        }));
        controller
            .emit(
                &response.session.id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), done.notified())
            .await
            .expect("workflow notification must be able to re-enter same session");
        wait_for_start_prompt_count(&controller, &response.session.id, 2).await;
    }

    #[tokio::test]
    async fn test_init_sessionsは_workflow_node_tabを復元し_active_session_modeを返す() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let regular = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Ask,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: true,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let step = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, None)),
            },
        )
        .unwrap();
        let open_tabs = OpenTabRegistry::default();

        let response = usecase
            .init_sessions(tmp.path().to_string_lossy().as_ref(), &open_tabs)
            .await
            .unwrap();

        assert!(open_tabs.contains(&step.id));
        assert!(!open_tabs.contains(&regular.id));
        assert_eq!(response.permission_mode, PermissionMode::Ask.as_str());
        assert!(response.plan_mode);
        assert_eq!(
            response
                .active_session
                .as_ref()
                .map(|session| session.session.id.as_str()),
            Some(regular.id.as_str())
        );
    }

    #[derive(Default)]
    struct RecordingAcceptedSendObligationDriver {
        reconciliations: Mutex<Vec<(String, String)>>,
        running: Mutex<Vec<(String, String, u64)>>,
        recovery_wake: Mutex<Option<AcceptedSendRecoveryWake>>,
    }

    #[async_trait::async_trait]
    impl AcceptedSendObligationDriver for RecordingAcceptedSendObligationDriver {
        async fn claim_immediate_turn_execution(
            &self,
            _operation_id: &str,
            _obligation_id: &str,
        ) -> Result<AcceptedSendExecutionClaim, ()> {
            Ok(AcceptedSendExecutionClaim::new(|| {}))
        }

        async fn claim_queued_turn_execution(
            &self,
            _operation_id: &str,
            _obligation_id: &str,
            _session_id: &str,
            _queue_item_id: &str,
            _event: AgentSessionEvent,
        ) -> Result<super::super::ports::AcceptedQueuedTurnExecutionClaimOutcome, ()> {
            Ok(
                super::super::ports::AcceptedQueuedTurnExecutionClaimOutcome::Claimed(
                    super::super::ports::AcceptedSendExecutionClaim::new(|| {}),
                ),
            )
        }

        async fn mark_turn_running(
            &self,
            operation_id: &str,
            obligation_id: &str,
            turn_id: u64,
        ) -> Result<(), ()> {
            self.running.lock().unwrap().push((
                operation_id.to_string(),
                obligation_id.to_string(),
                turn_id,
            ));
            Ok(())
        }

        async fn reconcile_turn_execution(
            &self,
            operation_id: &str,
            obligation_id: &str,
        ) -> Option<super::super::ports::AcceptedSendRecoveryWake> {
            self.reconciliations
                .lock()
                .unwrap()
                .push((operation_id.to_string(), obligation_id.to_string()));
            self.recovery_wake.lock().unwrap().take()
        }
    }

    #[tokio::test]
    async fn queued_driver_reconciliation_wakes_after_the_complete_claim_release_chain() {
        let first_release_observed = Arc::new(AtomicBool::new(false));
        let final_release_observed = Arc::new(AtomicBool::new(false));
        let wake_observed = Arc::new(AtomicBool::new(false));
        let claim = AcceptedSendExecutionClaim::new({
            let first_release_observed = Arc::clone(&first_release_observed);
            move || first_release_observed.store(true, Ordering::SeqCst)
        })
        .release_then({
            let first_release_observed = Arc::clone(&first_release_observed);
            let final_release_observed = Arc::clone(&final_release_observed);
            move || {
                assert!(
                    first_release_observed.load(Ordering::SeqCst),
                    "the driver's original claim release must run first"
                );
                final_release_observed.store(true, Ordering::SeqCst);
            }
        });
        let driver = RecordingAcceptedSendObligationDriver {
            recovery_wake: Mutex::new(Some(AcceptedSendRecoveryWake::new({
                let first_release_observed = Arc::clone(&first_release_observed);
                let final_release_observed = Arc::clone(&final_release_observed);
                let wake_observed = Arc::clone(&wake_observed);
                move || {
                    assert!(first_release_observed.load(Ordering::SeqCst));
                    assert!(
                        final_release_observed.load(Ordering::SeqCst),
                        "queued dispatch release must precede the recovery wake"
                    );
                    wake_observed.store(true, Ordering::SeqCst);
                }
            }))),
            ..Default::default()
        };
        let mut accepted_claim = Some(claim);

        arm_accepted_send_recovery_after_claim_release(
            &driver,
            "queued-reconcile",
            "queued-reconcile.exec",
            &mut accepted_claim,
        )
        .await;

        assert!(!first_release_observed.load(Ordering::SeqCst));
        assert!(!final_release_observed.load(Ordering::SeqCst));
        assert!(!wake_observed.load(Ordering::SeqCst));
        drop(accepted_claim.take());
        assert!(first_release_observed.load(Ordering::SeqCst));
        assert!(final_release_observed.load(Ordering::SeqCst));
        assert!(wake_observed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn accepted_immediate_send_keeps_execution_identity_on_current_turn() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let human_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "accepted prompt",
            None,
            None,
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .unwrap();

        usecase
            .execute_accepted_send(AcceptedSendExecution {
                request: AcceptedRuntimeSendInput {
                    content: "accepted prompt".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    images: Vec::new(),
                    mentions: Vec::new(),
                    editor_context: None,
                    base_system_prompt: None,
                    workflow_instructions: Vec::new(),
                },
                operation_id: "send-operation-1",
                execution_obligation_id: "send-operation-1.execute",
                session_id: &session.id,
                human_message_id: &human_message.id,
                assistant_message_id: Some(&agent_message.id),
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                reserved_turn_id: None,
            })
            .await
            .unwrap();

        let (operation_id, execution_obligation_id) = {
            let sessions = usecase.ctx.sessions.lock().await;
            let current_turn = sessions
                .get(&session.id)
                .and_then(|state| state.current_turn_input.as_ref())
                .expect("accepted turn input remains recoverable");
            (
                current_turn.accepted_operation_id.clone(),
                current_turn.execution_obligation_id.clone(),
            )
        };
        assert_eq!(operation_id.as_deref(), Some("send-operation-1"));
        assert_eq!(
            execution_obligation_id.as_deref(),
            Some("send-operation-1.execute")
        );
        assert!(controller.call_kinds_for(&session.id).contains(
            &TestRuntimeCallKind::StartTurnPrompt {
                prompt: "accepted prompt".to_string(),
            }
        ));
    }

    #[tokio::test]
    async fn accepted_turn_backend_recovery_submits_input_before_provider_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let obligation_driver = Arc::new(RecordingAcceptedSendObligationDriver::default());
        usecase.set_accepted_send_obligation_driver(obligation_driver.clone());
        let session =
            provider_establish_test_session(&session_store, tmp.path(), Some("dead-provider"));
        add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "previous context",
            None,
            None,
        )
        .unwrap();
        add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "previous answer",
            None,
            None,
        )
        .unwrap();
        let human_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "recover accepted prompt",
            None,
            None,
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .unwrap();
        controller.fail_next_resume_open();

        usecase
            .execute_accepted_send(AcceptedSendExecution {
                request: AcceptedRuntimeSendInput {
                    content: "recover accepted prompt".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    images: Vec::new(),
                    mentions: Vec::new(),
                    editor_context: None,
                    base_system_prompt: None,
                    workflow_instructions: Vec::new(),
                },
                operation_id: "send-recovery-operation",
                execution_obligation_id: "send-recovery-operation.exec",
                session_id: &session.id,
                human_message_id: &human_message.id,
                assistant_message_id: Some(&agent_message.id),
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                reserved_turn_id: None,
            })
            .await
            .unwrap();

        let calls_before_identity = controller.call_kinds_for(&session.id);
        assert_eq!(
            calls_before_identity
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2,
            "the dead resume is replaced exactly once"
        );
        let replacement_prompts = calls_before_identity
            .iter()
            .filter_map(|call| match call {
                TestRuntimeCallKind::StartTurnPrompt { prompt } => Some(prompt),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(replacement_prompts.len(), 1);
        assert!(replacement_prompts[0].contains("previous context"));
        assert!(replacement_prompts[0].contains("previous answer"));
        assert!(replacement_prompts[0].ends_with("recover accepted prompt"));
        assert!(!usecase.provider_session_is_confirmed(&session.id).await);
        assert!(
            usecase
                .owns_accepted_turn_execution(
                    &session.id,
                    "send-recovery-operation",
                    "send-recovery-operation.exec",
                )
                .await
        );
        assert!(usecase.pending_queue(&session.id).await.is_empty());
        tokio::time::timeout(Duration::from_secs(1), async {
            while obligation_driver.running.lock().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            obligation_driver.running.lock().unwrap().as_slice(),
            &[(
                "send-recovery-operation".to_string(),
                "send-recovery-operation.exec".to_string(),
                1,
            )]
        );

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "replacement-provider".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::StartTurn))
                .count(),
            1,
            "provider identity completion must not enqueue or submit the turn again"
        );
        let recovered_meta = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            recovered_meta.agent_session_id.as_deref(),
            Some("replacement-provider")
        );
        assert_eq!(
            recovered_meta.context_carry,
            Some(ContextCarryState::Reinjected)
        );
        assert_eq!(recovered_meta.context_reinjection_generation, None);
    }

    #[tokio::test]
    async fn accepted_turn_backend_lost_event_restarts_without_lock_reentry_or_second_claim() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let obligation_driver = Arc::new(RecordingAcceptedSendObligationDriver::default());
        usecase.set_accepted_send_obligation_driver(obligation_driver.clone());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let human_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "continue exact accepted turn",
            None,
            None,
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .unwrap();

        usecase
            .execute_accepted_send(AcceptedSendExecution {
                request: AcceptedRuntimeSendInput {
                    content: "continue exact accepted turn".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    images: Vec::new(),
                    mentions: Vec::new(),
                    editor_context: None,
                    base_system_prompt: None,
                    workflow_instructions: Vec::new(),
                },
                operation_id: "send-backend-lost-operation",
                execution_obligation_id: "send-backend-lost-operation.exec",
                session_id: &session.id,
                human_message_id: &human_message.id,
                assistant_message_id: Some(&agent_message.id),
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                reserved_turn_id: None,
            })
            .await
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 1).await;

        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        wait_for_start_prompt_count(&controller, &session.id, 2).await;

        assert!(
            usecase
                .owns_accepted_turn_execution(
                    &session.id,
                    "send-backend-lost-operation",
                    "send-backend-lost-operation.exec",
                )
                .await
        );
        assert!(usecase.pending_queue(&session.id).await.is_empty());
        assert_eq!(
            obligation_driver.running.lock().unwrap().as_slice(),
            &[(
                "send-backend-lost-operation".to_string(),
                "send-backend-lost-operation.exec".to_string(),
                1,
            )]
        );
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(
                    call,
                    TestRuntimeCallKind::StartTurnPrompt { prompt }
                        if prompt == "continue exact accepted turn"
                ))
                .count(),
            2,
            "one original submission and one replacement-runtime continuation are expected"
        );

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "replacement-after-loss".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::StartTurn))
                .count(),
            2,
            "identity completion must not submit a third input"
        );
        assert_eq!(
            session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .context_reinjection_generation,
            None
        );
    }

    #[tokio::test]
    async fn accepted_stop_fences_late_backend_loss_without_reopening_the_turn() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let human_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "stop this accepted turn",
            None,
            None,
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .unwrap();

        usecase
            .execute_accepted_send(AcceptedSendExecution {
                request: AcceptedRuntimeSendInput {
                    content: "stop this accepted turn".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    images: Vec::new(),
                    mentions: Vec::new(),
                    editor_context: None,
                    base_system_prompt: None,
                    workflow_instructions: Vec::new(),
                },
                operation_id: "send-stop-race-operation",
                execution_obligation_id: "send-stop-race-operation.exec",
                session_id: &session.id,
                human_message_id: &human_message.id,
                assistant_message_id: Some(&agent_message.id),
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                reserved_turn_id: None,
            })
            .await
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 1).await;

        let runtime_epoch = {
            let sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get(&session.id).expect("active runtime state");
            assert!(!state.queue_paused);
            assert_eq!(state.interrupt_requested_generation, None);
            state.runtime_epoch
        };
        let accepted_at = crate::usecase::agent_session::session::now_timestamp();
        append_session_events_blocking(
            &usecase.ctx,
            &session.id,
            vec![
                AgentSessionEvent::TurnInterruptRequested {
                    turn_id: 1,
                    at: accepted_at,
                },
                AgentSessionEvent::QueuePaused { at: accepted_at },
            ],
        )
        .await
        .unwrap();

        // The durable acceptance closes the interval before the production
        // gate can install its process-local fence.
        apply_runtime_event(
            &usecase.ctx,
            &session.id,
            runtime_epoch,
            crate::usecase::agent_session::session::now_timestamp(),
            AgentRuntimeEvent::BackendSessionCleared,
        )
        .await
        .unwrap();

        usecase
            .interrupt_provider_effect_after_stop_acceptance(&session.id, 1)
            .await
            .unwrap();
        {
            let sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get(&session.id).expect("active runtime state");
            assert!(state.queue_paused);
            assert_eq!(state.queue_paused_at, Some(accepted_at));
            assert_eq!(
                state.interrupt_requested_generation,
                Some(state.generation),
                "durable Stop must fence the exact active process generation"
            );
        }
        apply_runtime_event(
            &usecase.ctx,
            &session.id,
            runtime_epoch,
            crate::usecase::agent_session::session::now_timestamp(),
            AgentRuntimeEvent::BackendSessionCleared,
        )
        .await
        .unwrap();

        let calls = controller.call_kinds_for(&session.id);
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            1,
            "a provider-loss event after Stop acceptance must not open a replacement runtime"
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::StartTurn))
                .count(),
            1,
            "a provider-loss event after Stop acceptance must not resubmit the input"
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::Interrupt))
                .count(),
            1
        );
        assert!(!session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryStarted { .. }
            )));
    }

    #[tokio::test]
    async fn accepted_turn_event_recovery_open_failure_reconciles_the_exact_execution() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let driver = Arc::new(RecordingAcceptedSendObligationDriver::default());
        usecase.set_accepted_send_obligation_driver(driver.clone());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let human_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "accepted turn whose replacement fails",
            None,
            None,
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .unwrap();

        usecase
            .execute_accepted_send(AcceptedSendExecution {
                request: AcceptedRuntimeSendInput {
                    content: "accepted turn whose replacement fails".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    images: Vec::new(),
                    mentions: Vec::new(),
                    editor_context: None,
                    base_system_prompt: None,
                    workflow_instructions: Vec::new(),
                },
                operation_id: "send-replacement-open-failure",
                execution_obligation_id: "send-replacement-open-failure.exec",
                session_id: &session.id,
                human_message_id: &human_message.id,
                assistant_message_id: Some(&agent_message.id),
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                reserved_turn_id: None,
            })
            .await
            .unwrap();
        controller.fail_next_open();
        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let reconciled = !driver.reconciliations.lock().unwrap().is_empty();
                if reconciled && usecase.turn_phase(&session.id).await == Some(TurnPhase::Idle) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            *driver.reconciliations.lock().unwrap(),
            vec![(
                "send-replacement-open-failure".to_string(),
                "send-replacement-open-failure.exec".to_string(),
            )]
        );
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::StartTurn))
                .count(),
            1,
            "a failed replacement open must not submit the input again"
        );
    }

    #[tokio::test]
    async fn legacy_send_leaves_current_turn_without_accepted_execution_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            build_agent_runtime_usecase_with_controller(session_store, tmp.path());

        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();

        let sessions = usecase.ctx.sessions.lock().await;
        let current_turn = sessions
            .get(&response.session.id)
            .and_then(|state| state.current_turn_input.as_ref())
            .expect("legacy turn input remains available");
        assert!(current_turn.accepted_operation_id.is_none());
        assert!(current_turn.execution_obligation_id.is_none());
    }

    #[tokio::test]
    async fn provider_establishment_observation_retries_metadata_commit_without_reopening() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let commit_attempts = Arc::new(AtomicUsize::new(0));
        session_store.set_backend_established_hook_for_test(Arc::new({
            let commit_attempts = Arc::clone(&commit_attempts);
            move |_, _| {
                if commit_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err("injected provider metadata failure".to_string());
                }
                Ok(())
            }
        }));

        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "provider-session-1".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let meta = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.agent_session_id.as_deref(), Some("provider-session-1"));
        assert_eq!(meta.provider_session_generation, 1);
        assert_eq!(commit_attempts.load(Ordering::SeqCst), 2);
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn provider_establishment_commit_reply_loss_replays_exact_observation_once() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);

        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;
        local_store
            .fault_injector()
            .arm_crash_after_commit_before_readback();
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "provider-after-reply-loss".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let meta = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            meta.agent_session_id.as_deref(),
            Some("provider-after-reply-loss")
        );
        assert_eq!(meta.provider_session_generation, 1);
        assert!(
            meta.provider_session_observation_id.is_some(),
            "the durable generation must retain the exact replay identity"
        );
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            1,
            "metadata reply loss must not reopen the provider"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn provider_establishment_persistence_does_not_hold_runtime_event_locks() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let persistence_started = Arc::new(AtomicBool::new(false));
        let release_persistence = Arc::new(AtomicBool::new(false));
        session_store.set_backend_established_hook_for_test(Arc::new({
            let persistence_started = Arc::clone(&persistence_started);
            let release_persistence = Arc::clone(&release_persistence);
            move |_, _| {
                persistence_started.store(true, Ordering::SeqCst);
                while !release_persistence.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(())
            }
        }));

        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "provider-blocked-persistence".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            while !persistence_started.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let lock_result = tokio::time::timeout(Duration::from_millis(250), async {
            let _session_guard = usecase.ctx.session_locks.acquire(&session.id).await;
            let _runtime_event_guard = usecase.ctx.runtime_event_locks.acquire(&session.id).await;
        })
        .await;
        release_persistence.store(true, Ordering::SeqCst);
        assert!(
            lock_result.is_ok(),
            "provider metadata I/O must not retain either runtime-event serialization lock"
        );

        tokio::time::timeout(Duration::from_secs(1), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn provider_establishment_lifecycle_fence_clears_exact_pending_observation() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let commit_attempts = Arc::new(AtomicUsize::new(0));
        session_store.set_backend_established_hook_for_test(Arc::new({
            let commit_attempts = Arc::clone(&commit_attempts);
            move |_, _| {
                commit_attempts.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }));

        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;
        session_store
            .set_session_state(tmp.path(), &session.id, SessionState::Error)
            .unwrap();
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "late-provider-after-terminal".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let pending = usecase
                    .ctx
                    .sessions
                    .lock()
                    .await
                    .get(&session.id)
                    .is_some_and(|state| state.provider_session_establishment.is_some());
                if commit_attempts.load(Ordering::SeqCst) >= 1 && !pending {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(
            commit_attempts.load(Ordering::SeqCst),
            1,
            "a deterministic lifecycle fence must not enter the transient retry loop"
        );
        assert!(!usecase.provider_session_is_confirmed(&session.id).await);
        let meta = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.state, SessionState::Error);
        assert_eq!(meta.provider_session_generation, 0);
        assert!(meta.agent_session_id.is_none());
        assert!(meta.provider_session_observation_id.is_none());
    }

    #[tokio::test]
    async fn config_modes_persist_without_provider_effect() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        let provider_calls_before = controller.call_kinds_for(&session.id);

        usecase
            .set_permission_mode(&session.id, PermissionMode::Full)
            .await
            .unwrap();
        usecase.set_plan_mode(&session.id, true).await.unwrap();

        let saved = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.permission_mode, "full");
        assert!(saved.plan_mode);
        assert!(event_notifier
            .permission_modes()
            .contains(&(session.id.clone(), "full".to_string())));
        assert_eq!(
            controller.call_kinds_for(&session.id),
            provider_calls_before
        );
    }

    #[tokio::test]
    async fn cross_backend_set_model_changes_an_unstarted_empty_session_without_lifecycle_pause() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase.set_model(&session.id, "codex:gpt-5").await.unwrap();
        let saved = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.backend_id, "codex");
        assert_eq!(saved.selected_model.as_deref(), Some("gpt-5"));
        let model_updates = event_notifier.model_updates();
        assert_eq!(model_updates.len(), 1);
        assert_eq!(model_updates[0].0, session.id);
        assert_eq!(model_updates[0].2, "gpt-5");
        assert!(model_updates[0]
            .1
            .iter()
            .any(|model| model.id == "codex:gpt-5"));
        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session.id)
            .unwrap()
            .is_none());
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .into_iter()
                .filter(|kind| kind == &TestRuntimeCallKind::Close)
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn same_backend_model_is_persisted_now_and_applied_only_inside_turn_execution() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            Arc::new(RecordingAgentNotifier::default()),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();

        usecase
            .set_model(&session.id, "claude:claude-opus-4-8")
            .await
            .unwrap();

        assert_eq!(
            session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .selected_model
                .as_deref(),
            Some("claude-opus-4-8")
        );
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::SetModel));

        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "apply selected model".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: Some("claude-opus-4-8".to_string()),
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        let calls = controller.call_kinds_for(&session.id);
        let model_index = calls
            .iter()
            .position(|call| call == &TestRuntimeCallKind::SetModel)
            .expect("turn execution applies the persisted model");
        let start_index = calls
            .iter()
            .position(|call| matches!(call, TestRuntimeCallKind::StartTurn))
            .expect("turn starts");
        assert!(model_index < start_index);
    }

    #[tokio::test]
    async fn cross_backend_set_session_backend_changes_an_unstarted_session() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            Arc::new(RecordingAgentNotifier::default()),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase
            .set_session_backend(&session.id, "codex")
            .await
            .unwrap();

        let saved = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.backend_id, "codex");
        assert_eq!(saved.selected_model.as_deref(), Some("gpt-5"));
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .into_iter()
                .filter(|kind| kind == &TestRuntimeCallKind::Close)
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn cross_backend_set_model_rejects_each_locked_session_state() {
        #[derive(Clone, Copy)]
        enum LockedState {
            Messages,
            AgentSessionId,
            ActiveTurn,
        }

        for locked_state in [
            LockedState::Messages,
            LockedState::AgentSessionId,
            LockedState::ActiveTurn,
        ] {
            {
                let tmp = tempfile::tempdir().unwrap();
                let session_store = Arc::new(build_session_store());
                let (usecase, controller) =
                    build_agent_runtime_usecase_with_controller_and_notifiers(
                        session_store.clone(),
                        tmp.path(),
                        Arc::new(RecordingAgentNotifier::default()),
                        Arc::new(RecordingStatusNotifier::default()),
                    );
                let mut session = create_session_internal_with_attributes(
                    &session_store,
                    tmp.path(),
                    tmp.path().to_string_lossy().as_ref(),
                    Some("claude".to_string()),
                    PermissionMode::Edit,
                    SessionCreationAttributes {
                        selected_model: Some("claude-4-sonnet".to_string()),
                        plan_mode: false,
                        workflow_node_session: false,
                        workflow_node_context: None,
                    },
                )
                .unwrap();
                match locked_state {
                    LockedState::Messages => {
                        session.messages.push(ChatMessage {
                            id: "message-1".to_string(),
                            role: MessageRole::Human,
                            content: "hello".to_string(),
                            thinking: None,
                            activities: None,
                            parts: Some(vec![MessagePart::Text {
                                content: "hello".to_string(),
                                parent_tool_use_id: None,
                            }]),
                            streaming_final_seq: 0,
                            timestamp: 1.0,
                            mentions: None,
                        });
                        session_store
                            .save_full_session_for_restore(tmp.path(), &session)
                            .unwrap();
                    }
                    LockedState::AgentSessionId => {
                        session_store
                            .update_agent_session_id(
                                tmp.path(),
                                &session.id,
                                Some("agent-session".to_string()),
                            )
                            .unwrap();
                    }
                    LockedState::ActiveTurn => {
                        usecase
                            .insert_runtime_state_for_test(&session.id, TurnPhase::Streaming, false)
                            .await;
                    }
                }

                let result = usecase
                    .set_model(&session.id, "codex:gpt-5")
                    .await
                    .map(|_| ());

                assert_eq!(result, Err(AgentRuntimeError::BackendSelectionLocked));
                let saved = session_store
                    .get_session_meta(tmp.path(), &session.id)
                    .unwrap()
                    .unwrap();
                assert_eq!(saved.backend_id, "claude");
                assert_eq!(saved.selected_model.as_deref(), Some("claude-4-sonnet"));
                assert!(!controller
                    .call_kinds_for(&session.id)
                    .contains(&TestRuntimeCallKind::Close));
            }
        }
    }

    #[tokio::test]
    async fn backend_selection_persistence_failure_preserves_runtime_and_previous_selection() {
        for use_set_model in [true, false] {
            let tmp = tempfile::tempdir().unwrap();
            let session_store = Arc::new(build_session_store());
            let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
                session_store.clone(),
                tmp.path(),
                Arc::new(RecordingAgentNotifier::default()),
                Arc::new(RecordingStatusNotifier::default()),
            );
            let session = create_session_internal_with_attributes(
                &session_store,
                tmp.path(),
                tmp.path().to_string_lossy().as_ref(),
                Some("claude".to_string()),
                PermissionMode::Edit,
                SessionCreationAttributes {
                    selected_model: Some("claude-4-sonnet".to_string()),
                    plan_mode: false,
                    workflow_node_session: false,
                    workflow_node_context: None,
                },
            )
            .unwrap();
            usecase
                .start_session(
                    &session.id,
                    StartSessionOptions {
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                    },
                )
                .await
                .unwrap();
            std::fs::remove_file(
                tmp.path()
                    .join("sessions")
                    .join(&session.id)
                    .join("meta.json"),
            )
            .unwrap();

            let result = if use_set_model {
                usecase
                    .set_model(&session.id, "codex:gpt-5")
                    .await
                    .map(|_| ())
            } else {
                usecase
                    .set_session_backend(&session.id, "codex")
                    .await
                    .map(|_| ())
            };

            assert!(result.is_err());
            assert!(usecase.has_live_runtime(&session.id).await);
            let saved = session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap();
            assert_eq!(saved.backend_id, "claude");
            assert_eq!(saved.selected_model.as_deref(), Some("claude-4-sonnet"));
            assert_eq!(
                controller
                    .call_kinds_for(&session.id)
                    .into_iter()
                    .filter(|kind| kind == &TestRuntimeCallKind::Close)
                    .count(),
                0
            );
        }
    }

    #[tokio::test]
    async fn test_find_permission_request_五一件超の過去ページから解決できる() {
        // Given: a stored session whose permission request is older than the latest 50 messages.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            Some(vec![MessagePart::Permission {
                request: crate::usecase::agent_session::runtime::event_apply::pending_permission_request_from_msg(&PermissionRequestMsg {
                    id: "perm-old".to_string(),
                    tool_use_id: Some("toolu-old".to_string()),
                    tool_name: "Bash".to_string(),
                    kind: PermissionRequestKindMsg::ToolApproval,
                    input: Some(serde_json::json!({"command": "echo old"})),
                    plan: None,
                    allowed_prompts: Vec::new(),
                    questions: Vec::new(),
                    title: Some("Run command".to_string()),
                    display_name: None,
                    description: None,
                    decision_reason: None,
                })
                .unwrap(),
                status: PermissionPartStatus::Allowed,
                answers: None,
                parent_tool_use_id: None,
            }]),
            None,
        )
        .unwrap();
        for index in 0..55 {
            add_message_internal(
                &session_store,
                tmp.path(),
                &session.id,
                MessageRole::Agent,
                &format!("filler {index}"),
                None,
                None,
            )
            .unwrap();
        }

        // When: the permission presentation lookup runs from the latest page.
        let request = usecase
            .find_permission_request(&session.id, "perm-old")
            .await
            .unwrap()
            .expect("permission request");

        // Then: cursor pagination walks back to the older page and returns the stored request.
        assert_eq!(request.id, "perm-old");
        assert_eq!(request.tool_name, "Bash");
        assert_eq!(request.title.as_deref(), Some("Run command"));
    }

    #[tokio::test]
    async fn find_permission_request_returns_in_memory_pending_without_message_part() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase
            .insert_runtime_state_for_test(&session.id, TurnPhase::WaitingPermission, false)
            .await;
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session.id).expect("runtime state");
            state.pending_permission_request = Some(permission_request_msg("perm-pending-only"));
            state.streaming_parts.clear();
            state.domain_streaming_parts.clear();
        }

        let request = usecase
            .find_permission_request(&session.id, "perm-pending-only")
            .await
            .unwrap()
            .expect("permission request");

        assert_eq!(request.id, "perm-pending-only");
        assert_eq!(request.tool_name, "Bash");
    }

    #[tokio::test]
    async fn test_permission待機中に終端した後のdeny応答は_busyへ戻さない() {
        // Given: a running turn that has entered WaitingPermission.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(
                    crate::domain::agent_session::entities::PermissionRequest {
                        id: "perm-1".to_string(),
                        tool_use_id: Some("toolu-1".to_string()),
                        parent_tool_use_id: None,
                        tool_name: "Bash".to_string(),
                        body: crate::domain::agent_session::entities::PermissionRequestBody::ToolApproval {
                            input: crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                r#"{"command":"echo hi"}"#.to_string(),
                            ),
                        },
                        title: None,
                        display_name: None,
                        description: None,
                        decision_reason: None,
                        status: PermissionRequestStatus::Pending,
                    },
                ),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;

        // When: the backend completes the turn before the user denial arrives.
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        let err = usecase
            .respond_permission(
                &session_id,
                PermissionResponse {
                    request_id: "perm-1".to_string(),
                    decision: PermissionResponseDecision::Deny {
                        message: Some("no".to_string()),
                    },
                },
            )
            .await
            .unwrap_err();

        // Then: the late response is rejected and does not move the session back to Streaming.
        assert!(err.to_string().contains("No pending permission request"));
        assert_eq!(usecase.turn_phase(&session_id).await, Some(TurnPhase::Idle));
    }

    #[tokio::test]
    async fn respond_permission_runtime_failure_is_not_blindly_replayed_after_effect_claim() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier,
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-1")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;
        let before_events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        controller.fail_next_respond_permission();

        let err = usecase
            .respond_permission(
                &session_id,
                PermissionResponse {
                    request_id: "perm-1".to_string(),
                    decision: PermissionResponseDecision::Allow {
                        updated_input: None,
                        answers: None,
                    },
                },
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("permission response failure"));
        assert_eq!(
            session_store
                .load_session_events(tmp.path(), &session_id)
                .unwrap(),
            before_events
        );
        assert!(usecase
            .streaming_parts(&session_id)
            .await
            .iter()
            .any(|part| matches!(
                part,
                MessagePart::Permission {
                    status: PermissionPartStatus::Pending,
                    ..
                }
            )));
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::WaitingPermission)
        );
        let turn_id = session_store
            .get_session_meta(tmp.path(), &session_id)
            .unwrap()
            .and_then(|meta| meta.last_turn_id)
            .expect("active turn id");
        let obligation_id = format!("permission-response:{session_id}:{turn_id}:perm-1");
        let obligation = session_store
            .load_permission_response_obligation(&obligation_id)
            .unwrap()
            .expect("effect reservation");
        assert_eq!(
            obligation,
            crate::domain::local_event::ObligationStateRecord::EffectReserved
        );
        let provider_calls_before = controller
            .call_kinds_for(&session_id)
            .into_iter()
            .filter(|kind| matches!(kind, TestRuntimeCallKind::RespondPermission { .. }))
            .count();

        let retry_error = usecase
            .respond_permission(
                &session_id,
                PermissionResponse {
                    request_id: "perm-1".to_string(),
                    decision: PermissionResponseDecision::Allow {
                        updated_input: None,
                        answers: None,
                    },
                },
            )
            .await
            .unwrap_err();
        assert!(retry_error.to_string().contains("requires reconciliation"));
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .into_iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::RespondPermission { .. }))
                .count(),
            provider_calls_before
        );
    }

    #[tokio::test]
    async fn respond_permission_success_patches_before_persist_event_and_delta() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-1")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;
        mark_stall_observation_active_for_test(&usecase, &session_id).await;
        let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        session_store.set_persist_parts_hook_for_test({
            let order = Arc::clone(&order);
            Arc::new(move |_, _, parts| {
                assert!(parts.iter().any(|part| matches!(
                    part,
                    MessagePart::Permission {
                        status: PermissionPartStatus::Allowed,
                        ..
                    }
                )));
                order.lock().unwrap().push("persist");
                Ok(())
            })
        });
        session_store.set_append_event_hook_for_test({
            let order = Arc::clone(&order);
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::PermissionResolved { .. }) {
                    order.lock().unwrap().push("event");
                }
                Ok(())
            })
        });
        event_notifier.set_streaming_delta_hook({
            let order = Arc::clone(&order);
            Arc::new(move || {
                order.lock().unwrap().push("delta");
            })
        });

        usecase
            .respond_permission(
                &session_id,
                PermissionResponse {
                    request_id: "perm-1".to_string(),
                    decision: PermissionResponseDecision::Allow {
                        updated_input: None,
                        answers: None,
                    },
                },
            )
            .await
            .unwrap();

        wait_for_workflow_stall_cleared_count(&workflow_stall_notifier, 1).await;
        wait_for_stall_clear_count(&event_notifier, 1).await;
        assert_eq!(&*order.lock().unwrap(), &["persist", "event", "delta"]);
        assert_eq!(event_notifier.stall_clears().last(), Some(&session_id));
        assert_eq!(
            workflow_stall_notifier
                .cleared_notifications()
                .last()
                .map(|notification| notification.chat_session_id.as_str()),
            Some(session_id.as_str())
        );
        assert!(event_notifier
            .streaming_deltas()
            .iter()
            .any(|delta| delta.chat_session_id == session_id && delta.snapshot));
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
    }

    #[tokio::test]
    async fn test_waiting_permissionでtimeout超過してもwatchdogは許可後も非終端signalに留める() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::PermissionRequested(
                    crate::domain::agent_session::entities::PermissionRequest {
                        id: "perm-1".to_string(),
                        tool_use_id: Some("toolu-1".to_string()),
                        parent_tool_use_id: None,
                        tool_name: "Bash".to_string(),
                        body: crate::domain::agent_session::entities::PermissionRequestBody::ToolApproval {
                            input: crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                r#"{"command":"echo hi"}"#.to_string(),
                            ),
                        },
                        title: None,
                        display_name: None,
                        description: None,
                        decision_reason: None,
                        status: PermissionRequestStatus::Pending,
                    },
                ),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::WaitingPermission).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(event_notifier.stall_observations().is_empty());

        usecase
            .respond_permission(
                &session.id,
                PermissionResponse {
                    request_id: "perm-1".to_string(),
                    decision: PermissionResponseDecision::Allow {
                        updated_input: None,
                        answers: None,
                    },
                },
            )
            .await
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Streaming).await;
        wait_for_stall_observation_count(&event_notifier, 1).await;
        wait_for_call_count(&controller, &session.id, TestRuntimeCallKind::Reconnect, 1).await;

        let calls = controller.call_kinds_for(&session.id);
        assert!(!calls.contains(&TestRuntimeCallKind::Interrupt));
        assert!(!calls.contains(&TestRuntimeCallKind::Close));
        assert_eq!(
            event_notifier
                .stall_observations()
                .first()
                .map(|payload| payload.turn_phase),
            Some(TurnPhase::Streaming)
        );
    }

    #[tokio::test]
    async fn test_keep_aliveは_last_progress_atを更新する() {
        // Given: a streaming turn whose progress clock has gone stale
        // (e.g. a long-running tool keeps the CLI silent except keep_alive lines).
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store, tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Streaming).await;
        let stale_instant = std::time::Instant::now() - Duration::from_secs(3_600);
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session_id).unwrap();
            state.last_progress_at = Some(stale_instant);
            state.stall_signal_count = 1;
            state.stall_observation_active = true;
        }

        // When: the backend emits a keep_alive liveness event.
        controller
            .emit(&session_id, AgentRuntimeEvent::KeepAlive)
            .unwrap();

        // Then: the progress clock is refreshed and the active stall observation is cleared.
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                {
                    let sessions = usecase.ctx.sessions.lock().await;
                    let state = sessions.get(&session_id).unwrap();
                    let last_progress_at = state.last_progress_at.unwrap();
                    if last_progress_at > stale_instant && !state.stall_observation_active {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("keep_alive should refresh last_progress_at");
    }

    #[tokio::test]
    async fn test_workflow_stall_clear失敗時はactive_flagを残し次progressでretryする() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Streaming).await;
        mark_stall_observation_active_for_test(&usecase, &session_id).await;
        let stale_instant = std::time::Instant::now() - Duration::from_secs(3_600);
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session_id).unwrap();
            state.last_progress_at = Some(stale_instant);
        }
        workflow_stall_notifier.fail_next_stall_cleared();

        controller
            .emit(&session_id, AgentRuntimeEvent::KeepAlive)
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                {
                    let sessions = usecase.ctx.sessions.lock().await;
                    let state = sessions.get(&session_id).unwrap();
                    if state.last_progress_at.is_some_and(|at| at > stale_instant) {
                        assert!(state.stall_observation_active);
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("failed clear should still record progress without clearing active flag");
        assert!(workflow_stall_notifier.cleared_notifications().is_empty());
        assert!(event_notifier.stall_clears().is_empty());

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "resumed".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_workflow_stall_cleared_count(&workflow_stall_notifier, 1).await;
        wait_for_stall_clear_count(&event_notifier, 1).await;

        let sessions = usecase.ctx.sessions.lock().await;
        let state = sessions.get(&session_id).unwrap();
        assert!(!state.stall_observation_active);
        assert_eq!(event_notifier.stall_clears().last(), Some(&session_id));
        assert_eq!(
            workflow_stall_notifier
                .cleared_notifications()
                .last()
                .map(|notification| notification.chat_session_id.as_str()),
            Some(session_id.as_str())
        );
    }

    #[tokio::test]
    async fn b020_streaming_part_persistence_failure_hides_uncommitted_delta_everywhere() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let agent_message_id = response.agent_message.unwrap().id;
        let failed = Arc::new(AtomicBool::new(false));
        let allow_persist = Arc::new(AtomicBool::new(false));
        session_store.set_persist_parts_hook_for_test({
            let failed = Arc::clone(&failed);
            let allow_persist = Arc::clone(&allow_persist);
            Arc::new(move |_, _, parts| {
                if parts.iter().any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content == "unsaved")
                }) && !allow_persist.load(Ordering::SeqCst)
                {
                    failed.store(true, Ordering::SeqCst);
                    return Err("injected streaming snapshot failure".to_string());
                }
                Ok(())
            })
        });
        let delta_start = event_notifier.streaming_deltas().len();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "unsaved".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !failed.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the injected commit failure must be observed");

        assert!(
            !usecase
                .streaming_parts(&session_id)
                .await
                .iter()
                .any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content.contains("unsaved"))
                }),
            "an uncommitted part must not enter the live runtime projection"
        );
        let public_during_failure = usecase
            .get_session(&session_id)
            .await
            .unwrap()
            .expect("the session remains readable while its exact event is retained for retry");
        assert!(
            !public_during_failure
                .session
                .messages
                .iter()
                .any(|message| {
                    message.parts.as_deref().unwrap_or_default().iter().any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content.contains("unsaved"))
                })
                }),
            "the public read model must not expose an uncommitted part"
        );
        let reloaded_during_failure = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        assert!(
            !reloaded_during_failure.messages.iter().any(|message| {
                message.parts.as_deref().unwrap_or_default().iter().any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content.contains("unsaved"))
                })
            }),
            "a fresh durable reload must not expose an uncommitted part"
        );
        assert!(
            !event_notifier.streaming_deltas()[delta_start..]
                .iter()
                .flat_map(|delta| delta.parts.iter())
                .any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content.contains("unsaved"))
                }),
            "publication must wait for the durable commit"
        );

        allow_persist.store(true, Ordering::SeqCst);
        wait_for_streaming_text(&usecase, &session_id, "unsaved").await;
        assert!(failed.load(Ordering::SeqCst));
        assert!(event_notifier.streaming_deltas()[delta_start..]
            .iter()
            .flat_map(|delta| delta.parts.iter())
            .any(|part| {
                matches!(part, MessagePart::Text { content, .. } if content == "unsaved")
            }));
        let reloaded = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        assert!(reloaded
            .messages
            .iter()
            .find(|message| message.id == agent_message_id)
            .and_then(|message| message.parts.as_deref())
            .unwrap_or_default()
            .iter()
            .any(|part| {
                matches!(part, MessagePart::Text { content, .. } if content == "unsaved")
            }));

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "saved".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_streaming_text(&usecase, &session_id, "saved").await;
        let reloaded = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        let persisted_parts = reloaded
            .messages
            .iter()
            .find(|message| message.id == agent_message_id)
            .and_then(|message| message.parts.as_deref())
            .unwrap_or_default();
        assert!(persisted_parts.iter().any(|part| {
            matches!(part, MessagePart::Text { content, .. } if content.contains("saved"))
        }));
        assert!(persisted_parts.iter().any(|part| {
            matches!(part, MessagePart::Text { content, .. } if content.contains("unsaved"))
        }));
    }

    #[derive(Clone, Copy)]
    enum PermissionPartPersistenceFailure {
        Event,
        MessageProjection,
    }

    async fn assert_permission_part_persistence_failure_retries_before_publication(
        failure: PermissionPartPersistenceFailure,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let agent_message_id = response.agent_message.unwrap().id;
        let failed = Arc::new(AtomicBool::new(false));
        match failure {
            PermissionPartPersistenceFailure::Event => {
                session_store.set_append_event_hook_for_test({
                    let failed = Arc::clone(&failed);
                    Arc::new(move |_, event| {
                        if matches!(event, AgentSessionEvent::PermissionRequested { .. })
                            && !failed.swap(true, Ordering::SeqCst)
                        {
                            return Err("injected permission event failure".to_string());
                        }
                        Ok(())
                    })
                });
            }
            PermissionPartPersistenceFailure::MessageProjection => {
                session_store.set_persist_parts_hook_for_test({
                    let failed = Arc::clone(&failed);
                    Arc::new(move |_, _, parts| {
                        if parts.iter().any(|part| {
                            matches!(
                                part,
                                MessagePart::Permission {
                                    status: PermissionPartStatus::Pending,
                                    ..
                                }
                            )
                        }) && !failed.swap(true, Ordering::SeqCst)
                        {
                            return Err("injected permission projection failure".to_string());
                        }
                        Ok(())
                    })
                });
            }
        }
        let delta_start = event_notifier.streaming_deltas().len();
        let state_start = event_notifier.state_changes().len();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-unsaved")),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let live = usecase.get_session(&session_id).await.unwrap().unwrap();
                if failed.load(Ordering::SeqCst)
                    && live.turn_phase == TurnPhase::WaitingPermission
                    && live
                        .pending_permission_request
                        .as_ref()
                        .map(|request| request.id.as_str())
                        == Some("perm-unsaved")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the exact permission request should be retried and published");

        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::WaitingPermission)
        );
        let live = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(live.turn_phase, TurnPhase::WaitingPermission);
        assert_eq!(
            live.pending_permission_request
                .as_ref()
                .map(|request| request.id.as_str()),
            Some("perm-unsaved")
        );
        assert!(usecase.streaming_parts(&session_id).await.iter().any(|part| {
            matches!(part, MessagePart::Permission { request, .. } if request.id == "perm-unsaved")
        }));
        assert!(event_notifier.streaming_deltas()[delta_start..]
            .iter()
            .flat_map(|delta| delta.parts.iter())
            .any(|part| {
                matches!(part, MessagePart::Permission { request, .. } if request.id == "perm-unsaved")
            }));
        assert!(event_notifier.state_changes()[state_start..]
            .iter()
            .any(|change| {
                change.turn_phase == TurnPhase::WaitingPermission
                    && change
                        .pending_permission_request
                        .as_ref()
                        .map(|request| request.id.as_str())
                        == Some("perm-unsaved")
            }));
        assert!(session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::PermissionRequested { request, .. }
                    if request.id == "perm-unsaved"
            )));
        let reloaded = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        assert!(reloaded
            .messages
            .iter()
            .find(|message| message.id == agent_message_id)
            .and_then(|message| message.parts.as_deref())
            .unwrap_or_default()
            .iter()
            .any(|part| {
                matches!(part, MessagePart::Permission { request, .. } if request.id == "perm-unsaved")
            }));
    }

    #[tokio::test]
    async fn permission_event_failure_retries_before_publishing_pending_permission() {
        assert_permission_part_persistence_failure_retries_before_publication(
            PermissionPartPersistenceFailure::Event,
        )
        .await;
    }

    #[tokio::test]
    async fn permission_projection_failure_retries_before_publishing_pending_permission() {
        assert_permission_part_persistence_failure_retries_before_publication(
            PermissionPartPersistenceFailure::MessageProjection,
        )
        .await;
    }

    #[tokio::test]
    async fn test_streaming_delta_文字deltaを三三msでcoalesceする() {
        // Given: a started turn with a recording notifier.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();

        // When: the first delta opens the stream, then two more text deltas arrive within the
        // coalescing interval.
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "Hel".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_delta_count(&event_notifier, 1).await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "lo".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "!".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_delta_count(&event_notifier, 2).await;
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Then: the first event is a snapshot and the following two text deltas share one
        // append payload instead of emitting per backend event.
        let deltas = event_notifier.streaming_deltas();
        assert_eq!(deltas.len(), 2);
        assert!(deltas[0].snapshot);
        assert_eq!(deltas[0].seq, 1);
        assert_eq!(
            deltas[0].parts,
            vec![MessagePart::Text {
                content: "Hel".to_string(),
                parent_tool_use_id: None,
            }]
        );
        assert!(!deltas[1].snapshot);
        assert_eq!(deltas[1].seq, 2);
        assert_eq!(
            deltas[1].parts,
            vec![
                MessagePart::Text {
                    content: "lo".to_string(),
                    parent_tool_use_id: None,
                },
                MessagePart::Text {
                    content: "!".to_string(),
                    parent_tool_use_id: None,
                },
            ]
        );
    }

    #[tokio::test]
    async fn test_streaming_delta_emit失敗五連続で通常再送を打ち切りsnapshotフォールバックへ切り替わる(
    ) {
        // Given: a notifier that permanently fails to emit streaming deltas.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        event_notifier.set_streaming_delta_failure(true);
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();

        // When: the first delta keeps failing and another delta arrives after the fallback switch.
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "Hel".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |failures, _| failures >= 5)
            .await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "lo".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |_, suppressed| suppressed).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Then: attempts converge at the failure budget, every attempt is a snapshot resync, and
        // the fallback attempts carry the current full snapshot instead of the frozen retry.
        let deltas = event_notifier.streaming_deltas();
        assert_eq!(deltas.len(), 10);
        assert!(deltas.iter().all(|delta| delta.snapshot && delta.seq == 1));
        assert!(deltas.iter().any(|delta| delta.parts
            == vec![MessagePart::Text {
                content: "Hello".to_string(),
                parent_tool_use_id: None,
            }]));
        assert_eq!(
            usecase
                .stream_emit_failure_state_for_test(&session_id)
                .await,
            Some((10, true))
        );
    }

    #[tokio::test]
    async fn test_streaming_delta_フォールバック後にnotifier回復でsnapshot再同期しdelta配信を再開する(
    ) {
        // Given: streaming emits that fail past the fallback threshold.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        event_notifier.set_streaming_delta_failure(true);
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "Hel".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |failures, _| failures >= 6)
            .await;

        // When: the notifier recovers while the snapshot fallback is retrying.
        event_notifier.set_streaming_delta_failure(false);
        wait_for_stream_emit_failure_state(&usecase, &session_id, |failures, suppressed| {
            failures == 0 && !suppressed
        })
        .await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "lo".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_last_stream_delta(&event_notifier, |delta| !delta.snapshot).await;

        // Then: the snapshot resync lands with seq 1 and the following delta resumes appends.
        let deltas = event_notifier.streaming_deltas();
        let resync = &deltas[deltas.len() - 2];
        assert!(resync.snapshot);
        assert_eq!(resync.seq, 1);
        assert_eq!(
            resync.parts,
            vec![MessagePart::Text {
                content: "Hel".to_string(),
                parent_tool_use_id: None,
            }]
        );
        let resumed = deltas.last().unwrap();
        assert!(!resumed.snapshot);
        assert_eq!(resumed.seq, 2);
        assert_eq!(
            resumed.parts,
            vec![MessagePart::Text {
                content: "lo".to_string(),
                parent_tool_use_id: None,
            }]
        );
    }

    #[tokio::test]
    async fn b023_terminal_notification_failure_keeps_complete_terminal_live_and_reload_once() {
        // Given: streaming emits that fail until the emit stop threshold is reached.
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        event_notifier.set_streaming_delta_failure(true);
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "hello".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |_, suppressed| suppressed).await;
        let attempts_after_stop = event_notifier.streaming_deltas().len();

        // When: another delta arrives after emit suppression, then the turn completes.
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: " world".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(event_notifier.streaming_deltas().len(), attempts_after_stop);
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

        // Then: no further emit attempts happen, the turn completes, and the final message is
        // persisted with every accumulated part.
        assert_eq!(event_notifier.streaming_deltas().len(), attempts_after_stop);
        let live = usecase.get_session(&session_id).await.unwrap().unwrap();
        let restored = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        let agent_message = restored
            .messages
            .iter()
            .find(|message| message.role == MessageRole::Agent)
            .unwrap();
        assert_eq!(
            agent_message.parts.as_ref().unwrap(),
            &vec![MessagePart::Text {
                content: "hello world".to_string(),
                parent_tool_use_id: None,
            }]
        );
        assert_eq!(live.session.state, SessionState::Done);
        assert_eq!(restored.state, live.session.state);
        assert!(!live.queue_paused);
        assert!(live.pending_queue.is_empty());
        assert!(live.pending_permission_request.is_none());
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::TurnCompleted {
                        turn_id: 1,
                        stop_reason: None,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::FinalPartsRecorded { turn_id: 1, .. }
                ))
                .count(),
            1
        );
        assert_eq!(
            usecase
                .stream_emit_failure_state_for_test(&session_id)
                .await,
            Some((0, false))
        );
    }

    #[tokio::test]
    async fn crash終端snapshotはstreaming_emit完全停止後も回復したnotifierへ着地する() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        event_notifier.set_streaming_delta_failure(true);
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "partial output".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |_, suppressed| suppressed).await;

        event_notifier.set_streaming_delta_failure(false);
        let delivered_before_crash = event_notifier.delivered_streaming_deltas().len();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "CLI process exited".to_string(),
                },
            )
            .unwrap();
        wait_for_error_state_change(&event_notifier, &session_id).await;

        let delivered = event_notifier.delivered_streaming_deltas();
        assert!(delivered[delivered_before_crash..].iter().any(|delta| {
            delta.snapshot
                && delta.parts.iter().any(|part| {
                    matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                })
        }));
    }

    #[tokio::test]
    async fn test_streaming_delta_emit成功で連続失敗カウンタをリセットする() {
        // Given: streaming emits that fail a few times below the fallback threshold.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        event_notifier.set_streaming_delta_failure(true);
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "Hel".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |failures, _| failures >= 2)
            .await;

        // When: the notifier recovers before the fallback threshold.
        event_notifier.set_streaming_delta_failure(false);

        // Then: the retry succeeds, the counter resets, and delta delivery continues.
        wait_for_stream_emit_failure_state(&usecase, &session_id, |failures, suppressed| {
            failures == 0 && !suppressed
        })
        .await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "lo".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_last_stream_delta(&event_notifier, |delta| !delta.snapshot).await;
        let resumed = event_notifier.streaming_deltas().pop().unwrap();
        assert_eq!(resumed.seq, 2);
        assert_eq!(
            resumed.parts,
            vec![MessagePart::Text {
                content: "lo".to_string(),
                parent_tool_use_id: None,
            }]
        );
        assert_eq!(
            usecase
                .stream_emit_failure_state_for_test(&session_id)
                .await,
            Some((0, false))
        );
    }

    #[tokio::test]
    async fn test_turn終端後の_trailing_deltaはsnapshot_emitせず確定partsを変更しない() {
        // Given: a completed turn with persisted final parts.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "hello".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_delta_count(&event_notifier, 1).await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        let emitted_before_trailing = event_notifier.streaming_deltas().len();

        // When: a backend emits a delayed part after TurnCompleted.
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: " world".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Then: no standalone snapshot is emitted and the terminal winner's parts stay immutable.
        assert_eq!(
            event_notifier.streaming_deltas().len(),
            emitted_before_trailing
        );
        let restored = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        let agent_message = restored
            .messages
            .iter()
            .find(|message| message.role == MessageRole::Agent)
            .unwrap();
        assert_eq!(
            agent_message.parts.as_ref().unwrap(),
            &vec![MessagePart::Text {
                content: "hello".to_string(),
                parent_tool_use_id: None,
            }]
        );
    }

    #[tokio::test]
    async fn test_start_turn_保存済み会話をreinjectしてpromptへprefixする() {
        // Given: an existing session with prior messages but no backend session id.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "remember alpha",
            None,
            None,
        )
        .unwrap();
        add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "alpha acknowledged",
            None,
            None,
        )
        .unwrap();

        // When: the next turn starts through a lazy-open runtime.
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "what did I ask you to remember?".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        // Then: the backend receives the restore prefix and the session records Reinjected.
        let prompt = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .find_map(|kind| match kind {
                TestRuntimeCallKind::StartTurnPrompt { prompt } => Some(prompt),
                _ => None,
            })
            .expect("start prompt recorded");
        assert!(prompt.contains("releash_restored_conversation"));
        assert!(prompt.contains("remember alpha"));
        assert!(prompt.contains("alpha acknowledged"));
        assert!(prompt.ends_with("what did I ask you to remember?"));
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(
            loaded.session.context_carry,
            Some(ContextCarryState::Reinjected)
        );
    }

    async fn assert_recovery_start_commit_failure_retries_exact_trigger(resume_mismatch: bool) {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(
            &session_store,
            tmp.path(),
            resume_mismatch.then_some("stored-provider-session"),
        );
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;

        if !resume_mismatch {
            controller
                .emit(
                    &session.id,
                    AgentRuntimeEvent::SessionEstablished {
                        backend_session_id: "initial-provider-session".to_string(),
                        resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                    },
                )
                .unwrap();
            tokio::time::timeout(Duration::from_secs(1), async {
                while !usecase.provider_session_is_confirmed(&session.id).await {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
        }

        let fail_once = Arc::new(AtomicBool::new(true));
        let attempted_recovery_ids = Arc::new(Mutex::new(Vec::new()));
        session_store.set_append_event_hook_for_test(Arc::new({
            let fail_once = Arc::clone(&fail_once);
            let attempted_recovery_ids = Arc::clone(&attempted_recovery_ids);
            move |_, event| {
                if let AgentSessionEvent::BackendSessionRecoveryStarted { recovery_id, .. } = event
                {
                    attempted_recovery_ids
                        .lock()
                        .unwrap()
                        .push(recovery_id.clone());
                    if fail_once.swap(false, Ordering::SeqCst) {
                        return Err("injected recovery start commit failure".to_string());
                    }
                }
                Ok(())
            }
        }));

        let trigger = if resume_mismatch {
            AgentRuntimeEvent::SessionEstablished {
                backend_session_id: "mismatched-provider-session".to_string(),
                resume: crate::domain::agent_session::gateway::ResumeOutcome::Mismatch {
                    actual: "mismatched-provider-session".to_string(),
                },
            }
        } else {
            AgentRuntimeEvent::BackendSessionCleared
        };
        controller.emit(&session.id, trigger).unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;

        let attempted_recovery_ids = attempted_recovery_ids.lock().unwrap().clone();
        assert!(attempted_recovery_ids.len() >= 2);
        assert!(attempted_recovery_ids
            .iter()
            .all(|recovery_id| recovery_id == &attempted_recovery_ids[0]));
        let recovery_events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .into_iter()
            .filter_map(|event| match event {
                AgentSessionEvent::BackendSessionRecoveryStarted { recovery_id, .. } => {
                    Some(recovery_id)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(recovery_events, vec![attempted_recovery_ids[0].clone()]);
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2,
            "the retained trigger must not duplicate the provider recovery effect"
        );
    }

    #[tokio::test]
    async fn backend_session_cleared_retries_exact_recovery_trigger_after_begin_commit_failure() {
        assert_recovery_start_commit_failure_retries_exact_trigger(false).await;
    }

    #[tokio::test]
    async fn resume_mismatch_retries_exact_recovery_trigger_after_begin_commit_failure() {
        assert_recovery_start_commit_failure_retries_exact_trigger(true).await;
    }

    #[tokio::test]
    async fn provider_established_retries_same_recovery_completion_after_commit_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "initial-provider-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        let recovery_id = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .into_iter()
            .find_map(|event| match event {
                AgentSessionEvent::BackendSessionRecoveryStarted { recovery_id, .. } => {
                    Some(recovery_id)
                }
                _ => None,
            })
            .expect("backend-session-cleared must reserve one recovery identity");

        let completion_attempts = Arc::new(Mutex::new(0_usize));
        session_store.set_append_event_hook_for_test(Arc::new({
            let completion_attempts = Arc::clone(&completion_attempts);
            move |_, event| {
                if matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
                ) {
                    let mut attempts = completion_attempts.lock().unwrap();
                    *attempts += 1;
                    if *attempts == 1 {
                        return Err("injected recovery completion commit failure".to_string());
                    }
                }
                Ok(())
            }
        }));
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "replacement-provider-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let meta = session_store
                    .get_session_meta(tmp.path(), &session.id)
                    .unwrap()
                    .unwrap();
                if meta.provider_session_generation == 2
                    && meta.agent_session_id.as_deref() == Some("replacement-provider-session")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        assert_eq!(*completion_attempts.lock().unwrap(), 2);
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryCompleted {
                        recovery_id: actual,
                        ..
                    } if actual == &recovery_id
                ))
                .count(),
            1
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryFailed {
                recovery_id: actual,
                ..
            } if actual == &recovery_id
        )));
        assert!(usecase.provider_session_is_confirmed(&session.id).await);
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2,
            "retrying the completion commit must not reopen the provider"
        );
    }

    #[tokio::test]
    async fn recovery_failure_commit_retries_same_identity_without_reopening_provider() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "initial-provider-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        controller.fail_next_open_session();
        let failure_attempt_ids = Arc::new(Mutex::new(Vec::new()));
        session_store.set_append_event_hook_for_test(Arc::new({
            let failure_attempt_ids = Arc::clone(&failure_attempt_ids);
            move |_, event| {
                if let AgentSessionEvent::BackendSessionRecoveryFailed { recovery_id, .. } = event {
                    let mut ids = failure_attempt_ids.lock().unwrap();
                    ids.push(recovery_id.clone());
                    if ids.len() == 1 {
                        return Err("injected recovery failure commit failure".to_string());
                    }
                }
                Ok(())
            }
        }));
        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if session_store
                    .load_session_events(tmp.path(), &session.id)
                    .unwrap()
                    .iter()
                    .any(|event| {
                        matches!(
                            event,
                            AgentSessionEvent::BackendSessionRecoveryFailed { .. }
                        )
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let failure_attempt_ids = failure_attempt_ids.lock().unwrap().clone();
        assert_eq!(failure_attempt_ids.len(), 2);
        assert_eq!(failure_attempt_ids[0], failure_attempt_ids[1]);
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryStarted {
                        recovery_id: actual,
                        ..
                    } if actual == &failure_attempt_ids[0]
                ))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryFailed {
                        recovery_id: actual,
                        ..
                    } if actual == &failure_attempt_ids[0]
                ))
                .count(),
            1
        );
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2,
            "the recovery provider effect must run exactly once"
        );
        assert_eq!(
            session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .state,
            SessionState::Error
        );
    }

    #[tokio::test]
    async fn test_resume_mismatch_進行中turnをrequeueしてreinjectで再開する() {
        // Given: a session that tries to resume an old backend session.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("old-backend-session".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "remember alpha",
            None,
            None,
        )
        .unwrap();

        // When: the backend reports that the resumed id does not match the actual thread.
        let editor_context = AgentEditorContext {
            active_editor_path: Some("src/main.rs".to_string()),
            open_editor_paths: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
            selection: Some(AgentEditorSelection {
                file_path: "src/main.rs".to_string(),
                start_line: 4,
                end_line: 9,
            }),
        };
        let images = vec![ImageAttachment {
            data: "iVBORw==".to_string(),
            media_type: "image/png".to_string(),
        }];
        let mentions = vec![crate::domain::code::MentionReference {
            file_path: "src/lib.rs".to_string(),
            start_line: Some(12),
            end_line: Some(18),
        }];
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "continue".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: Some(images.clone()),
                mentions: Some(mentions.clone()),
                editor_context: Some(editor_context.clone()),
            })
            .await
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 1).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "actual-backend-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::Mismatch {
                        actual: "actual-backend-session".to_string(),
                    },
                },
            )
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1,
            "the retry must remain queued until the new backend session is established"
        );
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-backend-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 2).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Then: the retry prompt is reinjected, the stale backend id is cleared, and the
        // mismatched runtime was closed before reopening.
        let prompts = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .filter_map(|kind| match kind {
                TestRuntimeCallKind::StartTurnPrompt { prompt } => Some(prompt),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(prompts[0], "continue");
        assert!(prompts[1].contains("releash_restored_conversation"));
        assert!(prompts[1].contains("remember alpha"));
        assert!(prompts[1].ends_with("continue"));
        assert!(controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Close));
        let editor_contexts = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .filter_map(|kind| match kind {
                TestRuntimeCallKind::StartTurnEditorContext { editor_context } => editor_context,
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(editor_contexts.len(), 2);
        assert_eq!(
            editor_contexts[0],
            EditorContext::from(editor_context.clone())
        );
        assert_eq!(editor_contexts[1], EditorContext::from(editor_context));
        let turn_images = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .filter_map(|kind| match kind {
                TestRuntimeCallKind::StartTurnImages { images } => Some(images),
                _ => None,
            })
            .collect::<Vec<_>>();
        let expected_images = images
            .into_iter()
            .map(|image| AttachmentPayload {
                data: image.data,
                media_type: image.media_type,
            })
            .collect::<Vec<_>>();
        assert_eq!(turn_images, vec![expected_images.clone(), expected_images]);
        let system_prompts = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .filter_map(|kind| match kind {
                TestRuntimeCallKind::StartTurnSystemPrompt { system_prompt } => Some(system_prompt),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(system_prompts.len(), 2);
        assert_eq!(system_prompts[0], system_prompts[1]);
        assert!(system_prompts[0]
            .as_deref()
            .is_some_and(|prompt| prompt.contains("src/lib.rs")));
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(
            loaded.session.agent_session_id.as_deref(),
            Some("fresh-backend-session")
        );
        assert_eq!(
            loaded.session.context_carry,
            Some(ContextCarryState::Reinjected)
        );
        assert!(loaded.session.messages.iter().any(|message| {
            message.parts.as_deref().is_some_and(|parts| {
                parts.iter().any(|part| matches!(
                    part,
                    MessagePart::SystemNotification {
                        notification_type: crate::usecase::agent_session::session::SystemNotificationType::SessionRecovery,
                        label,
                        ..
                    } if label == "backend セッションを作り直したため文脈は引き継がれません"
                ))
            })
        }));

        let recovery_events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .into_iter()
            .filter(|event| {
                matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryStarted { .. }
                        | AgentSessionEvent::SessionConfigurationReactivated { .. }
                        | AgentSessionEvent::SessionGoalReactivated { .. }
                        | AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
                )
            })
            .collect::<Vec<_>>();
        let retried_mentions = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .into_iter()
            .filter_map(|event| match event {
                AgentSessionEvent::TurnStarted { prompt, .. } if prompt.content == "continue" => {
                    Some(prompt.mentions)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let expected_mentions = mentions;
        assert_eq!(
            retried_mentions,
            vec![expected_mentions.clone(), expected_mentions]
        );
        assert_eq!(recovery_events.len(), 4);
        let recovery_id = match &recovery_events[0] {
            AgentSessionEvent::BackendSessionRecoveryStarted { recovery_id, .. } => {
                recovery_id.clone()
            }
            event => panic!("unexpected recovery event: {event:?}"),
        };
        assert!(matches!(
            &recovery_events[1],
            AgentSessionEvent::SessionConfigurationReactivated {
                recovery_id: actual,
                provider_session_generation: 1,
                ..
            } if actual == &recovery_id
        ));
        assert!(matches!(
            &recovery_events[2],
            AgentSessionEvent::SessionGoalReactivated {
                recovery_id: actual,
                outcome: crate::usecase::agent_session::event_log::GoalReactivationOutcome::NoCurrentGoal,
                provider_session_generation: 1,
                ..
            } if actual == &recovery_id
        ));
        assert!(matches!(
            &recovery_events[3],
            AgentSessionEvent::BackendSessionRecoveryCompleted {
                recovery_id: actual,
                provider_session_generation: 1,
                ..
            } if actual == &recovery_id
        ));
    }

    #[test]
    fn completed_recovery_restore_policy_is_identical_after_runtime_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let original_store = Arc::new(build_session_store());
        let session = create_session_internal_with_attributes(
            &original_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        add_message_internal(
            &original_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "remember durable context",
            None,
            None,
        )
        .unwrap();
        original_store
            .begin_backend_session_recovery(
                tmp.path(),
                &session.id,
                "durable-recovery",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        original_store
            .complete_backend_session_recovery(
                tmp.path(),
                &session.id,
                "durable-recovery",
                0,
                "fresh-provider-session".to_string(),
            )
            .unwrap();

        let (running_usecase, _) =
            build_agent_runtime_usecase_with_controller(original_store.clone(), tmp.path());
        let without_restart = context_restore_policy_for_turn(
            &running_usecase.ctx,
            &session.id,
            "next-agent-message",
            true,
        )
        .unwrap();
        let prompt_without_restart =
            apply_restore_prompt_prefix("continue".to_string(), &without_restart.plan);
        let carry_without_restart = original_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .context_carry;
        drop(running_usecase);
        drop(original_store);

        let reopened_store = Arc::new(build_session_store());
        let (restarted_usecase, _) =
            build_agent_runtime_usecase_with_controller(reopened_store.clone(), tmp.path());
        let after_restart = context_restore_policy_for_turn(
            &restarted_usecase.ctx,
            &session.id,
            "next-agent-message",
            false,
        )
        .unwrap();
        let prompt_after_restart =
            apply_restore_prompt_prefix("continue".to_string(), &after_restart.plan);
        let meta_after_restart = reopened_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();

        assert!(without_restart.recovery_restore_required);
        assert!(after_restart.recovery_restore_required);
        assert_eq!(prompt_after_restart, prompt_without_restart);
        assert!(prompt_after_restart.contains("releash_restored_conversation"));
        assert!(prompt_after_restart.contains("remember durable context"));
        assert_eq!(meta_after_restart.context_carry, carry_without_restart);
        assert_eq!(meta_after_restart.context_reinjection_generation, Some(1));
    }

    #[tokio::test]
    async fn test_backend_session_clearedは新規sessionでturnを再開する() {
        // Given: a session that was previously resumed.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("backend-session".to_string());
        session.context_carry = Some(ContextCarryState::Resumed);
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "continue".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        // When: the backend reports that its resumable session was cleared.
        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "replacement-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 2).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Then: the dead backend id is replaced and the original turn is retried.
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(
            loaded.session.agent_session_id.as_deref(),
            Some("replacement-session")
        );
        assert_eq!(
            loaded.session.context_carry,
            Some(ContextCarryState::Failed)
        );
    }

    #[tokio::test]
    async fn recovery_reopens_with_latest_persisted_configuration_and_generation_two() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let normal_session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes::default(),
        )
        .unwrap();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "initial-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let generation = session_store
                    .get_session_meta(tmp.path(), &session.id)
                    .unwrap()
                    .unwrap()
                    .provider_session_generation;
                if generation == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .provider_session_generation,
            1
        );

        session_store
            .update_backend_selection(
                tmp.path(),
                &session.id,
                "claude".to_string(),
                Some("claude-4-opus".to_string()),
            )
            .unwrap();
        session_store
            .update_permission_mode(tmp.path(), &session.id, PermissionMode::FULL)
            .unwrap();
        session_store
            .update_plan_mode(tmp.path(), &session.id, true)
            .unwrap();
        let before_recovery = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .to_summary();
        let normal_before_recovery = session_store
            .get_session_meta(tmp.path(), &normal_session.id)
            .unwrap()
            .unwrap()
            .to_summary();

        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;

        let opens = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .filter_map(|kind| match kind {
                TestRuntimeCallKind::OpenSession {
                    resume,
                    model,
                    permission_mode,
                    plan_mode,
                    ..
                } => Some((resume, model, permission_mode, plan_mode)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(opens.len(), 2);
        assert_eq!(opens[1].0, None);
        assert_eq!(opens[1].1, "claude-4-opus");
        assert_eq!(opens[1].2, PermissionMode::Full);
        assert!(opens[1].3);

        let worktree_path = tmp.path().to_string_lossy().to_string();
        let tauri_sessions = usecase.list_sessions(&worktree_path).await.unwrap();
        let workspace_sessions =
            StoredWorkspaceSessionGateway::new(session_store.clone(), tmp.path().to_path_buf())
                .list_active_sessions(&worktree_path)
                .unwrap();
        let tauri_recovering = tauri_sessions
            .iter()
            .find(|summary| summary.id == session.id)
            .unwrap();
        let workspace_recovering = workspace_sessions
            .iter()
            .find(|summary| summary.id == session.id)
            .unwrap();
        assert_eq!(tauri_recovering.state, SessionState::Active);
        assert_eq!(tauri_recovering.updated_at, before_recovery.updated_at);
        assert_eq!(workspace_recovering.state, WorkspaceSessionState::Active);
        assert_eq!(workspace_recovering.updated_at, before_recovery.updated_at);
        let tauri_normal = tauri_sessions
            .iter()
            .find(|summary| summary.id == normal_session.id)
            .unwrap();
        let workspace_normal = workspace_sessions
            .iter()
            .find(|summary| summary.id == normal_session.id)
            .unwrap();
        assert_eq!(tauri_normal.state, normal_before_recovery.state);
        assert_eq!(tauri_normal.updated_at, normal_before_recovery.updated_at);
        assert_eq!(workspace_normal.state, WorkspaceSessionState::Idle);
        assert_eq!(
            workspace_normal.updated_at,
            normal_before_recovery.updated_at
        );

        let events_during_recovery = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        let recovery_id = events_during_recovery
            .iter()
            .find_map(|event| match event {
                AgentSessionEvent::BackendSessionRecoveryStarted {
                    recovery_id,
                    old_provider_session_generation: 1,
                    ..
                } => Some(recovery_id.clone()),
                _ => None,
            })
            .expect("recovery starts from the established generation");

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let generation = session_store
                    .get_session_meta(tmp.path(), &session.id)
                    .unwrap()
                    .unwrap()
                    .provider_session_generation;
                if generation == 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let recovered_meta = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(recovered_meta.backend_id, "claude");
        assert_eq!(
            recovered_meta.selected_model.as_deref(),
            Some("claude-4-opus")
        );
        assert_eq!(recovered_meta.permission_mode, PermissionMode::FULL);
        assert!(recovered_meta.plan_mode);
        assert_eq!(recovered_meta.provider_session_generation, 2);
        let recovered_events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(recovered_events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::SessionConfigurationReactivated {
                recovery_id: actual,
                provider_session_generation: 2,
                ..
            } if actual == &recovery_id
        )));
        assert!(recovered_events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::SessionGoalReactivated {
                recovery_id: actual,
                provider_session_generation: 2,
                ..
            } if actual == &recovery_id
        )));
        assert!(recovered_events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryCompleted {
                recovery_id: actual,
                provider_session_generation: 2,
                ..
            } if actual == &recovery_id
        )));
    }

    #[tokio::test]
    async fn test_codex_resume失敗はfresh_sessionで復活しdead_threadを再利用しない() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        let unaffected_session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes::default(),
        )
        .unwrap();
        controller.fail_next_resume_open();

        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "recover this turn".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        wait_for_open_count(&controller, &session.id, 2).await;
        let resumes = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .filter_map(|kind| match kind {
                TestRuntimeCallKind::OpenSession { resume, .. } => Some(resume),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            resumes,
            vec![Some("dead-thread".to_string()), None],
            "recovery must clear resume metadata before opening the replacement session"
        );
        assert!(session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryStarted {
                    old_provider_session_generation: 0,
                    reason: BackendSessionRecoveryReason::BackendSessionLost,
                    ..
                }
            )));
        assert!(!session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
            )));

        let worktree_path = tmp.path().to_string_lossy().to_string();
        let listed_during_recovery = tokio::time::timeout(
            Duration::from_secs(1),
            usecase.list_sessions(&worktree_path),
        )
        .await
        .expect("another session's list must not wait for recovery establishment")
        .unwrap();
        assert!(listed_during_recovery
            .iter()
            .any(|summary| summary.id == unaffected_session.id));
        assert_eq!(
            listed_during_recovery
                .iter()
                .find(|summary| summary.id == session.id)
                .expect("recovering session keeps its previously published summary")
                .agent_session_id
                .as_deref(),
            Some("dead-thread"),
            "the recovering session must not publish its cleared resume metadata"
        );

        let config_usecase = Arc::clone(&usecase);
        let config_session_id = session.id.clone();
        let config_update = tokio::spawn(async move {
            config_usecase
                .set_model(&config_session_id, "codex:gpt-5")
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !config_update.is_finished(),
            "configuration changes must remain blocked until recovery completes"
        );

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        config_update.await.unwrap().unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 1).await;
        let listed = usecase.list_sessions(&worktree_path).await.unwrap();
        let listed_session = listed
            .iter()
            .find(|summary| summary.id == session.id)
            .expect("recovered session remains listed");
        assert_eq!(
            listed_session.agent_session_id.as_deref(),
            Some("fresh-thread")
        );
        tokio::time::sleep(Duration::from_millis(20)).await;

        let recovered = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(
            recovered.session.agent_session_id.as_deref(),
            Some("fresh-thread")
        );
        let meta = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.provider_session_generation, 1);
        assert!(recovered.session.messages.iter().any(|message| {
            message.parts.as_deref().is_some_and(|parts| {
                parts.iter().any(|part| matches!(
                    part,
                    MessagePart::SystemNotification {
                        notification_type: crate::usecase::agent_session::session::SystemNotificationType::SessionRecovery,
                        ..
                    }
                ))
            })
        }));

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Idle).await;
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "follow up".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 2).await;
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2,
            "the recovered live runtime must not reopen the dead thread"
        );
    }

    #[tokio::test]
    async fn recovery_completion_without_pending_turn_publishes_notice_immediately() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();

        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        for _ in 0..2 {
            let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
            assert_eq!(
                loaded
                    .session
                    .messages
                    .iter()
                    .flat_map(|message| message.parts.as_deref().unwrap_or_default())
                    .filter(|part| matches!(
                        part,
                        MessagePart::SystemNotification {
                            notification_type: SystemNotificationType::SessionRecovery,
                            ..
                        }
                    ))
                    .count(),
                1
            );
        }
        assert!(!controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));
    }

    #[derive(Clone, Copy, Debug)]
    enum B036CrashBoundary {
        AfterRecoveryStart,
        AfterExternalEffect,
        AfterCompletion,
        BeforeMessagePublication,
    }

    #[tokio::test]
    async fn b036_recovery_crash_boundaries_preserve_identity_and_limit_effect_and_message_to_one()
    {
        for boundary in [
            B036CrashBoundary::AfterRecoveryStart,
            B036CrashBoundary::AfterExternalEffect,
            B036CrashBoundary::AfterCompletion,
            B036CrashBoundary::BeforeMessagePublication,
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let local_store =
                LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                    .unwrap();
            let session_store =
                Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
            let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
            session_store.set_local_event_repository(
                repository,
                local_store.installation_id().to_string(),
                Arc::new(AgentSessionProjectionCodecV1),
            );
            let session =
                provider_establish_test_session(&session_store, tmp.path(), Some("dead-provider"));
            let recovery_id = format!("b036-recovery-{boundary:?}");
            let old_generation = session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .provider_session_generation;

            let external_effect_count = match boundary {
                B036CrashBoundary::AfterRecoveryStart => {
                    session_store
                        .begin_backend_session_recovery(
                            tmp.path(),
                            &session.id,
                            &recovery_id,
                            BackendSessionRecoveryReason::BackendSessionLost,
                        )
                        .unwrap();
                    0
                }
                B036CrashBoundary::AfterExternalEffect => {
                    let (usecase, controller) =
                        build_agent_runtime_usecase_with_controller_and_spawner(
                            session_store.clone(),
                            tmp.path(),
                            Arc::new(DroppingSpawner),
                        );
                    controller.fail_next_open_session();
                    assert!(recover_backend_session_with_identity(
                        &usecase.ctx,
                        &session.id,
                        BackendSessionRecoveryReason::BackendSessionLost,
                        recovery_id.clone(),
                    )
                    .await
                    .is_ok());
                    let count = controller
                        .call_kinds_for(&session.id)
                        .iter()
                        .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                        .count();
                    assert_eq!(count, 1, "{boundary:?} must cross the effect port once");
                    count
                }
                B036CrashBoundary::AfterCompletion
                | B036CrashBoundary::BeforeMessagePublication => {
                    let (usecase, controller) = build_agent_runtime_usecase_with_controller(
                        session_store.clone(),
                        tmp.path(),
                    );
                    recover_backend_session_with_identity(
                        &usecase.ctx,
                        &session.id,
                        BackendSessionRecoveryReason::BackendSessionLost,
                        recovery_id.clone(),
                    )
                    .await
                    .unwrap();
                    let count = controller
                        .call_kinds_for(&session.id)
                        .iter()
                        .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                        .count();
                    assert_eq!(count, 1, "{boundary:?} must cross the effect port once");
                    session_store
                        .complete_backend_session_recovery(
                            tmp.path(),
                            &session.id,
                            &recovery_id,
                            old_generation,
                            "replacement-provider".to_string(),
                        )
                        .unwrap();
                    if matches!(boundary, B036CrashBoundary::BeforeMessagePublication) {
                        local_store.fault_injector().arm_fail_before_begin();
                        assert!(
                            reconcile_pending_recovery_message(&usecase.ctx, &session.id)
                                .await
                                .is_err()
                        );
                        assert!(session_store
                            .get_session_meta(tmp.path(), &session.id)
                            .unwrap()
                            .unwrap()
                            .pending_recovery_message
                            .is_some());
                    }
                    controller.close_event_streams_for_test(&session.id);
                    count
                }
            };

            let before_restart_events = session_store
                .load_session_events(tmp.path(), &session.id)
                .unwrap();
            assert_eq!(
                before_restart_events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        AgentSessionEvent::BackendSessionRecoveryStarted {
                            recovery_id: actual,
                            ..
                        } if actual == &recovery_id
                    ))
                    .count(),
                1,
                "{boundary:?} must retain exactly one recovery identity"
            );
            let completed_before_restart = matches!(
                boundary,
                B036CrashBoundary::AfterCompletion | B036CrashBoundary::BeforeMessagePublication
            );
            assert_eq!(
                before_restart_events.iter().any(|event| matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryCompleted {
                        recovery_id: actual,
                        ..
                    } if actual == &recovery_id
                )),
                completed_before_restart
            );
            drop(session_store);
            drop(local_store);
            tokio::task::yield_now().await;

            let reopened_store =
                LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                    .unwrap();
            let reopened_session_store =
                Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
            let repository: Arc<dyn LocalEventTransactionRepository> = reopened_store.clone();
            reopened_session_store.set_local_event_repository(
                repository,
                reopened_store.installation_id().to_string(),
                Arc::new(AgentSessionProjectionCodecV1),
            );
            let (restarted, restart_controller) = build_agent_runtime_usecase_with_controller(
                reopened_session_store.clone(),
                tmp.path(),
            );
            let first = restarted.get_session(&session.id).await.unwrap().unwrap();
            let second = restarted.get_session(&session.id).await.unwrap().unwrap();
            let recovery_message_count = |response: &GetSessionResponse| {
                response
                    .session
                    .messages
                    .iter()
                    .filter(|message| {
                        message.parts.as_deref().is_some_and(|parts| {
                            parts.iter().any(|part| {
                                matches!(
                                    part,
                                    MessagePart::SystemNotification {
                                        notification_type: SystemNotificationType::SessionRecovery,
                                        ..
                                    }
                                ) || matches!(
                                    part,
                                    MessagePart::Error { content, .. }
                                        if content.starts_with("backend session recovery failed:")
                                )
                            })
                        })
                    })
                    .count()
            };
            assert_eq!(recovery_message_count(&first), 1, "{boundary:?}");
            assert_eq!(recovery_message_count(&second), 1, "{boundary:?}");
            assert_eq!(
                external_effect_count
                    + restart_controller
                        .call_kinds_for(&session.id)
                        .iter()
                        .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                        .count(),
                external_effect_count,
                "restart must not repeat the recovery provider effect at {boundary:?}"
            );
            assert!(external_effect_count <= 1);

            let after_restart_events = reopened_session_store
                .load_session_events(tmp.path(), &session.id)
                .unwrap();
            let completed = after_restart_events
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        AgentSessionEvent::BackendSessionRecoveryCompleted {
                            recovery_id: actual,
                            ..
                        } if actual == &recovery_id
                    )
                })
                .count();
            let failed = after_restart_events
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        AgentSessionEvent::BackendSessionRecoveryFailed {
                            recovery_id: actual,
                            ..
                        } if actual == &recovery_id
                    )
                })
                .count();
            assert_eq!(completed + failed, 1, "{boundary:?} must be fully resolved");
            assert_eq!(completed, usize::from(completed_before_restart));
            assert!(reopened_session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .pending_recovery_message
                .is_none());
        }
    }

    #[tokio::test]
    async fn completed_recovery_notice_is_restored_once_before_the_next_turn_after_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let original_store = Arc::new(build_session_store());
        let session = create_session_internal_with_attributes(
            &original_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        original_store
            .begin_backend_session_recovery(
                tmp.path(),
                &session.id,
                "completed-before-restart",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        original_store
            .complete_backend_session_recovery(
                tmp.path(),
                &session.id,
                "completed-before-restart",
                0,
                "fresh-thread".to_string(),
            )
            .unwrap();
        let before_restart = original_store
            .load_full_session_for_restore(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert!(before_restart.messages.is_empty());
        assert!(original_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .pending_recovery_message
            .is_some());
        drop(original_store);

        let reopened_store = Arc::new(build_session_store());
        let (usecase, _) =
            build_agent_runtime_usecase_with_controller(reopened_store.clone(), tmp.path());
        let recovered = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(
            recovered
                .session
                .messages
                .iter()
                .flat_map(|message| message.parts.as_deref().unwrap_or_default())
                .filter(|part| matches!(
                    part,
                    MessagePart::SystemNotification {
                        notification_type: SystemNotificationType::SessionRecovery,
                        ..
                    }
                ))
                .count(),
            1
        );

        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "next turn after restart".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        let persisted = reopened_store
            .load_full_session_for_restore(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        let notice_index = persisted
            .messages
            .iter()
            .position(|message| {
                message.parts.as_deref().is_some_and(|parts| {
                    parts.iter().any(|part| {
                        matches!(
                            part,
                            MessagePart::SystemNotification {
                                notification_type: SystemNotificationType::SessionRecovery,
                                ..
                            }
                        )
                    })
                })
            })
            .unwrap();
        let next_turn_index = persisted
            .messages
            .iter()
            .position(|message| message.content == "next turn after restart")
            .unwrap();
        assert!(notice_index < next_turn_index);
        assert_eq!(
            persisted
                .messages
                .iter()
                .flat_map(|message| message.parts.as_deref().unwrap_or_default())
                .filter(|part| matches!(
                    part,
                    MessagePart::SystemNotification {
                        notification_type: SystemNotificationType::SessionRecovery,
                        ..
                    }
                ))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn recovery_notice_survives_retried_turn_start_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("old-session".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "retry me".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        controller.fail_next_start_turn();
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 2).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.state, SessionState::Error);
        assert!(loaded.session.messages.iter().any(|message| {
            message.parts.as_deref().is_some_and(|parts| {
                parts.iter().any(|part| {
                    matches!(
                        part,
                        MessagePart::SystemNotification {
                            notification_type: SystemNotificationType::SessionRecovery,
                            ..
                        }
                    )
                })
            })
        }));
    }

    #[tokio::test]
    async fn failed_recovery_error_part_is_reconciled_once_after_a_write_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        let fail_error_once = Arc::new(AtomicBool::new(true));
        session_store.set_persist_parts_hook_for_test(Arc::new({
            let fail_error_once = fail_error_once.clone();
            move |_, _, parts| {
                if parts
                    .iter()
                    .any(|part| matches!(part, MessagePart::Error { .. }))
                    && fail_error_once.swap(false, Ordering::SeqCst)
                {
                    return Err("injected recovery error persistence failure".to_string());
                }
                Ok(())
            }
        }));
        controller.fail_next_resume_open();
        controller.fail_next_open();

        let result = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "recover".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await;
        assert!(
            result.is_ok(),
            "the send was durably accepted before backend recovery failed"
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let meta = session_store
                    .get_session_meta(tmp.path(), &session.id)
                    .unwrap()
                    .unwrap();
                if !fail_error_once.load(Ordering::SeqCst)
                    && meta.state == SessionState::Error
                    && meta.pending_recovery_message.is_some()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the detached recovery failure must settle before reconciliation");
        let before_reconcile = session_store
            .load_full_session_for_restore(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(before_reconcile.state, SessionState::Error);
        assert!(!before_reconcile.messages.iter().any(|message| {
            message.parts.as_deref().is_some_and(|parts| {
                parts
                    .iter()
                    .any(|part| matches!(part, MessagePart::Error { .. }))
            })
        }));
        assert!(session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .pending_recovery_message
            .is_some());

        for _ in 0..2 {
            let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
            assert_eq!(loaded.session.state, SessionState::Error);
            assert_eq!(
                loaded
                    .session
                    .messages
                    .iter()
                    .flat_map(|message| message.parts.as_deref().unwrap_or_default())
                    .filter(|part| matches!(
                        part,
                        MessagePart::Error { content, .. }
                            if content.contains("injected test open failure")
                    ))
                    .count(),
                1
            );
        }
        assert!(session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .pending_recovery_message
            .is_none());
        assert!(session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryFailed { .. }
            )));
    }

    #[tokio::test]
    async fn startup_recovery_begin_failure_does_not_start_cleanup_effects() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        session_store.set_append_event_hook_for_test(Arc::new(|_, event| {
            if matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryStarted { .. }
            ) {
                return Err("injected recovery begin failure".to_string());
            }
            Ok(())
        }));
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();

        let result = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "startup recovery".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await;
        assert!(result.is_err());
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Interrupt));
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Close));
        assert!(event_notifier.streaming_deltas().is_empty());
        assert!(!session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryFailed { .. }
            )));
    }

    #[tokio::test]
    async fn live_recovery_begin_failure_preserves_runtime_and_state_without_provider_effect() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("live-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "live recovery".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Streaming).await;
        session_store.set_append_event_hook_for_test(Arc::new(|_, event| {
            if matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryStarted { .. }
            ) {
                return Err("injected recovery begin failure".to_string());
            }
            Ok(())
        }));

        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.turn_phase, TurnPhase::Streaming);
        assert_eq!(loaded.session.state, SessionState::Active);
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Interrupt));
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Close));
        assert!(event_notifier.streaming_deltas().is_empty());
    }

    #[tokio::test]
    async fn recovery_completion_commit_failure_does_not_publish_a_false_error() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        session_store.set_append_event_hook_for_test(Arc::new(|_, event| {
            if matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
            ) {
                return Err("injected completion commit failure".to_string());
            }
            Ok(())
        }));
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "recover".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        let loaded = session_store
            .load_full_session_for_restore(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.state, SessionState::Active);
        assert_eq!(loaded.context_carry, Some(ContextCarryState::Failed));
        assert!(loaded.agent_session_id.is_none());
        assert!(!loaded.messages.iter().any(|message| {
            message.role == MessageRole::Agent
                && message.parts.as_deref().is_some_and(|parts| {
                    parts
                        .iter()
                        .any(|part| matches!(part, MessagePart::Error { .. }))
                })
        }));
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(matches!(
            TurnEventLog::from_events(events.clone())
                .project()
                .backend_recovery,
            Some(BackendSessionRecoveryProjection::Recovering { .. })
        ));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
                | AgentSessionEvent::BackendSessionRecoveryFailed { .. }
        )));
    }

    #[tokio::test]
    async fn recovery_notice_persistence_failure_does_not_demote_completed_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "recover".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        let fail_notice_once = Arc::new(AtomicBool::new(true));
        session_store.set_append_message_hook_for_test(Arc::new({
            let fail_notice_once = fail_notice_once.clone();
            move |_, message| {
                if message.parts.as_deref().is_some_and(|parts| {
                    parts.iter().any(|part| {
                        matches!(
                            part,
                            MessagePart::SystemNotification {
                                notification_type: SystemNotificationType::SessionRecovery,
                                ..
                            }
                        )
                    })
                }) && fail_notice_once.swap(false, Ordering::SeqCst)
                {
                    return Err("injected recovery notice persistence failure".to_string());
                }
                Ok(())
            }
        }));
        let mut completion = usecase
            .ctx
            .sessions
            .lock()
            .await
            .get(&session.id)
            .and_then(|state| state.backend_recovery.as_ref())
            .expect("recovery is in progress before fresh establishment")
            .completion
            .subscribe();

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), completion.changed())
            .await
            .expect("recovery completion signal is sent")
            .unwrap();
        assert!(*completion.borrow());
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let recovery_finished = usecase
                    .ctx
                    .sessions
                    .lock()
                    .await
                    .get(&session.id)
                    .is_none_or(|state| state.backend_recovery.is_none());
                if recovery_finished {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let committed = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_ne!(committed.state, SessionState::Error);
        assert_eq!(committed.agent_session_id.as_deref(), Some("fresh-thread"));
        assert!(committed.pending_recovery_message.is_some());
        let listed = usecase
            .list_sessions(tmp.path().to_string_lossy().as_ref())
            .await
            .unwrap();
        assert_eq!(
            listed
                .iter()
                .find(|summary| summary.id == session.id)
                .unwrap()
                .agent_session_id
                .as_deref(),
            Some("fresh-thread"),
            "the publication snapshot is removed after the Completed commit"
        );

        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "next turn is not blocked".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_ne!(loaded.session.state, SessionState::Error);
        assert_eq!(
            loaded
                .session
                .messages
                .iter()
                .flat_map(|message| message.parts.as_deref().unwrap_or_default())
                .filter(|part| matches!(
                    part,
                    MessagePart::SystemNotification {
                        notification_type: SystemNotificationType::SessionRecovery,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert!(session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .pending_recovery_message
            .is_none());
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryFailed { .. }
        )));
    }

    #[tokio::test]
    async fn unfinished_durable_recovery_is_reconciled_and_blocks_new_turns_after_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let original_store = Arc::new(build_session_store());
        let session = create_session_internal_with_attributes(
            &original_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        original_store
            .begin_backend_session_recovery(
                tmp.path(),
                &session.id,
                "interrupted-recovery",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        drop(original_store);

        let reopened_store = Arc::new(build_session_store());
        let (usecase, _) =
            build_agent_runtime_usecase_with_controller(reopened_store.clone(), tmp.path());
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.state, SessionState::Error);
        assert!(loaded.session.messages.iter().any(|message| {
            message.parts.as_deref().is_some_and(|parts| {
                parts
                    .iter()
                    .any(|part| matches!(part, MessagePart::Error { .. }))
            })
        }));
        let message_count = loaded.session.messages.len();

        let result = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "must remain blocked".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await;
        assert!(result.is_err());
        assert_eq!(
            reopened_store
                .load_full_session_for_restore(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .messages
                .len(),
            message_count
        );
    }

    #[tokio::test]
    async fn public_send_and_workflow_lock_wait_for_recovery_completion() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;

        let send_usecase = Arc::clone(&usecase);
        let send_session = session.id.clone();
        let send_worktree = tmp.path().to_string_lossy().to_string();
        let send = tokio::spawn(async move {
            send_usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(send_session),
                    worktree_path: send_worktree,
                    content: "after recovery".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("codex".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await
        });
        let workflow_usecase = Arc::clone(&usecase);
        let workflow_session = session.id.clone();
        let workflow_lock = tokio::spawn(async move {
            let guard = workflow_usecase
                .acquire_session_control_after_recovery(&workflow_session)
                .await;
            drop(guard);
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!send.is_finished());
        assert!(!workflow_lock.is_finished());

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        send.await.unwrap().unwrap();
        workflow_lock.await.unwrap();
        assert!(session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
            )));
    }

    #[tokio::test]
    async fn public_close_waits_for_recovery_and_closed_state_does_not_reconcile_to_error() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;

        let close_usecase = Arc::clone(&usecase);
        let close_session_id = session.id.clone();
        let close =
            tokio::spawn(async move { close_usecase.close_session(&close_session_id).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!close.is_finished());

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        close.await.unwrap().unwrap();
        crate::usecase::agent_session::session::lifecycle_controller::SessionLifecycleController {
            session_store: &session_store,
            data_dir: tmp.path(),
        }
        .close_session_state(&session.id)
        .unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryFailed { .. }
        )));
        let reopened_store = Arc::new(build_session_store());
        let (reopened_usecase, _) =
            build_agent_runtime_usecase_with_controller(reopened_store, tmp.path());
        let reopened = reopened_usecase
            .get_session(&session.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reopened.session.state, SessionState::Closed);
    }

    #[tokio::test]
    async fn force_close_does_not_wait_for_recovery_establishment() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        assert!(usecase
            .ctx
            .sessions
            .lock()
            .await
            .get(&session.id)
            .is_some_and(|state| state.backend_recovery.is_some()));

        tokio::time::timeout(
            Duration::from_millis(200),
            usecase.force_close_session(&session.id),
        )
        .await
        .expect("force close must not wait for SessionEstablished")
        .unwrap();
    }

    #[tokio::test]
    async fn test_start_turn_locked_workflow_contextのstale_timeoutは_session_specへ渡さない() {
        // Given: a workflow-step session with explicit startup/stale timeout hints.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(Some(12), Some(3), Some(44))),
            },
        )
        .unwrap();

        // When: the workflow starts a turn.
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();

        // Then: startup hints are passed to the backend, but stale timeout remains
        // owned by the Rust stall watchdog and is not passed to backend stream watchdogs.
        assert!(controller.calls().iter().any(|call| {
            call.session_id == session.id
                && call.kind
                    == TestRuntimeCallKind::OpenSession {
                        startup_timeout_ms: Some(12_000),
                        startup_max_retries: Some(3),
                        stale_timeout_ms: None,
                        resume: None,
                        model: "claude-4-sonnet".to_string(),
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                    }
        }));
    }

    #[tokio::test]
    async fn start_turn_locked_rejects_a_durably_paused_workflow_session_until_resume() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, None)),
            },
        )
        .unwrap();
        session_store
            .append_session_event_and_project_state(
                tmp.path(),
                &session.id,
                AgentSessionEvent::QueuePaused { at: 42.0 },
            )
            .unwrap();
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );

        let error = usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "must wait for resume".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("Agent queue is paused"));
        assert_eq!(usecase.turn_phase(&session.id).await, Some(TurnPhase::Idle));
        assert!(usecase
            .get_session(&session.id)
            .await
            .unwrap()
            .unwrap()
            .session
            .messages
            .is_empty());
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnStarted { .. })));
        assert!(!controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));

        usecase.resume_queue(&session.id).await.unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run after resume".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 1).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn interrupt_before_turn_state_commit_prevents_provider_start_and_persists_pause() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, None)),
            },
        )
        .unwrap();
        let gate = Arc::new((Mutex::new((false, false)), Condvar::new()));
        session_store.set_append_event_hook_for_test({
            let gate = Arc::clone(&gate);
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::TurnStarted { .. }) {
                    let (lock, condvar) = &*gate;
                    let mut state = lock.lock().unwrap();
                    state.0 = true;
                    condvar.notify_all();
                    while !state.1 {
                        state = condvar.wait(state).unwrap();
                    }
                }
                Ok(())
            })
        });
        let start = {
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            tokio::spawn(async move {
                usecase
                    .start_turn_locked(
                        &session_id,
                        PermissionMode::Edit,
                        "run".to_string(),
                        None,
                        Vec::new(),
                    )
                    .await
            })
        };
        {
            let (lock, condvar) = &*gate;
            let mut state = lock.lock().unwrap();
            while !state.0 {
                let (next, timeout) = condvar.wait_timeout(state, Duration::from_secs(1)).unwrap();
                assert!(
                    !timeout.timed_out(),
                    "TurnStarted append hook was not reached"
                );
                state = next;
            }
        }
        {
            let sessions = usecase.ctx.sessions.lock().await;
            let state = sessions
                .get(&session.id)
                .expect("turn start intent must be registered before durable append");
            assert_eq!(state.phase, RuntimeSessionPhase::Streaming);
            assert_eq!(state.current_turn_id, state.last_turn_id);
            assert!(state.current_turn_id.is_some());
            assert_eq!(state.generation, 1);
            assert!(
                state.turn_started_at.is_none(),
                "reset_for_turn state must remain uncommitted at the TurnStarted append hook"
            );
            assert!(state.last_progress_at.is_none());
        }

        usecase.interrupt(&session.id).await.unwrap();

        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session.id)
            .unwrap()
            .is_some());
        assert!(session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnInterruptRequested { .. })));
        assert!(!controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));
        {
            let (lock, condvar) = &*gate;
            let mut state = lock.lock().unwrap();
            state.1 = true;
            condvar.notify_all();
        }
        start
            .await
            .unwrap()
            .expect("accepted Stop owns the interrupted start");
        assert!(!controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));

        let restarted =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());
        assert!(
            restarted
                .get_session(&session.id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
    }

    #[tokio::test]
    async fn test_stale_watchdog_無進捗turnをstall_signalに留めruntimeを閉じない() {
        // Given: a workflow-step session whose stale timeout is immediate for the test.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();

        // When: a turn starts and no runtime progress arrives.
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(&event_notifier, 1).await;
        let images = vec![ImageAttachment {
            data: "iVBORw==".to_string(),
            media_type: "image/png".to_string(),
        }];
        let mentions = vec![crate::domain::code::MentionReference {
            file_path: "src/main.rs".to_string(),
            start_line: Some(10),
            end_line: Some(20),
        }];
        let editor_context = AgentEditorContext {
            active_editor_path: Some("src/main.rs".to_string()),
            open_editor_paths: vec!["src/main.rs".to_string(), "README.md".to_string()],
            selection: Some(AgentEditorSelection {
                file_path: "src/main.rs".to_string(),
                start_line: 10,
                end_line: 20,
            }),
        };
        let response = tokio::time::timeout(
            Duration::from_millis(200),
            usecase.send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "next".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: Some(images.clone()),
                mentions: Some(mentions.clone()),
                editor_context: Some(editor_context.clone()),
            }),
        )
        .await
        .expect("send_message must not wait for stale recovery")
        .expect("stalled active turn on a non-steering backend must queue");
        wait_for_workflow_stall_notification_count(&workflow_stall_notifier, 1).await;

        // Then: the watchdog remains non-terminal and the follow-up is durably queued.
        assert!(response.agent_message.is_none());
        assert!(response.queued_turn.is_some());
        assert_eq!(response.pending_queue_count, 1);
        {
            let sessions = usecase.ctx.sessions.lock().await;
            let queued = sessions
                .get(&session.id)
                .and_then(|state| state.pending_queue.front())
                .expect("stalled follow-up must remain in the pending queue");
            assert_eq!(queued.content, "next");
            assert_eq!(queued.images, images);
            assert_eq!(queued.mentions, mentions);
            assert_eq!(queued.editor_context, Some(editor_context));
            assert_eq!(
                queued.existing_human_message_id.as_deref(),
                Some(response.human_message.id.as_str())
            );
        }
        let calls = controller.call_kinds_for(&session.id);
        assert!(calls.contains(&TestRuntimeCallKind::Reconnect));
        assert!(!calls.contains(&TestRuntimeCallKind::Interrupt));
        assert!(!calls.contains(&TestRuntimeCallKind::Close));
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.state, SessionState::Active);
        assert!(loaded
            .session
            .messages
            .iter()
            .any(|message| message.id == response.human_message.id));
        assert!(event_notifier.stall_observations().iter().any(|payload| {
            payload.chat_session_id == session.id
                && payload.turn_phase == TurnPhase::Streaming
                && payload.signal_count >= 1
        }));
        assert!(workflow_stall_notifier
            .notifications()
            .iter()
            .any(|payload| {
                payload.chat_session_id == session.id
                    && payload.turn_phase == "streaming"
                    && payload.signal_count >= 1
            }));

        // And: completion of the stalled turn drains the queued follow-up into a new turn.
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 2).await;
        assert!(usecase.pending_queue(&session.id).await.is_empty());
        assert!(controller.call_kinds_for(&session.id).contains(
            &TestRuntimeCallKind::StartTurnPrompt {
                prompt: "next".to_string(),
            }
        ));
    }

    #[tokio::test]
    async fn test_stall_signal後のsend_messageはactive_turnへsteerしqueueしない() {
        // Given: a stalled workflow-step turn backed by a runtime that supports active-turn steering.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        controller.enable_steering();
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(&event_notifier, 1).await;

        // When: retry/continue text is sent after the stall signal.
        let response = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "continue".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        // Then: the command reaches the active turn through steer and is not trapped behind the queue.
        assert!(response.agent_message.is_none());
        assert!(response.queued_turn.is_none());
        assert_eq!(response.pending_queue_count, 0);
        assert!(usecase.pending_queue(&session.id).await.is_empty());
        assert!(controller.call_kinds_for(&session.id).contains(
            &TestRuntimeCallKind::SteerPrompt {
                prompt: "continue".to_string(),
            }
        ));
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1,
            "steered intervention must not start a second turn"
        );
    }

    #[tokio::test]
    async fn test_stall_signal後のsteer失敗はhuman_messageを保存しない() {
        // Given: a stalled workflow-step turn backed by a runtime that advertises steering.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier);
        controller.enable_steering();
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(&event_notifier, 1).await;
        let before = usecase.get_session(&session.id).await.unwrap().unwrap();
        let before_message_count = before.session.messages.len();
        controller.fail_next_steer();

        // When: retry/continue text fails during active-turn steering.
        let send_error = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "continue".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .expect_err("steer failure must surface to the caller");

        // Then: the failed intervention is neither durable chat history nor a queued turn.
        assert!(
            format!("{send_error:?}").contains("injected test steer failure"),
            "unexpected steer error: {send_error:?}"
        );
        assert!(controller.call_kinds_for(&session.id).contains(
            &TestRuntimeCallKind::SteerPrompt {
                prompt: "continue".to_string(),
            }
        ));
        assert!(usecase.pending_queue(&session.id).await.is_empty());
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.messages.len(), before_message_count);
        assert!(!loaded
            .session
            .messages
            .iter()
            .any(|message| message.content == "continue"));
    }

    #[tokio::test]
    async fn test_stall_signal後のbackend進捗はsend_messageをqueueへ戻す() {
        // Given: an active turn whose previous stall signal made intervention routing available.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        controller.enable_steering();
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, None)),
            },
        )
        .unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Streaming).await;
        mark_stall_observation_active_for_test(&usecase, &session.id).await;

        // When: backend output resumes after the stall observation.
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "still running".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_delta_count(&event_notifier, 1).await;
        wait_for_workflow_stall_cleared_count(&workflow_stall_notifier, 1).await;
        wait_for_stall_clear_count(&event_notifier, 1).await;

        // Then: the signal counter is retained for the turn cap, but delivery routing is no
        // longer considered an active stall intervention.
        {
            let sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get(&session.id).unwrap();
            assert_eq!(state.stall_signal_count, 1);
            assert!(!state.stall_observation_active);
        }
        assert_eq!(
            workflow_stall_notifier
                .cleared_notifications()
                .last()
                .map(|notification| notification.chat_session_id.as_str()),
            Some(session.id.as_str())
        );
        assert_eq!(event_notifier.stall_clears().last(), Some(&session.id));
        let response = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "after progress".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        assert!(response.agent_message.is_none());
        assert!(response.queued_turn.is_some());
        assert_eq!(response.pending_queue_count, 1);
        assert!(!controller.call_kinds_for(&session.id).contains(
            &TestRuntimeCallKind::SteerPrompt {
                prompt: "after progress".to_string(),
            }
        ));
    }

    #[tokio::test]
    async fn test_stall_signal後のkeepaliveはworkflow_stallをclearする() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, None)),
            },
        )
        .unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Streaming).await;
        mark_stall_observation_active_for_test(&usecase, &session.id).await;

        controller
            .emit(&session.id, AgentRuntimeEvent::KeepAlive)
            .unwrap();
        wait_for_workflow_stall_cleared_count(&workflow_stall_notifier, 1).await;
        wait_for_stall_clear_count(&event_notifier, 1).await;

        let sessions = usecase.ctx.sessions.lock().await;
        let state = sessions.get(&session.id).unwrap();
        assert!(!state.stall_observation_active);
        assert_eq!(event_notifier.stall_clears().last(), Some(&session.id));
        assert_eq!(
            workflow_stall_notifier
                .cleared_notifications()
                .last()
                .map(|notification| notification.chat_session_id.as_str()),
            Some(session.id.as_str())
        );
    }

    #[tokio::test]
    async fn test_stall_signal中のbackend進捗はworkflow_observe後にclearされる() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier,
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, None)),
            },
        )
        .unwrap();
        workflow_stall_notifier.set_stall_observed_hook({
            let controller = controller.clone();
            let session_id = session.id.clone();
            Arc::new(move || {
                controller
                    .emit(&session_id, AgentRuntimeEvent::KeepAlive)
                    .unwrap();
            })
        });
        workflow_stall_notifier.set_stall_observed_record_delay(Duration::from_millis(50));

        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Streaming).await;
        let generation = {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session.id).unwrap();
            state.last_progress_at = Some(std::time::Instant::now() - Duration::from_secs(1));
            state.stall_signal_count =
                crate::usecase::agent_session::runtime::stale::MAX_STALL_SIGNALS - 1;
            state.stall_recovery_attempts =
                crate::usecase::agent_session::runtime::stale::MAX_STALL_RECOVERY_ATTEMPTS;
            state.generation
        };

        spawn_stale_watchdog_task(
            &usecase.ctx,
            session.id.clone(),
            generation,
            Duration::from_millis(1),
        );

        wait_for_workflow_stall_notification_count(&workflow_stall_notifier, 1).await;
        wait_for_workflow_stall_cleared_count(&workflow_stall_notifier, 1).await;

        assert_eq!(
            workflow_stall_notifier.event_order(),
            vec!["observed", "cleared"]
        );
        let sessions = usecase.ctx.sessions.lock().await;
        let state = sessions.get(&session.id).unwrap();
        assert!(!state.stall_observation_active);
    }

    #[tokio::test]
    async fn test_stale_watchdogはreconnect未対応backendでも介入点提示に留める() {
        // Given: a workflow-step session backed by a runtime whose reconnect capability is unavailable.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        controller.make_reconnect_unavailable();
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();

        // When: the turn reaches the stale threshold without backend progress.
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(&event_notifier, 1).await;
        wait_for_workflow_stall_notification_count(&workflow_stall_notifier, 1).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Then: Unavailable reconnect falls back to intervention signaling only.
        assert!(event_notifier.stall_observations().iter().any(|payload| {
            payload.chat_session_id == session.id && payload.turn_phase == TurnPhase::Streaming
        }));
        assert!(workflow_stall_notifier
            .notifications()
            .iter()
            .any(|payload| payload.chat_session_id == session.id
                && payload.turn_phase == "streaming"));
        let calls = controller.call_kinds_for(&session.id);
        assert!(!calls.contains(&TestRuntimeCallKind::Reconnect));
        assert!(!calls.contains(&TestRuntimeCallKind::Interrupt));
        assert!(!calls.contains(&TestRuntimeCallKind::Close));
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.state, SessionState::Active);
    }

    #[tokio::test]
    async fn test_stale_watchdogはreconnect_other失敗でも非破壊で上限までretryする() {
        // Given: a workflow-step session whose reconnect attempts fail with a generic backend
        // error rather than Unavailable.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        for _ in 0..crate::usecase::agent_session::runtime::stale::MAX_STALL_RECOVERY_ATTEMPTS {
            controller.fail_next_reconnect();
        }
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();

        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(
            &event_notifier,
            crate::usecase::agent_session::runtime::stale::MAX_STALL_SIGNALS as usize,
        )
        .await;
        wait_for_workflow_stall_notification_count(&workflow_stall_notifier, 1).await;
        wait_for_call_count(
            &controller,
            &session.id,
            TestRuntimeCallKind::Reconnect,
            crate::usecase::agent_session::runtime::stale::MAX_STALL_RECOVERY_ATTEMPTS as usize,
        )
        .await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(event_notifier.stall_observations().iter().any(|payload| {
            payload.chat_session_id == session.id && payload.turn_phase == TurnPhase::Streaming
        }));
        assert!(workflow_stall_notifier
            .notifications()
            .iter()
            .any(|payload| payload.chat_session_id == session.id
                && payload.turn_phase == "streaming"));
        let calls = controller.call_kinds_for(&session.id);
        assert_eq!(
            calls
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::Reconnect))
                .count(),
            crate::usecase::agent_session::runtime::stale::MAX_STALL_RECOVERY_ATTEMPTS as usize
        );
        assert!(!calls.contains(&TestRuntimeCallKind::Interrupt));
        assert!(!calls.contains(&TestRuntimeCallKind::Close));
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.state, SessionState::Active);
    }

    #[tokio::test]
    async fn test_stale_watchdogはツール実行中のturnにstall_signalを出さない() {
        // Given: a workflow-step session with a 1-second stale timeout.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(1))),
            },
        )
        .unwrap();

        // When: a turn starts and a tool call is dispatched whose result has not
        // arrived (a long-running command keeps the backend silent).
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::ToolUse {
                    id: "tool-1".to_string(),
                    tool: "Bash".to_string(),
                    input: crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                        "{}".to_string(),
                    ),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Then: the stale watchdog does not interrupt or recover the healthy tool-in-flight turn.
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Interrupt));
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Reconnect));
    }

    #[tokio::test]
    async fn test_stall_signal後のbackend明示abort完了でturnを確定する() {
        // Given: a workflow-step session whose stale timeout is observed before the backend
        // reports its explicit interrupt result.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(&event_notifier, 1).await;
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );

        // When: the backend reports an abort completion after the stall signal.
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Abort,
                    error: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Idle).await;

        // Then: the explicit backend terminal event, not the stall signal, determines the turn.
        let calls = controller.call_kinds_for(&session.id);
        assert!(!calls.contains(&TestRuntimeCallKind::Interrupt));
        assert!(!calls.contains(&TestRuntimeCallKind::Close));
        let session = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(session.session.state, SessionState::Idle);
    }

    #[tokio::test]
    async fn test_stale_watchdogはstall_signalとreconnectを上限で止める() {
        // Given: a workflow-step session whose stale timeout is immediate for the test.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();

        // When: a turn starts and remains silent past repeated stale observations.
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(
            &event_notifier,
            crate::usecase::agent_session::runtime::stale::MAX_STALL_SIGNALS as usize,
        )
        .await;
        wait_for_call_count(
            &controller,
            &session.id,
            TestRuntimeCallKind::Reconnect,
            crate::usecase::agent_session::runtime::stale::MAX_STALL_RECOVERY_ATTEMPTS as usize,
        )
        .await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Then: signals/reconnects are capped and the session is still live.
        let observations = event_notifier.stall_observations();
        assert_eq!(
            observations.len(),
            crate::usecase::agent_session::runtime::stale::MAX_STALL_SIGNALS as usize
        );
        assert!(observations.last().is_some_and(|payload| {
            payload.cap_reached
                && payload.signal_count
                    == crate::usecase::agent_session::runtime::stale::MAX_STALL_SIGNALS
        }));
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::Reconnect))
                .count(),
            crate::usecase::agent_session::runtime::stale::MAX_STALL_RECOVERY_ATTEMPTS as usize
        );
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Interrupt));
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Close));
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );
    }
}
