use crate::infrastructure::agent_session::runtime::AgentEditorContext;
use crate::infrastructure::agent_session::runtime::ImageAttachment;
use crate::infrastructure::agent_session::runtime::ModelInfo;
use crate::usecase::agent_session::event_log::TurnEventLog;
use crate::usecase::agent_session::session::ContextCarryState;
use crate::usecase::agent_session::session::MessagePart;
use crate::usecase::agent_session::session::TokenUsage;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::time::Instant;
use tokio::process::Child;
use tokio::process::ChildStdin;

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
    pub stdin: ChildStdin,
    pub backend_id: String,
    pub state: BridgeState,
    pub turn_phase: TurnPhase,
    pub sdk_session_id: Option<String>,
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
    /// Token for the most recent normally completed turn. Kept only while
    /// `last_message_id` can accept post-turn background task updates.
    pub(crate) post_turn_message_token: Option<String>,
    pub streaming_parts: Vec<MessagePart>,
    /// Per-turn durable event buffer. This is the fact stream used by projection;
    /// `streaming_parts` remains only as the existing cumulative live buffer.
    pub turn_event_log: TurnEventLog,
    /// Retained after turn_complete so post-turn background task events
    /// can still be accumulated and emitted via `agent-streaming-updated`.
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
    /// Token usage from the latest `result` message (extracted from modelUsage).
    /// Consumed by turn_complete handler and passed to the workflow runtime usecase.
    pub last_result_token_usage: Option<(u64, u64)>,
    /// Token usage from the latest SDK result, retained for desktop status display.
    pub latest_token_usage: Option<TokenUsage>,
    /// Count of streaming delta parts queued since the last successful emit.
    /// Acts as the dirty signal for coalescing — `> 0` means a flush is owed.
    /// The actual payload lives in `streaming_parts` (cumulative); this field
    /// only tracks how many entries have been added since the last flush so we
    /// can detect the count threshold and decide when there's work to do.
    pub(crate) pending_stream_part_count: usize,
    /// Accumulated payload bytes for parts queued since the last successful
    /// emit. Used to decide whether to flush early when the byte cap is
    /// reached. Mirrors `pending_stream_part_count` semantically — count and
    /// bytes are the only state we need; the delta entries themselves remain
    /// in `streaming_parts`.
    pub(crate) pending_stream_bytes: usize,
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

#[cfg(test)]
pub(crate) fn make_test_agent_process() -> AgentProcess {
    let mut child = tokio::process::Command::new("cat")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn cat test process");
    let stdin = child.stdin.take().expect("stdin");
    AgentProcess {
        stdin,
        backend_id: "mock".to_string(),
        state: BridgeState::Ready,
        turn_phase: TurnPhase::Idle,
        sdk_session_id: None,
        context_carry_on_ready: None,
        child,
        generation_id: 0,
        #[cfg(unix)]
        pgid: None,
        streaming_message_id: None,
        active_turn_token: None,
        post_turn_message_token: None,
        streaming_parts: Vec::new(),
        turn_event_log: TurnEventLog::default(),
        last_message_id: None,
        post_turn_base_untrusted_message_id: None,
        task_id_map: HashMap::new(),
        pending_messages: VecDeque::new(),
        current_permission_mode: "edit".to_string(),
        available_models: Vec::new(),
        selected_model: None,
        last_result_token_usage: None,
        latest_token_usage: None,
        pending_stream_part_count: 0,
        pending_stream_bytes: 0,
        last_stream_emit_at: None,
        streaming_timer_active: false,
        last_progress_at: None,
        turn_phase_since: Instant::now(),
        turn_seq: 0,
        turn_watchdog_active: false,
    }
}

/// Per-session agent process map: chat_session_id -> AgentProcess
pub type AgentProcessMap = HashMap<String, AgentProcess>;

#[cfg(test)]
mod tests {
    use super::{make_test_agent_process, AgentProcessMap, TurnPhase};
    use crate::usecase::agent_session::event_log::PromptInput;
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
        proc.turn_event_log.begin_turn(
            1,
            "human-1".to_string(),
            "agent-1".to_string(),
            PromptInput::default(),
            1.0,
        );
        proc.pending_stream_part_count = 3;
        proc.pending_stream_bytes = 128;
        proc.last_stream_emit_at = Some(Instant::now());
        proc.last_message_id = Some("agent-1".to_string());
        proc.post_turn_message_token = Some("turn-token".to_string());
        proc.post_turn_base_untrusted_message_id = Some("agent-1".to_string());
        proc.task_id_map
            .insert("task-1".to_string(), "tool-1".to_string());

        proc.reset_streaming_state_for_new_turn();

        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.turn_event_log.current_turn_id(), None);
        assert_eq!(proc.pending_stream_part_count, 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        assert_eq!(proc.last_stream_emit_at, None);
        assert_eq!(proc.last_message_id, None);
        assert_eq!(proc.post_turn_message_token, None);
        assert_eq!(proc.post_turn_base_untrusted_message_id, None);
        assert!(proc.task_id_map.is_empty());
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
        self.turn_event_log.clear();
        self.pending_stream_part_count = 0;
        self.pending_stream_bytes = 0;
        self.last_stream_emit_at = None;
        self.last_message_id = None;
        self.post_turn_message_token = None;
        self.post_turn_base_untrusted_message_id = None;
        self.task_id_map.clear();
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
