use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use crate::domain::agent_session::entities::MessagePart as DomainMessagePart;
use crate::domain::agent_session::gateway::AgentSessionRuntime;
use crate::usecase::agent_session::event_log::BackendSessionRecoveryReason;
use crate::usecase::agent_session::session::{
    ChatMessage, MessagePart, PermissionRequestMsg, TokenUsage,
};
use crate::usecase::agent_session::status::TurnPhase;

use super::ports::AgentStreamingDeltaPayload;
use super::queue::QueuedTurnInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeSessionPhase {
    Idle,
    Streaming,
    WaitingPermission,
}

impl From<RuntimeSessionPhase> for TurnPhase {
    fn from(value: RuntimeSessionPhase) -> Self {
        match value {
            RuntimeSessionPhase::Idle => Self::Idle,
            RuntimeSessionPhase::Streaming => Self::Streaming,
            RuntimeSessionPhase::WaitingPermission => Self::WaitingPermission,
        }
    }
}

pub(crate) struct RuntimeSessionState {
    pub backend_id: String,
    pub runtime: Option<Arc<dyn AgentSessionRuntime>>,
    pub phase: RuntimeSessionPhase,
    pub streaming_message_id: Option<String>,
    pub last_agent_message_id: Option<String>,
    pub domain_streaming_parts: Vec<DomainMessagePart>,
    pub streaming_parts: Vec<MessagePart>,
    pub streaming_delta_seq: u64,
    pub pending_stream_parts: Vec<MessagePart>,
    pub pending_stream_bytes: usize,
    pub pending_stream_snapshot: bool,
    pub retry_stream_delta: Option<PendingStreamDelta>,
    pub authoritative_stream_retries: VecDeque<PendingStreamDelta>,
    pub authoritative_stream_emit_failure_count: u32,
    pub authoritative_stream_flush_scheduled: bool,
    pub stream_emit_failure_count: u32,
    pub stream_emit_suppressed: bool,
    pub last_stream_emit_at: Option<Instant>,
    pub last_stream_persist_at: Option<Instant>,
    pub stream_flush_scheduled: bool,
    /// A process-exit Fatal emitted immediately after an already completed crash turn.
    pub pending_trailing_fatal_message: Option<String>,
    pub current_turn_id: Option<u64>,
    pub last_turn_id: Option<u64>,
    pub pending_permission_request: Option<PermissionRequestMsg>,
    pub pending_queue: VecDeque<QueuedTurnInput>,
    pub current_turn_input: Option<QueuedTurnInput>,
    pub latest_token_usage: Option<TokenUsage>,
    pub last_progress_at: Option<Instant>,
    pub turn_started_at: Option<Instant>,
    pub first_backend_event_recorded: bool,
    pub permission_wait_started_at: Option<Instant>,
    pub permission_wait_diagnostic_emitted: bool,
    pub permission_request_visibility: Option<PermissionRequestVisibility>,
    pub pending_permission_state_revision: u64,
    pub stall_signal_count: u32,
    pub stall_recovery_attempts: u32,
    pub stall_observation_active: bool,
    pub generation: u64,
    pub runtime_epoch: u64,
    pub provider_session_established: bool,
    pub backend_recovery: Option<BackendSessionRecoveryState>,
}

pub(crate) struct BackendSessionRecoveryState {
    pub recovery_id: String,
    pub old_provider_session_generation: u64,
    pub reason: BackendSessionRecoveryReason,
    pub completion: tokio::sync::watch::Sender<bool>,
}

#[derive(Debug, Clone)]
pub(crate) struct PermissionRequestVisibility {
    pub request_id: String,
    pub last_seen_at: Instant,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingStreamDelta {
    pub message_id: String,
    pub seq: u64,
    pub snapshot: bool,
    pub parts: Vec<MessagePart>,
    pub message: Option<ChatMessage>,
    /// Final/backend-owned snapshots replace an older retry payload when delivery fails.
    pub authoritative: bool,
}

impl PendingStreamDelta {
    pub(crate) fn to_delta_payload(&self, session_id: &str) -> AgentStreamingDeltaPayload {
        AgentStreamingDeltaPayload {
            chat_session_id: session_id.to_string(),
            message_id: self.message_id.clone(),
            seq: self.seq,
            snapshot: self.snapshot,
            parts: self.parts.clone(),
            message: self.message.clone(),
        }
    }
}

impl RuntimeSessionState {
    pub(crate) fn new(backend_id: String) -> Self {
        Self {
            backend_id,
            runtime: None,
            phase: RuntimeSessionPhase::Idle,
            streaming_message_id: None,
            last_agent_message_id: None,
            domain_streaming_parts: Vec::new(),
            streaming_parts: Vec::new(),
            streaming_delta_seq: 0,
            pending_stream_parts: Vec::new(),
            pending_stream_bytes: 0,
            pending_stream_snapshot: false,
            retry_stream_delta: None,
            authoritative_stream_retries: VecDeque::new(),
            authoritative_stream_emit_failure_count: 0,
            authoritative_stream_flush_scheduled: false,
            stream_emit_failure_count: 0,
            stream_emit_suppressed: false,
            last_stream_emit_at: None,
            last_stream_persist_at: None,
            stream_flush_scheduled: false,
            pending_trailing_fatal_message: None,
            current_turn_id: None,
            last_turn_id: None,
            pending_permission_request: None,
            pending_queue: VecDeque::new(),
            current_turn_input: None,
            latest_token_usage: None,
            last_progress_at: None,
            turn_started_at: None,
            first_backend_event_recorded: false,
            permission_wait_started_at: None,
            permission_wait_diagnostic_emitted: false,
            permission_request_visibility: None,
            pending_permission_state_revision: 0,
            stall_signal_count: 0,
            stall_recovery_attempts: 0,
            stall_observation_active: false,
            generation: 0,
            runtime_epoch: 0,
            provider_session_established: false,
            backend_recovery: None,
        }
    }

    pub(crate) fn bump_runtime_epoch(&mut self) -> u64 {
        self.runtime_epoch = self.runtime_epoch.saturating_add(1);
        self.provider_session_established = false;
        self.runtime_epoch
    }

    pub(crate) fn reset_for_turn(&mut self, turn_id: u64, message_id: String) {
        self.phase = RuntimeSessionPhase::Streaming;
        self.streaming_message_id = Some(message_id.clone());
        self.last_agent_message_id = Some(message_id);
        self.domain_streaming_parts.clear();
        self.streaming_parts.clear();
        self.streaming_delta_seq = 0;
        self.pending_stream_parts.clear();
        self.pending_stream_bytes = 0;
        self.pending_stream_snapshot = false;
        self.retry_stream_delta = None;
        self.stream_emit_failure_count = 0;
        self.stream_emit_suppressed = false;
        self.last_stream_emit_at = None;
        self.last_stream_persist_at = None;
        self.stream_flush_scheduled = false;
        self.pending_trailing_fatal_message = None;
        self.current_turn_id = Some(turn_id);
        self.last_turn_id = Some(turn_id);
        self.clear_pending_permission_request();
        self.current_turn_input = None;
        let now = Instant::now();
        self.last_progress_at = Some(now);
        self.turn_started_at = Some(now);
        self.first_backend_event_recorded = false;
        self.permission_wait_started_at = None;
        self.permission_wait_diagnostic_emitted = false;
        self.permission_request_visibility = None;
        self.stall_signal_count = 0;
        self.stall_recovery_attempts = 0;
        self.stall_observation_active = false;
        self.generation = self.generation.saturating_add(1);
    }

    pub(crate) fn mark_progress(&mut self, at: Instant) -> bool {
        self.record_progress(at);
        let had_active_stall_observation = self.stall_observation_active;
        self.stall_observation_active = false;
        had_active_stall_observation
    }

    pub(crate) fn record_progress(&mut self, at: Instant) -> bool {
        self.last_progress_at = Some(at);
        self.stall_observation_active
    }

    pub(crate) fn rollback_started_turn(&mut self) {
        self.phase = RuntimeSessionPhase::Idle;
        self.streaming_message_id = None;
        self.current_turn_id = None;
        self.clear_pending_permission_request();
        self.current_turn_input = None;
        self.domain_streaming_parts.clear();
        self.streaming_parts.clear();
        self.pending_stream_parts.clear();
        self.pending_stream_bytes = 0;
        self.pending_stream_snapshot = false;
        self.retry_stream_delta = None;
        self.stream_emit_failure_count = 0;
        self.stream_emit_suppressed = false;
        self.last_stream_emit_at = None;
        self.last_stream_persist_at = None;
        self.stream_flush_scheduled = false;
        self.pending_trailing_fatal_message = None;
        self.turn_started_at = None;
        self.first_backend_event_recorded = false;
        self.permission_wait_started_at = None;
        self.permission_wait_diagnostic_emitted = false;
        self.permission_request_visibility = None;
        self.stall_signal_count = 0;
        self.stall_recovery_attempts = 0;
        self.stall_observation_active = false;
    }

    pub(crate) fn set_pending_permission_request(&mut self, request: PermissionRequestMsg) -> u64 {
        self.pending_permission_request = Some(request);
        self.permission_request_visibility = None;
        self.bump_pending_permission_state_revision()
    }

    pub(crate) fn clear_pending_permission_request(&mut self) -> u64 {
        self.pending_permission_request = None;
        self.permission_request_visibility = None;
        self.bump_pending_permission_state_revision()
    }

    fn bump_pending_permission_state_revision(&mut self) -> u64 {
        self.pending_permission_state_revision =
            self.pending_permission_state_revision.saturating_add(1);
        self.pending_permission_state_revision
    }
}

pub(crate) type RuntimeSessionMap = HashMap<String, RuntimeSessionState>;
