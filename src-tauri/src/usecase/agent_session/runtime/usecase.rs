use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::domain::agent_session::entities::{
    AttachmentPayload, InterruptReason as DomainInterruptReason, MessagePart as DomainMessagePart,
    PermissionDecision as DomainPermissionDecision, PermissionRequestStatus, PermissionResponse,
    PermissionResponseDecision, ToolResultUpdate, TurnResult,
    TurnStopReason as DomainTurnStopReason,
};
use crate::domain::agent_session::gateway::{
    AgentBackendError, AgentRuntimeEvent, AgentSessionRuntime, SessionSpec, TurnInput,
};
use crate::domain::agent_session::value_objects::{EditorContext, ModelId, PermissionMode};
use crate::domain::agent_session::{ContextSnapshot, ContextSourceKind};
use crate::usecase::agent_session::backend_registry::{AgentBackendRegistry, BackendListResult};
use crate::usecase::agent_session::context::{
    BranchDiffContextPort, BuiltSystemContext, InstructionSourcePort, SystemContextEditorInput,
};
use crate::usecase::agent_session::event_log::{
    append_part_events, finalize_turn, AgentSessionEvent, InterruptReason as EventInterruptReason,
    PartEventMode, PromptInput, TurnEventLog, TurnStopReason as EventTurnStopReason,
    TurnTokenUsage, WorkflowTurnCompleteInput,
};
use crate::usecase::agent_session::session::{
    add_message_internal, apply_tool_result_update, create_session_with_model_and_plan_mode,
    ChatMessage, ChatSession, ContextCarryState, GetSessionResponse, ImageAttachment,
    InitialSessionPage, MessagePart, MessageRole, ModelInfo, OpenTabRegistry, PermissionRequestMsg,
    QueuedAgentTurn, SessionState, SessionStore, SessionSummary, INITIAL_SESSION_PAGE_LIMIT,
};
use crate::usecase::agent_session::status::{
    AgentStatusCenter, AgentStatusNotifier, SessionStatus, TurnPhase, TurnPhaseRepr,
};
use crate::usecase::agent_session::system_prompt::{
    build_session_system_prompt, persist_session_system_prompt_build,
    SessionSystemPromptBuildRequest,
};
use crate::usecase::workflow::ports::{
    WorkflowTurnCompleteNotification, WorkflowTurnFailureSignal, WorkflowTurnTokenUsage,
};

use super::context_restore::{
    apply_restore_prompt_prefix, context_restore_plan_for_session_before_turn, ContextRestorePlan,
};
use super::event_apply::{
    parts_from_domain, pending_permission_request_msg, token_usage_from_domain,
};
use super::ports::{
    AgentSessionEventNotifier, AgentSessionStateChangedPayload, AgentStreamingDeltaPayload,
    AgentTaskSpawner, WorkflowTurnCompleteNotifier,
};
use super::queue::QueuedTurnInput;
use super::session_state::{
    PendingStreamDelta, RuntimeSessionMap, RuntimeSessionPhase, RuntimeSessionState,
};
use super::stale::{
    effective_stale_timeout, has_in_flight_tool_use, remaining_until_stale,
    stale_timeout_for_session, stale_watchdog_should_continue_waiting,
    startup_max_retries_for_session, startup_timeout_for_session, turn_is_stale, STALE_CLOSE_GRACE,
    STALE_TIMEOUT_MESSAGE,
};
use super::streaming::{
    merge_streaming_append_delta_parts, parts_can_stream_as_append_delta,
    should_persist_streaming_snapshot, streaming_flush_decision, streaming_parts_byte_size,
    StreamingFlushDecision,
};

type SessionRuntimeLock = Arc<Mutex<()>>;
type SessionRuntimeLocks = Arc<Mutex<HashMap<String, SessionRuntimeLock>>>;

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
    StartupTimeout { retry_count: u32, max_retries: u32 },
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
            Self::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for AgentRuntimeError {}

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
            AgentBackendError::Unavailable(message)
            | AgentBackendError::Invalid(message)
            | AgentBackendError::Other(message) => Self::Other(message),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StartSessionOptions {
    pub permission_mode: PermissionMode,
    pub plan_mode: bool,
}

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageResponse {
    pub session: ChatSession,
    pub human_message: ChatMessage,
    pub agent_message: Option<ChatMessage>,
    pub queued_turn: Option<QueuedAgentTurn>,
    pub pending_queue: Vec<QueuedAgentTurn>,
    pub pending_queue_count: usize,
    pub can_change_backend: bool,
    pub sessions: Vec<SessionSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelQueuedTurnResponse {
    pub session_id: String,
    pub canceled_count: usize,
    pub pending_queue: Vec<QueuedAgentTurn>,
    pub pending_queue_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitSessionsResponse {
    pub sessions: Vec<SessionSummary>,
    pub active_session: Option<GetSessionResponse>,
    pub permission_mode: String,
    pub plan_mode: bool,
}

#[derive(Clone)]
struct RuntimeContext {
    session_store: Arc<SessionStore>,
    registry: Arc<AgentBackendRegistry>,
    status_center: Arc<AgentStatusCenter>,
    status_notifier: Arc<dyn AgentStatusNotifier>,
    notifier: Arc<dyn AgentSessionEventNotifier>,
    spawner: Arc<dyn AgentTaskSpawner>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    instruction_source: Arc<dyn InstructionSourcePort>,
    data_dir: Arc<PathBuf>,
    sessions: Arc<Mutex<RuntimeSessionMap>>,
    session_locks: SessionRuntimeLocks,
    workflow_turn_complete_notifier: Arc<RwLock<Option<Arc<dyn WorkflowTurnCompleteNotifier>>>>,
}

pub struct AgentSessionRuntimeUsecase {
    ctx: RuntimeContext,
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
                session_locks: Arc::new(Mutex::new(HashMap::new())),
                workflow_turn_complete_notifier: Arc::new(RwLock::new(None)),
            },
        }
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

    pub fn list_backends(&self) -> BackendListResult {
        self.ctx.registry.list_result()
    }

    pub(crate) fn backend_registry(&self) -> &AgentBackendRegistry {
        self.ctx.registry.as_ref()
    }

    pub async fn send_message(
        &self,
        req: SendAgentMessageRequest,
    ) -> Result<SendMessageResponse, AgentRuntimeError> {
        let mut session_guard = match req.chat_session_id.as_deref() {
            Some(session_id) => Some(self.acquire_session_lock(session_id).await),
            None => None,
        };
        let session = self.resolve_or_create_session(&req).await?;
        if session_guard.is_none() {
            session_guard = Some(self.acquire_session_lock(&session.id).await);
        }
        let images = req.images.unwrap_or_default();
        let mentions = req.mentions.unwrap_or_default();
        let session_id = session.id.clone();
        let backend_id = required_backend_id(&session)?;
        self.recover_queued_turn_if_idle_without_runtime(&session_id)
            .await;
        let human_parts = human_parts(&req.content, &images);
        let human_message = add_message_internal(
            &self.ctx.session_store,
            &self.ctx.data_dir,
            &session_id,
            MessageRole::Human,
            &req.content,
            (!human_parts.is_empty()).then_some(human_parts),
            (!mentions.is_empty()).then_some(mentions.clone()),
        )
        .map_err(AgentRuntimeError::Other)?;

        if self.is_turn_busy(&session_id).await {
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
            return self.send_response(
                &session_id,
                &session.worktree_path,
                human_message,
                None,
                Some(queued_view),
                pending_queue,
            );
        }

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
                editor_context: req.editor_context.map(EditorContext::from),
                system_prompt,
            },
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

    pub async fn start_session(
        &self,
        session_id: &str,
        opts: StartSessionOptions,
    ) -> Result<(), AgentRuntimeError> {
        let _session_guard = self.acquire_session_lock(session_id).await;
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
        self.ensure_runtime(&session, None).await.map(|_| ())
    }

    pub async fn interrupt(&self, session_id: &str) -> Result<(), AgentRuntimeError> {
        let runtime = {
            let sessions = self.ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .and_then(|state| state.runtime.clone())
        };
        if let Some(runtime) = runtime {
            runtime.interrupt().await.map_err(AgentRuntimeError::from)?;
        }
        Ok(())
    }

    pub async fn respond_permission(
        &self,
        session_id: &str,
        response: PermissionResponse,
    ) -> Result<(), AgentRuntimeError> {
        let _session_guard = self.acquire_session_lock(session_id).await;
        {
            let sessions = self.ctx.sessions.lock().await;
            let pending = sessions
                .get(session_id)
                .and_then(|state| state.pending_permission_request.as_ref())
                .ok_or_else(|| {
                    AgentRuntimeError::Other(format!(
                        "No pending permission request for session {session_id}"
                    ))
                })?;
            if pending.id != response.request_id {
                return Err(AgentRuntimeError::Other(format!(
                    "Permission request id mismatch: pending={}, response={}",
                    pending.id, response.request_id
                )));
            }
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
            .respond_permission(response.clone())
            .await
            .map_err(AgentRuntimeError::from)?;
        let (patched, did_resume_streaming, permission_wait_measurement) = {
            let mut sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return Ok(());
            };
            let patched = patch_permission_response_in_state(state, &response);
            let did_resume_streaming = state.phase == RuntimeSessionPhase::WaitingPermission
                && state
                    .pending_permission_request
                    .as_ref()
                    .is_some_and(|pending| pending.id == response.request_id);
            if did_resume_streaming {
                state.phase = RuntimeSessionPhase::Streaming;
                state.pending_permission_request = None;
                state.last_progress_at = Some(std::time::Instant::now());
            }
            let permission_wait_measurement = did_resume_streaming
                .then(|| {
                    let started_at = state.permission_wait_started_at.take()?;
                    let dims = session_telemetry_dimensions(
                        &self.ctx.session_store,
                        &self.ctx.data_dir,
                        session_id,
                    )?;
                    Some((started_at.elapsed(), dims))
                })
                .flatten();
            (patched, did_resume_streaming, permission_wait_measurement)
        };
        if let Some((elapsed, dims)) = permission_wait_measurement {
            crate::other::telemetry::record_agent_turn_duration(
                crate::other::telemetry::AgentTurn::PermissionWait,
                &dims,
                elapsed,
            );
        }
        if let Some((message_id, seq, parts, turn_id)) = patched {
            if let Err(error) = self.ctx.session_store.persist_message_parts(
                &self.ctx.data_dir,
                session_id,
                &message_id,
                &parts,
                seq,
                None,
            ) {
                log::warn!("failed to persist permission response patch for {session_id}: {error}");
            }
            append_permission_resolved_event(
                &self.ctx.session_store,
                &self.ctx.data_dir,
                session_id,
                turn_id,
                &response,
            );
            emit_streaming_delta_or_retry(
                &self.ctx,
                session_id,
                PendingStreamDelta {
                    message_id,
                    seq,
                    snapshot: true,
                    parts,
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
                    pending_permission_request: None,
                    exit_code: None,
                    completed_at: None,
                    interrupted: false,
                    session_state: Some(SessionState::Active),
                },
            );
        }
        Ok(())
    }

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
        self.ctx
            .session_store
            .update_permission_mode(&self.ctx.data_dir, session_id, mode.as_str())
            .map_err(AgentRuntimeError::Other)?;
        self.ctx
            .notifier
            .permission_mode_changed(session_id, mode.as_str());
        let plan_mode = self
            .ctx
            .session_store
            .get_session_meta(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .map(|meta| meta.plan_mode)
            .unwrap_or(false);
        if let Some(runtime) = self.live_runtime(session_id).await {
            if let Err(error) = runtime.set_permission_mode(mode, plan_mode).await {
                log::warn!("runtime permission mode sync failed for {session_id}: {error}");
            }
        }
        Ok(())
    }

    pub async fn set_plan_mode(
        &self,
        session_id: &str,
        plan_mode: bool,
    ) -> Result<(), AgentRuntimeError> {
        self.ctx
            .session_store
            .update_plan_mode(&self.ctx.data_dir, session_id, plan_mode)
            .map_err(AgentRuntimeError::Other)?;
        let mode = self
            .ctx
            .session_store
            .get_session_meta(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .map(|meta| PermissionMode::parse(&meta.permission_mode))
            .transpose()
            .map_err(|error| AgentRuntimeError::Other(error.to_string()))?
            .unwrap_or(PermissionMode::Edit);
        if let Some(runtime) = self.live_runtime(session_id).await {
            if let Err(error) = runtime.set_permission_mode(mode, plan_mode).await {
                log::warn!("runtime plan mode sync failed for {session_id}: {error}");
            }
        }
        Ok(())
    }

    pub async fn set_model(
        &self,
        session_id: &str,
        entry_id: &str,
    ) -> Result<(), AgentRuntimeError> {
        let entry = self
            .ctx
            .registry
            .resolve_model_entry(entry_id)
            .map_err(AgentRuntimeError::Other)?;
        self.ctx
            .session_store
            .update_backend_selection(
                &self.ctx.data_dir,
                session_id,
                entry.backend.clone(),
                Some(entry.model_id.clone()),
            )
            .map_err(AgentRuntimeError::Other)?;
        if let Ok(available_models) = self.ctx.registry.available_models(&entry.backend) {
            self.ctx
                .notifier
                .models_updated(session_id, available_models, entry.model_id.clone());
        }
        if let Some(runtime) = self.live_runtime(session_id).await {
            let model = ModelId::parse(&entry.model_id).map_err(AgentRuntimeError::Other)?;
            if let Err(error) = runtime.set_model(&model).await {
                log::warn!("runtime model sync failed for {session_id}: {error}");
            }
        }
        Ok(())
    }

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
        self.ctx
            .session_store
            .update_backend_selection(
                &self.ctx.data_dir,
                session_id,
                backend_id.to_string(),
                Some(selected_model),
            )
            .map_err(AgentRuntimeError::Other)?;
        self.close_session(session_id).await?;
        self.get_session(session_id)
            .await?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))
    }

    pub async fn set_session_title(
        &self,
        session_id: &str,
        title: &str,
    ) -> Result<(), AgentRuntimeError> {
        if let Some(runtime) = self.live_runtime(session_id).await {
            if let Err(error) = runtime.set_session_title(title).await {
                log::warn!("runtime session title sync failed for {session_id}: {error}");
            }
        }
        Ok(())
    }

    pub async fn close_session(&self, session_id: &str) -> Result<(), AgentRuntimeError> {
        let runtime = {
            let mut sessions = self.ctx.sessions.lock().await;
            sessions.remove(session_id).and_then(|state| state.runtime)
        };
        if let Some(runtime) = runtime {
            runtime.close().await;
        }
        Ok(())
    }

    pub async fn close_all(&self) {
        let runtimes = {
            let mut sessions = self.ctx.sessions.lock().await;
            sessions
                .drain()
                .filter_map(|(_, state)| state.runtime)
                .collect::<Vec<_>>()
        };
        for runtime in runtimes {
            runtime.close().await;
        }
    }

    pub async fn find_permission_request(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Result<Option<PermissionRequestMsg>, AgentRuntimeError> {
        {
            let sessions = self.ctx.sessions.lock().await;
            if let Some(request) = sessions
                .get(session_id)
                .and_then(|state| permission_request_from_parts(&state.streaming_parts, request_id))
            {
                return Ok(Some(request));
            }
        }

        let mut cursor = None;
        loop {
            let Some(page) = self
                .ctx
                .session_store
                .get_session_page(
                    &self.ctx.data_dir,
                    session_id,
                    cursor.clone(),
                    INITIAL_SESSION_PAGE_LIMIT,
                )
                .map_err(AgentRuntimeError::Other)?
            else {
                return Ok(None);
            };
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
                return Ok(None);
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(None);
            }
        }
    }

    pub async fn cancel_queued_turn(
        &self,
        session_id: &str,
        queued_turn_id: Option<&str>,
    ) -> Result<CancelQueuedTurnResponse, AgentRuntimeError> {
        let mut sessions = self.ctx.sessions.lock().await;
        let state = sessions.get_mut(session_id).ok_or_else(|| {
            AgentRuntimeError::Other(format!("No active agent runtime for session {session_id}"))
        })?;
        let before = state.pending_queue.len();
        match queued_turn_id {
            Some(id) => state.pending_queue.retain(|turn| turn.id != id),
            None => state.pending_queue.clear(),
        }
        let canceled_count = before.saturating_sub(state.pending_queue.len());
        if queued_turn_id.is_some() && canceled_count == 0 {
            return Err(AgentRuntimeError::Other(
                "Queued turn not found".to_string(),
            ));
        }
        let pending_queue = pending_queue_view(state);
        Ok(CancelQueuedTurnResponse {
            session_id: session_id.to_string(),
            canceled_count,
            pending_queue_count: pending_queue.len(),
            pending_queue,
        })
    }

    pub async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<GetSessionResponse>, AgentRuntimeError> {
        let Some((session, page)) = self
            .ctx
            .session_store
            .get_session_with_latest_page(
                &self.ctx.data_dir,
                session_id,
                INITIAL_SESSION_PAGE_LIMIT,
            )
            .map_err(AgentRuntimeError::Other)?
        else {
            return Ok(None);
        };
        let (turn_phase, pending_queue, latest_token_usage) = {
            let sessions = self.ctx.sessions.lock().await;
            match sessions.get(session_id) {
                Some(state) => (
                    TurnPhase::from(state.phase),
                    pending_queue_view(state),
                    state.latest_token_usage,
                ),
                None => (TurnPhase::Idle, Vec::new(), None),
            }
        };
        let available_models = self.available_models_for_session(&session)?;
        let total_count = page.total_count;
        let can_change_backend = session.messages.is_empty()
            && session.agent_session_id.is_none()
            && turn_phase == TurnPhase::Idle;
        let response = GetSessionResponse {
            session,
            turn_phase,
            available_models,
            can_change_backend,
            pending_queue_count: pending_queue.len(),
            pending_queue,
            initial_page: Some(InitialSessionPage {
                next_cursor: page.next_cursor,
                has_more: page.has_more,
                total_count,
            }),
            latest_token_usage: latest_token_usage.or(page.latest_token_usage),
        };
        Ok(Some(response))
    }

    pub async fn init_sessions(
        &self,
        worktree_path: &str,
        open_tabs: &OpenTabRegistry,
    ) -> Result<InitSessionsResponse, AgentRuntimeError> {
        let sessions = self
            .ctx
            .session_store
            .list_sessions(&self.ctx.data_dir, worktree_path)
            .map_err(AgentRuntimeError::Other)?;
        for session in &sessions {
            if session.is_workflow_step_session() && session.state != SessionState::Closed {
                open_tabs.add(&session.id);
            }
        }
        let active_candidate = sessions
            .iter()
            .find(|session| !session.is_workflow_step_session())
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

    pub async fn is_runtime_busy(&self, session_id: &str) -> bool {
        self.is_turn_busy(session_id).await
    }

    pub async fn has_live_runtime(&self, session_id: &str) -> bool {
        self.live_runtime(session_id).await.is_some()
    }

    pub async fn active_session_ids(&self, candidates: &[String]) -> HashSet<String> {
        let sessions = self.ctx.sessions.lock().await;
        candidates
            .iter()
            .filter(|id| sessions.contains_key(*id))
            .cloned()
            .collect()
    }

    pub async fn acquire_session_lock(&self, session_id: &str) -> SessionRuntimeLockGuard {
        acquire_session_runtime_lock(&self.ctx.session_locks, session_id).await
    }

    pub async fn start_turn_locked(
        &self,
        session_id: &str,
        permission_mode: PermissionMode,
        prompt: String,
        base_system_prompt: Option<String>,
        workflow_instructions: Vec<String>,
    ) -> Result<(), AgentRuntimeError> {
        let mut session = self
            .ctx
            .session_store
            .get_session_shell(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
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
            },
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

    #[cfg(test)]
    pub(crate) async fn drain_next_queued_turn_for_test(&self, session_id: &str) {
        start_next_queued_turn(&self.ctx, session_id).await;
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

    async fn resolve_or_create_session(
        &self,
        req: &SendAgentMessageRequest,
    ) -> Result<ChatSession, AgentRuntimeError> {
        if let Some(session_id) = req.chat_session_id.as_deref() {
            let mut session = self
                .ctx
                .session_store
                .get_session_shell(&self.ctx.data_dir, session_id)
                .map_err(AgentRuntimeError::Other)?
                .ok_or_else(|| {
                    AgentRuntimeError::Other(format!("Session not found: {session_id}"))
                })?;
            if session.permission_mode != req.permission_mode.as_str() {
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
            if session.plan_mode != req.plan_mode {
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

    async fn start_turn_for_session(
        &self,
        session: &ChatSession,
        human_message: &ChatMessage,
        agent_message_id: String,
        mut payload: TurnStartPayload,
    ) -> Result<(), AgentRuntimeError> {
        let restore_plan = if self.live_runtime(&session.id).await.is_none() {
            let persisted = self
                .ctx
                .session_store
                .load_full_session_for_restore(&self.ctx.data_dir, &session.id)
                .map_err(AgentRuntimeError::Other)?;
            context_restore_plan_for_session_before_turn(persisted.as_ref(), &agent_message_id)
        } else {
            ContextRestorePlan::NoContext
        };
        if restore_plan.carry_state() == Some(ContextCarryState::Reinjected) {
            match self.ctx.session_store.update_context_carry_if_changed(
                &self.ctx.data_dir,
                &session.id,
                Some(ContextCarryState::Reinjected),
            ) {
                Ok(Some(meta)) => self.ctx.notifier.context_carry_updated(
                    &session.id,
                    meta.agent_session_id,
                    meta.context_carry,
                    meta.updated_at,
                ),
                Ok(None) => {}
                Err(error) => {
                    log::warn!(
                        "failed to persist reinjected context carry for {}: {error}",
                        session.id
                    );
                }
            }
        }
        payload.prompt = apply_restore_prompt_prefix(payload.prompt, &restore_plan);
        let turn_id = next_turn_id(&self.ctx.session_store, &self.ctx.data_dir, &session.id)
            .map_err(AgentRuntimeError::Other)?;
        let backend_id = required_backend_id(session)?;
        let prompt_message = self
            .ctx
            .session_store
            .load_previous_human_message_before_agent(
                &self.ctx.data_dir,
                &session.id,
                &agent_message_id,
            )
            .map_err(AgentRuntimeError::Other)?
            .unwrap_or_else(|| human_message.clone());
        self.ctx
            .session_store
            .append_session_event_and_project_state(
                &self.ctx.data_dir,
                &session.id,
                AgentSessionEvent::TurnStarted {
                    turn_id,
                    message_id: prompt_message.id.clone(),
                    assistant_message_id: Some(agent_message_id.clone()),
                    prompt: PromptInput::from_human_message(&prompt_message),
                    at: prompt_message.timestamp,
                },
            )
            .map_err(AgentRuntimeError::Other)?;
        let generation = {
            let mut sessions = self.ctx.sessions.lock().await;
            let state = sessions
                .entry(session.id.clone())
                .or_insert_with(|| RuntimeSessionState::new(backend_id.clone()));
            state.reset_for_turn(turn_id, agent_message_id.clone());
            let mut current_turn_input = QueuedTurnInput::new(
                payload.prompt.clone(),
                payload.permission_mode,
                payload.plan_mode,
                payload.permission_profile_id.clone(),
                payload.images.clone(),
                session.worktree_path.clone(),
                payload.mentions.clone(),
                None,
            );
            current_turn_input.existing_human_message_id = Some(human_message.id.clone());
            current_turn_input.existing_agent_message_id = state.streaming_message_id.clone();
            state.current_turn_input = Some(current_turn_input);
            state.generation
        };
        let runtime = match self
            .ensure_runtime(session, payload.system_prompt.clone())
            .await
        {
            Ok(runtime) => runtime,
            Err(error) => {
                {
                    let mut sessions = self.ctx.sessions.lock().await;
                    if let Some(state) = sessions.get_mut(&session.id) {
                        if state.generation == generation {
                            state.rollback_started_turn();
                        }
                    }
                }
                let message = error.to_string();
                if let Err(persist_error) = self
                    .ctx
                    .session_store
                    .append_session_event_and_project_state(
                        &self.ctx.data_dir,
                        &session.id,
                        AgentSessionEvent::TurnInterrupted {
                            turn_id,
                            reason: EventInterruptReason::Crash,
                            exit_code: 1,
                            error: Some(message.clone()),
                        },
                    )
                {
                    log::warn!(
                        "failed to record runtime open failure for {}: {}",
                        session.id,
                        persist_error
                    );
                }
                emit_session_state_change(
                    &self.ctx.session_store,
                    &self.ctx.notifier,
                    &self.ctx.status_center,
                    &self.ctx.status_notifier,
                    &self.ctx.data_dir,
                    &session.id,
                    StateChange {
                        turn_phase: TurnPhase::Idle,
                        pending_permission_request: None,
                        exit_code: Some(1),
                        completed_at: Some(crate::usecase::agent_session::session::now_timestamp()),
                        interrupted: true,
                        session_state: Some(SessionState::Error),
                    },
                );
                return Err(error);
            }
        };
        let start_result = runtime
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
                editor_context: payload.editor_context,
            })
            .await;
        match start_result {
            Ok(()) => {
                self.spawn_stale_watchdog(
                    session.id.clone(),
                    generation,
                    stale_timeout_for_session(session),
                );
                emit_session_state_change(
                    &self.ctx.session_store,
                    &self.ctx.notifier,
                    &self.ctx.status_center,
                    &self.ctx.status_notifier,
                    &self.ctx.data_dir,
                    &session.id,
                    StateChange {
                        turn_phase: TurnPhase::Streaming,
                        pending_permission_request: None,
                        exit_code: None,
                        completed_at: None,
                        interrupted: false,
                        session_state: Some(SessionState::Active),
                    },
                );
                Ok(())
            }
            Err(error) => {
                {
                    let mut sessions = self.ctx.sessions.lock().await;
                    if let Some(state) = sessions.get_mut(&session.id) {
                        if state.generation == generation {
                            state.rollback_started_turn();
                        }
                    }
                }
                let message = error.to_string();
                if let Err(persist_error) = self
                    .ctx
                    .session_store
                    .append_session_event_and_project_state(
                        &self.ctx.data_dir,
                        &session.id,
                        AgentSessionEvent::TurnInterrupted {
                            turn_id,
                            reason: EventInterruptReason::Crash,
                            exit_code: 1,
                            error: Some(message.clone()),
                        },
                    )
                {
                    log::warn!(
                        "failed to record start_turn failure for {}: {}",
                        session.id,
                        persist_error
                    );
                }
                emit_session_state_change(
                    &self.ctx.session_store,
                    &self.ctx.notifier,
                    &self.ctx.status_center,
                    &self.ctx.status_notifier,
                    &self.ctx.data_dir,
                    &session.id,
                    StateChange {
                        turn_phase: TurnPhase::Idle,
                        pending_permission_request: None,
                        exit_code: Some(1),
                        completed_at: Some(crate::usecase::agent_session::session::now_timestamp()),
                        interrupted: true,
                        session_state: Some(SessionState::Error),
                    },
                );
                Err(AgentRuntimeError::from(error))
            }
        }
    }

    async fn ensure_runtime(
        &self,
        session: &ChatSession,
        system_prompt: Option<String>,
    ) -> Result<Arc<dyn AgentSessionRuntime>, AgentRuntimeError> {
        if let Some(runtime) = self.live_runtime(&session.id).await {
            return Ok(runtime);
        }
        open_runtime_for_session(&self.ctx, session, system_prompt).await
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

    async fn is_turn_busy(&self, session_id: &str) -> bool {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .map(|state| {
                state.phase != RuntimeSessionPhase::Idle || !state.pending_queue.is_empty()
            })
            .unwrap_or(false)
    }

    async fn pending_queue(&self, session_id: &str) -> Vec<QueuedAgentTurn> {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .map(pending_queue_view)
            .unwrap_or_default()
    }

    fn send_response(
        &self,
        session_id: &str,
        worktree_path: &str,
        human_message: ChatMessage,
        agent_message: Option<ChatMessage>,
        queued_turn: Option<QueuedAgentTurn>,
        pending_queue: Vec<QueuedAgentTurn>,
    ) -> Result<SendMessageResponse, AgentRuntimeError> {
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
        Ok(SendMessageResponse {
            session,
            human_message,
            agent_message,
            queued_turn,
            pending_queue_count: pending_queue.len(),
            pending_queue,
            can_change_backend: false,
            sessions,
        })
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
            let delay = {
                let _session_guard =
                    acquire_session_runtime_lock(&ctx.session_locks, &session_id).await;
                let sessions = ctx.sessions.lock().await;
                let Some(state) = sessions.get(&session_id) else {
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
                } else {
                    std::time::Duration::ZERO
                }
            };
            if delay.is_zero() {
                break;
            }
            tokio::time::sleep(delay).await;
        }

        let (workflow_notification, runtime) = {
            let _session_guard =
                acquire_session_runtime_lock(&ctx.session_locks, &session_id).await;
            let runtime = {
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
                    return;
                }
                state.runtime.take()
            };

            let workflow_notification = complete_turn(
                &ctx,
                &session_id,
                Some(generation),
                TurnResult::Interrupted {
                    reason: DomainInterruptReason::Timeout,
                    error: Some(STALE_TIMEOUT_MESSAGE.to_string()),
                },
            )
            .await;
            (workflow_notification, runtime)
        };

        if let Some(notification) = workflow_notification {
            dispatch_workflow_turn_complete_notification(
                &ctx.workflow_turn_complete_notifier,
                notification,
            )
            .await;
        }
        if let Some(runtime) = runtime {
            if let Err(error) = runtime.interrupt().await {
                log::warn!("failed to interrupt stale runtime for {session_id}: {error}");
            }
            tokio::time::sleep(STALE_CLOSE_GRACE).await;
            runtime.close().await;
        }
    }));
}

async fn open_runtime_for_session(
    ctx: &RuntimeContext,
    session: &ChatSession,
    system_prompt: Option<String>,
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
            stale_timeout: Some(stale_timeout_for_session(session)),
        })
        .await
        .map_err(AgentRuntimeError::from)?;
    let events = runtime.take_events();
    let runtime: Arc<dyn AgentSessionRuntime> = Arc::from(runtime);
    let runtime_epoch = {
        let mut sessions = ctx.sessions.lock().await;
        let state = sessions
            .entry(session.id.clone())
            .or_insert_with(|| RuntimeSessionState::new(backend_id.clone()));
        state.backend_id = backend_id;
        state.runtime = Some(Arc::clone(&runtime));
        state.bump_runtime_epoch()
    };
    spawn_event_pump_task(ctx, session.id.clone(), runtime_epoch, events);
    Ok(runtime)
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
            let actions = {
                let _session_guard =
                    acquire_session_runtime_lock(&ctx.session_locks, &session_id).await;
                apply_runtime_event(&ctx, &session_id, runtime_epoch, event).await
            };
            run_runtime_event_post_actions(&ctx, &session_id, actions).await;
        }
    }));
}

async fn handle_resume_mismatch(ctx: &RuntimeContext, session_id: &str) -> RuntimeEventPostActions {
    let mut actions = RuntimeEventPostActions::default();
    let runtime = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return actions;
        };
        let runtime = state.runtime.take();
        if let Some(mut current_turn) = state.current_turn_input.take() {
            current_turn.id = uuid::Uuid::new_v4().to_string();
            state.pending_queue.push_front(current_turn);
        }
        state.rollback_started_turn();
        runtime
    };
    actions.close_runtime(runtime);
    match ctx
        .session_store
        .update_resume_metadata_if_changed(&ctx.data_dir, session_id, None, None)
    {
        Ok(Some(meta)) => ctx.notifier.context_carry_updated(
            session_id,
            meta.agent_session_id,
            meta.context_carry,
            meta.updated_at,
        ),
        Ok(None) => {}
        Err(error) => {
            log::warn!("failed to clear resume metadata after mismatch for {session_id}: {error}");
        }
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
            pending_permission_request: None,
            exit_code: None,
            completed_at: None,
            interrupted: false,
            session_state: Some(SessionState::Active),
        },
    );
    actions.drain();
    actions
}

pub struct SessionRuntimeLockGuard {
    session_id: String,
    guard: Option<OwnedMutexGuard<()>>,
    locks: SessionRuntimeLocks,
}

async fn acquire_session_runtime_lock(
    session_locks: &SessionRuntimeLocks,
    session_id: &str,
) -> SessionRuntimeLockGuard {
    let lock = {
        let mut locks = session_locks.lock().await;
        locks
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let guard = lock.lock_owned().await;
    SessionRuntimeLockGuard {
        session_id: session_id.to_string(),
        guard: Some(guard),
        locks: Arc::clone(session_locks),
    }
}

impl Drop for SessionRuntimeLockGuard {
    fn drop(&mut self) {
        self.guard.take();
        let session_id = self.session_id.clone();
        let locks = Arc::clone(&self.locks);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    let mut locks = locks.lock().await;
                    if locks
                        .get(&session_id)
                        .is_some_and(|lock| Arc::strong_count(lock) == 1)
                    {
                        locks.remove(&session_id);
                    }
                });
            }
            Err(_) => {
                log::warn!(
                    "skip prune_session_runtime_lock: no tokio runtime (session={session_id})"
                );
            }
        }
    }
}

struct TurnStartPayload {
    prompt: String,
    images: Vec<ImageAttachment>,
    mentions: Vec<crate::domain::code::MentionReference>,
    permission_mode: PermissionMode,
    plan_mode: bool,
    permission_profile_id: Option<String>,
    editor_context: Option<EditorContext>,
    system_prompt: Option<String>,
}

#[derive(Default)]
struct RuntimeEventPostActions {
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

    fn close_runtime(&mut self, runtime: Option<Arc<dyn AgentSessionRuntime>>) {
        if let Some(runtime) = runtime {
            self.runtime_shutdowns.push(RuntimeShutdown::Close(runtime));
        }
    }
}

async fn run_runtime_event_post_actions(
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
        let _session_guard = acquire_session_runtime_lock(&ctx.session_locks, session_id).await;
        start_next_queued_turn(ctx, session_id).await;
    }
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

async fn record_first_backend_event_if_needed(ctx: &RuntimeContext, session_id: &str) {
    let measurement = {
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
        let Some(dims) =
            session_telemetry_dimensions(&ctx.session_store, &ctx.data_dir, session_id)
        else {
            return;
        };
        state.first_backend_event_recorded = true;
        Some((started_at.elapsed(), dims))
    };
    if let Some((elapsed, dims)) = measurement {
        crate::other::telemetry::record_agent_turn_duration(
            crate::other::telemetry::AgentTurn::FirstBackendEvent,
            &dims,
            elapsed,
        );
    }
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

async fn apply_runtime_event(
    ctx: &RuntimeContext,
    session_id: &str,
    runtime_epoch: u64,
    event: AgentRuntimeEvent,
) -> RuntimeEventPostActions {
    let is_current_runtime = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .is_some_and(|state| state.runtime_epoch == runtime_epoch)
    };
    if !is_current_runtime {
        log::debug!(
            "dropping {} from stale runtime epoch {runtime_epoch} for {session_id}",
            runtime_event_kind(&event)
        );
        return RuntimeEventPostActions::default();
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
                return handle_resume_mismatch(ctx, session_id).await;
            }
            let (agent_session_id, context_carry) = match resume {
                crate::domain::agent_session::gateway::ResumeOutcome::Resumed => (
                    Some(backend_session_id.clone()),
                    Some(ContextCarryState::Resumed),
                ),
                crate::domain::agent_session::gateway::ResumeOutcome::NotRequested => {
                    (Some(backend_session_id.clone()), None)
                }
                crate::domain::agent_session::gateway::ResumeOutcome::Mismatch { .. } => {
                    unreachable!("resume mismatch is handled before metadata update")
                }
            };
            match ctx.session_store.update_resume_metadata_if_changed(
                &ctx.data_dir,
                session_id,
                agent_session_id,
                context_carry.clone(),
            ) {
                Ok(Some(meta)) => ctx.notifier.context_carry_updated(
                    session_id,
                    meta.agent_session_id,
                    meta.context_carry,
                    meta.updated_at,
                ),
                Ok(None) => {}
                Err(error) => {
                    log::warn!("failed to persist backend session id for {session_id}: {error}");
                }
            }
        }
        AgentRuntimeEvent::BackendSessionCleared => {
            match ctx.session_store.update_resume_metadata_if_changed(
                &ctx.data_dir,
                session_id,
                None,
                Some(ContextCarryState::Failed),
            ) {
                Ok(Some(meta)) => ctx.notifier.context_carry_updated(
                    session_id,
                    meta.agent_session_id,
                    meta.context_carry,
                    meta.updated_at,
                ),
                Ok(None) => {}
                Err(error) => {
                    log::warn!("failed to clear backend session id for {session_id}: {error}");
                }
            }
        }
        AgentRuntimeEvent::PartsMerged(parts) => {
            apply_parts(ctx, session_id, parts, StreamingApplyMode::Coalesced).await;
        }
        AgentRuntimeEvent::PermissionRequested(request) => {
            let pending = pending_permission_request_msg(&request);
            apply_parts(
                ctx,
                session_id,
                vec![DomainMessagePart::Permission { request }],
                StreamingApplyMode::Immediate,
            )
            .await;
            if let Some(pending) = pending {
                let mut sessions = ctx.sessions.lock().await;
                if let Some(state) = sessions.get_mut(session_id) {
                    state.phase = RuntimeSessionPhase::WaitingPermission;
                    state.pending_permission_request = Some(pending.clone());
                    state.permission_wait_started_at = Some(std::time::Instant::now());
                }
                emit_session_state_change(
                    &ctx.session_store,
                    &ctx.notifier,
                    &ctx.status_center,
                    &ctx.status_notifier,
                    &ctx.data_dir,
                    session_id,
                    StateChange {
                        turn_phase: TurnPhase::WaitingPermission,
                        pending_permission_request: Some(pending),
                        exit_code: None,
                        completed_at: None,
                        interrupted: false,
                        session_state: Some(SessionState::Active),
                    },
                );
            }
        }
        AgentRuntimeEvent::PermissionModeChanged(mode) => {
            if let Some(saved_mode) = resync_permission_mode(
                &ctx.session_store,
                &ctx.sessions,
                &ctx.data_dir,
                session_id,
                mode,
            )
            .await
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
            let mut sessions = ctx.sessions.lock().await;
            if let Some(state) = sessions.get_mut(session_id) {
                if state.phase != RuntimeSessionPhase::Idle {
                    state.last_progress_at = Some(std::time::Instant::now());
                }
            }
        }
        AgentRuntimeEvent::TurnCompleted(result) => {
            let workflow_notification = complete_turn(ctx, session_id, None, result).await;
            let mut actions = RuntimeEventPostActions::workflow(workflow_notification);
            actions.drain();
            return actions;
        }
        AgentRuntimeEvent::Fatal { message } => {
            log::warn!("agent runtime fatal for {session_id}: {message}");
            let should_complete_crash = {
                let sessions = ctx.sessions.lock().await;
                sessions
                    .get(session_id)
                    .map(|state| state.phase != RuntimeSessionPhase::Idle)
                    .unwrap_or(false)
            };
            let mut actions = RuntimeEventPostActions::default();
            if should_complete_crash {
                actions.workflow_notification = complete_turn(
                    ctx,
                    session_id,
                    None,
                    TurnResult::Interrupted {
                        reason: DomainInterruptReason::Crash,
                        error: Some(message.clone()),
                    },
                )
                .await;
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
                }
            }
            if !should_complete_crash {
                if let Err(error) = ctx.session_store.set_session_state(
                    &ctx.data_dir,
                    session_id,
                    SessionState::Error,
                ) {
                    log::warn!("failed to persist fatal session state for {session_id}: {error}");
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
                        pending_permission_request: None,
                        exit_code: Some(1),
                        completed_at: Some(crate::usecase::agent_session::session::now_timestamp()),
                        interrupted: true,
                        session_state: Some(SessionState::Error),
                    },
                );
            }
            actions.drain();
            return actions;
        }
    };
    RuntimeEventPostActions::default()
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
) {
    let domain_parts = parts;
    let delta_parts = parts_from_domain(domain_parts.clone());
    if delta_parts.is_empty() {
        return;
    }
    let post_turn_message_id = {
        let sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get(session_id) else {
            return;
        };
        if state.phase == RuntimeSessionPhase::Idle {
            Some(state.last_agent_message_id.clone())
        } else {
            None
        }
    };
    if let Some(message_id) = post_turn_message_id {
        if let Some(message_id) = message_id {
            apply_post_turn_parts(
                &ctx.session_store,
                &ctx.data_dir,
                session_id,
                &message_id,
                delta_parts,
            );
        }
        return;
    }
    let (turn_id, message_id, emit_now, schedule_delay) = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        let message_id = state
            .streaming_message_id
            .clone()
            .or_else(|| state.last_agent_message_id.clone());
        let Some(message_id) = message_id else {
            return;
        };
        for part in &domain_parts {
            crate::domain::agent_session::entities::merge_part(
                &mut state.domain_streaming_parts,
                part.clone(),
            );
        }
        state.last_progress_at = Some(std::time::Instant::now());
        let can_append_delta = parts_can_stream_as_append_delta(&delta_parts);
        let requires_snapshot = mode == StreamingApplyMode::Immediate
            || state.streaming_delta_seq == 0
            || state.streaming_parts.is_empty()
            || !can_append_delta;
        if requires_snapshot {
            state.streaming_parts = parts_from_domain(state.domain_streaming_parts.clone());
        } else {
            merge_streaming_append_delta_parts(&mut state.streaming_parts, &delta_parts);
        }
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
        let turn_id = state.current_turn_id.or(state.last_turn_id);
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
        (turn_id, message_id, emit_now, schedule_delay)
    };
    if let Some(turn_id) = turn_id {
        append_durable_part_events(
            &ctx.session_store,
            &ctx.data_dir,
            session_id,
            turn_id,
            &message_id,
            &delta_parts,
        );
    }
    if emit_now {
        flush_streaming_update(ctx, session_id, false).await;
    } else if let Some(delay) = schedule_delay {
        spawn_delayed_stream_flush(ctx, session_id.to_string(), delay);
    }
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
        flush_streaming_update(&ctx, &session_id, false).await;
    }));
}

async fn flush_streaming_update(ctx: &RuntimeContext, session_id: &str, force_persist: bool) {
    let now = std::time::Instant::now();
    let (payload, persist_snapshot, emit_suppressed) = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        state.stream_flush_scheduled = false;
        let retry = state.retry_stream_delta.take();
        let payload = if let Some(retry) = retry {
            retry
        } else {
            let message_id = state
                .streaming_message_id
                .clone()
                .or_else(|| state.last_agent_message_id.clone());
            let Some(message_id) = message_id else {
                return;
            };
            if !state.pending_stream_snapshot && state.pending_stream_parts.is_empty() {
                return;
            }
            let snapshot = state.pending_stream_snapshot || state.streaming_delta_seq == 0;
            let parts = if snapshot {
                state.streaming_parts.clone()
            } else {
                std::mem::take(&mut state.pending_stream_parts)
            };
            state.pending_stream_bytes = 0;
            state.pending_stream_snapshot = false;
            PendingStreamDelta {
                message_id,
                seq: state.streaming_delta_seq.saturating_add(1),
                snapshot,
                parts,
            }
        };
        let persist =
            should_persist_streaming_snapshot(state.last_stream_persist_at, now, force_persist)
                .then(|| {
                    state.last_stream_persist_at = Some(now);
                    state.streaming_parts.clone()
                });
        (payload, persist, state.stream_emit_suppressed)
    };

    if let Some(parts) = persist_snapshot {
        if let Err(error) = ctx.session_store.persist_message_parts(
            &ctx.data_dir,
            session_id,
            &payload.message_id,
            &parts,
            payload.seq,
            None,
        ) {
            log::warn!("failed to persist coalesced streaming parts for {session_id}: {error}");
        }
    }

    if emit_suppressed {
        return;
    }

    let emitted = ctx.notifier.streaming_delta(AgentStreamingDeltaPayload {
        chat_session_id: session_id.to_string(),
        message_id: payload.message_id.clone(),
        seq: payload.seq,
        snapshot: payload.snapshot,
        parts: payload.parts.clone(),
    });

    let mut retry_delay = None;
    {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
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
}

async fn emit_streaming_delta_or_retry(
    ctx: &RuntimeContext,
    session_id: &str,
    payload: PendingStreamDelta,
) {
    {
        let sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get(session_id) else {
            return;
        };
        if state.stream_emit_suppressed {
            return;
        }
    }
    let now = std::time::Instant::now();
    let emitted = ctx.notifier.streaming_delta(AgentStreamingDeltaPayload {
        chat_session_id: session_id.to_string(),
        message_id: payload.message_id.clone(),
        seq: payload.seq,
        snapshot: payload.snapshot,
        parts: payload.parts.clone(),
    });
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

fn apply_post_turn_parts(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
    message_id: &str,
    incoming_parts: Vec<MessagePart>,
) {
    let Some(message) = (match session_store.load_full_session_for_restore(data_dir, session_id) {
        Ok(session) => session.and_then(|session| {
            session
                .messages
                .into_iter()
                .find(|message| message.id == message_id)
        }),
        Err(error) => {
            log::warn!("failed to load post-turn message base for {session_id}: {error}");
            return;
        }
    }) else {
        return;
    };
    let mut parts = message.parts.clone().unwrap_or_else(|| {
        if message.content.is_empty() {
            Vec::new()
        } else {
            vec![MessagePart::Text {
                content: message.content.clone(),
                parent_tool_use_id: None,
            }]
        }
    });
    for part in incoming_parts {
        merge_persisted_message_part(&mut parts, part);
    }
    if let Err(error) = session_store.persist_message_parts(
        data_dir,
        session_id,
        message_id,
        &parts,
        message.streaming_final_seq.saturating_add(1),
        None,
    ) {
        log::warn!("failed to persist post-turn parts for {session_id}: {error}");
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
                    content_ref: content_ref.map(Into::into),
                    summary: summary.map(Into::into),
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

async fn complete_turn(
    ctx: &RuntimeContext,
    session_id: &str,
    expected_generation: Option<u64>,
    result: crate::domain::agent_session::entities::TurnResult,
) -> Option<WorkflowTurnCompleteNotification> {
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
        return None;
    }
    flush_streaming_update(ctx, session_id, true).await;
    let terminal = terminal_projection(&result);
    let (message_id, parts, seq, turn_id, started_at, telemetry_dims) = {
        let mut sessions = ctx.sessions.lock().await;
        let state = sessions.get_mut(session_id)?;
        if state.phase == RuntimeSessionPhase::Idle
            || expected_generation.is_some_and(|generation| state.generation != generation)
        {
            return None;
        }
        state.phase = RuntimeSessionPhase::Idle;
        state.pending_permission_request = None;
        state.permission_wait_started_at = None;
        let message_id = state.streaming_message_id.clone();
        state.last_agent_message_id = message_id.clone();
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
        let turn_id = state.current_turn_id.or(state.last_turn_id);
        let started_at = state.turn_started_at.take();
        state.streaming_message_id = None;
        state.current_turn_id = None;
        state.pending_permission_request = None;
        state.current_turn_input = None;
        let telemetry_dims =
            session_telemetry_dimensions(&ctx.session_store, &ctx.data_dir, session_id);
        (
            message_id,
            state.streaming_parts.clone(),
            state.streaming_delta_seq,
            turn_id,
            started_at,
            telemetry_dims,
        )
    };
    let mut projected = None;
    if let (Some(turn_id), Some(message_id)) = (turn_id, message_id.clone()) {
        if let Err(error) = append_final_turn_events(
            &ctx.session_store,
            &ctx.data_dir,
            session_id,
            turn_id,
            &message_id,
            &parts,
            &terminal,
        ) {
            log::warn!("failed to record terminal turn events for {session_id}: {error}");
        }
        projected = ctx
            .session_store
            .load_session_events(&ctx.data_dir, session_id)
            .map(|events| TurnEventLog::from_events(events).project())
            .map_err(|error| {
                log::warn!("failed to project terminal turn events for {session_id}: {error}");
                error
            })
            .ok();
        let parts_to_persist = projected
            .as_ref()
            .map(|model| model.agent_parts_for_message(&message_id))
            .filter(|parts| !parts.is_empty())
            .unwrap_or_else(|| parts.clone());
        if let Err(error) = ctx.session_store.persist_message_parts(
            &ctx.data_dir,
            session_id,
            &message_id,
            &parts_to_persist,
            seq,
            Some(crate::usecase::agent_session::session::now_timestamp()),
        ) {
            log::warn!("failed to persist completed parts for {session_id}: {error}");
        }
    }
    {
        let mut sessions = ctx.sessions.lock().await;
        if let Some(state) = sessions.get_mut(session_id) {
            state.domain_streaming_parts.clear();
            state.streaming_parts.clear();
            state.streaming_delta_seq = 0;
            state.stream_emit_failure_count = 0;
            state.stream_emit_suppressed = false;
        }
    };
    let session_state = projected
        .as_ref()
        .map(|model| model.status.session_state.clone())
        .unwrap_or_else(|| terminal.session_state.clone());
    let lifecycle =
        crate::usecase::agent_session::session::lifecycle_controller::SessionLifecycleController {
            session_store: &ctx.session_store,
            data_dir: &ctx.data_dir,
        };
    if let Err(error) =
        lifecycle.complete_turn_state(session_id, terminal.exit_code, terminal.interrupted)
    {
        log::warn!("failed to persist terminal session state for {session_id}: {error}");
    }
    if let (Some(started_at), Some(dims)) = (started_at, telemetry_dims) {
        crate::other::telemetry::record_agent_turn_duration(
            crate::other::telemetry::AgentTurn::Complete,
            &dims,
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
            pending_permission_request: None,
            exit_code: Some(terminal.exit_code),
            completed_at: Some(crate::usecase::agent_session::session::now_timestamp()),
            interrupted: terminal.interrupted,
            session_state: Some(session_state),
        },
    );
    workflow_notification
}

async fn start_next_queued_turn(ctx: &RuntimeContext, session_id: &str) {
    let (queued, runtime) = {
        let sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get(session_id) else {
            return;
        };
        if state.phase != RuntimeSessionPhase::Idle {
            return;
        }
        let Some(queued) = state.pending_queue.front().cloned() else {
            return;
        };
        (queued, state.runtime.clone())
    };

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
    if queued.worktree_path != session.worktree_path {
        log::warn!(
            "queued turn worktree mismatch for {session_id}: queued={}, session={}",
            queued.worktree_path,
            session.worktree_path
        );
    }
    if let Some(existing_agent_message_id) = queued.existing_agent_message_id.as_deref() {
        log::debug!(
            "queued turn {session_id} carries existing agent message id {existing_agent_message_id}"
        );
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
            log::warn!("failed to build queued turn system prompt for {session_id}: {error}");
            None
        }
    };
    let runtime = match runtime {
        Some(runtime) => runtime,
        None => match open_runtime_for_session(ctx, &session, system_prompt.clone()).await {
            Ok(runtime) => runtime,
            Err(error) => {
                log::warn!("failed to reopen runtime for queued turn {session_id}: {error}");
                let _ = ctx.session_store.set_session_state(
                    &ctx.data_dir,
                    session_id,
                    SessionState::Error,
                );
                emit_session_state_change(
                    &ctx.session_store,
                    &ctx.notifier,
                    &ctx.status_center,
                    &ctx.status_notifier,
                    &ctx.data_dir,
                    session_id,
                    StateChange {
                        turn_phase: TurnPhase::Idle,
                        pending_permission_request: None,
                        exit_code: Some(1),
                        completed_at: Some(crate::usecase::agent_session::session::now_timestamp()),
                        interrupted: true,
                        session_state: Some(SessionState::Error),
                    },
                );
                return;
            }
        },
    };
    let turn_id = match next_turn_id(&ctx.session_store, &ctx.data_dir, session_id) {
        Ok(turn_id) => turn_id,
        Err(error) => {
            log::warn!("failed to allocate queued turn id for {session_id}: {error}");
            return;
        }
    };
    {
        let sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get(session_id) else {
            return;
        };
        if state.pending_queue.front().map(|front| front.id.as_str()) != Some(queued.id.as_str()) {
            return;
        }
    }
    let agent_message =
        match queued_agent_message(&ctx.session_store, &ctx.data_dir, session_id, &queued) {
            Ok(message) => message,
            Err(error) => {
                log::warn!("failed to append queued agent message for {session_id}: {error}");
                return;
            }
        };
    let human_message = queued_human_message(&queued);
    let mut queued_for_turn = queued.clone();
    queued_for_turn.existing_human_message_id = Some(human_message.id.clone());
    queued_for_turn.existing_agent_message_id = Some(agent_message.id.clone());
    {
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
    let restore_plan = if had_runtime {
        ContextRestorePlan::NoContext
    } else {
        let persisted = match ctx
            .session_store
            .load_full_session_for_restore(&ctx.data_dir, session_id)
        {
            Ok(session) => session,
            Err(error) => {
                log::warn!("failed to load queued turn restore context for {session_id}: {error}");
                None
            }
        };
        context_restore_plan_for_session_before_turn(persisted.as_ref(), &agent_message.id)
    };
    if restore_plan.carry_state() == Some(ContextCarryState::Reinjected) {
        match ctx.session_store.update_context_carry_if_changed(
            &ctx.data_dir,
            session_id,
            Some(ContextCarryState::Reinjected),
        ) {
            Ok(Some(meta)) => ctx.notifier.context_carry_updated(
                session_id,
                meta.agent_session_id,
                meta.context_carry,
                meta.updated_at,
            ),
            Ok(None) => {}
            Err(error) => {
                log::warn!(
                    "failed to persist queued reinjected context carry for {session_id}: {error}"
                );
            }
        }
    }
    let prompt = apply_restore_prompt_prefix(queued.content.clone(), &restore_plan);
    if let Err(error) = ctx.session_store.append_session_event_and_project_state(
        &ctx.data_dir,
        session_id,
        AgentSessionEvent::TurnStarted {
            turn_id,
            message_id: human_message.id.clone(),
            assistant_message_id: Some(agent_message.id.clone()),
            prompt: PromptInput::from_human_message(&human_message),
            at: human_message.timestamp,
        },
    ) {
        log::warn!("failed to append queued TurnStarted for {session_id}: {error}");
        return;
    }
    let generation = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        state.reset_for_turn(turn_id, agent_message.id.clone());
        state.current_turn_input = Some(queued_for_turn.clone());
        state.generation
    };
    if let Err(error) = runtime
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
    {
        log::warn!("failed to start queued turn for {session_id}: {error}");
        {
            let mut sessions = ctx.sessions.lock().await;
            if let Some(state) = sessions.get_mut(session_id) {
                if state.generation == generation {
                    state.rollback_started_turn();
                }
            }
        }
        let _ = ctx.session_store.append_session_event_and_project_state(
            &ctx.data_dir,
            session_id,
            AgentSessionEvent::TurnInterrupted {
                turn_id,
                reason: EventInterruptReason::Crash,
                exit_code: 1,
                error: Some(error.to_string()),
            },
        );
        emit_session_state_change(
            &ctx.session_store,
            &ctx.notifier,
            &ctx.status_center,
            &ctx.status_notifier,
            &ctx.data_dir,
            session_id,
            StateChange {
                turn_phase: TurnPhase::Idle,
                pending_permission_request: None,
                exit_code: Some(1),
                completed_at: Some(crate::usecase::agent_session::session::now_timestamp()),
                interrupted: true,
                session_state: Some(SessionState::Error),
            },
        );
    } else {
        {
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
        spawn_stale_watchdog_task(
            ctx,
            session_id.to_string(),
            generation,
            stale_timeout_for_session(&session),
        );
        emit_session_state_change(
            &ctx.session_store,
            &ctx.notifier,
            &ctx.status_center,
            &ctx.status_notifier,
            &ctx.data_dir,
            session_id,
            StateChange {
                turn_phase: TurnPhase::Streaming,
                pending_permission_request: None,
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
            Some(request.clone())
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

fn required_backend_id(session: &ChatSession) -> Result<String, AgentRuntimeError> {
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
        ContextSourceKind::WorkflowState => "releash_workflow_state",
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
struct StateChange {
    turn_phase: TurnPhase,
    pending_permission_request: Option<PermissionRequestMsg>,
    exit_code: Option<i64>,
    completed_at: Option<f64>,
    interrupted: bool,
    session_state: Option<SessionState>,
}

#[derive(Debug, Clone)]
struct TerminalProjection {
    exit_code: i64,
    interrupted: bool,
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
    let events = session_store.load_session_events(data_dir, session_id)?;
    Ok(TurnEventLog::from_events(events)
        .current_turn_id()
        .unwrap_or(0)
        .saturating_add(1))
}

fn append_durable_part_events(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
    turn_id: u64,
    message_id: &str,
    parts: &[MessagePart],
) {
    if !parts.iter().any(part_records_durable_event) {
        return;
    }
    let mut events = if parts.iter().any(part_needs_event_history) {
        match session_store.load_session_events(data_dir, session_id) {
            Ok(events) => events,
            Err(error) => {
                log::warn!("failed to load session events for {session_id}: {error}");
                return;
            }
        }
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
    for event in events.into_iter().skip(before) {
        if let Err(error) =
            session_store.append_session_event_without_projection(data_dir, session_id, event)
        {
            log::warn!("failed to append session part event for {session_id}: {error}");
            return;
        }
    }
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
    let decision = permission_decision_from_response(response);
    let answers = permission_answers_from_response(response);
    let mut patched = false;
    for part in &mut state.domain_streaming_parts {
        let DomainMessagePart::Permission { request } = part else {
            continue;
        };
        if request.id != response.request_id {
            continue;
        }
        request.status = PermissionRequestStatus::Resolved {
            decision,
            answers: answers.clone(),
        };
        patched = true;
    }
    if !patched {
        return None;
    }
    state.streaming_parts = parts_from_domain(state.domain_streaming_parts.clone());
    state.streaming_delta_seq = state.streaming_delta_seq.saturating_add(1);
    let message_id = state
        .streaming_message_id
        .clone()
        .or_else(|| state.last_agent_message_id.clone())?;
    let turn_id = state.current_turn_id.or(state.last_turn_id)?;
    Some((
        message_id,
        state.streaming_delta_seq,
        state.streaming_parts.clone(),
        turn_id,
    ))
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

fn append_permission_resolved_event(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
    turn_id: u64,
    response: &PermissionResponse,
) {
    let decision = match &response.decision {
        PermissionResponseDecision::Allow { .. } => {
            crate::usecase::agent_session::event_log::PermissionDecision::Allowed
        }
        PermissionResponseDecision::Deny { .. } => {
            crate::usecase::agent_session::event_log::PermissionDecision::Denied
        }
    };
    let answers = match &response.decision {
        PermissionResponseDecision::Allow { answers, .. } => answers
            .as_ref()
            .and_then(|payload| serde_json::from_str::<serde_json::Value>(payload.as_str()).ok()),
        PermissionResponseDecision::Deny { .. } => None,
    };
    if let Err(error) = session_store.append_session_event_and_project_state(
        data_dir,
        session_id,
        AgentSessionEvent::PermissionResolved {
            turn_id,
            tool_use_id: None,
            request_id: Some(response.request_id.clone()),
            decision,
            answers,
        },
    ) {
        log::warn!("failed to append permission resolved event for {session_id}: {error}");
    }
}

async fn resync_permission_mode(
    session_store: &Arc<SessionStore>,
    sessions: &Arc<Mutex<RuntimeSessionMap>>,
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
    let runtime = {
        let sessions = sessions.lock().await;
        sessions
            .get(session_id)
            .and_then(|state| state.runtime.clone())
    };
    if let Some(runtime) = runtime {
        if let Err(error) = runtime
            .set_permission_mode(saved_mode, meta.plan_mode)
            .await
        {
            log::warn!("failed to resync permission mode for {session_id}: {error}");
        }
    }
    Some(saved_mode)
}

fn append_final_turn_events(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
    turn_id: u64,
    message_id: &str,
    parts: &[MessagePart],
    terminal: &TerminalProjection,
) -> Result<(), String> {
    session_store.append_session_event_and_project_state(
        data_dir,
        session_id,
        AgentSessionEvent::FinalPartsRecorded {
            turn_id,
            message_id: message_id.to_string(),
            parts: parts.to_vec(),
        },
    )?;
    match &terminal.event {
        TerminalEventProjection::Completed {
            stop_reason,
            token_usage,
        } => {
            session_store.append_session_event_and_project_state(
                data_dir,
                session_id,
                AgentSessionEvent::TurnCompleted {
                    turn_id,
                    exit_code: terminal.exit_code,
                    stop_reason: *stop_reason,
                    token_usage: *token_usage,
                },
            )?;
        }
        TerminalEventProjection::Interrupted { reason, error } => {
            let mut events = session_store.load_session_events(data_dir, session_id)?;
            let before = events.len();
            finalize_turn(
                &mut events,
                turn_id,
                *reason,
                error.clone(),
                terminal.exit_code,
            );
            for event in events.into_iter().skip(before) {
                session_store
                    .append_session_event_and_project_state(data_dir, session_id, event)?;
            }
        }
    }
    Ok(())
}

fn terminal_projection(result: &TurnResult) -> TerminalProjection {
    match result {
        TurnResult::Completed {
            stop_reason,
            token_usage,
        } => TerminalProjection {
            exit_code: 0,
            interrupted: false,
            session_state: SessionState::Done,
            event: TerminalEventProjection::Completed {
                stop_reason: stop_reason.map(map_turn_stop_reason),
                token_usage: token_usage.map(turn_token_usage_from_domain),
            },
        },
        TurnResult::Failed { token_usage, .. } => TerminalProjection {
            exit_code: 1,
            interrupted: false,
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
            };
            TerminalProjection {
                exit_code,
                interrupted: true,
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

fn emit_session_state_change(
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
        pending_permission_request: change.pending_permission_request.clone(),
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
    let session_state = change
        .session_state
        .unwrap_or_else(|| session.state.clone());
    let worktree_path = session.worktree_path.clone();
    let workflow_context = session.workflow_step_context.clone();
    let workflow_execution_state = match change.turn_phase {
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
        workflow_step: workflow_context
            .as_ref()
            .map(|context| context.step_name.clone()),
        workflow_execution_state,
        workflow_execution_id: workflow_context
            .as_ref()
            .map(|context| context.run_id.clone()),
        workflow_run_index: workflow_context.as_ref().map(|context| context.run_index),
        workflow_step_progress: None,
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
        context: crate::other::telemetry::TurnContext::from_workflow_step(
            session.is_workflow_step_session(),
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

    async fn set_permission_mode(
        &self,
        _mode: PermissionMode,
        _plan_mode: bool,
    ) -> Result<(), AgentBackendError> {
        Err(AgentBackendError::Other(
            "injected test permission mode failure".to_string(),
        ))
    }

    async fn set_model(&self, _model: &ModelId) -> Result<(), AgentBackendError> {
        Err(AgentBackendError::Other(
            "injected test model failure".to_string(),
        ))
    }

    async fn set_session_title(&self, _title: &str) -> Result<(), AgentBackendError> {
        Ok(())
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

    async fn set_permission_mode(
        &self,
        _mode: PermissionMode,
        _plan_mode: bool,
    ) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn set_model(&self, _model: &ModelId) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn set_session_title(&self, _title: &str) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn close(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::gateway::{
        AgentBackend, AgentSessionRuntime, ForkSessionRequest,
    };
    use crate::domain::agent_session::value_objects::{
        BackendCapabilities, ModelDescriptor, SkillEntry,
    };
    use crate::domain::workflow::WorkflowStepContext;
    use crate::test_support::{
        build_agent_runtime_usecase_with_controller,
        build_agent_runtime_usecase_with_controller_and_notifiers, build_session_store,
        TestRuntimeCallKind,
    };
    use crate::usecase::agent_session::runtime::ports::{
        AgentSessionEventNotifier, AgentSessionStateChangedPayload, AgentStreamingDeltaPayload,
    };
    use crate::usecase::agent_session::session::{
        create_session_internal_with_attributes, ChatMessage, MessagePart, PermissionPartStatus,
        PermissionRequestKindMsg, SessionCreationAttributes,
    };
    use crate::usecase::agent_session::status::{
        AgentStatusChanges, AgentStatusNotifier, TurnPhaseRepr,
    };
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::Notify;

    struct TokioSpawner;

    impl AgentTaskSpawner for TokioSpawner {
        fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
            tokio::spawn(future);
        }
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

        async fn archive_session(
            &self,
            _backend_session_id: &str,
            _cwd: &str,
        ) -> Result<(), AgentBackendError> {
            Ok(())
        }

        async fn unarchive_session(
            &self,
            _backend_session_id: &str,
            _cwd: &str,
        ) -> Result<(), AgentBackendError> {
            Ok(())
        }

        async fn fork_session(
            &self,
            _req: ForkSessionRequest,
        ) -> Result<Option<String>, AgentBackendError> {
            Ok(None)
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
            model: "gpt-5.5",
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
        state_changes: Mutex<Vec<AgentSessionStateChangedPayload>>,
        streaming_deltas: Mutex<Vec<AgentStreamingDeltaPayload>>,
        permission_modes: Mutex<Vec<(String, String)>>,
        model_updates: Mutex<Vec<(String, Vec<ModelInfo>, String)>>,
        streaming_delta_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
        fail_streaming_delta: Mutex<bool>,
    }

    impl RecordingAgentNotifier {
        fn state_changes(&self) -> Vec<AgentSessionStateChangedPayload> {
            self.state_changes.lock().unwrap().clone()
        }

        fn streaming_deltas(&self) -> Vec<AgentStreamingDeltaPayload> {
            self.streaming_deltas.lock().unwrap().clone()
        }

        fn permission_modes(&self) -> Vec<(String, String)> {
            self.permission_modes.lock().unwrap().clone()
        }

        fn model_updates(&self) -> Vec<(String, Vec<ModelInfo>, String)> {
            self.model_updates.lock().unwrap().clone()
        }

        fn set_streaming_delta_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
            *self.streaming_delta_hook.lock().unwrap() = Some(hook);
        }

        fn set_streaming_delta_failure(&self, fail: bool) {
            *self.fail_streaming_delta.lock().unwrap() = fail;
        }
    }

    impl AgentSessionEventNotifier for RecordingAgentNotifier {
        fn session_state_changed(&self, payload: AgentSessionStateChangedPayload) {
            self.state_changes.lock().unwrap().push(payload);
        }

        fn streaming_delta(&self, payload: AgentStreamingDeltaPayload) -> bool {
            if let Some(hook) = self.streaming_delta_hook.lock().unwrap().clone() {
                hook();
            }
            self.streaming_deltas.lock().unwrap().push(payload);
            !*self.fail_streaming_delta.lock().unwrap()
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

    fn workflow_step_context(
        startup_timeout_secs: Option<u64>,
        startup_max_retries: Option<u32>,
        stale_timeout_secs: Option<u64>,
    ) -> WorkflowStepContext {
        WorkflowStepContext {
            run_id: "run-1".to_string(),
            workflow_name: "workflow".to_string(),
            step_name: "step".to_string(),
            run_index: 0,
            parent_step_name: None,
            parent_run_index: None,
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
                if notifier
                    .streaming_deltas()
                    .last()
                    .is_some_and(|delta| predicate(delta))
                {
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
                workflow_step_session: false,
                workflow_step_context: None,
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
    async fn test_failed終端したturnの後も同一sessionへの次sendは新turnを開始できる() {
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

        // Then: a new turn starts immediately instead of being queued.
        assert!(second.agent_message.is_some());
        assert!(second.queued_turn.is_none());
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
                session_store,
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
        usecase.drain_next_queued_turn_for_test(&session_id).await;
        wait_for_start_prompt_count(&controller, &session_id, 3).await;
        assert!(usecase.pending_queue(&session_id).await.is_empty());
    }

    #[tokio::test]
    async fn fatal_closes_runtime_preserves_queued_turn_and_drain_restarts_it() {
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
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert!(event_notifier.state_changes().iter().any(|change| {
            change.chat_session_id == session_id
                && change.turn_phase == TurnPhase::Idle
                && change.session_state == Some(SessionState::Error)
        }));
        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);

        controller.release_start_turn();
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
    async fn test_init_sessionsは_workflow_step_tabを復元し_active_session_modeを返す() {
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
                workflow_step_session: false,
                workflow_step_context: None,
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
                workflow_step_session: true,
                workflow_step_context: Some(workflow_step_context(None, None, None)),
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

    #[tokio::test]
    async fn set_permission_mode_persists_and_notifies_when_runtime_sync_fails() {
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
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_step_session: false,
                workflow_step_context: None,
            },
        )
        .unwrap();
        usecase
            .insert_failing_runtime_state_for_test(&session.id)
            .await;

        usecase
            .set_permission_mode(&session.id, PermissionMode::Full)
            .await
            .unwrap();

        let saved = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.permission_mode, "full");
        assert!(event_notifier
            .permission_modes()
            .contains(&(session.id.clone(), "full".to_string())));
    }

    #[tokio::test]
    async fn set_model_persists_and_notifies_dto_when_runtime_sync_fails() {
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
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-4-sonnet".to_string()),
                plan_mode: false,
                workflow_step_session: false,
                workflow_step_context: None,
            },
        )
        .unwrap();
        usecase
            .insert_failing_runtime_state_for_test(&session.id)
            .await;

        usecase.set_model(&session.id, "codex:gpt-5").await.unwrap();

        let saved = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.backend_id, "codex");
        assert_eq!(saved.selected_model.as_deref(), Some("gpt-5"));
        let updates = event_notifier.model_updates();
        let (_, available_models, selected_model) = updates
            .iter()
            .find(|(session_id, _, _)| session_id == &session.id)
            .expect("model update notification");
        assert_eq!(selected_model, "gpt-5");
        assert!(available_models.iter().any(|model| {
            model.id == "codex:gpt-5" && model.backend == "codex" && model.model_id == "gpt-5"
        }));
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
                workflow_step_session: false,
                workflow_step_context: None,
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
                request: PermissionRequestMsg {
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
                },
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
    async fn respond_permission_runtime_failure_leaves_state_parts_and_event_log_unchanged() {
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
    }

    #[tokio::test]
    async fn respond_permission_success_patches_before_persist_event_and_delta() {
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
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-1")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;
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

        assert_eq!(&*order.lock().unwrap(), &["persist", "event", "delta"]);
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
    async fn test_waiting_permissionでtimeout超過してもwatchdogは許可後にstale終端する() {
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
                workflow_step_session: true,
                workflow_step_context: Some(workflow_step_context(None, None, Some(1))),
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
        tokio::time::sleep(Duration::from_millis(1_200)).await;

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

        wait_for_call(&controller, &session.id, TestRuntimeCallKind::Close).await;
        assert!(controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Interrupt));
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
            sessions.get_mut(&session_id).unwrap().last_progress_at = Some(stale_instant);
        }

        // When: the backend emits a keep_alive liveness event.
        controller
            .emit(&session_id, AgentRuntimeEvent::KeepAlive)
            .unwrap();

        // Then: the progress clock is refreshed so the stale watchdog does not
        // interrupt the healthy turn.
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                {
                    let sessions = usecase.ctx.sessions.lock().await;
                    let last_progress_at =
                        sessions.get(&session_id).unwrap().last_progress_at.unwrap();
                    if last_progress_at > stale_instant {
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
    async fn test_streaming_delta_emit完全停止後もturnは完了し確定messageが保存される() {
        // Given: streaming emits that fail until the emit stop threshold is reached.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
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
        assert_eq!(
            usecase
                .stream_emit_failure_state_for_test(&session_id)
                .await,
            Some((0, false))
        );
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
    async fn test_turn終端後の_trailing_deltaはsnapshot_emitせず確定messageへ即時保存する() {
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

        // Then: no standalone snapshot is emitted and the saved agent message is merged in place.
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
                content: "hello world".to_string(),
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
                workflow_step_session: false,
                workflow_step_context: None,
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
                workflow_step_session: false,
                workflow_step_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("old-backend-session".to_string());
        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &session)
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
        wait_for_start_prompt_count(&controller, &session.id, 2).await;

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
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.agent_session_id, None);
        assert_eq!(
            loaded.session.context_carry,
            Some(ContextCarryState::Reinjected)
        );
    }

    #[tokio::test]
    async fn test_backend_session_clearedは_context_carry_failedを書き込む() {
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
                workflow_step_session: false,
                workflow_step_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("backend-session".to_string());
        session.context_carry = Some(ContextCarryState::Resumed);
        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &session)
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
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Then: the persisted resume metadata is cleared and the carry state is Failed.
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.agent_session_id, None);
        assert_eq!(
            loaded.session.context_carry,
            Some(ContextCarryState::Failed)
        );
    }

    #[tokio::test]
    async fn test_start_turn_locked_workflow_contextのtimeoutを_session_specへ渡す() {
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
                workflow_step_session: true,
                workflow_step_context: Some(workflow_step_context(Some(12), Some(3), Some(44))),
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

        // Then: the backend receives the workflow timeout hints through SessionSpec.
        assert!(controller.calls().iter().any(|call| {
            call.session_id == session.id
                && call.kind
                    == TestRuntimeCallKind::OpenSession {
                        startup_timeout_ms: Some(12_000),
                        startup_max_retries: Some(3),
                        stale_timeout_ms: Some(44_000),
                    }
        }));
    }

    #[tokio::test]
    async fn test_stale_watchdog_無進捗turnをtimeout終端してruntimeを閉じる() {
        // Given: a workflow-step session whose stale timeout is immediate for the test.
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
                workflow_step_session: true,
                workflow_step_context: Some(workflow_step_context(None, None, Some(0))),
            },
        )
        .unwrap();
        controller.pause_interrupt();

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
        wait_for_call(&controller, &session.id, TestRuntimeCallKind::Interrupt).await;
        let send_response = tokio::time::timeout(
            Duration::from_millis(200),
            usecase.send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "next".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            }),
        )
        .await
        .expect("send_message must not wait for stale runtime interrupt/close")
        .unwrap();
        assert!(send_response.agent_message.is_some() || send_response.queued_turn.is_some());
        controller.release_interrupt();
        wait_for_call(&controller, &session.id, TestRuntimeCallKind::Close).await;

        // Then: the stale runtime is interrupted/closed after the same session accepted input.
        let calls = controller.call_kinds_for(&session.id);
        assert!(calls.contains(&TestRuntimeCallKind::Interrupt));
        assert!(calls.contains(&TestRuntimeCallKind::Close));
    }

    #[tokio::test]
    async fn test_stale_watchdogはツール実行中のturnをtimeout終端しない() {
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
                workflow_step_session: true,
                workflow_step_context: Some(workflow_step_context(None, None, Some(1))),
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

        // Then: the stale watchdog does not interrupt the healthy tool-in-flight turn.
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Interrupt));
    }

    #[tokio::test]
    async fn test_stale_timeout後の遅延abort完了は_error状態を上書きしない() {
        // Given: a workflow-step session whose stale timeout completes before the backend
        // reports its interrupt result.
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
                workflow_step_session: true,
                workflow_step_context: Some(workflow_step_context(None, None, Some(0))),
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
        wait_for_call(&controller, &session.id, TestRuntimeCallKind::Close).await;

        // When: the backend reports an abort completion after the stale timeout has already
        // persisted the terminal Error state.
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Abort,
                    error: None,
                }),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Then: complete_turn treats the delayed completion as stale and leaves Error intact.
        assert_eq!(usecase.turn_phase(&session.id).await, Some(TurnPhase::Idle));
        let session = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(session.session.state, SessionState::Error);
    }
}
