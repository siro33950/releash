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
    pub closing: bool,
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
    pub terminal_turn_id: Option<u64>,
    pub pending_permission_request: Option<PermissionRequestMsg>,
    pub pending_queue: VecDeque<QueuedTurnInput>,
    pub queue_paused: bool,
    pub queue_paused_at: Option<f64>,
    pub interrupt_requested_generation: Option<u64>,
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
    /// Once the provider-side recovery attempt has failed, retain the exact
    /// failure until its terminal recovery batch is durably acknowledged.
    /// The runtime event pump can then retry the same recovery identity
    /// without reopening the provider or inventing a second failure.
    pub pending_failure: Option<String>,
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
        Self::with_queue_pause(backend_id, None)
    }

    pub(crate) fn with_queue_pause(backend_id: String, queue_paused_at: Option<f64>) -> Self {
        Self {
            backend_id,
            runtime: None,
            closing: false,
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
            terminal_turn_id: None,
            pending_permission_request: None,
            pending_queue: VecDeque::new(),
            queue_paused: queue_paused_at.is_some(),
            queue_paused_at,
            interrupt_requested_generation: None,
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

    pub(crate) fn register_turn_start_intent(&mut self, turn_id: u64, message_id: String) -> u64 {
        self.phase = RuntimeSessionPhase::Streaming;
        self.streaming_message_id = Some(message_id.clone());
        self.current_turn_id = Some(turn_id);
        self.last_turn_id = Some(turn_id);
        self.terminal_turn_id = None;
        self.generation = self.generation.saturating_add(1);
        self.generation
    }

    pub(crate) fn commit_turn_start(&mut self, message_id: String) {
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
    }

    pub(crate) fn reset_for_turn(&mut self, turn_id: u64, message_id: String) {
        self.register_turn_start_intent(turn_id, message_id.clone());
        self.commit_turn_start(message_id);
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
        self.terminal_turn_id = None;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_pause_is_initialized_false_and_survives_turn_state_changes() {
        let mut state = RuntimeSessionState::new("codex".to_string());
        assert!(!state.queue_paused);

        state.queue_paused = true;
        state.queue_paused_at = Some(1.0);
        state.reset_for_turn(1, "message-1".to_string());
        assert!(state.queue_paused);
        assert_eq!(state.queue_paused_at, Some(1.0));

        state.rollback_started_turn();
        assert!(state.queue_paused);
        assert_eq!(state.queue_paused_at, Some(1.0));
    }

    #[test]
    fn durable_queue_pause_hydrates_runtime_state() {
        let state = RuntimeSessionState::with_queue_pause("codex".to_string(), Some(42.0));

        assert!(state.queue_paused);
        assert_eq!(state.queue_paused_at, Some(42.0));
    }

    #[test]
    fn turn_start_intent_registers_ownership_before_committing_turn_state() {
        let mut state = RuntimeSessionState::new("codex".to_string());

        let generation = state.register_turn_start_intent(7, "message-7".to_string());

        assert_eq!(generation, 1);
        assert_eq!(state.phase, RuntimeSessionPhase::Streaming);
        assert_eq!(state.current_turn_id, Some(7));
        assert_eq!(state.last_turn_id, Some(7));
        assert_eq!(state.streaming_message_id.as_deref(), Some("message-7"));
        assert!(state.turn_started_at.is_none());
        assert!(state.last_progress_at.is_none());

        state.commit_turn_start("message-7".to_string());

        assert_eq!(state.generation, generation);
        assert!(state.turn_started_at.is_some());
        assert!(state.last_progress_at.is_some());
    }
}
