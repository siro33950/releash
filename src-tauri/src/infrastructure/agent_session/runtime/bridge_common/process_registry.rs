use crate::infrastructure::agent_session::runtime::turn_latency::TurnLatencyState;
use crate::infrastructure::agent_session::runtime::AgentEditorContext;
use crate::infrastructure::agent_session::runtime::ImageAttachment;
use crate::infrastructure::agent_session::runtime::ModelInfo;
use crate::usecase::agent_session::event_log::{TurnEventLog, TurnStopReason};
use crate::usecase::agent_session::session::ContextCarryState;
use crate::usecase::agent_session::session::MessagePart;
use crate::usecase::agent_session::session::TokenUsage;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::process::Child;
use tokio::process::ChildStdin;
use tokio::sync::Mutex;

pub type AgentStdin = Arc<Mutex<ChildStdin>>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BridgeState {
    Initializing,
    Ready,
    Streaming,
    Crashed,
}

/// SDK's turn lifecycle phase, exposed to the frontend as the single source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnPhase {
    /// Waiting for user input / turn completed
    Idle,
    /// Actively processing a user prompt (SDK iterator yielding before `result`)
    Streaming,
    /// SDK blocked on `canUseTool` promise — waiting for permission response
    WaitingPermission,
}

/// A message queued while the agent is streaming, consumed on turn_complete.
pub struct PendingMessage {
    pub id: String,
    pub content: String,
    pub created_at: f64,
    pub client_sent_at_ms: Option<f64>,
    pub request_received_at_ms: Option<f64>,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub images: Vec<ImageAttachment>,
    pub worktree_path: String,
    pub mentions: Vec<crate::domain::code::MentionReference>,
    pub editor_context: Option<AgentEditorContext>,
    pub existing_human_message_id: Option<String>,
    pub existing_agent_message_id: Option<String>,
}

pub struct AgentProcess {
    pub stdin: AgentStdin,
    pub backend_id: String,
    pub state: BridgeState,
    pub turn_phase: TurnPhase,
    pub sdk_session_id: Option<String>,
    pub system_prompt_fingerprint: Option<String>,
    pub context_carry_on_ready: Option<ContextCarryState>,
    #[cfg_attr(unix, allow(dead_code))]
    pub child: Child,
    pub generation_id: u64,
    #[cfg(unix)]
    pub pgid: Option<u32>,
    pub streaming_message_id: Option<String>,
    /// Rust-issued per-turn token echoed by the Claude bridge. Uses the agent
    /// message id so stale bridge events cannot complete or mutate a later turn.
    pub active_turn_token: Option<String>,
    pub(crate) turn_latency: Option<TurnLatencyState>,
    /// Token for the most recent normally completed turn. Kept only while
    /// `last_message_id` can accept post-turn background task updates.
    pub(crate) post_turn_message_token: Option<String>,
    pub streaming_parts: Vec<MessagePart>,
    /// Number of entries in `streaming_parts` covered by the most recent
    /// successfully emitted delta. This is metadata only; live resync derives
    /// the confirmed view from `streaming_parts` plus pending rollback records.
    pub(crate) confirmed_stream_part_len: usize,
    /// Per-turn durable event buffer. This is the fact stream used by projection;
    /// `streaming_parts` remains only as the existing cumulative live buffer.
    pub turn_event_log: TurnEventLog,
    /// Retained after turn_complete so post-turn background task events
    /// can still be accumulated and emitted via `agent-streaming-delta`.
    pub last_message_id: Option<String>,
    /// Message id whose store-backed parts cannot be trusted as a post-turn
    /// base because the latest full-message persist is pending or failed.
    pub(crate) post_turn_base_untrusted_message_id: Option<String>,
    /// Maps background task_id (agentId) -> tool_use_id.
    /// Populated from ToolResult content ("agentId: XXX"), used to fill
    /// missing tool_use_id in task_notification messages from the SDK.
    pub task_id_map: HashMap<String, String>,
    /// Pending messages queued by send_agent_message while the current turn is busy.
    /// Auto-consumed FIFO on turn_complete by the stdout reader.
    pub pending_messages: VecDeque<PendingMessage>,
    /// Runtime permission mode tracked from SDK notifications.
    /// Holds the abstract mode (ask / edit / full) and is updated when the SDK
    /// reports a transition that maps cleanly to an abstract mode; unmapped values
    /// (e.g. transient "plan" from Claude SDK) are ignored.
    pub current_permission_mode: String,
    /// Available models from Agent SDK.
    pub available_models: Vec<ModelInfo>,
    /// Currently selected model for this session (None = SDK default).
    pub selected_model: Option<String>,
    pub(crate) stale_timeout: Duration,
    /// Token usage from the latest `result` message (extracted from modelUsage).
    /// Consumed by turn_complete handler and passed to the workflow runtime usecase.
    pub last_result_token_usage: Option<(u64, u64)>,
    /// Typed provider stop reason for the current turn, consumed by TurnCompleted.
    pub(crate) current_turn_stop_reason: Option<TurnStopReason>,
    /// Token usage from the latest SDK result, retained for desktop status display.
    pub latest_token_usage: Option<TokenUsage>,
    /// Concrete streaming delta parts queued since the last successful emit.
    /// This is intentionally only the pending delta payload, not a cumulative
    /// snapshot. It preserves in-place update deltas whose target part is not
    /// the tail of `streaming_parts`.
    pub(crate) pending_stream_parts: Vec<MessagePart>,
    /// Previous values for in-place updates that are pending emit. This stays
    /// bounded by the unconfirmed delta payload and lets resync avoid a second
    /// cumulative parts buffer.
    pub(crate) pending_stream_part_rollbacks: Vec<StreamPartRollback>,
    /// Frozen retry payload for a delta whose previous emit attempt failed.
    /// New deltas are accumulated in `pending_stream_parts` and receive the
    /// next seq after this retry succeeds, so a delivered duplicate seq always
    /// carries the identical payload.
    pub(crate) retry_stream_delta: Option<PendingStreamDelta>,
    /// Accumulated payload bytes for parts queued since the last successful
    /// emit. Used to decide whether to flush early when the byte cap is
    /// reached.
    pub(crate) pending_stream_bytes: usize,
    /// Monotonic streaming delta sequence for the current agent message.
    /// Reset only when a new turn starts; post-turn background updates keep
    /// incrementing the completed turn's message sequence.
    pub(crate) streaming_delta_seq: u64,
    /// Last confirmed streaming delta seq per message. This is seq metadata
    /// only, not a delta history buffer; it lets late post-turn updates keep
    /// `(session_id, message_id)` sequence continuity after a new turn reset.
    pub(crate) streaming_delta_seq_by_message: HashMap<String, u64>,
    /// Persisted ref resyncs that must wait until a failed streaming delta
    /// retry has been delivered. Keyed by message id.
    pub(crate) pending_persisted_tool_output_resyncs:
        HashMap<String, PendingPersistedToolOutputResync>,
    /// Timestamp of the most recent successful streaming emit. `None` means
    /// the first emit for this turn — flush immediately.
    pub(crate) last_stream_emit_at: Option<Instant>,
    /// True while a per-turn auxiliary streaming-flush timer task is alive.
    /// Set when the timer is spawned at streaming start; cleared by the timer
    /// itself when it exits (turn ended and the buffer drained). Used to
    /// avoid spawning a duplicate timer on overlapping turn starts.
    pub(crate) streaming_timer_active: bool,
    /// Last meaningful SDK/bridge progress observed for the current turn.
    pub last_progress_at: Option<Instant>,
    /// Timestamp of the current turn phase; used as the streaming stale fallback.
    pub turn_phase_since: Instant,
    /// Monotonic per-process turn sequence. Watchdogs capture it to avoid
    /// acting on a later turn that reused the same bridge process.
    pub turn_seq: u64,
    /// True while a per-turn stale watchdog task is alive.
    pub(crate) turn_watchdog_active: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingStreamDelta {
    pub(crate) message_id: String,
    pub(crate) seq: u64,
    pub(crate) snapshot: bool,
    pub(crate) parts: Vec<MessagePart>,
    pub(crate) retry_snapshot_parts: Option<Vec<MessagePart>>,
    pub(crate) part_count: usize,
    pub(crate) pending_bytes: usize,
    pub(crate) rollbacks: Vec<StreamPartRollback>,
    pub(crate) confirmed_stream_part_len_after_success: usize,
    pub(crate) updates_live_counter: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingPersistedToolOutputResync {
    pub(crate) parts: Vec<MessagePart>,
}

#[derive(Debug, Clone)]
pub(crate) struct StreamPartRollback {
    pub(crate) index: usize,
    pub(crate) previous: MessagePart,
}

#[cfg(test)]
pub(crate) fn make_test_agent_process() -> AgentProcess {
    let mut command = test_echo_command();
    let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn test echo process");
    let stdin = child.stdin.take().expect("stdin");
    AgentProcess {
        stdin: Arc::new(Mutex::new(stdin)),
        backend_id: "mock".to_string(),
        state: BridgeState::Ready,
        turn_phase: TurnPhase::Idle,
        sdk_session_id: None,
        system_prompt_fingerprint: None,
        context_carry_on_ready: None,
        child,
        generation_id: 0,
        #[cfg(unix)]
        pgid: None,
        streaming_message_id: None,
        active_turn_token: None,
        turn_latency: None,
        post_turn_message_token: None,
        streaming_parts: Vec::new(),
        confirmed_stream_part_len: 0,
        turn_event_log: TurnEventLog::default(),
        last_message_id: None,
        post_turn_base_untrusted_message_id: None,
        task_id_map: HashMap::new(),
        pending_messages: VecDeque::new(),
        current_permission_mode: "edit".to_string(),
        available_models: Vec::new(),
        selected_model: None,
        stale_timeout: Duration::from_secs(180),
        last_result_token_usage: None,
        current_turn_stop_reason: None,
        latest_token_usage: None,
        pending_stream_parts: Vec::new(),
        pending_stream_part_rollbacks: Vec::new(),
        retry_stream_delta: None,
        pending_stream_bytes: 0,
        streaming_delta_seq: 0,
        streaming_delta_seq_by_message: HashMap::new(),
        pending_persisted_tool_output_resyncs: HashMap::new(),
        last_stream_emit_at: None,
        streaming_timer_active: false,
        last_progress_at: None,
        turn_phase_since: Instant::now(),
        turn_seq: 0,
        turn_watchdog_active: false,
    }
}

#[cfg(all(test, unix))]
pub(crate) fn test_echo_command() -> tokio::process::Command {
    tokio::process::Command::new("cat")
}

#[cfg(all(test, windows))]
pub(crate) fn test_echo_command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("cmd");
    command.args(["/C", "more"]);
    command
}

/// Per-session agent process map: chat_session_id -> AgentProcess
pub type AgentProcessMap = HashMap<String, AgentProcess>;

#[cfg(test)]
mod tests {
    use super::{make_test_agent_process, AgentProcessMap, TurnPhase};
    use crate::usecase::agent_session::event_log::{PromptInput, TurnStopReason};
    use crate::usecase::agent_session::session::MessagePart;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    #[test]
    fn process_registry_starts_empty() {
        let map: AgentProcessMap = HashMap::new();

        assert!(map.is_empty());
    }

    #[tokio::test]
    async fn reset_streaming_state_for_new_turn_clears_per_turn_state() {
        let mut proc = make_test_agent_process();
        proc.streaming_parts.push(MessagePart::Text {
            content: "hello".to_string(),
            parent_tool_use_id: None,
        });
        proc.confirmed_stream_part_len = 1;
        proc.turn_event_log.begin_turn(
            1,
            "human-1".to_string(),
            "agent-1".to_string(),
            PromptInput::default(),
            1.0,
        );
        proc.pending_stream_parts = vec![MessagePart::Text {
            content: "pending".to_string(),
            parent_tool_use_id: None,
        }];
        proc.pending_stream_part_rollbacks = vec![super::StreamPartRollback {
            index: 0,
            previous: MessagePart::Text {
                content: "hello".to_string(),
                parent_tool_use_id: None,
            },
        }];
        proc.pending_stream_bytes = 128;
        proc.streaming_delta_seq = 9;
        proc.streaming_delta_seq_by_message
            .insert("agent-1".to_string(), 9);
        proc.last_stream_emit_at = Some(Instant::now());
        proc.last_message_id = Some("agent-1".to_string());
        proc.post_turn_message_token = Some("turn-token".to_string());
        proc.post_turn_base_untrusted_message_id = Some("agent-1".to_string());
        proc.task_id_map
            .insert("task-1".to_string(), "tool-1".to_string());
        proc.current_turn_stop_reason = Some(TurnStopReason::Refusal);

        proc.reset_streaming_state_for_new_turn();

        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.confirmed_stream_part_len, 0);
        assert_eq!(proc.turn_event_log.current_turn_id(), None);
        assert!(proc.pending_stream_parts.is_empty());
        assert!(proc.pending_stream_part_rollbacks.is_empty());
        assert!(proc.retry_stream_delta.is_none());
        assert_eq!(proc.pending_stream_bytes, 0);
        assert_eq!(proc.streaming_delta_seq, 0);
        assert_eq!(proc.streaming_delta_seq_by_message.get("agent-1"), Some(&9));
        assert_eq!(proc.last_stream_emit_at, None);
        assert_eq!(proc.last_message_id, None);
        assert_eq!(proc.post_turn_message_token, None);
        assert_eq!(proc.post_turn_base_untrusted_message_id, None);
        assert!(proc.task_id_map.is_empty());
        assert_eq!(proc.current_turn_stop_reason, None);
    }

    #[tokio::test]
    async fn begin_turn_liveness_increments_sequence_and_sets_progress_timestamp() {
        let mut proc = make_test_agent_process();
        proc.turn_seq = 41;
        let previous = Instant::now() - Duration::from_secs(1);
        proc.turn_phase_since = previous;
        proc.last_progress_at = Some(previous);

        proc.begin_turn_liveness();

        assert_eq!(proc.turn_seq, 42);
        assert_eq!(proc.last_progress_at, Some(proc.turn_phase_since));
        assert!(proc.turn_phase_since > previous);
    }

    #[tokio::test]
    async fn touch_liveness_updates_last_progress_timestamp() {
        let mut proc = make_test_agent_process();
        let previous = Instant::now() - Duration::from_secs(1);
        proc.last_progress_at = Some(previous);

        proc.touch_liveness();

        assert!(proc
            .last_progress_at
            .is_some_and(|updated| updated > previous));
    }

    #[tokio::test]
    async fn mark_turn_phase_since_now_updates_timestamp_without_changing_phase() {
        let mut proc = make_test_agent_process();
        proc.turn_phase = TurnPhase::WaitingPermission;
        let previous = Instant::now() - Duration::from_secs(1);
        proc.turn_phase_since = previous;

        proc.mark_turn_phase_since_now();

        assert!(proc.turn_phase_since > previous);
        assert_eq!(proc.turn_phase, TurnPhase::WaitingPermission);
    }
}
impl AgentProcess {
    /// Reset per-turn streaming state (cumulative parts, coalescing buffer,
    /// last-emit timestamp, retained message id, task id map). Called on every
    /// path that begins a new agent turn so the coalescer doesn't carry over
    /// residue from the previous turn — e.g. a stale `last_stream_emit_at`
    /// would block the first emit of the new turn from firing immediately.
    pub(crate) fn reset_streaming_state_for_new_turn(&mut self) {
        self.streaming_parts.clear();
        self.confirmed_stream_part_len = 0;
        self.turn_event_log.clear();
        self.pending_stream_parts.clear();
        self.pending_stream_part_rollbacks.clear();
        self.retry_stream_delta = None;
        self.pending_stream_bytes = 0;
        self.streaming_delta_seq = 0;
        self.pending_persisted_tool_output_resyncs.clear();
        self.last_stream_emit_at = None;
        self.last_message_id = None;
        self.post_turn_message_token = None;
        self.post_turn_base_untrusted_message_id = None;
        self.task_id_map.clear();
        self.current_turn_stop_reason = None;
    }

    pub(crate) fn begin_turn_liveness(&mut self) {
        let now = Instant::now();
        self.turn_seq = self.turn_seq.saturating_add(1);
        self.last_progress_at = Some(now);
        self.turn_phase_since = now;
    }

    pub(crate) fn touch_liveness(&mut self) {
        self.last_progress_at = Some(Instant::now());
    }

    pub(crate) fn mark_turn_phase_since_now(&mut self) {
        self.turn_phase_since = Instant::now();
    }
}
#[cfg(test)]
mod moved_tests {

    use super::super::process_registry::*;

    #[test]
    fn agent_process_map_starts_empty() {
        let map = AgentProcessMap::new();
        assert!(map.is_empty());
    }

    #[test]
    fn bridge_state_transitions() {
        let state = BridgeState::Initializing;
        assert_eq!(state, BridgeState::Initializing);
        assert_ne!(state, BridgeState::Ready);
        assert_ne!(state, BridgeState::Streaming);
        assert_ne!(state, BridgeState::Crashed);
    }
}
