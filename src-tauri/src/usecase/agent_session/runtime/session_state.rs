use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use crate::domain::agent_session::entities::MessagePart as DomainMessagePart;
use crate::domain::agent_session::gateway::AgentSessionRuntime;
use crate::usecase::agent_session::session::{MessagePart, PermissionRequestMsg, TokenUsage};
use crate::usecase::agent_session::status::TurnPhase;

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
    pub last_stream_emit_at: Option<Instant>,
    pub last_stream_persist_at: Option<Instant>,
    pub stream_flush_scheduled: bool,
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
    pub generation: u64,
    pub runtime_epoch: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingStreamDelta {
    pub message_id: String,
    pub seq: u64,
    pub snapshot: bool,
    pub parts: Vec<MessagePart>,
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
            last_stream_emit_at: None,
            last_stream_persist_at: None,
            stream_flush_scheduled: false,
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
            generation: 0,
            runtime_epoch: 0,
        }
    }

    pub(crate) fn bump_runtime_epoch(&mut self) -> u64 {
        self.runtime_epoch = self.runtime_epoch.saturating_add(1);
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
        self.last_stream_emit_at = None;
        self.last_stream_persist_at = None;
        self.stream_flush_scheduled = false;
        self.current_turn_id = Some(turn_id);
        self.last_turn_id = Some(turn_id);
        self.pending_permission_request = None;
        self.current_turn_input = None;
        let now = Instant::now();
        self.last_progress_at = Some(now);
        self.turn_started_at = Some(now);
        self.first_backend_event_recorded = false;
        self.permission_wait_started_at = None;
        self.generation = self.generation.saturating_add(1);
    }

    pub(crate) fn rollback_started_turn(&mut self) {
        self.phase = RuntimeSessionPhase::Idle;
        self.streaming_message_id = None;
        self.current_turn_id = None;
        self.pending_permission_request = None;
        self.current_turn_input = None;
        self.domain_streaming_parts.clear();
        self.streaming_parts.clear();
        self.pending_stream_parts.clear();
        self.pending_stream_bytes = 0;
        self.pending_stream_snapshot = false;
        self.retry_stream_delta = None;
        self.last_stream_emit_at = None;
        self.last_stream_persist_at = None;
        self.stream_flush_scheduled = false;
        self.turn_started_at = None;
        self.first_backend_event_recorded = false;
        self.permission_wait_started_at = None;
    }
}

pub(crate) type RuntimeSessionMap = HashMap<String, RuntimeSessionState>;
