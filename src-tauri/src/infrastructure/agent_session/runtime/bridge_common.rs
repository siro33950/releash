use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

static GENERATION_COUNTER: AtomicU64 = AtomicU64::new(1);

use serde::{Deserialize, Serialize};
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;

use crate::app_data_dir::resolve_data_dir;
use crate::infrastructure::agent_session::runtime::context_restore::{
    context_restore_plan_for_session, context_restore_plan_for_session_before_turn,
    context_restore_plan_from_meta, ContextRestorePlan, RestoreContextPayload,
};
use crate::infrastructure::agent_session::runtime::runtime_coordinator::{
    acquire_spawn_session_guard, clear_pending_turn_starting, clear_session_closing,
    is_pending_turn_starting, mark_pending_turn_starting, mark_session_closing,
    prune_session_runtime_lock, wait_until_session_close_finished,
};
use crate::infrastructure::agent_session::runtime::{
    AgentEditorContext, AgentMessage, BackendRuntimeConfig, ImageAttachment, ModelInfo,
    SessionConfig, SessionHandle,
};
#[cfg(test)]
use crate::usecase::agent_session::session::create_session_internal;
#[cfg(test)]
use crate::usecase::agent_session::session::image_attachment::detect_image_mime;
use crate::usecase::agent_session::session::validate_image_bytes;
use crate::usecase::agent_session::session::{
    add_message_internal, now_timestamp, parts_to_legacy, ChatMessage, ChatSession,
    ContextCarryState, GetSessionResponse, InitialSessionPage, MessagePart, MessageRole,
    PageCursor, SessionMeta, SessionPage, SessionStore, SessionSummary, SystemNotificationType,
    TokenUsage, INITIAL_SESSION_PAGE_LIMIT,
};

pub(crate) use crate::infrastructure::agent_session::runtime::runtime_coordinator::acquire_session_runtime_lock;

pub const CLAUDE_BACKEND_ID: &str = "claude";
pub const CODEX_BACKEND_ID: &str = "codex";
pub(crate) const DEFER_AGENT_SESSION_ID_PERSIST_ON_READY: &str =
    "defer_agent_session_id_persist_on_ready";

use crate::usecase::agent_session::session::errors::session_target_rejected;

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
    pub streaming_parts: Vec<MessagePart>,
    /// Retained after turn_complete so post-turn background task events
    /// can still be accumulated and emitted via `agent-streaming-updated`.
    pub last_message_id: Option<String>,
    /// Message id whose store-backed parts cannot be trusted as a post-turn
    /// base because the latest full-message persist is pending or failed.
    post_turn_base_untrusted_message_id: Option<String>,
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
    pending_stream_part_count: usize,
    /// Accumulated payload bytes for parts queued since the last successful
    /// emit. Used to decide whether to flush early when the byte cap is
    /// reached. Mirrors `pending_stream_part_count` semantically — count and
    /// bytes are the only state we need; the delta entries themselves remain
    /// in `streaming_parts`.
    pending_stream_bytes: usize,
    /// Timestamp of the most recent successful streaming emit. `None` means
    /// the first emit for this turn — flush immediately.
    last_stream_emit_at: Option<Instant>,
    /// True while a per-turn auxiliary streaming-flush timer task is alive.
    /// Set when the timer is spawned at streaming start; cleared by the timer
    /// itself when it exits (turn ended and the buffer drained). Used to
    /// avoid spawning a duplicate timer on overlapping turn starts.
    streaming_timer_active: bool,
    /// Last meaningful SDK/bridge progress observed for the current turn.
    pub last_progress_at: Option<Instant>,
    /// Timestamp of the current turn phase; used as the streaming stale fallback.
    pub turn_phase_since: Instant,
    /// Monotonic per-process turn sequence. Watchdogs capture it to avoid
    /// acting on a later turn that reused the same bridge process.
    pub turn_seq: u64,
    /// True while a per-turn stale watchdog task is alive.
    turn_watchdog_active: bool,
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
        streaming_parts: Vec::new(),
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

/// drain 時に永続化する人間メッセージの parts を pending から構築する。
/// 画像が無ければ None（content は add_message_internal が text として扱う）。
fn pending_human_parts(pending: &PendingMessage) -> Option<Vec<MessagePart>> {
    if pending.images.is_empty() {
        return None;
    }
    let mut p: Vec<MessagePart> = Vec::new();
    if !pending.content.is_empty() {
        p.push(MessagePart::Text {
            content: pending.content.clone(),
            parent_tool_use_id: None,
        });
    }
    for img in &pending.images {
        p.push(MessagePart::Image {
            data: img.data.clone(),
            media_type: img.media_type.clone(),
        });
    }
    Some(p)
}

fn pending_message_to_queued_turn(
    pending: &PendingMessage,
) -> crate::usecase::agent_session::session::QueuedAgentTurn {
    const PREVIEW_MAX_CHARS: usize = 160;
    let mut preview: String = pending.content.chars().take(PREVIEW_MAX_CHARS).collect();
    if pending.content.chars().count() > PREVIEW_MAX_CHARS {
        preview.push_str("...");
    }
    crate::usecase::agent_session::session::QueuedAgentTurn {
        id: pending.id.clone(),
        content_preview: preview,
        created_at: pending.created_at,
        permission_mode: pending.permission_mode.clone(),
        image_count: pending.images.len(),
    }
}

fn pending_existing_turn_ids(pending: &PendingMessage) -> Option<(&str, &str)> {
    Some((
        pending.existing_human_message_id.as_deref()?,
        pending.existing_agent_message_id.as_deref()?,
    ))
}

fn pending_queue_view(
    proc: &AgentProcess,
) -> Vec<crate::usecase::agent_session::session::QueuedAgentTurn> {
    proc.pending_messages
        .iter()
        .map(pending_message_to_queued_turn)
        .collect()
}

fn token_usage_from_result_message(msg: &serde_json::Value) -> Option<TokenUsage> {
    let model_usage = msg.get("modelUsage").and_then(|v| v.as_object())?;
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut total_tokens: u64 = 0;
    let mut saw_explicit_total = false;
    let mut context_window_tokens: Option<u64> = None;

    for usage in model_usage.values() {
        let input = usage
            .get("inputTokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output = usage
            .get("outputTokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        input_tokens += input;
        output_tokens += output;
        if let Some(total) = usage.get("totalTokens").and_then(|v| v.as_u64()) {
            total_tokens += total;
            saw_explicit_total = true;
        }
        if let Some(window) = usage.get("contextWindowTokens").and_then(|v| v.as_u64()) {
            context_window_tokens =
                Some(context_window_tokens.map_or(window, |current| current.max(window)));
        }
    }

    if input_tokens == 0 && output_tokens == 0 && !saw_explicit_total {
        return None;
    }

    Some(TokenUsage {
        input_tokens,
        output_tokens,
        total_tokens: Some(if saw_explicit_total {
            total_tokens
        } else {
            input_tokens + output_tokens
        }),
        context_window_tokens,
    })
}

pub(crate) async fn is_agent_step_runtime_busy(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
) -> bool {
    if is_pending_turn_starting(chat_session_id).await {
        return true;
    }
    let map = handles.lock().await;
    map.get(chat_session_id).is_some_and(|proc| {
        proc.state == BridgeState::Initializing
            || proc.state == BridgeState::Streaming
            || proc.turn_phase == TurnPhase::WaitingPermission
            || !proc.pending_messages.is_empty()
    })
}

#[cfg(test)]
async fn agent_session_has_pending_message(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
) -> bool {
    let map = handles.lock().await;
    map.get(chat_session_id)
        .is_some_and(|proc| !proc.pending_messages.is_empty())
}

pub(crate) fn bridge_script_names(
    backend_id: &str,
) -> Result<(&'static str, &'static str), String> {
    match backend_id {
        CODEX_BACKEND_ID => Err(
            "Codex uses codex app-server directly; the legacy Node bridge is disabled".to_string(),
        ),
        _ => Ok((
            "claude-sdk-bridge.mjs",
            "generated/bridges/claude-sdk-bridge.bundled.mjs",
        )),
    }
}

pub(crate) fn dev_bridge_path(backend_id: &str) -> Result<PathBuf, String> {
    let (dev_name, _) = bridge_script_names(backend_id)?;
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join(dev_name))
}

pub(crate) fn resolve_bridge_script<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    backend_id: &str,
) -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    {
        let dev_path = dev_bridge_path(backend_id)?;
        if dev_path.exists() {
            return Ok(dev_path);
        }
    }

    let (_, bundled_name) = bridge_script_names(backend_id)?;
    app.path()
        .resource_dir()
        .map(|d| d.join(bundled_name))
        .map_err(|e| format!("Failed to resolve resource dir: {e}"))
}

pub(crate) async fn write_bridge_command(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    payload: serde_json::Value,
) -> Result<(), String> {
    let data = format!("{payload}\n");
    let mut map = handles.lock().await;
    let proc = map
        .get_mut(chat_session_id)
        .ok_or_else(|| format!("No active agent process for session {chat_session_id}"))?;
    proc.stdin
        .write_all(data.as_bytes())
        .await
        .map_err(|e| format!("Failed to write bridge command: {e}"))?;
    proc.stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush bridge command: {e}"))?;
    Ok(())
}

async fn write_bridge_command_for_captured_turn(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    captured_gen_id: u64,
    captured_turn_seq: u64,
    payload: serde_json::Value,
) -> Result<bool, String> {
    let data = format!("{payload}\n");
    let mut map = handles.lock().await;
    let Some(proc) = map.get_mut(chat_session_id) else {
        return Ok(false);
    };
    if proc.generation_id != captured_gen_id || proc.turn_seq != captured_turn_seq {
        return Ok(false);
    }
    proc.stdin
        .write_all(data.as_bytes())
        .await
        .map_err(|e| format!("Failed to write bridge command: {e}"))?;
    proc.stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush bridge command: {e}"))?;
    Ok(true)
}

const PERSIST_INTERVAL_MS: u64 = 1000;
const BRIDGE_EOF_ERROR_MESSAGE: &str = "Bridge process exited unexpectedly.";
pub(crate) const STALE_EXIT_CODE: i64 = 124;
const STALE_TIMEOUT_SECS: u64 = 180;
const STALE_RECOVERY_GRACE_SECS: u64 = 10;
const WATCHDOG_TICK_SECS: u64 = 5;
const STALE_ERROR_MESSAGE: &str = "Claude 応答が停止したため中断しました。もう一度お試しください。";

/// Aggregation interval for `agent-streaming-updated` / `AgentStreamSync`.
/// Roughly 30fps — balances UI smoothness against re-render cost.
const STREAMING_EMIT_INTERVAL_MS: u64 = 33;

/// Maximum number of pending delta parts before we flush early.
/// Acts as a flush threshold in normal operation, and as a soft cap
/// (we keep accepting parts even past this) while delivery is failing.
const STREAMING_PENDING_PART_LIMIT: usize = 1000;

/// Maximum cumulative byte size of pending delta payloads before we flush early.
/// Same semantics as `STREAMING_PENDING_PART_LIMIT`: flush threshold in normal
/// operation, soft cap (allowed to overflow) while delivery is failing.
const STREAMING_PENDING_BYTE_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnLivenessTimeout {
    Stale,
}

impl TurnLivenessTimeout {
    fn user_message(self) -> &'static str {
        match self {
            Self::Stale => STALE_ERROR_MESSAGE,
        }
    }
}

fn evaluate_turn_liveness(
    turn_phase: TurnPhase,
    last_progress_at: Option<Instant>,
    turn_phase_since: Instant,
    now: Instant,
) -> Option<TurnLivenessTimeout> {
    match turn_phase {
        TurnPhase::Streaming => {
            let base = last_progress_at.unwrap_or(turn_phase_since);
            (now.duration_since(base) > Duration::from_secs(STALE_TIMEOUT_SECS))
                .then_some(TurnLivenessTimeout::Stale)
        }
        TurnPhase::Idle | TurnPhase::WaitingPermission => None,
    }
}

fn backend_runtime_config<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    backend_id: &str,
) -> BackendRuntimeConfig {
    app.try_state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>()
        .and_then(
            |registry| match registry.runtime_config_for(backend_id, app) {
                Ok(config) => Some(config),
                Err(e) => {
                    log::warn!("backend '{backend_id}' runtime config could not be resolved: {e}");
                    None
                }
            },
        )
        .unwrap_or_default()
}

/// 指定 backend の登録モデル一覧を config.toml から取得する。
///
/// - registry 未指定（テスト等）: `Ok(Vec::new())`
/// - registry の lookup が失敗（config 未紐付け／schema 未対応／lock 失敗）: `Err`
///
/// 「登録済みモデルが 0 件」と infrastructure 故障を呼び出し側で区別できるよう
/// 必ず Err を伝播する。表示専用経路は warn + 既存値維持で空上書きを防ぐこと、
/// 永続化に絡む経路は Err をそのまま呼び出し元に伝えること。
fn available_models_for_backend(
    backend_id: &str,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
) -> Result<Vec<ModelInfo>, String> {
    match registry {
        Some(registry) => registry.available_models(backend_id),
        None => Ok(Vec::new()),
    }
}

/// 既存セッションの `selected_model` を「常に非 null」へ解決する lazy migration ヘルパ。
///
/// モデル「未選択（None）」状態は廃止されたが、`ChatSession.selected_model` の永続化型は
/// 既存 JSON 互換のため `Option<String>` のまま。応答・Bridge 送信時に `None` を backend の
/// 既定モデル（[`crate::infrastructure::agent_session::runtime::AgentBackendRegistry::default_model_for`]）へ解決してから使う。
///
/// - `Some(model)`: そのまま採用する。
/// - `None` + registry あり: 既定モデルに解決する。registry 取得失敗時は warn を残し `None` を返す
///   （表示専用・emit 経路で UI 描画を妨げないため）。
/// - `None` + registry なし（テスト等）: `None` のまま。
fn resolve_selected_model(
    selected_model: Option<String>,
    backend_id: &str,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
) -> Option<String> {
    if selected_model.is_some() {
        return selected_model;
    }
    let registry = registry?;
    match registry.default_model_for(backend_id) {
        Ok(model) => Some(model),
        Err(e) => {
            log::warn!("selected_model の既定解決に失敗（backend '{backend_id}'）: {e}");
            None
        }
    }
}

/// 応答（[`GetSessionResponse`]）向けの厳格な `selected_model` 解決。
///
/// 契約: `GetSessionResponse.selected_model`（`ChatSession` から flatten）は常に非 null で
/// シリアライズされる。`ChatSession.selected_model` は `skip_serializing_if = Option::is_none`
/// のため、`None` のまま応答へ載せると JSON からフィールドが脱落し、フロントの必須 `string`
/// 契約に反する。registry が与えられている本番経路では既定モデルへ解決できない場合に `Err` を
/// 返し、フィールド脱落を防ぐ。registry 未指定（テスト等）では `None` のままとし、緩い
/// [`resolve_selected_model`] と挙動を合わせる。
fn resolve_selected_model_for_response(
    selected_model: Option<String>,
    backend_id: &str,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
) -> Result<Option<String>, String> {
    if selected_model.is_some() {
        return Ok(selected_model);
    }
    let Some(registry) = registry else {
        return Ok(None);
    };
    registry.default_model_for(backend_id).map(Some)
}

impl AgentProcess {
    /// Reset per-turn streaming state (cumulative parts, coalescing buffer,
    /// last-emit timestamp, retained message id, task id map). Called on every
    /// path that begins a new agent turn so the coalescer doesn't carry over
    /// residue from the previous turn — e.g. a stale `last_stream_emit_at`
    /// would block the first emit of the new turn from firing immediately.
    fn reset_streaming_state_for_new_turn(&mut self) {
        self.streaming_parts.clear();
        self.pending_stream_part_count = 0;
        self.pending_stream_bytes = 0;
        self.last_stream_emit_at = None;
        self.last_message_id = None;
        self.post_turn_base_untrusted_message_id = None;
        self.task_id_map.clear();
    }

    fn begin_turn_liveness(&mut self) {
        let now = Instant::now();
        self.turn_seq = self.turn_seq.saturating_add(1);
        self.last_progress_at = Some(now);
        self.turn_phase_since = now;
    }

    fn touch_liveness(&mut self) {
        self.last_progress_at = Some(Instant::now());
    }

    fn mark_turn_phase_since_now(&mut self) {
        self.turn_phase_since = Instant::now();
    }

    /// Write setMode commands to the Bridge stdin before a turn starts.
    async fn sync_pre_turn_settings(&mut self, permission_mode: &str) -> Result<(), String> {
        let mode_data = build_set_mode_command_for_backend(permission_mode, &self.backend_id)?;
        self.stdin
            .write_all(mode_data.as_bytes())
            .await
            .map_err(|e| format!("Failed to write setMode: {e}"))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush setMode: {e}"))?;

        Ok(())
    }
}

fn release_completed_turn_streaming_buffer(proc: &mut AgentProcess) -> Vec<MessagePart> {
    let released_parts = std::mem::take(&mut proc.streaming_parts);
    proc.pending_stream_part_count = 0;
    proc.pending_stream_bytes = 0;
    proc.last_stream_emit_at = None;
    released_parts
}

fn mark_post_turn_store_base_untrusted(proc: &mut AgentProcess, message_id: &str) {
    proc.post_turn_base_untrusted_message_id = Some(message_id.to_string());
}

fn clear_post_turn_store_base_untrusted_after_persist_success(
    proc: &mut AgentProcess,
    message_id: &str,
) {
    if proc.post_turn_base_untrusted_message_id.as_deref() == Some(message_id) {
        proc.post_turn_base_untrusted_message_id = None;
    }
}

async fn clear_post_turn_store_base_untrusted_for_message(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    message_id: &str,
) {
    let mut map = handles.lock().await;
    if let Some(proc) = map.get_mut(chat_session_id) {
        clear_post_turn_store_base_untrusted_after_persist_success(proc, message_id);
    }
}

#[cfg(unix)]
fn pids_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("pids")
}

#[cfg(unix)]
fn validate_session_id_for_path(chat_session_id: &str) -> Result<(), String> {
    if chat_session_id.is_empty()
        || chat_session_id.contains('/')
        || chat_session_id.contains('\\')
        || chat_session_id.contains("..")
        || chat_session_id.contains('\0')
    {
        return Err(format!(
            "Invalid chat_session_id for PID file: {chat_session_id:?}"
        ));
    }
    Ok(())
}

/// On-disk representation of a PID file (v1).
///
/// Records the bridge process group, plus identification of the Releash app
/// instance that owns it. `cleanup_orphan_processes` uses `owner_app_pid` and
/// `owner_start_time` to distinguish "left over from a previous crash of this
/// app instance" from "currently owned by another live Releash instance" — the
/// latter must not be touched (issue #1024).
#[cfg(unix)]
#[derive(Serialize, Deserialize)]
struct PidFileV1 {
    version: u32,
    pgid: i32,
    owner_app_pid: u32,
    /// Platform-specific start time of `owner_app_pid`. Used to detect PID
    /// reuse: if the recorded `owner_app_pid` is alive but its start time
    /// differs from the recorded value, the PID was recycled and the file is
    /// stale.
    owner_start_time: u64,
}

/// Linux: read field 22 (`starttime`) of `/proc/{pid}/stat`. Value is the
/// process start time expressed in clock ticks since system boot — stable
/// across queries for the same process.
#[cfg(all(unix, target_os = "linux"))]
fn get_process_start_time(pid: u32) -> Result<u64, String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|e| format!("Failed to read /proc/{pid}/stat: {e}"))?;
    // The `comm` field (field 2) can contain spaces and parens; the last ')'
    // marks its end. Fields after are space-separated.
    let rparen = stat
        .rfind(')')
        .ok_or_else(|| format!("Malformed /proc/{pid}/stat: missing ')'"))?;
    let after = stat[rparen + 1..].trim();
    let fields: Vec<&str> = after.split_whitespace().collect();
    // After ')' the next field is `state` (field 3). `starttime` is field 22,
    // so the index into `fields` is 22 - 3 = 19.
    let starttime = fields
        .get(19)
        .ok_or_else(|| format!("/proc/{pid}/stat missing starttime field"))?;
    starttime
        .parse::<u64>()
        .map_err(|e| format!("Failed to parse starttime in /proc/{pid}/stat: {e}"))
}

/// macOS: query `proc_bsdinfo` via `proc_pidinfo(pid, PROC_PIDTBSDINFO, ...)`
/// and combine `pbi_start_tvsec`/`pbi_start_tvusec` into microseconds since
/// epoch. The value is fixed for the lifetime of the process.
#[cfg(all(unix, target_os = "macos"))]
fn get_process_start_time(pid: u32) -> Result<u64, String> {
    use std::mem::MaybeUninit;
    let mut info: MaybeUninit<libc::proc_bsdinfo> = MaybeUninit::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let ret = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr() as *mut libc::c_void,
            size,
        )
    };
    if ret <= 0 {
        return Err(format!(
            "proc_pidinfo(PROC_PIDTBSDINFO) failed for {pid}: {}",
            std::io::Error::last_os_error()
        ));
    }
    if ret < size {
        return Err(format!(
            "proc_pidinfo(PROC_PIDTBSDINFO) returned {ret} bytes, expected {size}"
        ));
    }
    let info = unsafe { info.assume_init() };
    Ok(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}

/// Unsupported Unix flavor: return an error. Callers treat this as "owner
/// identity unverifiable" and conservatively skip cleanup of unfamiliar files.
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn get_process_start_time(_pid: u32) -> Result<u64, String> {
    Err("Unsupported platform for process start_time lookup".to_string())
}

#[cfg(unix)]
fn save_pgid(app_data_dir: &Path, chat_session_id: &str, pgid: u32) -> Result<(), String> {
    validate_session_id_for_path(chat_session_id)?;
    let dir = pids_dir(app_data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create pids dir: {e}"))?;
    let owner_app_pid = std::process::id();
    // start_time が取得できないプラットフォーム/失敗時は 0 を保存する。
    // cleanup 側では 0 を「未検証」として扱い、live owner なら保守的に skip する
    // （bridge spawn そのものを失敗させない: issue #1024）。
    let owner_start_time = get_process_start_time(owner_app_pid).unwrap_or(0);
    let payload = PidFileV1 {
        version: 1,
        pgid: pgid as i32,
        owner_app_pid,
        owner_start_time,
    };
    let json = serde_json::to_string(&payload)
        .map_err(|e| format!("Failed to serialize PID file: {e}"))?;
    let file = dir.join(format!("{chat_session_id}.pid"));
    // Atomic write: tmp + rename. Avoids leaving a half-written file readable
    // by a concurrent cleanup pass.
    let tmp = dir.join(format!("{chat_session_id}.pid.tmp"));
    std::fs::write(&tmp, json).map_err(|e| format!("Failed to write pid file: {e}"))?;
    std::fs::rename(&tmp, &file).map_err(|e| format!("Failed to rename pid file: {e}"))?;
    Ok(())
}

#[cfg(unix)]
fn remove_pgid(app_data_dir: &Path, chat_session_id: &str) {
    if validate_session_id_for_path(chat_session_id).is_err() {
        return;
    }
    let file = pids_dir(app_data_dir).join(format!("{chat_session_id}.pid"));
    let _ = std::fs::remove_file(file);
}

#[cfg(unix)]
pub fn cleanup_orphan_processes(app_data_dir: &Path) {
    let dir = pids_dir(app_data_dir);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return, // No pids dir — nothing to clean up
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "pid") {
            continue;
        }
        let contents = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Failed to read PID file {}: {e}", path.display());
                continue;
            }
        };

        let parsed: PidFileV1 = match serde_json::from_str::<PidFileV1>(contents.trim()) {
            Ok(p) => p,
            Err(_) => {
                // Legacy or unknown format. Conservatively skip — touching it
                // could destroy a live owner's bookkeeping (issue #1024).
                log::warn!(
                    "PID file {} is not in PidFileV1 format; skipping cleanup",
                    path.display()
                );
                continue;
            }
        };

        if parsed.pgid <= 1 {
            log::warn!(
                "Invalid PGID {} in {}, removing file",
                parsed.pgid,
                path.display()
            );
            let _ = std::fs::remove_file(&path);
            continue;
        }

        // Determine whether the recorded owner is still our own previous run
        // (cleanup OK) or a *different* live Releash instance (must skip).
        let owner_pid_i32 = parsed.owner_app_pid as i32;
        let owner_alive = owner_pid_i32 > 1 && unsafe { libc::kill(owner_pid_i32, 0) } == 0;
        if owner_alive {
            match get_process_start_time(parsed.owner_app_pid) {
                Ok(current_start_time) if current_start_time == parsed.owner_start_time => {
                    log::info!(
                        "PID file {} is owned by live instance (pid={}); skipping cleanup",
                        path.display(),
                        parsed.owner_app_pid
                    );
                    continue;
                }
                Ok(_) => {
                    log::info!(
                        "PID file {} owner pid {} appears to have been reused; proceeding with cleanup",
                        path.display(),
                        parsed.owner_app_pid
                    );
                }
                Err(e) => {
                    // live owner だが start_time を検証できない: 保守的に skip
                    // する（unsupported プラットフォームや一時的 I/O 失敗で他
                    // インスタンスの bridge を誤殺しないため: issue #1024）。
                    log::warn!(
                        "Failed to read start_time for owner pid {} of {}: {e}; skipping cleanup",
                        parsed.owner_app_pid,
                        path.display()
                    );
                    continue;
                }
            }
        }

        // Orphan: owner is dead, or PID was reused, or start_time unverifiable.
        let pgid = parsed.pgid;
        let alive = unsafe { libc::killpg(pgid, 0) } == 0;
        if alive {
            log::info!(
                "Cleaning up orphan process group {pgid} from {}",
                path.display()
            );
            unsafe {
                libc::killpg(pgid, libc::SIGTERM);
            }
            // Give processes time to exit, then force kill
            std::thread::sleep(std::time::Duration::from_secs(2));
            let still_alive = unsafe { libc::killpg(pgid, 0) } == 0;
            if still_alive {
                log::warn!("Orphan process group {pgid} did not exit, sending SIGKILL");
                unsafe {
                    libc::killpg(pgid, libc::SIGKILL);
                }
            }
        }
        let _ = std::fs::remove_file(&path);
    }
}

fn persist_streaming_parts<R: tauri::Runtime>(
    session_store: &SessionStore,
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    message_id: &str,
    parts: &[MessagePart],
    completed_at: Option<f64>,
) -> bool {
    let data_dir = match resolve_data_dir(app) {
        Ok(d) => d,
        Err(e) => {
            log::warn!(
                "Failed to resolve data dir for streaming persist (session {chat_session_id}): {e}"
            );
            return false;
        }
    };
    match session_store.persist_message_parts(
        &data_dir,
        chat_session_id,
        message_id,
        parts,
        completed_at,
    ) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("Failed to persist streaming parts for session {chat_session_id}: {e}");
            false
        }
    }
}

fn load_post_turn_base_parts_from_store<R: tauri::Runtime>(
    session_store: &SessionStore,
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    message_id: &str,
) -> Option<Vec<MessagePart>> {
    let data_dir = match resolve_data_dir(app) {
        Ok(d) => d,
        Err(e) => {
            log::warn!(
                "Failed to resolve data dir for post-turn streaming reseed \
                 (session {chat_session_id}, message {message_id}): {e}"
            );
            return None;
        }
    };
    let session = match session_store.load_full_session_for_restore(&data_dir, chat_session_id) {
        Ok(Some(s)) => s,
        Ok(None) => {
            log::warn!(
                "Session not found for post-turn streaming reseed: \
                 session {chat_session_id}, message {message_id}"
            );
            return None;
        }
        Err(e) => {
            log::warn!(
                "Failed to get session for post-turn streaming reseed \
                 (session {chat_session_id}, message {message_id}): {e}"
            );
            return None;
        }
    };
    let Some(message) = session.messages.iter().find(|m| m.id == message_id) else {
        log::warn!(
            "Message not found for post-turn streaming reseed: \
             session {chat_session_id}, message {message_id}"
        );
        return None;
    };
    Some(message.parts.clone().unwrap_or_default())
}

fn emit_session_state_changed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    turn_phase: TurnPhase,
    exit_code: Option<i64>,
) {
    use tauri::Emitter;
    let completed_at = exit_code.map(|_| now_timestamp());
    let _ = app.emit(
        "agent-session-state-changed",
        serde_json::json!({
            "chat_session_id": chat_session_id,
            "turn_phase": turn_phase,
            "exit_code": exit_code,
            "completed_at": completed_at,
        }),
    );
}

/// Emit the cumulative `streaming_parts` payload over both delivery channels.
///
fn to_status_turn_phase(turn_phase: TurnPhase) -> crate::usecase::agent_session::status::TurnPhase {
    match turn_phase {
        TurnPhase::Idle => crate::usecase::agent_session::status::TurnPhase::Idle,
        TurnPhase::Streaming => crate::usecase::agent_session::status::TurnPhase::Streaming,
        TurnPhase::WaitingPermission => {
            crate::usecase::agent_session::status::TurnPhase::WaitingPermission
        }
    }
}

fn to_agent_stream_part_msg(part: MessagePart) -> crate::protocol::AgentStreamPartMsg {
    match part {
        MessagePart::Thinking {
            content,
            parent_tool_use_id,
        } => crate::protocol::AgentStreamPartMsg::Thinking {
            content,
            parent_tool_use_id,
        },
        MessagePart::Text {
            content,
            parent_tool_use_id,
        } => crate::protocol::AgentStreamPartMsg::Text {
            content,
            parent_tool_use_id,
        },
        MessagePart::ToolUse {
            tool,
            input,
            id,
            parent_tool_use_id,
        } => crate::protocol::AgentStreamPartMsg::ToolUse {
            tool,
            input,
            id,
            parent_tool_use_id,
        },
        MessagePart::ToolResult {
            content,
            is_error,
            tool_use_id,
            parent_tool_use_id,
        } => crate::protocol::AgentStreamPartMsg::ToolResult {
            content,
            is_error,
            tool_use_id,
            parent_tool_use_id,
        },
        MessagePart::Error {
            content,
            parent_tool_use_id,
        } => crate::protocol::AgentStreamPartMsg::Error {
            content,
            parent_tool_use_id,
        },
        MessagePart::Permission {
            request,
            status,
            answers,
            parent_tool_use_id,
        } => crate::protocol::AgentStreamPartMsg::Permission {
            request,
            status,
            answers,
            parent_tool_use_id,
        },
        MessagePart::TaskStatus {
            task_tool_use_id,
            status,
            description,
            summary,
        } => crate::protocol::AgentStreamPartMsg::TaskStatus {
            task_tool_use_id,
            status,
            description,
            summary,
        },
        MessagePart::TodoListSnapshot { items } => {
            crate::protocol::AgentStreamPartMsg::TodoListSnapshot {
                items: items
                    .into_iter()
                    .map(|item| crate::protocol::AgentTodoListItemMsg {
                        text: item.text,
                        completed: item.completed,
                    })
                    .collect(),
            }
        }
        MessagePart::SystemNotification {
            notification_type,
            status,
            label,
            detail,
            hook_id,
        } => crate::protocol::AgentStreamPartMsg::SystemNotification {
            notification_type: notification_type.as_str().to_string(),
            status,
            label,
            detail,
            hook_id,
        },
        MessagePart::Image { data, media_type } => {
            crate::protocol::AgentStreamPartMsg::Image { data, media_type }
        }
        MessagePart::ImageRef { attachment } => crate::protocol::AgentStreamPartMsg::ImageRef {
            attachment: crate::protocol::AgentStreamAttachmentRefMsg {
                id: attachment.id,
                media_type: attachment.media_type,
                byte_size: attachment.byte_size,
            },
        },
    }
}

/// Returns `(tauri_ok, ws_ok)`. `tauri_ok` reflects whether the Tauri event
/// dispatcher accepted the payload. `ws_ok` is always `true` on the
/// production broadcaster path: `WsBroadcaster::send_stream_sync` is a
/// best-effort enqueue (slot writes cannot fail and downstream WS transport
/// failure is recovered by the next flush re-sending the cumulative
/// `streaming_parts`). Treating the WS channel as always-true here matches
/// that contract; callers that need to simulate a WS-side failure (unit
/// tests) drive `apply_streaming_emit_result` directly.
fn emit_streaming_parts<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    message_id: &str,
    parts: Vec<MessagePart>,
) -> (bool, bool) {
    use tauri::{Emitter, Manager};
    let payload = serde_json::json!({
        "chat_session_id": chat_session_id,
        "message_id": message_id,
        "parts": parts,
    });
    crate::other::telemetry::record_payload_size(
        crate::other::telemetry::Payload::TauriEvent,
        || {
            serde_json::to_vec(&payload)
                .map(|body| body.len())
                .unwrap_or(0)
        },
    );
    let tauri_ok = app.emit("agent-streaming-updated", &payload).is_ok();
    if let Some(broadcaster) = app.try_state::<Arc<crate::ws_bridge::WsBroadcaster>>() {
        broadcaster.send_stream_sync(crate::protocol::AgentStreamSync {
            session_id: chat_session_id.to_string(),
            message_id: message_id.to_string(),
            parts: parts.into_iter().map(to_agent_stream_part_msg).collect(),
        });
    }
    (tauri_ok, true)
}

/// Estimate the wire byte size contributed by one delta part. Used to decide
/// whether the pending buffer has crossed the byte cap. Exact values aren't
/// required — only proportional growth matters.
fn part_byte_size(part: &MessagePart) -> usize {
    match part {
        MessagePart::Text { content, .. }
        | MessagePart::Thinking { content, .. }
        | MessagePart::Error { content, .. }
        | MessagePart::ToolResult { content, .. } => content.len(),
        MessagePart::ToolUse {
            tool, input, id, ..
        } => tool.len() + id.len() + serde_json::to_string(input).map(|s| s.len()).unwrap_or(0),
        MessagePart::Permission {
            request,
            status,
            answers,
            ..
        } => {
            status.len()
                + serde_json::to_string(request).map(|s| s.len()).unwrap_or(0)
                + answers
                    .as_ref()
                    .and_then(|a| serde_json::to_string(a).ok())
                    .map(|s| s.len())
                    .unwrap_or(0)
        }
        MessagePart::TaskStatus {
            task_tool_use_id,
            status,
            description,
            summary,
        } => {
            task_tool_use_id.len()
                + status.len()
                + description.as_ref().map(|s| s.len()).unwrap_or(0)
                + summary.as_ref().map(|s| s.len()).unwrap_or(0)
        }
        MessagePart::TodoListSnapshot { items } => {
            items.iter().map(|item| item.text.len() + 1).sum()
        }
        MessagePart::SystemNotification {
            notification_type,
            status,
            label,
            detail,
            hook_id,
        } => {
            notification_type.as_str().len()
                + status.len()
                + label.len()
                + detail.as_ref().map(|s| s.len()).unwrap_or(0)
                + hook_id.as_ref().map(|s| s.len()).unwrap_or(0)
        }
        MessagePart::Image { data, media_type } => data.len() + media_type.len(),
        MessagePart::ImageRef { attachment } => {
            attachment.id.len() + attachment.media_type.len() + std::mem::size_of::<u64>()
        }
    }
}

/// Record that delta parts have been queued for the next coalescing flush.
/// We only track the count and total byte size — the actual delta entries
/// remain in `streaming_parts` (cumulative), so storing them twice would
/// just inflate memory for no benefit. The count is the dirty signal; bytes
/// drives the byte-cap flush trigger.
fn enqueue_pending_delta(proc: &mut AgentProcess, delta: &[MessagePart]) {
    for p in delta {
        proc.pending_stream_bytes = proc.pending_stream_bytes.saturating_add(part_byte_size(p));
    }
    proc.pending_stream_part_count = proc.pending_stream_part_count.saturating_add(delta.len());
}

/// True when the pending buffer has crossed either the count or byte threshold.
/// While delivery is succeeding this triggers an immediate flush; while
/// delivery is failing the buffer continues to grow past these thresholds.
fn pending_exceeds_threshold(proc: &AgentProcess) -> bool {
    proc.pending_stream_part_count >= STREAMING_PENDING_PART_LIMIT
        || proc.pending_stream_bytes >= STREAMING_PENDING_BYTE_LIMIT
}

/// True when enough time has elapsed since the last successful emit for the
/// next-delta flush trigger to fire. First emit (no `last_stream_emit_at`)
/// always returns true so the initial chunk reaches the UI without delay.
fn streaming_interval_elapsed(proc: &AgentProcess) -> bool {
    match proc.last_stream_emit_at {
        None => true,
        Some(t) => t.elapsed() >= Duration::from_millis(STREAMING_EMIT_INTERVAL_MS),
    }
}

/// Snapshot of pending-flush bookkeeping captured before an emit attempt.
/// Holds enough metadata to build a failure log without re-reading the
/// process state, and is the source of `apply_streaming_emit_result`.
#[derive(Debug, Clone)]
struct StreamingFlushSnapshot {
    parts: Vec<MessagePart>,
    part_count: usize,
    buffer_len: usize,
    pending_bytes: usize,
}

/// Prepare a streaming flush: snapshot the consolidated cumulative parts and
/// the buffer metadata. Returns `None` when the pending buffer is empty so
/// callers can short-circuit the emit (idle-tick / double-flush no-op).
fn prepare_streaming_flush(proc: &AgentProcess) -> Option<StreamingFlushSnapshot> {
    if proc.pending_stream_part_count == 0 {
        return None;
    }
    let parts = consolidate_parts_from_slice(&proc.streaming_parts);
    Some(StreamingFlushSnapshot {
        part_count: parts.len(),
        buffer_len: proc.pending_stream_part_count,
        pending_bytes: proc.pending_stream_bytes,
        parts,
    })
}

/// Apply the emit result to the coalescing state. On success clears the
/// pending buffer and bumps `last_stream_emit_at`; on failure retains both
/// (so the next flush retries the cumulative payload) and emits a warning
/// log containing only non-body metadata. Returns whether the emit succeeded.
fn apply_streaming_emit_result(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    message_id: &str,
    snapshot: &StreamingFlushSnapshot,
    tauri_ok: bool,
    ws_ok: bool,
) -> bool {
    if tauri_ok && ws_ok {
        proc.pending_stream_part_count = 0;
        proc.pending_stream_bytes = 0;
        let now = Instant::now();
        if let Some(previous) = proc.last_stream_emit_at {
            crate::other::telemetry::record_emit_interval(now.duration_since(previous));
        }
        proc.last_stream_emit_at = Some(now);
        true
    } else {
        // NB: deliberately exclude payload content / tool I/O / mentions —
        // those are external user data and must not appear in logs.
        log::warn!(
            "agent-streaming-updated emit failure: chat_session={} message_id={} \
             part_count={} buffer_len={} pending_bytes={} tauri_ok={} ws_ok={}",
            chat_session_id,
            message_id,
            snapshot.part_count,
            snapshot.buffer_len,
            snapshot.pending_bytes,
            tauri_ok,
            ws_ok
        );
        false
    }
}

/// Run the prepare → emit → apply sequence with a caller-supplied emit
/// function. Extracting this lets unit tests drive the production flush
/// pipeline with a recording emit closure, instead of mirroring the prepare
/// / apply calls inline (which used to drift from the production path).
///
/// The closure receives the cumulative `MessagePart` slice destined for the
/// frontend and returns `(tauri_ok, ws_ok)` matching `emit_streaming_parts`.
fn force_flush_pending_streaming<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    message_id: &str,
    mut emit: F,
) -> bool
where
    F: FnMut(&[MessagePart]) -> (bool, bool),
{
    let Some(snapshot) = prepare_streaming_flush(proc) else {
        return true;
    };
    let (tauri_ok, ws_ok) = emit(&snapshot.parts);
    apply_streaming_emit_result(
        proc,
        chat_session_id,
        message_id,
        &snapshot,
        tauri_ok,
        ws_ok,
    )
}

/// Force-flush pending streaming delta before a turn-phase transition
/// (permission_request, turn_complete, tool boundary, error). Returns
/// `true` when the process was in `Streaming` state so the caller knows to
/// emit a `agent-session-state-changed` notification after releasing the
/// lock. The flush runs FIRST so the frontend never observes a state
/// transition ahead of the tail content for the current message.
///
/// The emit closure mirrors `emit_streaming_parts`: it receives the message
/// id and the consolidated cumulative parts and returns `(tauri_ok, ws_ok)`.
/// Production callers pass a closure that delegates to `emit_streaming_parts`;
/// unit tests pass a recording closure to verify the ordering invariant.
fn flush_streaming_before_transition<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    mut emit_stream: F,
) -> bool
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    let was_streaming = proc.state == BridgeState::Streaming;
    let Some(mid) = proc.streaming_message_id.clone() else {
        return was_streaming;
    };
    let _ = force_flush_pending_streaming(proc, chat_session_id, &mid, |parts| {
        emit_stream(&mid, parts)
    });
    was_streaming
}

/// Effect returned by `run_permission_request_transition_locked`. The caller
/// (production stdout reader / unit tests) inspects `did_transition` to decide
/// whether to emit `agent-session-state-changed(WaitingPermission)` after
/// releasing the process lock.
#[derive(Debug, Default, PartialEq, Eq)]
struct PermissionRequestTransition {
    did_transition: bool,
}

/// Run the in-lock part of the `permission_request` transition: force-flush
/// the pending streaming delta first, then — only when the process was in
/// `Streaming` — promote `turn_phase` to `WaitingPermission`. The flush runs
/// before the state mutation so the frontend never observes a state change
/// ahead of the tail content. The caller is responsible for emitting the
/// state-change notification outside the lock when `did_transition` is true.
fn run_permission_request_transition_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    emit_stream: F,
) -> PermissionRequestTransition
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    let was_streaming = flush_streaming_before_transition(proc, chat_session_id, emit_stream);
    if was_streaming {
        proc.turn_phase = TurnPhase::WaitingPermission;
        proc.mark_turn_phase_since_now();
    }
    PermissionRequestTransition {
        did_transition: was_streaming,
    }
}

/// Effect returned by `run_turn_complete_transition_locked`. Carries the
/// data the caller needs to perform the post-lock follow-ups: state-change
/// emission, message persistence, and workflow hooks. `was_streaming` gates
/// the `agent-session-state-changed(Idle)` emission.
#[derive(Debug, Default)]
struct TurnCompleteTransition {
    was_streaming: bool,
    final_msg_id: Option<String>,
    final_parts: Vec<MessagePart>,
    turn_token_usage: Option<(u64, u64)>,
    released_streaming_parts: Vec<MessagePart>,
}

/// Run the in-lock part of the `turn_complete` transition: force-flush
/// pending streaming delta first, then mutate `state` / `turn_phase` and
/// snapshot the data the caller needs after releasing the lock. Mirrors the
/// production stdout reader so tests can drive the exact same code path
/// (instead of mirroring prepare/apply inline).
fn run_turn_complete_transition_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    exit_code: i64,
    emit_stream: F,
) -> TurnCompleteTransition
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    if proc.turn_phase == TurnPhase::Idle && proc.state != BridgeState::Initializing {
        return TurnCompleteTransition::default();
    }
    let was_streaming = flush_streaming_before_transition(proc, chat_session_id, emit_stream);
    proc.state = if exit_code == 0 {
        BridgeState::Ready
    } else {
        BridgeState::Crashed
    };
    proc.turn_phase = TurnPhase::Idle;
    proc.turn_phase_since = Instant::now();
    proc.last_progress_at = None;
    proc.turn_watchdog_active = false;
    let turn_token_usage = proc.last_result_token_usage.take();
    let final_parts = consolidate_parts_from_slice(&proc.streaming_parts);
    let final_msg_id = proc.streaming_message_id.take();
    if final_msg_id.is_some() {
        proc.last_message_id.clone_from(&final_msg_id);
    }
    if was_streaming && !final_parts.is_empty() {
        if let Some(ref mid) = final_msg_id {
            mark_post_turn_store_base_untrusted(proc, mid);
        }
    }
    let released_streaming_parts = if exit_code != 0 || proc.pending_stream_part_count == 0 {
        release_completed_turn_streaming_buffer(proc)
    } else {
        Vec::new()
    };
    TurnCompleteTransition {
        was_streaming,
        final_msg_id,
        final_parts,
        turn_token_usage,
        released_streaming_parts,
    }
}

fn finalize_turn_as_timeout_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    timeout: TurnLivenessTimeout,
    emit_stream: F,
) -> TurnCompleteTransition
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    if proc.turn_phase == TurnPhase::Idle {
        return TurnCompleteTransition::default();
    }
    let error_part = MessagePart::Error {
        content: timeout.user_message().to_string(),
        parent_tool_use_id: None,
    };
    proc.streaming_parts.push(error_part.clone());
    enqueue_pending_delta(proc, &[error_part]);
    run_turn_complete_transition_locked(proc, chat_session_id, STALE_EXIT_CODE, emit_stream)
}

#[derive(Debug, Default)]
struct BridgeErrorTransition {
    turn_complete: TurnCompleteTransition,
    was_initializing: bool,
    context_restore_failed_on_init: bool,
}

fn run_bridge_error_transition_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    msg: &serde_json::Value,
    emit_stream: F,
) -> BridgeErrorTransition
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    let was_initializing = proc.state == BridgeState::Initializing;
    if proc.state == BridgeState::Streaming {
        // `accumulate_sdk_message` does not synthesize Error parts; add it here
        // so the crash payload carries the error before the turn-complete flush.
        let part = sdk_error_part_from_message(msg);
        proc.streaming_parts.push(part.clone());
        enqueue_pending_delta(proc, std::slice::from_ref(&part));
    }
    let turn_complete = run_turn_complete_transition_locked(proc, chat_session_id, 1, emit_stream);
    let context_restore_failed_on_init = !turn_complete.was_streaming
        && was_initializing
        && proc.context_carry_on_ready.take().is_some();

    BridgeErrorTransition {
        turn_complete,
        was_initializing,
        context_restore_failed_on_init,
    }
}

/// Effect returned by `apply_respond_permission_locked`. `did_transition` is
/// `true` only when the process was actually in `WaitingPermission`; this
/// gates both the post-lock `agent-session-state-changed(Streaming)`
/// emission and the per-turn auxiliary timer restart.
#[derive(Debug, Default, PartialEq, Eq)]
struct PermissionResponseTransition {
    did_transition: bool,
}

/// Run the in-lock part of `respond_agent_permission`: flip `turn_phase`
/// back to `Streaming` (only when actually waiting), patch the matching
/// `Permission` part status in the streaming buffer, enqueue the updated
/// part as a pending delta, and force-flush so the frontend observes the
/// permission decision before the state-change notification. The caller
/// handles stdin write before, and timer restart / state-change emission
/// after.
fn apply_respond_permission_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    request_id: &str,
    behavior: &str,
    answers_value: Option<&serde_json::Value>,
    mut emit_stream: F,
) -> PermissionResponseTransition
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    let did_transition = proc.turn_phase == TurnPhase::WaitingPermission;
    if did_transition {
        proc.turn_phase = TurnPhase::Streaming;
        proc.mark_turn_phase_since_now();
        proc.touch_liveness();
    }
    let new_status = if behavior == "allow" {
        "allowed"
    } else {
        "denied"
    };
    let mut found_part: Option<MessagePart> = None;
    for part in &mut proc.streaming_parts {
        if let MessagePart::Permission {
            request,
            status,
            answers,
            ..
        } = part
        {
            if request.get("request_id").and_then(|v| v.as_str()) == Some(request_id) {
                *status = new_status.to_string();
                if let Some(av) = answers_value {
                    *answers = Some(av.clone());
                }
                found_part = Some(part.clone());
            }
        }
    }
    let emit_msg_id = proc.streaming_message_id.clone();
    if let (Some(mid), Some(part)) = (emit_msg_id, found_part) {
        enqueue_pending_delta(proc, std::slice::from_ref(&part));
        force_flush_pending_streaming(proc, chat_session_id, &mid, |parts| {
            emit_stream(&mid, parts)
        });
    }
    PermissionResponseTransition { did_transition }
}

/// Per-delta flush decision used by the stdout reader. `post_turn` is true
/// when the delta is arriving after `turn_complete` (background-task events
/// piggy-backed on the closed turn) — those are always force-flushed so the
/// post-turn UI does not stall on the throttle.
fn should_flush_per_delta(proc: &AgentProcess, delta: &[MessagePart], post_turn: bool) -> bool {
    let force = post_turn || delta_has_tool_event(delta) || pending_exceeds_threshold(proc);
    force || streaming_interval_elapsed(proc)
}

#[derive(Debug, PartialEq, Eq)]
enum PostTurnBaseRequirement {
    RequiresBase,
    AccumulatedWithoutParts,
    NotAccumulated,
}

/// Keep this classifier in lockstep with `accumulate_sdk_message`; the
/// `post_turn_base_requirement_matches_accumulate_sdk_message` test covers the
/// msg_type/subtype table and should fail when either side drifts.
fn post_turn_base_requirement_for_empty_buffer(
    msg: &serde_json::Value,
    task_id_map: &HashMap<String, String>,
) -> PostTurnBaseRequirement {
    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match msg_type {
        "stream_event" => {
            let Some(delta) = msg
                .get("event")
                .filter(|event| {
                    event.get("type").and_then(|v| v.as_str()) == Some("content_block_delta")
                })
                .and_then(|event| event.get("delta"))
            else {
                return PostTurnBaseRequirement::NotAccumulated;
            };
            match delta.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "text_delta" if delta.get("text").and_then(|v| v.as_str()).is_some() => {
                    PostTurnBaseRequirement::RequiresBase
                }
                "thinking_delta" if delta.get("thinking").and_then(|v| v.as_str()).is_some() => {
                    PostTurnBaseRequirement::RequiresBase
                }
                _ => PostTurnBaseRequirement::NotAccumulated,
            }
        }
        "assistant" => {
            let has_part_change = msg
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(|content| content.as_array())
                .is_some_and(|content| {
                    content.iter().any(|block| {
                        if block.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
                            return false;
                        }
                        if block.get("name").and_then(|v| v.as_str()) == Some("TodoWrite") {
                            let input = block
                                .get("input")
                                .cloned()
                                .unwrap_or(serde_json::Value::Object(Default::default()));
                            extract_todo_items(&input).is_some()
                        } else {
                            true
                        }
                    })
                });
            if has_part_change {
                PostTurnBaseRequirement::RequiresBase
            } else {
                PostTurnBaseRequirement::AccumulatedWithoutParts
            }
        }
        "user" => {
            let has_tool_result = msg
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(|content| content.as_array())
                .is_some_and(|content| {
                    content.iter().any(|block| {
                        block.get("type").and_then(|v| v.as_str()) == Some("tool_result")
                    })
                });
            if has_tool_result {
                PostTurnBaseRequirement::RequiresBase
            } else {
                PostTurnBaseRequirement::AccumulatedWithoutParts
            }
        }
        "todo_list_snapshot" => {
            if extract_todo_items(msg).is_some() {
                PostTurnBaseRequirement::RequiresBase
            } else {
                PostTurnBaseRequirement::AccumulatedWithoutParts
            }
        }
        "permission_denied" | "permission_request" => PostTurnBaseRequirement::RequiresBase,
        "system" => {
            let subtype = msg.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
            match subtype {
                "task_started" | "task_notification" | "task_progress" => {
                    PostTurnBaseRequirement::RequiresBase
                }
                "task_updated" => {
                    let mut tool_use_id = msg
                        .get("tool_use_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if tool_use_id.is_empty() {
                        if let Some(task_id) = msg.get("task_id").and_then(|v| v.as_str()) {
                            if let Some(mapped) = task_id_map.get(task_id) {
                                tool_use_id = mapped.clone();
                            }
                        }
                    }
                    if tool_use_id.is_empty() {
                        PostTurnBaseRequirement::AccumulatedWithoutParts
                    } else {
                        PostTurnBaseRequirement::RequiresBase
                    }
                }
                "init" => PostTurnBaseRequirement::NotAccumulated,
                "compact_boundary" => PostTurnBaseRequirement::RequiresBase,
                "hook_started"
                | "hook_progress"
                | "hook_response"
                | "files_persisted"
                | "local_command_output"
                | "codex_realtime" => PostTurnBaseRequirement::AccumulatedWithoutParts,
                _ => {
                    if msg.get("status").and_then(|v| v.as_str()) == Some("compacting") {
                        PostTurnBaseRequirement::RequiresBase
                    } else {
                        PostTurnBaseRequirement::NotAccumulated
                    }
                }
            }
        }
        // Empty-buffer post-turn errors keep the pre-existing observed behavior:
        // they are forwarded to the dedicated error handler, not persisted as
        // post-turn message parts from this generic accumulation path.
        "error" => PostTurnBaseRequirement::NotAccumulated,
        _ => PostTurnBaseRequirement::NotAccumulated,
    }
}

#[derive(Debug, Default)]
struct AccumulateStreamMessageEffect {
    accumulated: bool,
    emit_msg_id: Option<String>,
    should_persist: bool,
    persist_parts: Vec<MessagePart>,
    post_turn_reseed_message_id: Option<String>,
    start_streaming_timer: bool,
    released_streaming_parts: Vec<MessagePart>,
}

fn accumulate_loaded_post_turn_base_without_streaming_state<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    msg: &serde_json::Value,
    base_mid: String,
    base_parts: Vec<MessagePart>,
    emit_stream: &mut F,
) -> AccumulateStreamMessageEffect
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    let mut parts = base_parts;
    let mut task_id_map = task_id_map_from_parts(&parts);
    let prev_parts = parts.clone();
    let (acc, updated_parts) = accumulate_sdk_message(msg, &mut parts, &mut task_id_map);

    if !acc {
        return AccumulateStreamMessageEffect::default();
    }

    let mut delta: Vec<MessagePart> = parts[prev_parts.len()..].to_vec();
    if let Some(up) = updated_parts {
        delta.extend(up);
    }
    if delta.is_empty() && parts == prev_parts {
        return AccumulateStreamMessageEffect {
            accumulated: true,
            ..AccumulateStreamMessageEffect::default()
        };
    }

    let persist_parts = consolidate_parts_from_slice(&parts);
    let _ = emit_stream(&base_mid, &persist_parts);

    log::warn!(
        "Persisting stale post-turn streaming reseed into loaded base: \
         session {chat_session_id}, loaded message {base_mid}, current message {:?}, state {:?}",
        proc.last_message_id,
        proc.state
    );

    AccumulateStreamMessageEffect {
        accumulated: true,
        emit_msg_id: Some(base_mid),
        should_persist: true,
        persist_parts,
        ..AccumulateStreamMessageEffect::default()
    }
}

fn accumulate_stream_or_post_turn_message_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    msg: &serde_json::Value,
    elapsed_persist_ms: u64,
    mut emit_stream: F,
    post_turn_base: Option<(String, Vec<MessagePart>)>,
) -> AccumulateStreamMessageEffect
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    let in_streaming = proc.state == BridgeState::Streaming && proc.streaming_message_id.is_some();
    let post_turn = !in_streaming && proc.last_message_id.is_some();

    if let Some((base_mid, _)) = post_turn_base.as_ref() {
        if !post_turn || proc.last_message_id.as_deref() != Some(base_mid.as_str()) {
            let (base_mid, base_parts) = post_turn_base.expect("checked post_turn_base");
            return accumulate_loaded_post_turn_base_without_streaming_state(
                proc,
                chat_session_id,
                msg,
                base_mid,
                base_parts,
                &mut emit_stream,
            );
        }
    }

    if !in_streaming && !post_turn {
        return AccumulateStreamMessageEffect::default();
    }

    let mid = if in_streaming {
        proc.streaming_message_id.clone()
    } else {
        proc.last_message_id.clone()
    };

    if post_turn && proc.streaming_parts.is_empty() && post_turn_base.is_none() {
        match post_turn_base_requirement_for_empty_buffer(msg, &proc.task_id_map) {
            PostTurnBaseRequirement::RequiresBase => {}
            PostTurnBaseRequirement::AccumulatedWithoutParts => {
                return AccumulateStreamMessageEffect {
                    accumulated: true,
                    ..AccumulateStreamMessageEffect::default()
                };
            }
            PostTurnBaseRequirement::NotAccumulated => {
                return AccumulateStreamMessageEffect::default();
            }
        }
    }

    if post_turn && proc.streaming_parts.is_empty() {
        let Some(ref mid) = mid else {
            return AccumulateStreamMessageEffect::default();
        };
        if proc.post_turn_base_untrusted_message_id.as_deref() == Some(mid.as_str()) {
            log::warn!(
                "Skipping post-turn streaming update because persisted base is not trusted: \
                 session {chat_session_id}, message {mid}"
            );
            return AccumulateStreamMessageEffect {
                accumulated: true,
                ..AccumulateStreamMessageEffect::default()
            };
        }
    }

    if post_turn && proc.streaming_parts.is_empty() {
        let Some(ref mid) = mid else {
            return AccumulateStreamMessageEffect::default();
        };
        match post_turn_base {
            Some((base_mid, base_parts)) if base_mid == mid.as_str() => {
                proc.streaming_parts = base_parts;
            }
            Some((base_mid, _)) => {
                log::warn!(
                    "Post-turn streaming reseed message mismatch: session {chat_session_id}, \
                     current message {mid}, loaded message {base_mid}"
                );
                return AccumulateStreamMessageEffect {
                    post_turn_reseed_message_id: Some(mid.clone()),
                    ..AccumulateStreamMessageEffect::default()
                };
            }
            None => {
                return AccumulateStreamMessageEffect {
                    post_turn_reseed_message_id: Some(mid.clone()),
                    ..AccumulateStreamMessageEffect::default()
                };
            }
        }
    }

    let prev_len = proc.streaming_parts.len();
    let accumulation =
        accumulate_sdk_message_with_liveness(msg, &mut proc.streaming_parts, &mut proc.task_id_map);
    let acc = accumulation.handled;
    let updated_parts = accumulation.updated_parts;
    if !acc {
        if post_turn {
            let start_streaming_timer = proc.pending_stream_part_count > 0;
            let released_streaming_parts = if !start_streaming_timer {
                release_completed_turn_streaming_buffer(proc)
            } else {
                Vec::new()
            };
            return AccumulateStreamMessageEffect {
                start_streaming_timer,
                released_streaming_parts,
                ..AccumulateStreamMessageEffect::default()
            };
        }
        return AccumulateStreamMessageEffect::default();
    }

    // Refresh the turn-liveness clock so the watchdog (#1178) does not time out
    // an actively streaming turn. Mirrors the pre-refactor inline accumulation.
    if in_streaming && accumulation.liveness {
        proc.touch_liveness();
    }

    let mut delta: Vec<MessagePart> = proc.streaming_parts[prev_len..].to_vec();
    if let Some(up) = updated_parts {
        delta.extend(up);
    }

    if delta.is_empty() {
        if post_turn {
            let start_streaming_timer = proc.pending_stream_part_count > 0;
            let released_streaming_parts = if !start_streaming_timer {
                release_completed_turn_streaming_buffer(proc)
            } else {
                Vec::new()
            };
            return AccumulateStreamMessageEffect {
                accumulated: true,
                start_streaming_timer,
                released_streaming_parts,
                ..AccumulateStreamMessageEffect::default()
            };
        }
        return AccumulateStreamMessageEffect {
            accumulated: true,
            ..AccumulateStreamMessageEffect::default()
        };
    }

    enqueue_pending_delta(proc, &delta);

    if should_flush_per_delta(proc, &delta, post_turn) {
        if let Some(ref mid) = mid {
            let _ = force_flush_pending_streaming(proc, chat_session_id, mid, |parts| {
                emit_stream(mid, parts)
            });
        }
    }

    let should_persist = post_turn || elapsed_persist_ms >= PERSIST_INTERVAL_MS;
    let persist_parts = if should_persist {
        consolidate_parts_from_slice(&proc.streaming_parts)
    } else {
        Vec::new()
    };
    if post_turn && should_persist {
        if let Some(ref mid) = mid {
            mark_post_turn_store_base_untrusted(proc, mid);
        }
    }

    let start_streaming_timer = post_turn && proc.pending_stream_part_count > 0;
    let released_streaming_parts = if post_turn && !start_streaming_timer {
        release_completed_turn_streaming_buffer(proc)
    } else {
        Vec::new()
    };

    AccumulateStreamMessageEffect {
        accumulated: true,
        emit_msg_id: mid,
        should_persist,
        persist_parts,
        post_turn_reseed_message_id: None,
        start_streaming_timer,
        released_streaming_parts,
    }
}

async fn accumulate_stream_or_post_turn_message<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    msg: &serde_json::Value,
    elapsed_persist_ms: u64,
) -> AccumulateStreamMessageEffect {
    let mut post_turn_base: Option<(String, Vec<MessagePart>)> = None;

    for _ in 0..2 {
        let effect = {
            let mut map = handles.lock().await;
            if let Some(proc) = map.get_mut(chat_session_id) {
                let effect = accumulate_stream_or_post_turn_message_locked(
                    proc,
                    chat_session_id,
                    msg,
                    elapsed_persist_ms,
                    |mid, parts| emit_streaming_parts(app, chat_session_id, mid, parts.to_vec()),
                    post_turn_base.take(),
                );
                if effect.start_streaming_timer {
                    spawn_streaming_timer(app, handles, chat_session_id, proc);
                }
                effect
            } else {
                AccumulateStreamMessageEffect::default()
            }
        };

        let Some(message_id) = effect.post_turn_reseed_message_id.clone() else {
            return effect;
        };

        let Some(base_parts) =
            load_post_turn_base_parts_from_store(session_store, app, chat_session_id, &message_id)
        else {
            return AccumulateStreamMessageEffect {
                accumulated: true,
                ..AccumulateStreamMessageEffect::default()
            };
        };
        post_turn_base = Some((message_id, base_parts));
    }

    log::warn!(
        "Post-turn streaming reseed did not stabilize after retry: session {chat_session_id}"
    );
    AccumulateStreamMessageEffect {
        accumulated: true,
        ..AccumulateStreamMessageEffect::default()
    }
}

/// One iteration of the auxiliary timer loop. Bound to a single process by
/// the caller (generation_id / state checks happen above this helper). The
/// emit closure mirrors `force_flush_pending_streaming` so tests can drive
/// the same code path the production timer uses.
///
#[derive(Debug, Default)]
struct StreamingTimerTickEffect {
    keep_running: bool,
    released_streaming_parts: Vec<MessagePart>,
}

/// `keep_running` is `true` when the timer should continue running this turn,
/// and `false` when the loop should exit (turn is over and the buffer has been
/// fully drained).
fn run_streaming_timer_tick<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    mut emit: F,
) -> StreamingTimerTickEffect
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    let pending = proc.pending_stream_part_count > 0;
    let streaming = proc.state == BridgeState::Streaming;
    if !pending && !streaming {
        // Turn ended and the buffer is empty — timer has nothing left to do.
        return StreamingTimerTickEffect {
            keep_running: false,
            released_streaming_parts: release_completed_turn_streaming_buffer(proc),
        };
    }
    if !pending || !streaming_interval_elapsed(proc) {
        return StreamingTimerTickEffect {
            keep_running: true,
            ..StreamingTimerTickEffect::default()
        };
    }
    let Some(mid) = proc
        .streaming_message_id
        .clone()
        .or_else(|| proc.last_message_id.clone())
    else {
        return StreamingTimerTickEffect {
            keep_running: true,
            ..StreamingTimerTickEffect::default()
        };
    };
    let flushed =
        force_flush_pending_streaming(proc, chat_session_id, &mid, |parts| emit(&mid, parts));
    if !streaming && flushed && proc.pending_stream_part_count == 0 {
        return StreamingTimerTickEffect {
            keep_running: false,
            released_streaming_parts: release_completed_turn_streaming_buffer(proc),
        };
    }
    StreamingTimerTickEffect {
        keep_running: true,
        ..StreamingTimerTickEffect::default()
    }
}

/// Returns `true` when any delta part represents a tool invocation boundary.
/// Used to force a flush around tool start/end so the UI never shows a stale
/// frame across these transitions.
fn delta_has_tool_event(delta: &[MessagePart]) -> bool {
    delta.iter().any(|p| {
        matches!(
            p,
            MessagePart::ToolUse { .. } | MessagePart::ToolResult { .. }
        )
    })
}

#[derive(Debug, Default)]
struct BridgeEofCrashTransition {
    turn_complete: TurnCompleteTransition,
    was_initializing: bool,
    sdk_error_message: Option<String>,
    context_restore_failed_on_init: bool,
    /// Ready/Idle EOF means the process is not reusable. Callers can remove it
    /// immediately only when no pending queue needs to survive until respawn.
    should_evict: bool,
}

fn run_bridge_eof_crash_transition_locked<F>(
    generation_matches: bool,
    proc: &mut AgentProcess,
    chat_session_id: &str,
    emit_stream: F,
) -> BridgeEofCrashTransition
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    if !generation_matches {
        return BridgeEofCrashTransition::default();
    }

    let was_streaming = proc.state == BridgeState::Streaming;
    let was_initializing = proc.state == BridgeState::Initializing;
    // Ready/Idle EOF: the turn already completed but the child exited, so the
    // process is no longer reusable and must be evicted before the next send.
    let should_evict = proc.state == BridgeState::Ready && proc.turn_phase == TurnPhase::Idle;
    let sdk_error_message = if was_streaming || was_initializing {
        Some(format!("{}: {BRIDGE_EOF_ERROR_MESSAGE}", proc.backend_id))
    } else {
        None
    };

    if was_streaming {
        let part = MessagePart::Error {
            content: format!("Error: {BRIDGE_EOF_ERROR_MESSAGE}"),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(part.clone());
        enqueue_pending_delta(proc, &[part]);
    }

    let turn_complete = if was_streaming || was_initializing {
        run_turn_complete_transition_locked(proc, chat_session_id, -1, emit_stream)
    } else {
        TurnCompleteTransition::default()
    };
    let context_restore_failed_on_init = !turn_complete.was_streaming
        && was_initializing
        && proc.context_carry_on_ready.take().is_some();

    BridgeEofCrashTransition {
        turn_complete,
        was_initializing,
        sdk_error_message,
        context_restore_failed_on_init,
        should_evict,
    }
}

fn retire_ready_eof_runtime_locked(map: &mut AgentProcessMap, chat_session_id: &str) -> bool {
    let has_pending_messages = map
        .get(chat_session_id)
        .is_some_and(|proc| !proc.pending_messages.is_empty());

    if has_pending_messages {
        if let Some(proc) = map.get_mut(chat_session_id) {
            // Keep the dead process as a non-user-visible respawn marker so
            // ensure_runtime_for_turn can carry pending_messages into the next
            // runtime through the same path as other crashed replacements.
            proc.state = BridgeState::Crashed;
            proc.turn_phase = TurnPhase::Idle;
        }
        false
    } else {
        map.remove(chat_session_id).is_some()
    }
}

fn ready_idle_child_exited(proc: &mut AgentProcess, chat_session_id: &str) -> bool {
    if proc.state != BridgeState::Ready || proc.turn_phase != TurnPhase::Idle {
        return false;
    }

    match proc.child.try_wait() {
        Ok(Some(_status)) => true,
        Ok(None) => false,
        Err(e) => {
            log::warn!("Failed to inspect ready agent process {chat_session_id}: {e}");
            false
        }
    }
}

enum RuntimeSpawnDecision {
    Missing,
    Replace(Box<AgentProcess>),
    Reuse,
}

fn take_runtime_requiring_spawn_locked(
    map: &mut AgentProcessMap,
    chat_session_id: &str,
) -> RuntimeSpawnDecision {
    let Some(proc) = map.get_mut(chat_session_id) else {
        return RuntimeSpawnDecision::Missing;
    };

    let should_replace = if proc.state == BridgeState::Crashed {
        true
    } else if ready_idle_child_exited(proc, chat_session_id) {
        proc.state = BridgeState::Crashed;
        proc.turn_phase = TurnPhase::Idle;
        true
    } else {
        false
    };

    if should_replace {
        RuntimeSpawnDecision::Replace(Box::new(
            map.remove(chat_session_id)
                .expect("runtime existed when replacement was requested"),
        ))
    } else {
        RuntimeSpawnDecision::Reuse
    }
}

/// 状態遷移時に AgentStatusCenter へ通知する統一エントリ。
/// session_store から metadata だけを引いて worktree_path / SessionState を取得する。
/// `session_state_override` を渡すと、ストア値より優先される（Bridge crash 時など）。
pub(crate) fn notify_status_transition<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    turn_phase: TurnPhase,
    session_state_override: Option<crate::usecase::agent_session::session::SessionState>,
) {
    use crate::usecase::agent_session::status::{
        current_timestamp, AgentStatusCenter, SessionStatus, TurnPhaseRepr,
    };

    let data_dir = match resolve_data_dir(app) {
        Ok(d) => d,
        Err(_) => return,
    };
    let meta = match session_store.get_session_meta(&data_dir, chat_session_id) {
        Ok(Some(meta)) => meta,
        _ => return,
    };
    let worktree_path = meta.worktree_path.clone();
    let session_state = session_state_override.unwrap_or_else(|| meta.state.clone());

    let status_turn_phase = to_status_turn_phase(turn_phase);
    let agent_state =
        AgentStatusCenter::derive_agent_state(status_turn_phase, session_state.clone());

    if let Some(center) = app.try_state::<Arc<AgentStatusCenter>>() {
        let (wf_step, wf_state) = center
            .get_session(chat_session_id)
            .map(|s| (s.workflow_step, s.workflow_execution_state))
            .unwrap_or((None, None));
        let status = SessionStatus {
            chat_session_id: chat_session_id.to_string(),
            worktree_id: worktree_path.clone(),
            worktree_path: worktree_path.clone(),
            pty_id: None,
            agent_state: agent_state.clone(),
            turn_phase: TurnPhaseRepr::from(status_turn_phase),
            session_state,
            pending_permission: matches!(turn_phase, TurnPhase::WaitingPermission),
            last_activity_at: current_timestamp(),
            workflow_step: wf_step,
            workflow_execution_state: wf_state,
        };
        let changes = center.update_session(status);
        let broadcaster = app
            .try_state::<Arc<crate::ws_bridge::WsBroadcaster>>()
            .map(|state| state.inner().clone());
        crate::agent_status_events::emit_agent_status_changes(app, broadcaster.as_deref(), changes);
    }
}

const CLOSE_TIMEOUT_SECS: u64 = 5;

fn emit_permission_mode_changed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    mode: &str,
) {
    use tauri::Emitter;
    let _ = app.emit(
        "agent-permission-mode-changed",
        serde_json::json!({
            "chat_session_id": chat_session_id,
            "permission_mode": mode,
        }),
    );
}

/// SDK 由来の `permissionMode` 通知を保存値ベースで処理する。
/// Spec issues-947: 保存値の読み取り失敗時は SDK 値に fallback せず、log::error! を残して
/// runtime/UI を更新せずに通知の処理だけスキップする（保存値が edit/full のセッションを
/// 誤って ask に落とさないため）。後段の `write_bridge_command` 失敗も log に記録する。
async fn handle_sdk_permission_mode_notification<R: tauri::Runtime>(
    sdk_mode: &str,
    app: &tauri::AppHandle<R>,
    session_store: &std::sync::Arc<crate::usecase::agent_session::session::SessionStore>,
    handles: &std::sync::Arc<
        tokio::sync::Mutex<
            crate::infrastructure::agent_session::runtime::bridge_common::AgentProcessMap,
        >,
    >,
    chat_session_id: &str,
) {
    let sdk_abstract =
        crate::infrastructure::agent_session::runtime::permission_flags::mode_from_claude_flag(
            sdk_mode,
        );
    let data_dir = match crate::app_data_dir::resolve_data_dir(app) {
        Ok(dir) => dir,
        Err(e) => {
            log::error!(
                "Failed to resolve data dir for SDK permissionMode notification \
                 (chat_session_id={chat_session_id}): {e}"
            );
            return;
        }
    };
    let saved_meta = match session_store.get_session_meta(&data_dir, chat_session_id) {
        Ok(meta) => meta,
        Err(e) => {
            log::error!(
                "Failed to read saved session metadata for SDK permissionMode notification \
                 (chat_session_id={chat_session_id}): {e}"
            );
            return;
        }
    };
    let Some(meta) = saved_meta else {
        log::error!(
            "Saved session not found for SDK permissionMode notification \
             (chat_session_id={chat_session_id})"
        );
        return;
    };
    let canonical_mode = match crate::permission::PermissionMode::parse(&meta.permission_mode) {
        Ok(mode) => mode,
        Err(e) => {
            log::error!(
                "Saved permission_mode is invalid for SDK permissionMode notification \
                     (chat_session_id={chat_session_id}): {e}"
            );
            return;
        }
    };
    let canonical_str = canonical_mode.as_str();
    let (backend_for_resync, needs_resync) = {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(chat_session_id) {
            proc.current_permission_mode = canonical_str.to_string();
            let backend_id = proc.backend_id.clone();
            let resync = sdk_abstract.is_some_and(|mode| mode != canonical_mode);
            (Some(backend_id), resync)
        } else {
            (None, false)
        }
    };
    emit_permission_mode_changed(app, chat_session_id, canonical_str);
    if !needs_resync {
        return;
    }
    let Some(backend_id) = backend_for_resync else {
        return;
    };
    let payload = build_set_mode_payload_for_mode(canonical_mode, &backend_id);
    if let Err(e) = write_bridge_command(handles, chat_session_id, payload).await {
        log::error!(
            "Failed to resync permission mode to bridge \
             (chat_session_id={chat_session_id}, backend_id={backend_id}): {e}"
        );
    }
}

/// 抽象モード文字列（"ask"/"edit"/"full"）→ バックエンド固有の setMode コマンドを生成する。
/// 対象外の値が渡された場合はエラー（境界で検証済みを前提）。
fn build_set_mode_command_for_backend(
    permission_mode: &str,
    backend_id: &str,
) -> Result<String, String> {
    let pm =
        crate::permission::PermissionMode::parse(permission_mode).map_err(|e| e.to_string())?;
    Ok(build_set_mode_command_for_mode(pm, backend_id))
}

fn build_set_mode_payload_for_mode(
    pm: crate::permission::PermissionMode,
    backend_id: &str,
) -> serde_json::Value {
    let mut payload = serde_json::json!({ "type": "setMode" });
    let obj = payload
        .as_object_mut()
        .expect("setMode payload is an object");
    for (k, v) in bridge_permission_fields(pm, backend_id, false) {
        obj.insert(k, v);
    }
    payload
}

fn build_set_mode_command_for_mode(
    pm: crate::permission::PermissionMode,
    backend_id: &str,
) -> String {
    format!("{}\n", build_set_mode_payload_for_mode(pm, backend_id))
}

/// `(PermissionMode, backend_id)` から JS bridge の init / setMode コマンドに載せる
/// permission 関連フィールドのみを生成する。init と setMode の双方からこのヘルパー経由で
/// 同じ変換ロジックを参照し、Claude/Codex 判定とフラグ変換の重複実装を防ぐ
/// （Spec issues-947: バックエンド変換層の DRY 化）。
fn bridge_permission_fields(
    pm: crate::permission::PermissionMode,
    backend_id: &str,
    plan_mode: bool,
) -> Vec<(String, serde_json::Value)> {
    use crate::infrastructure::agent_session::runtime::permission_flags::{
        claude_flag_from_mode, codex_approval_policy_from_mode, codex_sandbox_mode_from_mode,
    };
    if backend_id == CODEX_BACKEND_ID {
        if plan_mode {
            return vec![
                (
                    "approvalPolicy".to_string(),
                    serde_json::Value::String("on-request".to_string()),
                ),
                (
                    "sandboxMode".to_string(),
                    serde_json::Value::String("read-only".to_string()),
                ),
                (
                    "collaborationMode".to_string(),
                    serde_json::Value::String("plan".to_string()),
                ),
            ];
        }
        vec![
            (
                "approvalPolicy".to_string(),
                serde_json::Value::String(codex_approval_policy_from_mode(pm).to_string()),
            ),
            (
                "sandboxMode".to_string(),
                serde_json::Value::String(codex_sandbox_mode_from_mode(pm).to_string()),
            ),
        ]
    } else {
        vec![(
            "permissionMode".to_string(),
            serde_json::Value::String(
                if plan_mode {
                    "plan"
                } else {
                    claude_flag_from_mode(pm)
                }
                .to_string(),
            ),
        )]
    }
}

fn build_set_model_command(model_id: &str) -> String {
    let cmd = serde_json::json!({
        "type": "setModel",
        "modelId": model_id,
    });
    format!("{}\n", cmd)
}

/// Append text/thinking chunk to streaming parts as an individual part.
/// Each chunk is retained as a separate `MessagePart`; consolidation into
/// merged same-type runs is performed by `consolidate_parts_from_slice` when
/// generating emit/persist payloads.
fn append_to_parts(
    parts: &mut Vec<MessagePart>,
    part_type: &str,
    chunk: &str,
    parent_tool_use_id: Option<String>,
) {
    match part_type {
        "text" => parts.push(MessagePart::Text {
            content: chunk.to_string(),
            parent_tool_use_id,
        }),
        "thinking" => parts.push(MessagePart::Thinking {
            content: chunk.to_string(),
            parent_tool_use_id,
        }),
        _ => {}
    }
}

/// Normalize accumulated streaming parts by merging consecutive same-type
/// text/thinking chunks sharing the same `parent_tool_use_id`.
/// During streaming, `append_to_parts` keeps each chunk as an individual part;
/// this helper produces the consolidated view used both for streaming emit
/// payloads (via `prepare_streaming_flush`) and for persistence.
fn consolidate_parts_from_slice(parts: &[MessagePart]) -> Vec<MessagePart> {
    let mut result: Vec<MessagePart> = Vec::with_capacity(parts.len());
    for part in parts {
        match (part, result.last_mut()) {
            (
                MessagePart::Text {
                    content,
                    parent_tool_use_id,
                },
                Some(MessagePart::Text {
                    content: last_content,
                    parent_tool_use_id: last_pid,
                }),
            ) if parent_tool_use_id == last_pid => {
                last_content.push_str(content);
            }
            (
                MessagePart::Text {
                    content,
                    parent_tool_use_id,
                },
                _,
            ) => result.push(MessagePart::Text {
                content: content.clone(),
                parent_tool_use_id: parent_tool_use_id.clone(),
            }),
            (
                MessagePart::Thinking {
                    content,
                    parent_tool_use_id,
                },
                Some(MessagePart::Thinking {
                    content: last_content,
                    parent_tool_use_id: last_pid,
                }),
            ) if parent_tool_use_id == last_pid => {
                last_content.push_str(content);
            }
            (
                MessagePart::Thinking {
                    content,
                    parent_tool_use_id,
                },
                _,
            ) => result.push(MessagePart::Thinking {
                content: content.clone(),
                parent_tool_use_id: parent_tool_use_id.clone(),
            }),
            _ => {
                result.push(part.clone());
            }
        }
    }
    result
}

/// Extract tool_result content from SDK content blocks.
fn extract_tool_result_content(content: &serde_json::Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(|v| v.as_str()) == Some("text") {
                    b.get("text")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    String::new()
}

fn extract_todo_items(
    value: &serde_json::Value,
) -> Option<Vec<crate::usecase::agent_session::session::TodoListItem>> {
    let items_value = value
        .get("items")
        .or_else(|| value.get("todos"))
        .or_else(|| value.get("todo_list"))?;
    let items = items_value.as_array()?;
    let parsed = items
        .iter()
        .filter_map(|item| {
            let text = item
                .get("text")
                .or_else(|| item.get("content"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            let completed = item
                .get("completed")
                .or_else(|| item.get("done"))
                .and_then(|v| v.as_bool())
                .or_else(|| {
                    item.get("status").and_then(|v| {
                        v.as_str()
                            .map(|status| matches!(status, "completed" | "done"))
                    })
                })
                .unwrap_or(false);
            Some(crate::usecase::agent_session::session::TodoListItem {
                text: text.to_string(),
                completed,
            })
        })
        .collect::<Vec<_>>();
    Some(parsed)
}

fn todo_update_log(items: &[crate::usecase::agent_session::session::TodoListItem]) -> String {
    let completed = items.iter().filter(|item| item.completed).count();
    format!("TODO を更新しました（{completed}/{} 完了）", items.len())
}

fn push_todo_snapshot(
    parts: &mut Vec<MessagePart>,
    items: Vec<crate::usecase::agent_session::session::TodoListItem>,
) {
    parts.push(MessagePart::Text {
        content: todo_update_log(&items),
        parent_tool_use_id: None,
    });
    if let Some(existing) = parts
        .iter_mut()
        .rev()
        .find(|part| matches!(part, MessagePart::TodoListSnapshot { .. }))
    {
        *existing = MessagePart::TodoListSnapshot { items };
    } else {
        parts.push(MessagePart::TodoListSnapshot { items });
    }
}

fn push_or_update_tool_result(
    parts: &mut Vec<MessagePart>,
    content: String,
    is_error: bool,
    tool_use_id: Option<String>,
    parent_tool_use_id: Option<String>,
) -> Option<MessagePart> {
    if let Some(tool_use_id_ref) = tool_use_id.as_deref() {
        if let Some(index) = parts.iter().rposition(|part| {
            matches!(
                part,
                MessagePart::ToolResult {
                    tool_use_id: Some(id),
                    ..
                } if id == tool_use_id_ref
            )
        }) {
            let MessagePart::ToolResult {
                content: existing,
                is_error: existing_error,
                parent_tool_use_id: existing_parent,
                ..
            } = &mut parts[index]
            else {
                return None;
            };
            let mut delta_content = String::new();
            if !content.is_empty() {
                if content.contains(existing.as_str()) || existing.is_empty() {
                    delta_content = content.clone();
                    *existing = content;
                } else {
                    existing.push_str(&content);
                    delta_content = content;
                }
            }
            *existing_error = *existing_error || is_error;
            if existing_parent.is_none() {
                *existing_parent = parent_tool_use_id;
            }
            return Some(MessagePart::ToolResult {
                content: delta_content,
                is_error: *existing_error,
                tool_use_id: Some(tool_use_id_ref.to_string()),
                parent_tool_use_id: existing_parent.clone(),
            });
        }
    }
    parts.push(MessagePart::ToolResult {
        content,
        is_error,
        tool_use_id,
        parent_tool_use_id,
    });
    None
}

fn push_or_update_tool_use(
    parts: &mut Vec<MessagePart>,
    tool: String,
    input: serde_json::Value,
    id: String,
    parent_tool_use_id: Option<String>,
) -> Option<MessagePart> {
    if let Some(index) = parts.iter().rposition(|part| {
        matches!(
            part,
            MessagePart::ToolUse {
                id: existing_id,
                ..
            } if existing_id == &id
        )
    }) {
        let MessagePart::ToolUse {
            tool: existing_tool,
            input: existing_input,
            parent_tool_use_id: existing_parent,
            ..
        } = &mut parts[index]
        else {
            return None;
        };
        *existing_tool = tool;
        *existing_input = input;
        if existing_parent.is_none() {
            *existing_parent = parent_tool_use_id;
        }
        return Some(parts[index].clone());
    }

    parts.push(MessagePart::ToolUse {
        tool,
        input,
        id,
        parent_tool_use_id,
    });
    None
}

/// Returns true if the message should be forwarded as agent-sdk-message.
/// Non-accumulated messages (meta events) are always forwarded.
/// permission_request is accumulated (for streaming delta) but ALSO forwarded
/// for SET_PENDING_PERMISSION dispatch on the frontend.
fn should_forward_sdk_message(accumulated: bool, msg_type: &str) -> bool {
    !accumulated || msg_type == "permission_request"
}

struct SdkMessageAccumulation {
    handled: bool,
    updated_parts: Option<Vec<MessagePart>>,
    liveness: bool,
}

fn is_explicit_liveness_progress_message(msg: &serde_json::Value) -> bool {
    msg.get("type").and_then(|v| v.as_str()) == Some("system")
        && matches!(
            msg.get("subtype").and_then(|v| v.as_str()),
            Some("task_started" | "task_notification" | "task_progress" | "task_updated")
        )
}

fn accumulate_sdk_message_with_liveness(
    msg: &serde_json::Value,
    parts: &mut Vec<MessagePart>,
    task_id_map: &mut HashMap<String, String>,
) -> SdkMessageAccumulation {
    let prev_len = parts.len();
    let (handled, updated_parts) = accumulate_sdk_message(msg, parts, task_id_map);
    let liveness = handled
        && (parts.len() > prev_len
            || updated_parts
                .as_ref()
                .is_some_and(|parts| !parts.is_empty())
            || is_explicit_liveness_progress_message(msg));

    SdkMessageAccumulation {
        handled,
        updated_parts,
        liveness,
    }
}

/// Extract background task ID from tool_result content.
/// Handles both Task tool ("agentId: a72ca50") and Bash tool ("with ID: b8625ae") formats.
fn extract_agent_id(content: &str) -> Option<&str> {
    // Try known prefixes in order
    for prefix in &["agentId: ", "with ID: "] {
        if let Some(pos) = content.find(prefix) {
            let start = pos + prefix.len();
            let rest = &content[start..];
            let end = rest
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
                .unwrap_or(rest.len());
            if end > 0 {
                return Some(&rest[..end]);
            }
        }
    }
    None
}

fn task_id_map_from_parts(parts: &[MessagePart]) -> HashMap<String, String> {
    parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::ToolResult {
                content,
                tool_use_id: Some(tool_use_id),
                ..
            } => extract_agent_id(content)
                .map(|agent_id| (agent_id.to_string(), tool_use_id.clone())),
            _ => None,
        })
        .collect()
}

/// Synthesize an `Error` message part from a bridge `error` SDK message.
/// `accumulate_sdk_message` deliberately does not turn `error` messages into
/// parts (it would resurrect/persist an empty post-turn buffer); error
/// handlers add the Error part explicitly instead.
fn sdk_error_part_from_message(msg: &serde_json::Value) -> MessagePart {
    let error_text = msg
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown error");
    let parent_tool_use_id = msg
        .get("parent_tool_use_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    MessagePart::Error {
        content: format!("Error: {}", error_text),
        parent_tool_use_id,
    }
}

/// Parse SDK message and accumulate into streaming_parts.
/// Returns (accumulated, updated_parts):
/// - accumulated: true if the message was handled and should NOT be forwarded as agent-sdk-message.
/// - updated_parts: Some(parts) when an existing part was updated in-place (e.g. compaction/hook completion).
///   These must be emitted as delta since they are not captured by the `parts[prev_len..]` diff.
///
/// Keep this accumulator in lockstep with `post_turn_base_requirement_for_empty_buffer`;
/// the classifier decides whether an empty post-turn buffer must be reseeded
/// before this function can safely mutate parts.
fn accumulate_sdk_message(
    msg: &serde_json::Value,
    parts: &mut Vec<MessagePart>,
    task_id_map: &mut HashMap<String, String>,
) -> (bool, Option<Vec<MessagePart>>) {
    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let parent_tool_use_id = msg
        .get("parent_tool_use_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    match msg_type {
        "stream_event" => {
            if let Some(event) = msg.get("event") {
                let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if event_type == "content_block_delta" {
                    if let Some(delta) = event.get("delta") {
                        let delta_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if delta_type == "text_delta" {
                            if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                                append_to_parts(parts, "text", text, parent_tool_use_id);
                                return (true, None);
                            }
                        } else if delta_type == "thinking_delta" {
                            if let Some(thinking) = delta.get("thinking").and_then(|v| v.as_str()) {
                                append_to_parts(parts, "thinking", thinking, parent_tool_use_id);
                                return (true, None);
                            }
                        }
                    }
                }
            }
            (false, None)
        }
        "assistant" => {
            let mut updated_parts = Vec::new();
            if let Some(message) = msg.get("message") {
                if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                    for block in content {
                        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if block_type == "tool_use" {
                            let tool = block
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let input = block
                                .get("input")
                                .cloned()
                                .unwrap_or(serde_json::Value::Object(Default::default()));
                            let id = block
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if tool == "TodoWrite" {
                                if let Some(items) = extract_todo_items(&input) {
                                    push_todo_snapshot(parts, items);
                                }
                                continue;
                            }
                            if let Some(updated) = push_or_update_tool_use(
                                parts,
                                tool,
                                input,
                                id,
                                parent_tool_use_id.clone(),
                            ) {
                                updated_parts.push(updated);
                            }
                        }
                    }
                }
            }
            (true, (!updated_parts.is_empty()).then_some(updated_parts))
        }
        "user" => {
            let mut updated_parts = Vec::new();
            if let Some(message) = msg.get("message") {
                if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                    for block in content {
                        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if block_type == "tool_result" {
                            let tool_use_id = block
                                .get("tool_use_id")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let raw_content = block
                                .get("content")
                                .cloned()
                                .unwrap_or(serde_json::Value::String(String::new()));
                            let content_str = extract_tool_result_content(&raw_content);
                            let is_error = block
                                .get("is_error")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            // Extract agentId from background task tool_result
                            if let Some(tuid) = &tool_use_id {
                                if let Some(agent_id) = extract_agent_id(&content_str) {
                                    task_id_map.insert(agent_id.to_string(), tuid.clone());
                                }
                            }
                            if let Some(updated) = push_or_update_tool_result(
                                parts,
                                content_str,
                                is_error,
                                tool_use_id,
                                parent_tool_use_id.clone(),
                            ) {
                                updated_parts.push(updated);
                            }
                        }
                    }
                }
            }
            (true, (!updated_parts.is_empty()).then_some(updated_parts))
        }
        "todo_list_snapshot" => {
            if let Some(items) = extract_todo_items(msg) {
                push_todo_snapshot(parts, items);
                return (true, None);
            }
            (true, None)
        }
        "permission_denied" => {
            let tool_name = msg
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("Permission");
            let tool_use_id = msg
                .get("tool_use_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let decision_reason = msg
                .get("decision_reason")
                .and_then(|v| v.as_str())
                .or_else(|| msg.get("message").and_then(|v| v.as_str()))
                .unwrap_or("Permission denied");
            parts.push(MessagePart::Permission {
                request: serde_json::json!({
                    "type": "permission_denied",
                    "request_id": msg
                        .get("request_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("permission-denied"),
                    "tool_name": tool_name,
                    "display_name": tool_name,
                    "input": msg.get("input").cloned().unwrap_or(serde_json::Value::Null),
                    "tool_use_id": tool_use_id,
                    "decision_reason": decision_reason,
                    "description": msg.get("message").and_then(|v| v.as_str()).unwrap_or(decision_reason),
                }),
                status: "denied".to_string(),
                answers: None,
                parent_tool_use_id,
            });
            (true, None)
        }
        "permission_request" => {
            let request = msg.clone();
            parts.push(MessagePart::Permission {
                request,
                status: "pending".to_string(),
                answers: None,
                parent_tool_use_id,
            });
            (true, None)
        }
        "system" => {
            let subtype = msg.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
            match subtype {
                "task_started" | "task_notification" | "task_progress" => {
                    let mut tool_use_id = msg
                        .get("tool_use_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    // SDK task_notification omits tool_use_id; resolve via task_id mapping
                    if tool_use_id.is_empty() {
                        if let Some(task_id) = msg.get("task_id").and_then(|v| v.as_str()) {
                            if let Some(mapped) = task_id_map.get(task_id) {
                                tool_use_id = mapped.clone();
                            }
                        }
                    }
                    let status = match subtype {
                        "task_started" => "started",
                        "task_progress" => "progress",
                        "task_notification" => msg
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("started"),
                        _ => "started",
                    };
                    let description = msg
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let summary = msg
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    parts.push(MessagePart::TaskStatus {
                        task_tool_use_id: tool_use_id,
                        status: status.to_string(),
                        description,
                        summary,
                    });
                    (true, None)
                }
                "task_updated" => {
                    let mut tool_use_id = msg
                        .get("tool_use_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if tool_use_id.is_empty() {
                        if let Some(task_id) = msg.get("task_id").and_then(|v| v.as_str()) {
                            if let Some(mapped) = task_id_map.get(task_id) {
                                tool_use_id = mapped.clone();
                            }
                        }
                    }
                    if tool_use_id.is_empty() {
                        return (true, None);
                    }
                    let patch = msg.get("patch").unwrap_or(msg);
                    let status = patch
                        .get("status")
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            patch
                                .get("error")
                                .filter(|v| !v.is_null())
                                .map(|_| "failed")
                        })
                        .or_else(|| {
                            patch
                                .get("is_backgrounded")
                                .and_then(|v| v.as_bool())
                                .filter(|value| *value)
                                .map(|_| "backgrounded")
                        })
                        .unwrap_or("progress")
                        .to_string();
                    let description = patch
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let summary = patch
                        .get("summary")
                        .or_else(|| patch.get("message"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    if let Some(index) = parts.iter().rposition(|part| {
                        matches!(
                            part,
                            MessagePart::TaskStatus {
                                task_tool_use_id,
                                ..
                            } if task_tool_use_id == &tool_use_id
                        )
                    }) {
                        let MessagePart::TaskStatus {
                            status: existing_status,
                            description: existing_description,
                            summary: existing_summary,
                            ..
                        } = &mut parts[index]
                        else {
                            return (true, None);
                        };
                        *existing_status = status;
                        if description.is_some() {
                            *existing_description = description;
                        }
                        if summary.is_some() {
                            *existing_summary = summary;
                        }
                        return (true, Some(vec![parts[index].clone()]));
                    }
                    parts.push(MessagePart::TaskStatus {
                        task_tool_use_id: tool_use_id,
                        status,
                        description,
                        summary,
                    });
                    (true, None)
                }
                "init" => (false, None), // init message → forward (not accumulated)
                "compact_boundary" => {
                    // Compaction completed: find the in-progress compaction part and update it
                    let trigger = msg
                        .get("compact_metadata")
                        .and_then(|m| m.get("trigger"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let pre_tokens = msg
                        .get("compact_metadata")
                        .and_then(|m| m.get("pre_summary_token_count"))
                        .and_then(|v| v.as_u64())
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let detail = format!("trigger={trigger}, {pre_tokens} tokens");

                    // Walk parts in reverse to find the in-progress compaction notification
                    let mut updated_part = None;
                    for part in parts.iter_mut().rev() {
                        if let MessagePart::SystemNotification {
                            notification_type,
                            status,
                            label,
                            detail: d,
                            ..
                        } = part
                        {
                            if *notification_type == SystemNotificationType::Compaction
                                && status == "in_progress"
                            {
                                *status = "completed".to_string();
                                *label = "Conversation compacted".to_string();
                                *d = Some(detail.clone());
                                updated_part = Some(part.clone());
                                break;
                            }
                        }
                    }
                    if let Some(p) = updated_part {
                        (true, Some(vec![p]))
                    } else {
                        // No in-progress compaction found, add a completed one directly
                        parts.push(MessagePart::SystemNotification {
                            notification_type: SystemNotificationType::Compaction,
                            status: "completed".to_string(),
                            label: "Conversation compacted".to_string(),
                            detail: Some(detail),
                            hook_id: None,
                        });
                        (true, None)
                    }
                }
                "hook_started"
                | "hook_progress"
                | "hook_response"
                | "files_persisted"
                | "local_command_output"
                | "codex_realtime" => (true, None),
                _ => {
                    // Check for status=compacting (subtype may be empty/"" for status messages)
                    let status = msg.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    if status == "compacting" {
                        parts.push(MessagePart::SystemNotification {
                            notification_type: SystemNotificationType::Compaction,
                            status: "in_progress".to_string(),
                            label: "Compacting conversation...".to_string(),
                            detail: None,
                            hook_id: None,
                        });
                        (true, None)
                    } else {
                        (false, None) // permissionMode sync, other system messages → forward
                    }
                }
            }
        }
        "error" => (false, None), // Forward for handleBridgeError; error handlers add Error parts explicitly.
        _ => (false, None),
    }
}

/// agent process に session 固有 env を渡すための (key, value) 一覧を組み立てる。
///
/// spec issues-1022 "Agent process environment contract" の実装:
/// - `RELEASH_SESSION_ID`: agent process 自身の chat_session_id。agent CLI 呼出時に
///   `--session-id "$RELEASH_SESSION_ID"` を付ければ Releash 側 SessionStore lookup から
///   identity (backend / model) が解決される。
/// - `RELEASH_BASE_BRANCH`: 当該 worktree の base ブランチ名。reviewer agent が
///   `git diff "$RELEASH_BASE_BRANCH"...HEAD` で今回の差分のみを対象化するために使う。
///   解決できない場合 (unborn / detached / 未設定) は env を立てない。
///
/// facet template に `{{session_id}}` のような動的解決値を持ち込まず、Spec issues-1054 の
/// `{{vars.<name>}}` 静的値原則を破らない経路で session 固有値を agent に届ける単一責任 helper。
pub(crate) fn session_specific_env_overrides(
    chat_session_id: &str,
    base_branch: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut env = vec![("RELEASH_SESSION_ID", chat_session_id.to_string())];
    if let Some(b) = base_branch {
        env.push(("RELEASH_BASE_BRANCH", b.to_string()));
    }
    env
}

fn claude_bridge_watchdog_env_overrides() -> Vec<(&'static str, String)> {
    vec![
        (
            "CLAUDE_STREAM_IDLE_TIMEOUT_MS",
            (STALE_TIMEOUT_SECS * 1000).to_string(),
        ),
        ("CLAUDE_ENABLE_STREAM_WATCHDOG", "1".to_string()),
        ("CLAUDE_ENABLE_BYTE_WATCHDOG", "1".to_string()),
        ("CLAUDE_CODE_MAX_RETRIES", "10".to_string()),
        ("API_TIMEOUT_MS", "600000".to_string()),
    ]
}

/// ユーザー指定の system_prompt に Releash CLI の long help を append する。
///
/// spec issues-1022 "Agent process environment contract": Agent process の
/// system_prompt には Releash CLI の long help が常に含まれ、Agent は help を
/// 別経路で取得する必要を持たない。clap derive 由来 (`cli::render_long_help`) を
/// 単一ソースとし、Agent 向けに別個の文字列を手書きしない。
///
/// - `None` または空文字 → `Some(<help>)`
/// - `Some(user)` → `Some("{user}\n\n{help}")`
fn compose_system_prompt(user: Option<String>) -> Option<String> {
    let help = crate::cli::render_long_help();
    let composed = match user {
        Some(ref s) if !s.is_empty() => format!("{s}\n\n{help}"),
        _ => help.to_string(),
    };
    Some(composed)
}

#[allow(clippy::too_many_arguments)]
async fn spawn_bridge_process<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    backend_id: String,
    session_id: Option<String>,
    cwd: &str,
    permission_mode: String,
    plan_mode: bool,
    selected_model: Option<String>,
    system_prompt: Option<String>,
    restore_context: Option<RestoreContextPayload>,
) -> Result<(), String> {
    let bridge_path = resolve_bridge_script(app, &backend_id)?;
    if !bridge_path.exists() {
        return Err(format!(
            "Bridge script not found: {}",
            bridge_path.display()
        ));
    }

    // spawn 前にパーミッションモードを検証する。Tauri/WS 境界で検証済みのはずだが、
    // 内部経路の保護として二重に弾く（不正値で子プロセス起動を許さない）。
    let initial_permission_mode = permission_mode;
    crate::permission::PermissionMode::parse(&initial_permission_mode)
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    let data_dir = resolve_data_dir(app)
        .map_err(|e| format!("Failed to resolve data dir for session {chat_session_id}: {e}"))?;

    let mut cmd = Command::new("node");
    cmd.arg(
        bridge_path
            .to_str()
            .ok_or_else(|| "Bridge script path contains invalid UTF-8".to_string())?,
    )
    .current_dir(cwd)
    // Remove Claude Code nesting-detection env vars so the SDK-spawned
    // `claude` CLI does not refuse to start.
    .env_remove("CLAUDECODE")
    .env_remove("CLAUDE_CODE_ENTRYPOINT")
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());

    // spec issues-1054: agent bridge にも起動環境別 alias が解決可能な PATH と
    // `RELEASH_DATA_DIR` を伝搬する（bridge 経由で呼ばれるツールが alias を解決できるように）。
    match crate::path_aliases::prepare_child_env(app.path().app_data_dir().ok()) {
        Ok(env) => {
            for (k, v) in env {
                cmd.env(k, v);
            }
        }
        Err(e) => {
            // 提示する alias と実行環境の不整合を避けるため、wrapper 作成失敗時は
            // bridge 起動を中止する（spec issues-1054「agent 子プロセスへの実行
            // 環境の伝搬」: PATH 経由で alias 解決可能な環境を約束する）。
            return Err(format!(
                "failed to prepare alias child env for agent bridge: {e}"
            ));
        }
    }

    // spec issues-1022 "Agent process environment contract": agent process 自身が
    // 自分の chat_session_id を env 経由で参照できるよう、session 固有 env を
    // pure helper 経由で組み立てて設置する。
    // 周辺入口（agent bridge）は gateway 実装へ直接依存せず、composition root が AppState へ
    // 配線した code usecase を取得して base 名を解決する。エラーは移行前と同じく None に倒す。
    let base_branch = app
        .state::<crate::adaptor::controller::state::AppState>()
        .code_usecase
        .resolve_effective_base_branch_name(cwd)
        .ok()
        .flatten();
    for (k, v) in session_specific_env_overrides(chat_session_id, base_branch.as_deref()) {
        cmd.env(k, v);
    }
    for (k, v) in claude_bridge_watchdog_env_overrides() {
        cmd.env(k, v);
    }

    #[cfg(unix)]
    // SAFETY: setsid() is async-signal-safe per POSIX.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn node process: {e}"))?;

    #[cfg(unix)]
    let pgid = child.id();
    #[cfg(unix)]
    if let Some(pg) = pgid {
        if let Err(e) = save_pgid(&data_dir, chat_session_id, pg) {
            log::error!("Failed to save PGID file, killing spawned process group: {e}");
            unsafe {
                libc::killpg(pg as libc::pid_t, libc::SIGKILL);
            }
            return Err(format!(
                "Failed to save PGID file for session {chat_session_id}: {e}"
            ));
        }
    }

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Failed to capture stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Failed to capture stderr".to_string())?;

    // Send init command（permission_mode は抽象モード文字列を期待）。
    // initial_permission_mode は spawn 前に検証済み（上方参照）。
    // spec issues-1022 "Agent process environment contract": ユーザー指定の
    // system_prompt に Releash CLI の long help を append したものを Agent に渡す。
    let composed_system_prompt = compose_system_prompt(system_prompt);
    let mut init_cmd = build_init_cmd(
        cwd,
        &initial_permission_mode,
        plan_mode,
        &session_id,
        &backend_id,
        BridgeInitOptions {
            system_prompt: composed_system_prompt,
            selected_model: selected_model.as_deref(),
            restore_context: restore_context.as_ref(),
        },
    )?;
    let runtime_config = backend_runtime_config(app, &backend_id);
    if let Some(init_obj) = init_cmd.as_object_mut() {
        for (key, value) in runtime_config.bridge_init_options {
            init_obj.insert(key, value);
        }
    }
    let init_data = format!("{}\n", init_cmd);
    stdin
        .write_all(init_data.as_bytes())
        .await
        .map_err(|e| format!("Failed to write init command: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush init command: {e}"))?;

    // Store process
    let gen_id = GENERATION_COUNTER.fetch_add(1, Ordering::SeqCst);
    let context_carry_on_ready = if session_id.is_some() {
        Some(ContextCarryState::Resumed)
    } else if restore_context
        .as_ref()
        .is_some_and(|payload| !payload.prompt_prefix.trim().is_empty())
    {
        Some(ContextCarryState::Reinjected)
    } else {
        None
    };
    {
        let mut map = handles.lock().await;
        map.insert(
            chat_session_id.to_string(),
            AgentProcess {
                stdin,
                backend_id,
                state: BridgeState::Initializing,
                turn_phase: TurnPhase::Idle,
                sdk_session_id: session_id,
                context_carry_on_ready,
                child,
                generation_id: gen_id,
                #[cfg(unix)]
                pgid,
                streaming_message_id: None,
                streaming_parts: Vec::new(),
                last_message_id: None,
                post_turn_base_untrusted_message_id: None,
                task_id_map: HashMap::new(),
                pending_messages: VecDeque::new(),
                current_permission_mode: initial_permission_mode.clone(),
                available_models: Vec::new(),
                selected_model,
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
            },
        );
    }

    // 初期 SessionStatus を AgentStatusCenter に登録（Idle で初期化）
    notify_status_transition(app, session_store, chat_session_id, TurnPhase::Idle, None);

    // Spawn stdout reader (process-lifetime)
    let handles_stdout = Arc::clone(handles);
    let session_store_clone = Arc::clone(session_store);
    let app_stdout = app.clone();
    let csid_stdout = chat_session_id.to_string();
    let captured_gen_id = gen_id;
    tokio::spawn(async move {
        use tauri::Emitter;
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut last_persist_time = std::time::Instant::now();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty() {
                continue;
            }
            if let Ok(mut msg) = serde_json::from_str::<serde_json::Value>(&line) {
                msg["chat_session_id"] = serde_json::Value::String(csid_stdout.clone());

                let defer_agent_session_id_persist_on_ready =
                    take_defer_agent_session_id_persist_on_ready(&mut msg);
                let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

                match msg_type {
                    "supported_commands" => {
                        let commands = supported_commands_from_bridge_message(&msg);
                        let payload = crate::protocol::AgentSupportedCommandsUpdated {
                            chat_session_id: csid_stdout.clone(),
                            commands: commands
                                .into_iter()
                                .map(|command| crate::protocol::AgentSupportedCommandMsg {
                                    name: command.name,
                                    description: command.description,
                                    argument_hint: command.argument_hint,
                                })
                                .collect(),
                        };
                        let _ = app_stdout.emit("agent-supported-commands-updated", &payload);
                    }
                    "session_ready" => {
                        let (
                            became_ready,
                            context_carry_on_ready,
                            resume_mismatch,
                            requeue_candidate,
                        ) = {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                // Only transition to Ready if still Initializing (not already Streaming)
                                let was_initializing = proc.state == BridgeState::Initializing;
                                if was_initializing {
                                    proc.state = BridgeState::Ready;
                                }
                                let ready_session_id =
                                    msg.get("session_id").and_then(|v| v.as_str());
                                let requested_resume_id = proc.sdk_session_id.clone();
                                let context_carry_on_ready = proc.context_carry_on_ready.take();
                                let resume_mismatch = session_ready_resume_mismatch(
                                    context_carry_on_ready.as_ref(),
                                    requested_resume_id.as_deref(),
                                    ready_session_id,
                                );
                                let requeue_candidate = if resume_mismatch {
                                    streaming_turn_requeue_candidate(proc)
                                } else {
                                    None
                                };
                                if let Some(sid) = ready_session_id {
                                    proc.sdk_session_id = Some(sid.to_string());
                                    if !resume_mismatch && !defer_agent_session_id_persist_on_ready
                                    {
                                        persist_agent_session_id(
                                            &app_stdout,
                                            &session_store_clone,
                                            &csid_stdout,
                                            sid,
                                        );
                                    }
                                }
                                (
                                    was_initializing,
                                    context_carry_on_ready,
                                    resume_mismatch,
                                    requeue_candidate,
                                )
                            } else {
                                (false, None, false, None)
                            }
                        };
                        if resume_mismatch {
                            let requeued_streaming_turn = if let Some(candidate) = requeue_candidate
                            {
                                requeue_streaming_turn_for_resume_mismatch(
                                    &app_stdout,
                                    &handles_stdout,
                                    &session_store_clone,
                                    &csid_stdout,
                                    candidate,
                                )
                                .await
                            } else {
                                false
                            };
                            persist_resume_mismatch_for_reinject(
                                &app_stdout,
                                &session_store_clone,
                                &csid_stdout,
                            );
                            crash_agent_process_for_context_reinject(
                                &app_stdout,
                                &handles_stdout,
                                &csid_stdout,
                            )
                            .await;
                            if requeued_streaming_turn {
                                emit_session_state_changed(
                                    &app_stdout,
                                    &csid_stdout,
                                    TurnPhase::Idle,
                                    None,
                                );
                                notify_status_transition(
                                    &app_stdout,
                                    &session_store_clone,
                                    &csid_stdout,
                                    TurnPhase::Idle,
                                    None,
                                );
                                if let Some(pending) =
                                    take_pending_message(&handles_stdout, &csid_stdout).await
                                {
                                    let app_p = app_stdout.clone();
                                    let ss_p = Arc::clone(&session_store_clone);
                                    let h_p = Arc::clone(&handles_stdout);
                                    let csid_p = csid_stdout.clone();
                                    let handle = tokio::runtime::Handle::current();
                                    std::thread::spawn(move || {
                                        handle.block_on(async move {
                                            start_pending_message_turn(
                                                &app_p, &h_p, &ss_p, &csid_p, pending,
                                            )
                                            .await;
                                        });
                                    });
                                }
                            }
                            continue;
                        } else if let Some(context_carry) = context_carry_on_ready {
                            persist_context_carry_state(
                                &app_stdout,
                                &session_store_clone,
                                &csid_stdout,
                                context_carry,
                            );
                        }
                        let _ = app_stdout.emit("agent-sdk-message", &msg);
                        // Initializing 中（起動直後）に enqueue された pending を Ready 化
                        // 時に drain する。drain トリガーが turn_complete のみだと、ターンが
                        // 一度も開始されないためキューが永久に消費されない（再起動／履歴
                        // 復帰直後に送信が発火しない不具合の修正）。turn_complete と同じく
                        // session_runtime_lock を保持しない経路で起動する。
                        if became_ready {
                            if let Some(pending) =
                                take_pending_message(&handles_stdout, &csid_stdout).await
                            {
                                let app_p = app_stdout.clone();
                                let ss_p = Arc::clone(&session_store_clone);
                                let h_p = Arc::clone(&handles_stdout);
                                let csid_p = csid_stdout.clone();
                                let handle = tokio::runtime::Handle::current();
                                std::thread::spawn(move || {
                                    handle.block_on(async move {
                                        start_pending_message_turn(
                                            &app_p, &h_p, &ss_p, &csid_p, pending,
                                        )
                                        .await;
                                    });
                                });
                            }
                        }
                    }
                    "session_cleared" => {
                        {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                proc.sdk_session_id = None;
                            }
                        }
                        let result = resolve_data_dir(&app_stdout).and_then(|data_dir| {
                            session_store_clone
                                .update_agent_session_id_if_changed(&data_dir, &csid_stdout, None)
                                .map(|_| ())
                        });
                        if let Err(e) = result {
                            log::warn!("Failed to clear agent session id for {csid_stdout}: {e}");
                        }
                        let _ = app_stdout.emit("agent-sdk-message", &msg);
                    }
                    "result" => {
                        if let Some(token_usage) = token_usage_from_result_message(&msg) {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                proc.last_result_token_usage =
                                    Some((token_usage.input_tokens, token_usage.output_tokens));
                                proc.latest_token_usage = Some(token_usage);
                            }
                        }
                        // Forward result message to frontend
                        let _ = app_stdout.emit("agent-sdk-message", &msg);
                    }
                    "turn_complete" => {
                        let exit_code = msg.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0);
                        // session_runtime_lock の保持はローカル proc 状態遷移の
                        // 区間に限定する。workflow turn-complete usecase や pending
                        // message 消費は lock を保持しない経路で行い、それらが必要に応じ
                        // 自前で lock を取得する設計とする（再入デッドロックを防ぐ）。
                        let (effect, context_restore_failed_on_init) = {
                            let _runtime_guard = acquire_session_runtime_lock(&csid_stdout).await;
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                // Run the in-lock transition through the shared
                                // helper so production and unit tests exercise the
                                // exact same flush → state-mutation order. Flush
                                // failure is best-effort — we still mutate state so
                                // turn_complete notification fires.
                                let effect = run_turn_complete_transition_locked(
                                    proc,
                                    &csid_stdout,
                                    exit_code,
                                    |mid, parts| {
                                        emit_streaming_parts(
                                            &app_stdout,
                                            &csid_stdout,
                                            mid,
                                            parts.to_vec(),
                                        )
                                    },
                                );
                                let context_restore_failed_on_init = !effect.was_streaming
                                    && exit_code != 0
                                    && proc.context_carry_on_ready.take().is_some();

                                // User turn succeeded: persist agent_session_id to SessionStore
                                if effect.was_streaming && exit_code == 0 {
                                    if let Some(sid) = &proc.sdk_session_id {
                                        persist_agent_session_id(
                                            &app_stdout,
                                            &session_store_clone,
                                            &csid_stdout,
                                            sid,
                                        );
                                    }
                                }
                                (effect, context_restore_failed_on_init)
                            } else {
                                (TurnCompleteTransition::default(), false)
                            }
                            // _runtime_guard はこのスコープを抜けて drop される
                        };

                        // Resume failure (error during init) → clear stale agent_session_id
                        if !effect.was_streaming && exit_code != 0 {
                            persist_context_carry_failed_after_init_error(
                                &app_stdout,
                                &session_store_clone,
                                &csid_stdout,
                                true,
                                context_restore_failed_on_init,
                            );
                        }

                        // Emit state change only for user turns (was Streaming)
                        complete_streaming_turn_post_lock(
                            &app_stdout,
                            &session_store_clone,
                            &handles_stdout,
                            &csid_stdout,
                            exit_code,
                            effect,
                            true,
                        )
                        .await;
                    }
                    "error" => {
                        let error_msg = msg
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown bridge error");
                        log::error!("Bridge error [{}]: {}", csid_stdout, error_msg);

                        let transition = {
                            let _runtime_guard = acquire_session_runtime_lock(&csid_stdout).await;
                            let mut map = handles_stdout.lock().await;
                            map.get_mut(&csid_stdout).map(|proc| {
                                run_bridge_error_transition_locked(
                                    proc,
                                    &csid_stdout,
                                    &msg,
                                    |mid, parts| {
                                        emit_streaming_parts(
                                            &app_stdout,
                                            &csid_stdout,
                                            mid,
                                            parts.to_vec(),
                                        )
                                    },
                                )
                            })
                        };

                        let _ = app_stdout.emit("agent-sdk-message", &msg);

                        let transition = transition.unwrap_or_default();
                        let was_initializing = transition.was_initializing;
                        let context_restore_failed_on_init =
                            transition.context_restore_failed_on_init;
                        let turn_complete = transition.turn_complete;
                        complete_streaming_turn_post_lock(
                            &app_stdout,
                            &session_store_clone,
                            &handles_stdout,
                            &csid_stdout,
                            1,
                            turn_complete,
                            true,
                        )
                        .await;
                        if was_initializing {
                            notify_status_transition(
                                &app_stdout,
                                &session_store_clone,
                                &csid_stdout,
                                TurnPhase::Idle,
                                Some(crate::usecase::agent_session::session::SessionState::Error),
                            );
                        }
                        // Init error → clear stale agent_session_id to prevent infinite resume loop
                        if was_initializing
                            || msg
                                .get("clear_session_id")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                            || msg
                                .get("context_carry_failed")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                        {
                            persist_context_carry_failed_after_init_error(
                                &app_stdout,
                                &session_store_clone,
                                &csid_stdout,
                                was_initializing
                                    || msg
                                        .get("clear_session_id")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false),
                                context_restore_failed_on_init
                                    || msg
                                        .get("context_carry_failed")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false),
                            );
                        }
                    }
                    _ => {
                        // Accumulate into streaming buffer, enqueue delta into the
                        // coalescing buffer, and flush when warranted. We hold the
                        // lock across the flush so the emit observes consistent
                        // state with `streaming_parts`.
                        let elapsed_persist = last_persist_time.elapsed().as_millis() as u64;
                        let mut effect = accumulate_stream_or_post_turn_message(
                            &app_stdout,
                            &session_store_clone,
                            &handles_stdout,
                            &csid_stdout,
                            &msg,
                            elapsed_persist,
                        )
                        .await;

                        // Periodic persist (1s interval) — consolidate outside lock
                        if effect.should_persist {
                            if let Some(ref mid) = effect.emit_msg_id {
                                last_persist_time = Instant::now();
                                let persisted = persist_streaming_parts(
                                    &session_store_clone,
                                    &app_stdout,
                                    &csid_stdout,
                                    mid,
                                    &effect.persist_parts,
                                    None,
                                );
                                if persisted {
                                    clear_post_turn_store_base_untrusted_for_message(
                                        &handles_stdout,
                                        &csid_stdout,
                                        mid,
                                    )
                                    .await;
                                }
                            }
                        }
                        drop(std::mem::take(&mut effect.released_streaming_parts));

                        // Handle permissionMode sync from SDK on Rust side
                        // Claude SDK は "default"/"acceptEdits"/"bypassPermissions"/"plan" を送る。
                        // 永続化は検証済み Tauri/WS リクエストや明示 UI 操作経由に限定するため、
                        // ここでは SessionStore の値は書き換えない（Spec issues-947）。
                        //
                        // SDK は ExitPlanMode 等の遷移時に "default" を送ることがあるが、保存値が
                        // edit/full のセッションでは runtime/UI が ask に落ちると整合性が崩れる。
                        // そのため SDK 通知をきっかけにせず、保存済み ChatSession.permission_mode を
                        // 正典として読み直し、ランタイムと UI をそれに合わせる。保存値と SDK 値が
                        // ズレた場合は setMode を再送して bridge を保存値に追従させる。
                        if msg_type == "system" {
                            if let Some(sdk_mode) =
                                msg.get("permissionMode").and_then(|v| v.as_str())
                            {
                                // 保存値を正典として扱う経路。読み取り失敗時は SDK 由来の値で
                                // fallback せず、log::error! を出して runtime/UI を更新せずに
                                // 当該通知の処理だけスキップする（Spec issues-947: 保存値が
                                // edit/full のセッションを ask に落とすような誤更新を排除）。
                                handle_sdk_permission_mode_notification(
                                    sdk_mode,
                                    &app_stdout,
                                    &session_store_clone,
                                    &handles_stdout,
                                    &csid_stdout,
                                )
                                .await;
                            }
                        }

                        // For permission_request, force-flush pending stream content
                        // BEFORE emitting agent-sdk-message so the UI receives the
                        // accumulated text before SET_PENDING_PERMISSION dispatches
                        // and the WaitingPermission state notification fires.
                        // Order: buffer flush → state notify → followup.
                        let permission_did_transition = if msg_type == "permission_request" {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                let effect = run_permission_request_transition_locked(
                                    proc,
                                    &csid_stdout,
                                    |mid, parts| {
                                        emit_streaming_parts(
                                            &app_stdout,
                                            &csid_stdout,
                                            mid,
                                            parts.to_vec(),
                                        )
                                    },
                                );
                                effect.did_transition
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        // Forward non-accumulated messages (meta events) as agent-sdk-message.
                        // permission_request needs both delta emit AND forwarding for SET_PENDING_PERMISSION.
                        if should_forward_sdk_message(effect.accumulated, msg_type) {
                            let _ = app_stdout.emit("agent-sdk-message", &msg);
                        }

                        if permission_did_transition {
                            emit_session_state_changed(
                                &app_stdout,
                                &csid_stdout,
                                TurnPhase::WaitingPermission,
                                None,
                            );
                            notify_status_transition(
                                &app_stdout,
                                &session_store_clone,
                                &csid_stdout,
                                TurnPhase::WaitingPermission,
                                None,
                            );
                        }
                    }
                }
            }
        }
        // EOF — process exited; verify generation to avoid acting on stale events.
        // Streaming 中の終了だけでなく、Initializing (session_ready 前) の終了も
        // AgentStatusCenter に Error として伝搬させる。Initializing の場合は
        // turn_id=-1 を伴う Idle emit は行わない（streaming が無かったため）。
        let (transition, should_remove_pid_file) = {
            let _runtime_guard = acquire_session_runtime_lock(&csid_stdout).await;
            let mut map = handles_stdout.lock().await;
            if let Some(proc) = map.get_mut(&csid_stdout) {
                let generation_matches = proc.generation_id == captured_gen_id;
                let transition = run_bridge_eof_crash_transition_locked(
                    generation_matches,
                    proc,
                    &csid_stdout,
                    |mid, parts| {
                        emit_streaming_parts(&app_stdout, &csid_stdout, mid, parts.to_vec())
                    },
                );
                // Ready/Idle EOF: the completed-but-dead process must be retired so
                // the next send re-spawns instead of writing into a dead runtime.
                let should_remove_pid_file = transition.should_evict
                    && retire_ready_eof_runtime_locked(&mut map, &csid_stdout);
                (transition, should_remove_pid_file)
            } else {
                (BridgeEofCrashTransition::default(), false)
            }
        };
        if should_remove_pid_file {
            #[cfg(unix)]
            if let Ok(data_dir) = resolve_data_dir(&app_stdout) {
                remove_pgid(&data_dir, &csid_stdout);
            }
        }
        if transition.context_restore_failed_on_init {
            persist_context_carry_failed_after_init_error(
                &app_stdout,
                &session_store_clone,
                &csid_stdout,
                true,
                true,
            );
        }
        if let Some(message) = transition.sdk_error_message.as_deref() {
            let _ = app_stdout.emit(
                "agent-sdk-message",
                serde_json::json!({
                    "type": "error",
                    "message": message,
                    "chat_session_id": &csid_stdout,
                }),
            );
        }
        let was_initializing = transition.was_initializing;
        let effect = transition.turn_complete;
        if effect.was_streaming {
            complete_streaming_turn_post_lock(
                &app_stdout,
                &session_store_clone,
                &handles_stdout,
                &csid_stdout,
                -1,
                effect,
                true,
            )
            .await;
        } else if was_initializing {
            notify_status_transition(
                &app_stdout,
                &session_store_clone,
                &csid_stdout,
                TurnPhase::Idle,
                Some(crate::usecase::agent_session::session::SessionState::Error),
            );
        }
    });

    // Spawn stderr reader (process-lifetime)
    let csid_stderr = chat_session_id.to_string();
    tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.is_empty() {
                log::warn!("bridge stderr [{}]: {}", csid_stderr, line);
            }
        }
    });

    // The auxiliary streaming-flush timer is spawned per-turn from
    // `spawn_streaming_timer`, not at process spawn — process-lifetime ticks
    // would hold `AgentProcessMap` lock every 33ms even while idle.

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum TurnWatchdogDecision {
    Continue,
    Timeout(TurnLivenessTimeout),
    BreakClearFlag,
    BreakKeepFlag,
}

fn turn_watchdog_decision(
    proc: &AgentProcess,
    captured_gen_id: u64,
    captured_turn_seq: u64,
    now: Instant,
) -> TurnWatchdogDecision {
    if proc.generation_id != captured_gen_id || proc.turn_seq != captured_turn_seq {
        return TurnWatchdogDecision::BreakKeepFlag;
    }
    if proc.turn_phase == TurnPhase::Idle || proc.state == BridgeState::Crashed {
        return TurnWatchdogDecision::BreakClearFlag;
    }
    match evaluate_turn_liveness(
        proc.turn_phase,
        proc.last_progress_at,
        proc.turn_phase_since,
        now,
    ) {
        Some(timeout) => TurnWatchdogDecision::Timeout(timeout),
        None => TurnWatchdogDecision::Continue,
    }
}

fn try_mark_turn_watchdog_active(proc: &mut AgentProcess) -> bool {
    if proc.turn_watchdog_active {
        return false;
    }
    proc.turn_watchdog_active = true;
    true
}

async fn complete_streaming_turn_post_lock<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    exit_code: i64,
    effect: TurnCompleteTransition,
    consume_pending: bool,
) {
    if !effect.was_streaming {
        return;
    }

    if let Some(ref mid) = effect.final_msg_id {
        if !effect.final_parts.is_empty() {
            let persisted = persist_streaming_parts(
                session_store,
                app,
                chat_session_id,
                mid,
                &effect.final_parts,
                Some(now_timestamp()),
            );
            if persisted {
                clear_post_turn_store_base_untrusted_for_message(handles, chat_session_id, mid)
                    .await;
            }
        }
    }

    emit_session_state_changed(app, chat_session_id, TurnPhase::Idle, Some(exit_code));
    let override_state = if exit_code != 0 {
        Some(crate::usecase::agent_session::session::SessionState::Error)
    } else {
        None
    };
    notify_status_transition(
        app,
        session_store,
        chat_session_id,
        TurnPhase::Idle,
        override_state,
    );

    let pending = if consume_pending {
        take_pending_message(handles, chat_session_id).await
    } else {
        None
    };
    spawn_workflow_turn_complete_notification(
        app.clone(),
        Arc::clone(session_store),
        Arc::clone(handles),
        chat_session_id.to_string(),
        exit_code,
        effect.final_parts,
        effect.turn_token_usage,
        pending,
    );
}

struct TimeoutFinalizeOutcome {
    completed: bool,
    continue_watchdog: bool,
    captured_pgid: Option<u32>,
}

struct TimeoutFinalizeTransition {
    effect: Option<TurnCompleteTransition>,
    continue_watchdog: bool,
    captured_pgid: Option<u32>,
}

fn run_timeout_finalize_transition_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    captured_gen_id: u64,
    captured_turn_seq: u64,
    now: Instant,
    emit_stream: F,
) -> TimeoutFinalizeTransition
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    let timeout = match turn_watchdog_decision(proc, captured_gen_id, captured_turn_seq, now) {
        TurnWatchdogDecision::Timeout(timeout) => timeout,
        TurnWatchdogDecision::Continue => {
            return TimeoutFinalizeTransition {
                effect: None,
                continue_watchdog: true,
                captured_pgid: None,
            };
        }
        TurnWatchdogDecision::BreakClearFlag => {
            proc.turn_watchdog_active = false;
            return TimeoutFinalizeTransition {
                effect: None,
                continue_watchdog: false,
                captured_pgid: None,
            };
        }
        TurnWatchdogDecision::BreakKeepFlag => {
            return TimeoutFinalizeTransition {
                effect: None,
                continue_watchdog: false,
                captured_pgid: None,
            };
        }
    };

    #[cfg(unix)]
    let captured_pgid = proc.pgid;
    #[cfg(not(unix))]
    let captured_pgid = None;

    let effect = finalize_turn_as_timeout_locked(proc, chat_session_id, timeout, emit_stream);
    TimeoutFinalizeTransition {
        effect: Some(effect),
        continue_watchdog: false,
        captured_pgid,
    }
}

async fn finalize_timed_out_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    captured_gen_id: u64,
    captured_turn_seq: u64,
) -> TimeoutFinalizeOutcome {
    let transition;
    {
        let _runtime_guard = acquire_session_runtime_lock(chat_session_id).await;
        let mut map = handles.lock().await;
        let Some(proc) = map.get_mut(chat_session_id) else {
            return TimeoutFinalizeOutcome {
                completed: false,
                continue_watchdog: false,
                captured_pgid: None,
            };
        };
        transition = run_timeout_finalize_transition_locked(
            proc,
            chat_session_id,
            captured_gen_id,
            captured_turn_seq,
            Instant::now(),
            |mid, parts| emit_streaming_parts(app, chat_session_id, mid, parts.to_vec()),
        );
    }

    let Some(effect) = transition.effect else {
        return TimeoutFinalizeOutcome {
            completed: false,
            continue_watchdog: transition.continue_watchdog,
            captured_pgid: transition.captured_pgid,
        };
    };

    if !effect.was_streaming {
        return TimeoutFinalizeOutcome {
            completed: false,
            continue_watchdog: false,
            captured_pgid: transition.captured_pgid,
        };
    }

    complete_streaming_turn_post_lock(
        app,
        session_store,
        handles,
        chat_session_id,
        STALE_EXIT_CODE,
        effect,
        true,
    )
    .await;

    TimeoutFinalizeOutcome {
        completed: true,
        continue_watchdog: false,
        captured_pgid: transition.captured_pgid,
    }
}

async fn recover_timed_out_bridge<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    captured_gen_id: u64,
    captured_turn_seq: u64,
    captured_pgid: Option<u32>,
) {
    let interrupt_sent = match write_bridge_command_for_captured_turn(
        handles,
        chat_session_id,
        captured_gen_id,
        captured_turn_seq,
        serde_json::json!({ "type": "interrupt" }),
    )
    .await
    {
        Ok(sent) => sent,
        Err(e) => {
            log::warn!("Failed to interrupt timed-out bridge {chat_session_id}: {e}");
            false
        }
    };

    if interrupt_sent {
        tokio::time::sleep(Duration::from_secs(STALE_RECOVERY_GRACE_SECS)).await;
    }

    let (remove_current_pid_file, sweep_captured_pgid) = {
        let mut map = handles.lock().await;
        mark_timed_out_bridge_for_recovery_locked(
            &mut map,
            chat_session_id,
            captured_gen_id,
            captured_turn_seq,
            captured_pgid,
        )
    };

    #[cfg(unix)]
    {
        if let (true, Some(pg)) = (sweep_captured_pgid, captured_pgid) {
            sweep_process_group(pg).await;
        }
        if remove_current_pid_file {
            if let Ok(data_dir) = resolve_data_dir(app) {
                remove_pgid(&data_dir, chat_session_id);
            }
        }
    }

    #[cfg(not(unix))]
    {
        if remove_current_pid_file {
            let _ = app;
        }
    }
}

fn mark_timed_out_bridge_for_recovery_locked(
    map: &mut AgentProcessMap,
    chat_session_id: &str,
    captured_gen_id: u64,
    captured_turn_seq: u64,
    captured_pgid: Option<u32>,
) -> (bool, bool) {
    if let Some(proc) = map.get_mut(chat_session_id) {
        if proc.generation_id == captured_gen_id
            && proc.turn_seq == captured_turn_seq
            && proc.state != BridgeState::Ready
        {
            proc.state = BridgeState::Crashed;
            proc.turn_phase = TurnPhase::Idle;
            proc.turn_watchdog_active = false;
            proc.last_progress_at = None;
            proc.mark_turn_phase_since_now();
            return (true, true);
        }
    }

    #[cfg(unix)]
    {
        let current_owns_captured_pgid = captured_pgid.is_some_and(|pg| {
            map.get(chat_session_id)
                .and_then(|proc| proc.pgid)
                .is_some_and(|current_pg| current_pg == pg)
        });
        (
            false,
            captured_pgid.is_some() && !current_owns_captured_pgid,
        )
    }
    #[cfg(not(unix))]
    {
        let _ = captured_pgid;
        (false, false)
    }
}

fn spawn_turn_watchdog<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    proc: &mut AgentProcess,
) {
    if !try_mark_turn_watchdog_active(proc) {
        return;
    }
    let app_watchdog = app.clone();
    let handles_watchdog = Arc::clone(handles);
    let session_store_watchdog = Arc::clone(session_store);
    let csid_watchdog = chat_session_id.to_string();
    let captured_gen_id = proc.generation_id;
    let captured_turn_seq = proc.turn_seq;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(WATCHDOG_TICK_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            let decision = {
                let mut map = handles_watchdog.lock().await;
                let Some(proc) = map.get_mut(&csid_watchdog) else {
                    break;
                };
                match turn_watchdog_decision(
                    proc,
                    captured_gen_id,
                    captured_turn_seq,
                    Instant::now(),
                ) {
                    TurnWatchdogDecision::BreakClearFlag => {
                        proc.turn_watchdog_active = false;
                        TurnWatchdogDecision::BreakClearFlag
                    }
                    other => other,
                }
            };

            match decision {
                TurnWatchdogDecision::Continue => {}
                TurnWatchdogDecision::BreakKeepFlag | TurnWatchdogDecision::BreakClearFlag => break,
                TurnWatchdogDecision::Timeout(_) => {
                    let outcome = finalize_timed_out_turn(
                        &app_watchdog,
                        &handles_watchdog,
                        &session_store_watchdog,
                        &csid_watchdog,
                        captured_gen_id,
                        captured_turn_seq,
                    )
                    .await;
                    if outcome.completed {
                        recover_timed_out_bridge(
                            &app_watchdog,
                            &handles_watchdog,
                            &csid_watchdog,
                            captured_gen_id,
                            captured_turn_seq,
                            outcome.captured_pgid,
                        )
                        .await;
                        break;
                    }
                    if outcome.continue_watchdog {
                        continue;
                    }
                    break;
                }
            }
        }
    });
}

/// Loop control decision for the per-turn streaming timer. Extracted as a
/// pure function so the spawn loop's exit/flag-management semantics are
/// covered by unit tests, instead of relying on a tokio task to be observable
/// from the test harness.
#[derive(Debug, PartialEq, Eq)]
enum TimerDecision {
    /// Generation matches and process is still streaming — run the tick.
    Continue,
    /// Generation matches and process has crashed — exit and release the
    /// active flag so a future turn can spawn a fresh timer.
    BreakClearFlag,
    /// Generation no longer matches: a newer process owns the slot, and its
    /// own timer is responsible for the flag. Exit without touching it.
    BreakKeepFlag,
}

/// Decide what the streaming timer should do at the top of each tick.
fn streaming_timer_decision(proc: &AgentProcess, captured_gen_id: u64) -> TimerDecision {
    if proc.generation_id != captured_gen_id {
        return TimerDecision::BreakKeepFlag;
    }
    if proc.state == BridgeState::Crashed {
        return TimerDecision::BreakClearFlag;
    }
    TimerDecision::Continue
}

/// Idempotency gate for `spawn_streaming_timer`. Marks the timer slot active
/// and returns `true` when the caller should spawn; returns `false` when a
/// timer is already running for this process (duplicate spawn no-op).
fn try_mark_streaming_timer_active(proc: &mut AgentProcess) -> bool {
    if proc.streaming_timer_active {
        return false;
    }
    proc.streaming_timer_active = true;
    true
}

/// Spawn the per-turn auxiliary streaming-flush timer. Ticks every
/// `STREAMING_EMIT_INTERVAL_MS` and drains the pending coalescing buffer so
/// silent gaps between deltas (e.g. SDK ingesting a tool result) still
/// surface buffered content within one interval. Exits when the turn ends
/// and the buffer is fully drained, on generation mismatch, or on crash.
/// Idempotent: a second call while a timer is already alive is a no-op.
fn spawn_streaming_timer<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    proc: &mut AgentProcess,
) {
    if !try_mark_streaming_timer_active(proc) {
        return;
    }
    let handles_timer = Arc::clone(handles);
    let app_timer = app.clone();
    let csid_timer = chat_session_id.to_string();
    let captured_gen_id_timer = proc.generation_id;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(STREAMING_EMIT_INTERVAL_MS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Skip the immediate first tick — the stdout reader's per-delta path
        // handles the very first emit.
        interval.tick().await;
        loop {
            interval.tick().await;
            let tick_effect = {
                let mut map = handles_timer.lock().await;
                let Some(proc) = map.get_mut(&csid_timer) else {
                    // Process removed — no flag to clear.
                    break;
                };
                match streaming_timer_decision(proc, captured_gen_id_timer) {
                    TimerDecision::BreakKeepFlag => break,
                    TimerDecision::BreakClearFlag => {
                        proc.streaming_timer_active = false;
                        break;
                    }
                    TimerDecision::Continue => {}
                }
                let tick_effect = run_streaming_timer_tick(proc, &csid_timer, |mid, parts| {
                    emit_streaming_parts(&app_timer, &csid_timer, mid, parts.to_vec())
                });
                if !tick_effect.keep_running {
                    proc.streaming_timer_active = false;
                }
                tick_effect
            };
            let StreamingTimerTickEffect {
                keep_running,
                released_streaming_parts,
            } = tick_effect;
            drop(released_streaming_parts);
            if !keep_running {
                break;
            }
        }
    });
}

pub(crate) async fn get_session_internal<R: tauri::Runtime>(
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    app: &tauri::AppHandle<R>,
    session_id: &str,
) -> Result<Option<GetSessionResponse>, String> {
    let data_dir = resolve_data_dir(app)?;
    get_session_internal_with_data_dir(session_store, handles, registry, &data_dir, session_id)
        .await
}

async fn get_session_internal_with_data_dir(
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    data_dir: &Path,
    session_id: &str,
) -> Result<Option<GetSessionResponse>, String> {
    let session = session_store.get_session_with_latest_page(
        data_dir,
        session_id,
        INITIAL_SESSION_PAGE_LIMIT,
    )?;
    match session {
        None => Ok(None),
        Some((mut session, page)) => {
            let initial_page = Some(InitialSessionPage {
                next_cursor: page.next_cursor,
                has_more: page.has_more,
                total_count: page.total_count,
            });
            let (turn_phase, streaming_parts, streaming_mid, pending_queue, latest_token_usage) = {
                let map = handles.lock().await;
                if let Some(proc) = map.get(session_id) {
                    // Prefer the newest queued pending message's permission_mode when present,
                    // because prepare_send_agent_message_internal persists a new mode to
                    // SessionStore and pending_messages while busy without updating
                    // current_permission_mode. Falling back to current_permission_mode
                    // keeps in-flight runtime changes (e.g. SDK-driven transitions) visible.
                    session.permission_mode = proc
                        .pending_messages
                        .back()
                        .map(|pending| pending.permission_mode.clone())
                        .unwrap_or_else(|| proc.current_permission_mode.clone());
                    let phase = proc.turn_phase;
                    let pending_queue = pending_queue_view(proc);
                    let latest_token_usage = proc.latest_token_usage;
                    if proc.state == BridgeState::Streaming {
                        (
                            phase,
                            consolidate_parts_from_slice(&proc.streaming_parts),
                            proc.streaming_message_id.clone(),
                            pending_queue,
                            latest_token_usage,
                        )
                    } else {
                        (phase, Vec::new(), None, pending_queue, latest_token_usage)
                    }
                } else {
                    (TurnPhase::Idle, Vec::new(), None, Vec::new(), None)
                }
            };

            if turn_phase == TurnPhase::Streaming || turn_phase == TurnPhase::WaitingPermission {
                if let Some(ref mid) = streaming_mid {
                    if !streaming_parts.is_empty() {
                        if let Some(msg) = session.messages.iter_mut().find(|m| m.id == *mid) {
                            msg.parts = Some(streaming_parts);
                        }
                    }
                }
            }

            // 永続的なモデル一覧の owner は config.toml 単一。プロセス内キャッシュは
            // 参照しない（プロセス側の `proc.available_models` は emit 整合用にのみ
            // 維持される）。
            // get_session は表示専用経路のため、infrastructure 故障で取得に失敗した場合は
            // warn を残して空一覧を返し、上位の UI 描画を妨げない。
            let backend_id = session
                .backend_id
                .clone()
                .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string());
            let available_models =
                available_models_for_backend(&backend_id, registry).unwrap_or_else(|e| {
                    log::warn!(
                        "get_session: backend '{backend_id}' のモデル一覧取得に失敗（空一覧で応答）: {e}"
                    );
                    Vec::new()
                });

            // モデル未選択状態は廃止。既存セッションの None は既定モデルへ解決して返す。
            // 応答の selected_model は常に非 null（flatten + skip_serializing_if のため、
            // None だとフィールドが脱落しフロントの必須 string 契約に反する）。
            let selected_model = resolve_selected_model_for_response(
                session.selected_model.take(),
                &backend_id,
                registry,
            )?;
            session.selected_model = selected_model.as_deref().map(|model_id| {
                crate::domain::agent_session::model_entry_id(&backend_id, model_id)
            });

            Ok(Some(GetSessionResponse {
                session,
                turn_phase: turn_phase.into(),
                available_models: available_models.into_iter().map(Into::into).collect(),
                pending_queue_count: pending_queue.len(),
                pending_queue,
                initial_page,
                latest_token_usage,
            }))
        }
    }
}

pub(crate) async fn get_session_page_internal_with_data_dir(
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    data_dir: &Path,
    session_id: &str,
    cursor: Option<PageCursor>,
    limit: usize,
) -> Result<Option<SessionPage>, String> {
    let mut page = match session_store.get_session_page(data_dir, session_id, cursor, limit)? {
        Some(page) => page,
        None => return Ok(None),
    };
    let (streaming_overlay, latest_token_usage) = {
        let map = handles.lock().await;
        if let Some(proc) = map.get(session_id) {
            let streaming_overlay = if proc.state == BridgeState::Streaming {
                proc.streaming_message_id
                    .as_ref()
                    .filter(|_| !proc.streaming_parts.is_empty())
                    .map(|message_id| {
                        (
                            message_id.clone(),
                            consolidate_parts_from_slice(&proc.streaming_parts),
                        )
                    })
            } else {
                None
            };
            (streaming_overlay, proc.latest_token_usage)
        } else {
            (None, None)
        }
    };
    page.latest_token_usage = latest_token_usage;
    if let Some((message_id, parts)) = streaming_overlay {
        if let Some(message) = page
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
        {
            message.parts = Some(parts);
        }
    }
    Ok(Some(page))
}

fn can_change_session_backend_from_meta(
    session: &crate::usecase::agent_session::session::SessionMeta,
) -> bool {
    session.message_count == 0 && session.agent_session_id.is_none()
}

/// spec issues-1023: 初期 active 候補は workflow step として起動された session を
/// 除外し、free chat（`workflow_step_session == false`）の先頭を採用する。free chat が
/// 1 件もない場合は active 候補無し（`None`）で、UI は空状態を描く。
fn pick_initial_active_session_candidate(sessions: &[SessionSummary]) -> Option<&SessionSummary> {
    sessions.iter().find(|s| !s.is_workflow_step_session())
}

fn ensure_session_backend_selected(
    session_store: &SessionStore,
    registry: &crate::infrastructure::agent_session::runtime::AgentBackendRegistry,
    data_dir: &Path,
    mut session: ChatSession,
) -> Result<ChatSession, String> {
    if session.backend_id.is_none() {
        let backend_id = registry.resolve_default_id()?;
        let selected_model = registry.default_model_for(&backend_id).ok();
        session.backend_id = Some(backend_id.clone());
        session.selected_model = selected_model.clone();
        session.updated_at = now_timestamp();
        session_store.update_backend_selection(
            data_dir,
            &session.id,
            backend_id,
            selected_model,
        )?;
    }
    Ok(session)
}

async fn remove_stale_unstarted_agent_process(
    handles: &Arc<Mutex<AgentProcessMap>>,
    data_dir: &Path,
    chat_session_id: &str,
) {
    let stale_process = {
        let mut map = handles.lock().await;
        map.remove(chat_session_id)
    };

    if let Some(mut proc) = stale_process {
        log::warn!(
            "Removing stale agent process for unstarted session {chat_session_id} after backend change"
        );
        #[cfg(unix)]
        {
            if let Some(pg) = proc.pgid {
                unsafe {
                    libc::killpg(pg as libc::pid_t, libc::SIGKILL);
                }
            } else if let Err(e) = proc.child.kill().await {
                log::warn!("Failed to kill stale agent process {chat_session_id}: {e}");
            }
            remove_pgid(data_dir, chat_session_id);
        }
        #[cfg(not(unix))]
        {
            if let Err(e) = proc.child.kill().await {
                log::warn!("Failed to kill stale agent process {chat_session_id}: {e}");
            }
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), proc.child.wait()).await;
    }
}

async fn set_session_backend_internal(
    session_store: &Arc<SessionStore>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    data_dir: &Path,
    chat_session_id: &str,
    backend_id: String,
) -> Result<GetSessionResponse, String> {
    let resolved_backend_id = registry.resolve_backend_id(Some(backend_id))?;
    let meta = session_store
        .get_session_meta(data_dir, chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;

    if !can_change_session_backend_from_meta(&meta) {
        return Err(format!(
            "Cannot change backend after the first message has been sent: {chat_session_id}"
        ));
    }

    session_store.update_backend_selection(
        data_dir,
        chat_session_id,
        resolved_backend_id.clone(),
        Some(registry.default_model_for(&resolved_backend_id)?),
    )?;
    remove_stale_unstarted_agent_process(handles, data_dir, chat_session_id).await;

    get_session_internal_with_data_dir(
        session_store,
        handles,
        Some(registry),
        data_dir,
        chat_session_id,
    )
    .await?
    .ok_or_else(|| format!("Session not found: {chat_session_id}"))
}

pub async fn set_session_backend(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<
        '_,
        Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    >,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
    backend_id: String,
) -> Result<GetSessionResponse, String> {
    let data_dir = resolve_data_dir(&app)?;
    set_session_backend_internal(
        session_store.inner(),
        registry.inner(),
        handles.inner(),
        &data_dir,
        &chat_session_id,
        backend_id,
    )
    .await
}

pub async fn get_session(
    state: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    registry: tauri::State<
        '_,
        Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    >,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Option<GetSessionResponse>, String> {
    get_session_internal(
        state.inner(),
        handles.inner(),
        Some(registry.inner()),
        &app,
        &session_id,
    )
    .await
}

#[derive(Debug, Clone)]
pub(crate) struct PersistedSpawnInfo {
    pub resume_sid: Option<String>,
    pub selected_model: Option<String>,
    pub backend_id: String,
    pub permission_profile_id: Option<String>,
    pub context_restore_plan: ContextRestorePlan,
}

/// Retrieve persisted session fields needed for spawning a Bridge process.
///
/// モデル「未選択（None）」状態は廃止されたが、`ChatSession.selected_model` の永続化型は
/// 既存 JSON 互換のため `Option<String>` のまま。spawn 経路では `None` を backend の
/// 既定モデル（[`crate::infrastructure::agent_session::runtime::AgentBackendRegistry::default_model_for`]）へ lazy 解決して
/// から Bridge へ渡す。
fn get_persisted_spawn_info<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
) -> Result<PersistedSpawnInfo, String> {
    let data_dir = resolve_data_dir(app)?;
    let registry =
        app.try_state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>();
    let meta = session_store.get_session_meta(&data_dir, chat_session_id)?;
    resolve_spawn_info_from_meta_or_full(
        session_store,
        &data_dir,
        chat_session_id,
        meta,
        registry.as_deref(),
        None,
    )
}

fn require_session_meta_for_turn(
    session_store: &SessionStore,
    data_dir: &Path,
    chat_session_id: &str,
) -> Result<SessionMeta, String> {
    session_store
        .get_session_meta(data_dir, chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))
}

fn get_required_persisted_spawn_info_for_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
) -> Result<PersistedSpawnInfo, String> {
    let data_dir = resolve_data_dir(app)?;
    let registry =
        app.try_state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>();
    let meta = require_session_meta_for_turn(session_store, &data_dir, chat_session_id)?;
    resolve_spawn_info_from_meta_or_full(
        session_store,
        &data_dir,
        chat_session_id,
        Some(meta),
        registry.as_deref(),
        None,
    )
}

fn get_required_persisted_spawn_info_before_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
    streaming_agent_message_id: &str,
) -> Result<PersistedSpawnInfo, String> {
    let data_dir = resolve_data_dir(app)?;
    let registry =
        app.try_state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>();
    let meta = require_session_meta_for_turn(session_store, &data_dir, chat_session_id)?;
    resolve_spawn_info_from_meta_or_full(
        session_store,
        &data_dir,
        chat_session_id,
        Some(meta),
        registry.as_deref(),
        Some(streaming_agent_message_id),
    )
}

fn resolve_spawn_info_from_meta(
    meta: SessionMeta,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    context_restore_plan: ContextRestorePlan,
) -> PersistedSpawnInfo {
    let backend_id = meta
        .backend_id
        .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string());
    let selected_model = resolve_selected_model(meta.selected_model, &backend_id, registry);
    PersistedSpawnInfo {
        resume_sid: context_restore_plan
            .resume_session_id()
            .map(ToString::to_string),
        selected_model,
        backend_id,
        permission_profile_id: meta.permission_profile_id,
        context_restore_plan,
    }
}

fn resolve_spawn_info_from_meta_or_full(
    session_store: &SessionStore,
    data_dir: &Path,
    chat_session_id: &str,
    meta: Option<SessionMeta>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    before_turn_message_id: Option<&str>,
) -> Result<PersistedSpawnInfo, String> {
    let Some(meta) = meta else {
        return Ok(resolve_spawn_info_with_plan(
            None,
            registry,
            ContextRestorePlan::NoContext,
        ));
    };
    if let Some(plan) = context_restore_plan_from_meta(&meta) {
        return Ok(resolve_spawn_info_from_meta(meta, registry, plan));
    }
    let persisted = session_store.load_full_session_for_restore(data_dir, chat_session_id)?;
    let context_restore_plan = match before_turn_message_id {
        Some(message_id) => {
            context_restore_plan_for_session_before_turn(persisted.as_ref(), message_id)
        }
        None => context_restore_plan_for_session(persisted.as_ref()),
    };
    Ok(resolve_spawn_info_with_plan(
        persisted,
        registry,
        context_restore_plan,
    ))
}

/// 永続化セッションから spawn 情報を組み立てる純粋関数。
///
/// `selected_model == None` は registry の既定モデルへ解決する（モデル未選択状態は廃止）。
/// registry 未指定（テスト等）では `None` のままとする。
#[cfg(test)]
pub(crate) fn resolve_spawn_info(
    persisted: Option<ChatSession>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
) -> PersistedSpawnInfo {
    let context_restore_plan = context_restore_plan_for_session(persisted.as_ref());
    resolve_spawn_info_with_plan(persisted, registry, context_restore_plan)
}

fn resolve_spawn_info_with_plan(
    persisted: Option<ChatSession>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    context_restore_plan: ContextRestorePlan,
) -> PersistedSpawnInfo {
    let (resume_sid, selected_model, backend_id, permission_profile_id, context_restore_plan) =
        persisted_spawn_info_from_session(persisted, context_restore_plan);
    let selected_model = resolve_selected_model(selected_model, &backend_id, registry);
    PersistedSpawnInfo {
        resume_sid,
        selected_model,
        backend_id,
        permission_profile_id,
        context_restore_plan,
    }
}

fn persisted_spawn_info_from_session(
    session: Option<ChatSession>,
    context_restore_plan: ContextRestorePlan,
) -> (
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    ContextRestorePlan,
) {
    session
        .map(|s| {
            (
                context_restore_plan
                    .resume_session_id()
                    .map(ToString::to_string),
                s.selected_model,
                s.backend_id
                    .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string()),
                s.permission_profile_id,
                context_restore_plan.clone(),
            )
        })
        .unwrap_or((
            None,
            None,
            CLAUDE_BACKEND_ID.to_string(),
            None,
            context_restore_plan,
        ))
}

#[derive(Debug, Clone, PartialEq)]
struct SessionContextCarryUpdate {
    chat_session_id: String,
    agent_session_id: Option<String>,
    context_carry: Option<ContextCarryState>,
    updated_at: f64,
}

impl SessionContextCarryUpdate {
    fn from_meta(meta: &crate::usecase::agent_session::session::SessionMeta) -> Self {
        Self {
            chat_session_id: meta.id.clone(),
            agent_session_id: meta.agent_session_id.clone(),
            context_carry: meta.context_carry.clone(),
            updated_at: meta.updated_at,
        }
    }

    fn to_protocol(&self) -> crate::protocol::AgentSessionContextCarryUpdated {
        crate::protocol::AgentSessionContextCarryUpdated {
            chat_session_id: self.chat_session_id.clone(),
            agent_session_id: self.agent_session_id.clone(),
            context_carry: self.context_carry.clone(),
            updated_at: self.updated_at,
        }
    }
}

fn emit_session_context_carry_update<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    update: SessionContextCarryUpdate,
) {
    use tauri::Emitter;

    let payload = update.to_protocol();
    let _ = app.emit("agent-session-context-carry-updated", &payload);
}

fn session_ready_resume_mismatch(
    context_carry_on_ready: Option<&ContextCarryState>,
    requested_resume_id: Option<&str>,
    ready_session_id: Option<&str>,
) -> bool {
    if context_carry_on_ready != Some(&ContextCarryState::Resumed) {
        return false;
    }
    let Some(requested_resume_id) = requested_resume_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    ready_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        != Some(requested_resume_id)
}

#[derive(Debug, Clone)]
struct StreamingTurnRequeueCandidate {
    streaming_message_id: String,
    permission_mode: String,
}

fn streaming_turn_requeue_candidate(proc: &AgentProcess) -> Option<StreamingTurnRequeueCandidate> {
    if proc.state != BridgeState::Streaming {
        return None;
    }
    Some(StreamingTurnRequeueCandidate {
        streaming_message_id: proc.streaming_message_id.clone()?,
        permission_mode: proc.current_permission_mode.clone(),
    })
}

fn pending_content_from_human_message(message: &ChatMessage) -> String {
    if !message.content.is_empty() {
        return message.content.clone();
    }
    message.parts.as_deref().map_or_else(String::new, |parts| {
        let (content, _, _) = parts_to_legacy(parts);
        content
    })
}

fn pending_images_from_human_message(message: &ChatMessage) -> Vec<ImageAttachment> {
    message
        .parts
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|part| match part {
            MessagePart::Image { data, media_type } => Some(ImageAttachment {
                data: data.clone(),
                media_type: media_type.clone(),
            }),
            MessagePart::ImageRef { .. } => None,
            _ => None,
        })
        .collect()
}

fn pending_mentions_from_human_message(
    message: &ChatMessage,
) -> Vec<crate::domain::code::MentionReference> {
    message
        .mentions
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(crate::usecase::agent_session::session::MessageMention::into_domain)
        .collect()
}

fn pending_message_from_streaming_turn(
    session: &ChatSession,
    candidate: &StreamingTurnRequeueCandidate,
) -> Option<PendingMessage> {
    let agent_index = session
        .messages
        .iter()
        .position(|message| message.id == candidate.streaming_message_id)?;
    if session.messages.get(agent_index)?.role != MessageRole::Agent {
        return None;
    }
    let human_index = agent_index.checked_sub(1)?;
    let human_message = session.messages.get(human_index)?;
    if human_message.role != MessageRole::Human {
        return None;
    }
    Some(PendingMessage {
        id: uuid::Uuid::new_v4().to_string(),
        content: pending_content_from_human_message(human_message),
        created_at: human_message.timestamp,
        permission_mode: candidate.permission_mode.clone(),
        plan_mode: session.plan_mode,
        images: pending_images_from_human_message(human_message),
        worktree_path: session.worktree_path.clone(),
        mentions: pending_mentions_from_human_message(human_message),
        editor_context: None,
        existing_human_message_id: Some(human_message.id.clone()),
        existing_agent_message_id: Some(candidate.streaming_message_id.clone()),
    })
}

async fn requeue_streaming_turn_for_resume_mismatch<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &SessionStore,
    chat_session_id: &str,
    candidate: StreamingTurnRequeueCandidate,
) -> bool {
    let data_dir = match resolve_data_dir(app) {
        Ok(data_dir) => data_dir,
        Err(e) => {
            log::warn!("Failed to resolve data dir for resume mismatch requeue: {e}");
            return false;
        }
    };
    let session = match session_store.load_full_session_for_restore(&data_dir, chat_session_id) {
        Ok(Some(session)) => session,
        Ok(None) => {
            log::warn!("Session not found for resume mismatch requeue: {chat_session_id}");
            return false;
        }
        Err(e) => {
            log::warn!("Failed to load session for resume mismatch requeue: {e}");
            return false;
        }
    };
    let Some(pending) = pending_message_from_streaming_turn(&session, &candidate) else {
        log::warn!("Streaming turn not found for resume mismatch requeue: {chat_session_id}");
        return false;
    };

    let mut map = handles.lock().await;
    let Some(proc) = map.get_mut(chat_session_id) else {
        return false;
    };
    if proc.state != BridgeState::Streaming
        || proc.streaming_message_id.as_deref() != Some(candidate.streaming_message_id.as_str())
    {
        return false;
    }
    proc.pending_messages.push_front(pending);
    true
}

fn take_defer_agent_session_id_persist_on_ready(msg: &mut serde_json::Value) -> bool {
    msg.as_object_mut()
        .and_then(|object| object.remove(DEFER_AGENT_SESSION_ID_PERSIST_ON_READY))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn save_session_context_carry(
    session_store: &SessionStore,
    data_dir: &Path,
    chat_session_id: &str,
    context_carry: ContextCarryState,
) -> Result<Option<SessionContextCarryUpdate>, String> {
    session_store
        .update_context_carry_if_changed(data_dir, chat_session_id, Some(context_carry))
        .map(|updated| updated.as_ref().map(SessionContextCarryUpdate::from_meta))
}

fn save_resume_mismatch_for_reinject(
    session_store: &SessionStore,
    data_dir: &Path,
    chat_session_id: &str,
) -> Result<Option<SessionContextCarryUpdate>, String> {
    session_store
        .update_resume_metadata_if_changed(data_dir, chat_session_id, None, None)
        .map(|updated| updated.as_ref().map(SessionContextCarryUpdate::from_meta))
}

fn persist_resume_mismatch_for_reinject<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
) {
    match resolve_data_dir(app).and_then(|data_dir| {
        save_resume_mismatch_for_reinject(session_store, &data_dir, chat_session_id)
    }) {
        Ok(Some(update)) => emit_session_context_carry_update(app, update),
        Ok(None) => {}
        Err(e) => {
            log::warn!("Failed to prepare context reinjection for {chat_session_id}: {e}");
        }
    }
}

pub(crate) fn persist_context_carry_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
    context_carry: ContextCarryState,
) {
    match resolve_data_dir(app).and_then(|data_dir| {
        save_session_context_carry(session_store, &data_dir, chat_session_id, context_carry)
    }) {
        Ok(Some(update)) => emit_session_context_carry_update(app, update),
        Ok(None) => {}
        Err(e) => {
            log::warn!("Failed to persist context carry state for {chat_session_id}: {e}");
        }
    }
}

fn persist_agent_session_id<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
    agent_session_id: &str,
) {
    let agent_session_id = agent_session_id.trim();
    if agent_session_id.is_empty() {
        return;
    }
    let result = resolve_data_dir(app).and_then(|data_dir| {
        session_store
            .update_agent_session_id_if_changed(
                &data_dir,
                chat_session_id,
                Some(agent_session_id.to_string()),
            )
            .map(|_| ())
    });
    if let Err(e) = result {
        log::warn!("Failed to persist agent session id for {chat_session_id}: {e}");
    }
}

fn should_mark_context_carry_failed_after_init_error(
    context_carry: Option<&ContextCarryState>,
    force_context_carry_failed: bool,
) -> bool {
    if force_context_carry_failed
        || matches!(
            context_carry,
            Some(ContextCarryState::Resumed | ContextCarryState::Reinjected)
        )
    {
        return context_carry != Some(&ContextCarryState::Failed);
    }
    false
}

pub(crate) fn persist_context_carry_failed_after_init_error<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    chat_session_id: &str,
    clear_agent_session_id: bool,
    force_context_carry_failed: bool,
) {
    let result = resolve_data_dir(app).and_then(|data_dir| {
        let Some(meta) = session_store.get_session_meta(&data_dir, chat_session_id)? else {
            return Ok(None);
        };
        let next_agent_session_id = if clear_agent_session_id {
            None
        } else {
            meta.agent_session_id.clone()
        };
        let next_context_carry = if should_mark_context_carry_failed_after_init_error(
            meta.context_carry.as_ref(),
            force_context_carry_failed,
        ) {
            Some(ContextCarryState::Failed)
        } else {
            meta.context_carry.clone()
        };
        session_store
            .update_resume_metadata_if_changed(
                &data_dir,
                chat_session_id,
                next_agent_session_id,
                next_context_carry,
            )
            .map(|updated| updated.as_ref().map(SessionContextCarryUpdate::from_meta))
    });
    match result {
        Ok(Some(update)) => emit_session_context_carry_update(app, update),
        Ok(None) => {}
        Err(e) => {
            log::warn!("Failed to persist context carry failure for {chat_session_id}: {e}");
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_agent_session_internal<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    cwd: &str,
    permission_mode: Option<String>,
    plan_mode: bool,
    system_prompt: Option<String>,
) -> Result<(), String> {
    // 抽象パーミッションモードを境界で解決・検証する。
    // - Some: その場で検証（Tauri/WS 境界が既に弾いている想定だが内部経路でも二重防御）。
    // - None: 内部呼び出し（workflow engine 等）として保存済みセッション値を明示参照する。
    let resolved_permission_mode = match permission_mode {
        Some(value) => crate::permission::PermissionMode::parse(&value)
            .map(|m| m.as_str().to_string())
            .map_err(|e| e.to_string())?,
        None => {
            let data_dir = resolve_data_dir(app)?;
            let meta = session_store
                .get_session_meta(&data_dir, chat_session_id)?
                .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
            crate::permission::PermissionMode::parse(&meta.permission_mode)
                .map(|m| m.as_str().to_string())
                .map_err(|e| e.to_string())?
        }
    };

    wait_until_session_close_finished(chat_session_id).await;
    let _spawn_guard = acquire_spawn_session_guard(chat_session_id).await;
    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get(chat_session_id) {
            if proc.state != BridgeState::Crashed {
                return Ok(());
            }
        }
        map.remove(chat_session_id);
    }

    let spawn_info = get_persisted_spawn_info(app, session_store, chat_session_id)?;

    if spawn_info.backend_id == CODEX_BACKEND_ID {
        let backend = codex_backend_from_app(app)?;
        backend
            .start_session(SessionConfig {
                chat_session_id: chat_session_id.to_string(),
                cwd: cwd.to_string(),
                permission_mode: Some(resolved_permission_mode),
                plan_mode,
                permission_profile_id: spawn_info.permission_profile_id,
                system_prompt,
            })
            .await?;
        return Ok(());
    }

    spawn_bridge_process(
        app,
        handles,
        session_store,
        chat_session_id,
        spawn_info.backend_id,
        spawn_info.resume_sid,
        cwd,
        resolved_permission_mode,
        plan_mode,
        spawn_info.selected_model,
        system_prompt,
        spawn_info.context_restore_plan.restore_context().cloned(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_agent_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    cwd: &str,
    permission_mode: &str,
    plan_mode: bool,
    prompt: &str,
    streaming_message_id: &str,
    images: &[ImageAttachment],
) -> Result<(), String> {
    let spawn_info =
        get_required_persisted_spawn_info_for_turn(app, session_store, chat_session_id)?;
    if spawn_info.backend_id == CODEX_BACKEND_ID {
        return start_codex_backend_turn(
            app,
            chat_session_id,
            permission_mode,
            plan_mode,
            prompt,
            streaming_message_id,
            images,
        )
        .await;
    }

    start_agent_turn_with_runtime_spawner(
        Some(app),
        Some(session_store),
        handles,
        chat_session_id,
        permission_mode,
        prompt,
        streaming_message_id,
        images,
        || async {
            wait_until_session_close_finished(chat_session_id).await;
            let spawn_info = get_required_persisted_spawn_info_before_turn(
                app,
                session_store,
                chat_session_id,
                streaming_message_id,
            )?;

            spawn_bridge_process(
                app,
                handles,
                session_store,
                chat_session_id,
                spawn_info.backend_id,
                spawn_info.resume_sid,
                cwd,
                permission_mode.to_string(),
                plan_mode,
                spawn_info.selected_model,
                None,
                spawn_info.context_restore_plan.restore_context().cloned(),
            )
            .await
        },
    )
    .await?;

    // Emit state change so frontend can track turn phase
    emit_session_state_changed(app, chat_session_id, TurnPhase::Streaming, None);
    notify_status_transition(
        app,
        session_store,
        chat_session_id,
        TurnPhase::Streaming,
        None,
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn start_agent_turn_locked<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    cwd: &str,
    permission_mode: &str,
    plan_mode: bool,
    prompt: &str,
    streaming_message_id: &str,
    images: &[ImageAttachment],
) -> Result<(), String> {
    let spawn_info =
        get_required_persisted_spawn_info_for_turn(app, session_store, chat_session_id)?;
    if spawn_info.backend_id == CODEX_BACKEND_ID {
        return start_codex_backend_turn(
            app,
            chat_session_id,
            permission_mode,
            plan_mode,
            prompt,
            streaming_message_id,
            images,
        )
        .await;
    }

    start_agent_turn_with_runtime_spawner_locked(
        Some(app),
        Some(session_store),
        handles,
        chat_session_id,
        permission_mode,
        prompt,
        streaming_message_id,
        images,
        || async {
            let spawn_info = get_required_persisted_spawn_info_before_turn(
                app,
                session_store,
                chat_session_id,
                streaming_message_id,
            )?;

            spawn_bridge_process(
                app,
                handles,
                session_store,
                chat_session_id,
                spawn_info.backend_id,
                spawn_info.resume_sid,
                cwd,
                permission_mode.to_string(),
                plan_mode,
                spawn_info.selected_model,
                None,
                spawn_info.context_restore_plan.restore_context().cloned(),
            )
            .await
        },
    )
    .await?;

    // Emit state change so frontend can track turn phase
    emit_session_state_changed(app, chat_session_id, TurnPhase::Streaming, None);
    notify_status_transition(
        app,
        session_store,
        chat_session_id,
        TurnPhase::Streaming,
        None,
    );

    Ok(())
}

fn codex_backend_from_app<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<Arc<dyn crate::infrastructure::agent_session::runtime::AgentBackend>, String> {
    let registry = app
        .try_state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>()
        .ok_or_else(|| "AgentBackendRegistry is not registered".to_string())?;
    registry
        .get(CODEX_BACKEND_ID)
        .ok_or_else(|| format!("Agent backend not found: {CODEX_BACKEND_ID}"))
}

async fn start_codex_backend_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    permission_mode: &str,
    plan_mode: bool,
    prompt: &str,
    streaming_message_id: &str,
    images: &[ImageAttachment],
) -> Result<(), String> {
    let backend = codex_backend_from_app(app)?;
    backend
        .send_message(
            &SessionHandle {
                chat_session_id: chat_session_id.to_string(),
                backend_id: CODEX_BACKEND_ID.to_string(),
            },
            AgentMessage {
                content: prompt.to_string(),
                streaming_message_id: streaming_message_id.to_string(),
                images: images.to_vec(),
                permission_mode: permission_mode.to_string(),
                plan_mode,
                permission_profile_id: None,
                editor_context: None,
            },
        )
        .await
}

#[allow(clippy::too_many_arguments)]
async fn start_agent_turn_with_runtime_spawner<R: tauri::Runtime, F, Fut>(
    app: Option<&tauri::AppHandle<R>>,
    session_store: Option<&Arc<SessionStore>>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    permission_mode: &str,
    prompt: &str,
    streaming_message_id: &str,
    images: &[ImageAttachment],
    spawn_runtime: F,
) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    wait_until_session_close_finished(chat_session_id).await;
    let _runtime_guard = acquire_session_runtime_lock(chat_session_id).await;
    start_agent_turn_with_runtime_spawner_locked(
        app,
        session_store,
        handles,
        chat_session_id,
        permission_mode,
        prompt,
        streaming_message_id,
        images,
        spawn_runtime,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn start_agent_turn_with_runtime_spawner_locked<R: tauri::Runtime, F, Fut>(
    app: Option<&tauri::AppHandle<R>>,
    session_store: Option<&Arc<SessionStore>>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    permission_mode: &str,
    prompt: &str,
    streaming_message_id: &str,
    images: &[ImageAttachment],
    spawn_runtime: F,
) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let canonical_permission_mode =
        crate::permission::PermissionMode::parse(permission_mode).map_err(|e| e.to_string())?;
    ensure_runtime_for_turn(handles, chat_session_id, spawn_runtime).await?;

    // Send message command.
    // Even if a message is sent while the SDK is still processing an interrupt,
    // the Bridge's promptGenerator queues it and only yields after the current turn completes.
    // The SDK calls generator.next() only when ready for the next turn, providing ordering guarantee.
    let msg_cmd = build_message_cmd(prompt, images);
    let data = format!("{}\n", msg_cmd);

    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(chat_session_id) {
            proc.sync_pre_turn_settings(permission_mode).await?;

            proc.current_permission_mode = canonical_permission_mode.as_str().to_string();
            proc.state = BridgeState::Streaming;
            proc.turn_phase = TurnPhase::Streaming;
            proc.streaming_message_id = Some(streaming_message_id.to_string());
            proc.reset_streaming_state_for_new_turn();
            proc.begin_turn_liveness();
            proc.stdin
                .write_all(data.as_bytes())
                .await
                .map_err(|e| format!("Failed to write message: {e}"))?;
            proc.stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush message: {e}"))?;
            if let Some(app) = app {
                spawn_streaming_timer(app, handles, chat_session_id, proc);
                if let Some(session_store) = session_store {
                    spawn_turn_watchdog(app, handles, session_store, chat_session_id, proc);
                }
            }
        } else {
            return Err(format!("No agent process for session {chat_session_id}"));
        }
    }

    Ok(())
}

async fn ensure_runtime_for_turn<F, Fut>(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    spawn_runtime: F,
) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let mut removed_crashed_process: Option<AgentProcess> = None;
    let mut preserved_pending_messages = VecDeque::new();
    let needs_spawn = {
        let mut map = handles.lock().await;
        match take_runtime_requiring_spawn_locked(&mut map, chat_session_id) {
            RuntimeSpawnDecision::Missing => true,
            RuntimeSpawnDecision::Replace(mut proc) => {
                preserved_pending_messages.append(&mut proc.pending_messages);
                removed_crashed_process = Some(*proc);
                true
            }
            RuntimeSpawnDecision::Reuse => false,
        }
    };

    if !needs_spawn {
        return Ok(());
    }

    let _spawn_guard = acquire_spawn_session_guard(chat_session_id).await;
    let needs_spawn_after_wait = {
        let mut map = handles.lock().await;
        match take_runtime_requiring_spawn_locked(&mut map, chat_session_id) {
            RuntimeSpawnDecision::Missing => true,
            RuntimeSpawnDecision::Replace(mut proc) => {
                preserved_pending_messages.append(&mut proc.pending_messages);
                if removed_crashed_process.is_none() {
                    removed_crashed_process = Some(*proc);
                }
                true
            }
            RuntimeSpawnDecision::Reuse => false,
        }
    };
    if needs_spawn_after_wait {
        if let Err(e) = spawn_runtime().await {
            let mut map = handles.lock().await;
            if let Some(mut partial_proc) = map.remove(chat_session_id) {
                preserved_pending_messages.append(&mut partial_proc.pending_messages);
            }
            if let Some(mut proc) = removed_crashed_process {
                proc.pending_messages = preserved_pending_messages;
                map.insert(chat_session_id.to_string(), proc);
            }
            return Err(e);
        }
    }
    prepend_pending_messages_to_runtime(handles, chat_session_id, preserved_pending_messages).await;
    Ok(())
}

async fn prepend_pending_messages_to_runtime(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    mut pending_messages: VecDeque<PendingMessage>,
) {
    if pending_messages.is_empty() {
        return;
    }
    let mut map = handles.lock().await;
    if let Some(proc) = map.get_mut(chat_session_id) {
        let mut existing = std::mem::take(&mut proc.pending_messages);
        pending_messages.append(&mut existing);
        proc.pending_messages = pending_messages;
    }
}

async fn crash_agent_process_for_context_reinject<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
) {
    let mut map = handles.lock().await;
    let Some(proc) = map.get_mut(chat_session_id) else {
        return;
    };
    proc.state = BridgeState::Crashed;
    proc.turn_phase = TurnPhase::Idle;
    proc.turn_watchdog_active = false;
    proc.last_progress_at = None;
    proc.mark_turn_phase_since_now();
    proc.sdk_session_id = None;
    proc.context_carry_on_ready = None;
    #[cfg(unix)]
    {
        if let Some(pg) = proc.pgid {
            unsafe {
                libc::killpg(pg as libc::pid_t, libc::SIGKILL);
            }
        } else if let Err(e) = proc.child.kill().await {
            log::warn!("Failed to kill stale resume process {chat_session_id}: {e}");
        }
        if let Ok(data_dir) = resolve_data_dir(app) {
            remove_pgid(&data_dir, chat_session_id);
        }
    }
    #[cfg(not(unix))]
    if let Err(e) = proc.child.kill().await {
        log::warn!("Failed to kill stale resume process {chat_session_id}: {e}");
    }
}

async fn interrupt_active_agent_turn(
    handles: &Arc<Mutex<AgentProcessMap>>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    chat_session_id: &str,
) -> Result<(), String> {
    let backend_id = {
        let map = handles.lock().await;
        map.get(chat_session_id)
            .map(|proc| proc.backend_id.clone())
            .ok_or_else(|| format!("No active agent process for session {chat_session_id}"))?
    };

    if backend_id == CODEX_BACKEND_ID {
        let backend = registry
            .get(&backend_id)
            .ok_or_else(|| format!("Agent backend not found: {backend_id}"))?;
        return backend
            .interrupt(
                &crate::infrastructure::agent_session::runtime::SessionHandle {
                    chat_session_id: chat_session_id.to_string(),
                    backend_id,
                },
            )
            .await;
    }

    write_bridge_command(
        handles,
        chat_session_id,
        serde_json::json!({ "type": "interrupt" }),
    )
    .await
}

/// Detach a pending message queued during streaming and mark its follow-up turn
/// as in-flight so tab close observes the step as busy until resume starts.
async fn take_pending_message(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
) -> Option<PendingMessage> {
    let pending = {
        let mut map = handles.lock().await;
        map.get_mut(chat_session_id)
            .and_then(|p| p.pending_messages.pop_front())
    };
    if pending.is_some() {
        mark_pending_turn_starting(chat_session_id).await;
    }
    pending
}

fn pending_turn_start_failed_log_message() -> &'static str {
    "consume_pending_message_failed code=pending_turn_start_failed message=failed_to_start_pending_turn"
}

fn prepare_pending_turn_messages(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    chat_session_id: &str,
    pending: &PendingMessage,
) -> Result<(ChatMessage, ChatMessage, bool), String> {
    if let Some((human_message_id, agent_message_id)) = pending_existing_turn_ids(pending) {
        let session = session_store
            .load_full_session_for_restore(data_dir, chat_session_id)?
            .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
        let human_msg = session
            .messages
            .iter()
            .find(|message| message.id == human_message_id && message.role == MessageRole::Human)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Pending turn human message not found: {chat_session_id}/{human_message_id}"
                )
            })?;
        let agent_msg = session
            .messages
            .iter()
            .find(|message| message.id == agent_message_id && message.role == MessageRole::Agent)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Pending turn agent message not found: {chat_session_id}/{agent_message_id}"
                )
            })?;
        return Ok((human_msg, agent_msg, false));
    }

    let human_parts = pending_human_parts(pending);
    let human_mentions = if pending.mentions.is_empty() {
        None
    } else {
        Some(pending.mentions.clone())
    };
    let human_msg = add_message_internal(
        session_store,
        data_dir,
        chat_session_id,
        MessageRole::Human,
        &pending.content,
        human_parts,
        human_mentions,
    )?;
    let agent_msg = add_message_internal(
        session_store,
        data_dir,
        chat_session_id,
        MessageRole::Agent,
        "",
        None,
        None,
    )?;
    Ok((human_msg, agent_msg, true))
}

/// Consume a pending message queued during streaming and start the follow-up turn.
///
/// Acquires `session_runtime_lock(chat_session_id)` internally via the standard
/// `start_agent_turn` path. Callers must NOT hold the lock for this session id,
/// otherwise tokio Mutex non-reentrancy will deadlock (see issues-929).
async fn start_pending_message_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    pending: PendingMessage,
) {
    // 2. Add empty agent message
    let data_dir = match resolve_data_dir(app) {
        Ok(d) => d,
        Err(e) => {
            log::error!("consume_pending_message: failed to resolve data dir: {e}");
            clear_pending_turn_starting(chat_session_id).await;
            return;
        }
    };

    let (human_msg, agent_msg, emit_consumed_messages) =
        match prepare_pending_turn_messages(session_store, &data_dir, chat_session_id, &pending) {
            Ok(messages) => messages,
            Err(e) => {
                log::error!("consume_pending_message: failed to prepare pending messages: {e}");
                clear_pending_turn_starting(chat_session_id).await;
                return;
            }
        };

    // 3. Emit event so UI can update with the new human + agent messages
    if emit_consumed_messages {
        use tauri::Emitter;
        let _ = app.emit(
            "agent-pending-message-consumed",
            serde_json::json!({
                "chat_session_id": chat_session_id,
                "queued_turn_id": pending.id,
                "human_message": human_msg,
                "agent_message": agent_msg,
            }),
        );
    }

    let resolved_prompt = app
        .state::<crate::adaptor::controller::state::AppState>()
        .code_usecase
        .resolve_mentions_or_fallback(&pending.worktree_path, &pending.content, &pending.mentions);

    if let Err(_e) = start_agent_turn(
        app,
        handles,
        session_store,
        chat_session_id,
        &pending.worktree_path,
        &pending.permission_mode,
        pending.plan_mode,
        &resolved_prompt,
        &agent_msg.id,
        &pending.images,
    )
    .await
    {
        log::error!("{}", pending_turn_start_failed_log_message());
    }
    clear_pending_turn_starting(chat_session_id).await;
}

pub async fn interrupt_agent_query(
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    registry: tauri::State<
        '_,
        Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    >,
    chat_session_id: String,
) -> Result<(), String> {
    interrupt_active_agent_turn(handles.inner(), registry.inner(), &chat_session_id).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelQueuedTurnResponse {
    pub session_id: String,
    pub canceled_count: usize,
    pub pending_queue: Vec<crate::usecase::agent_session::session::QueuedAgentTurn>,
    pub pending_queue_count: usize,
}

pub async fn cancel_agent_queued_turn_internal(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    queued_turn_id: Option<&str>,
) -> Result<CancelQueuedTurnResponse, String> {
    let mut map = handles.lock().await;
    let proc = map
        .get_mut(chat_session_id)
        .ok_or_else(|| format!("No active agent process for session {chat_session_id}"))?;
    let before = proc.pending_messages.len();
    match queued_turn_id {
        Some(id) => proc.pending_messages.retain(|pending| pending.id != id),
        None => proc.pending_messages.clear(),
    }
    let canceled_count = before.saturating_sub(proc.pending_messages.len());
    if queued_turn_id.is_some() && canceled_count == 0 {
        return Err("Queued turn not found".to_string());
    }
    let pending_queue = pending_queue_view(proc);
    Ok(CancelQueuedTurnResponse {
        session_id: chat_session_id.to_string(),
        canceled_count,
        pending_queue_count: pending_queue.len(),
        pending_queue,
    })
}

pub(crate) async fn close_agent_session_internal<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
) -> Result<(), String> {
    #[cfg(unix)]
    let pgid: Option<u32>;
    let child_to_kill: Option<Child>;

    mark_session_closing(chat_session_id).await;
    {
        let mut map = handles.lock().await;
        if let Some(mut proc) = map.remove(chat_session_id) {
            #[cfg(unix)]
            {
                pgid = proc.pgid;
            }
            if let Err(e) = proc.stdin.write_all(b"{\"type\":\"close\"}\n").await {
                log::warn!("Failed to send close command for session {chat_session_id}: {e}");
            }
            if let Err(e) = proc.stdin.flush().await {
                log::warn!("Failed to flush close command for session {chat_session_id}: {e}");
            }
            child_to_kill = Some(proc.child);
        } else {
            // No process to close — already gone. Keep any existing close marker owned by
            // an in-flight close; clearing it here would allow a stale process group race.
            clear_session_closing(chat_session_id).await;
            return Ok(());
        }
    }

    #[cfg(unix)]
    {
        let app_clone = app.clone();
        let csid_for_pid = chat_session_id.to_string();
        tokio::spawn(async move {
            if let Some(mut child) = child_to_kill {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(CLOSE_TIMEOUT_SECS),
                    child.wait(),
                )
                .await
                {
                    Ok(Ok(_)) => {
                        if let Some(pg) = pgid {
                            sweep_process_group(pg).await;
                        }
                    }
                    _ => {
                        if let Some(pg) = pgid {
                            sweep_process_group(pg).await;
                        } else if let Err(e) = child.kill().await {
                            log::warn!("Failed to kill agent process {csid_for_pid}: {e}");
                        }
                        let _ = child.wait().await;
                    }
                }
            }
            if let Ok(data_dir) = resolve_data_dir(&app_clone) {
                remove_pgid(&data_dir, &csid_for_pid);
            }
            clear_session_closing(&csid_for_pid).await;
            prune_session_runtime_lock(&csid_for_pid).await;
        });
    }

    #[cfg(not(unix))]
    if let Some(mut child) = child_to_kill {
        let csid_for_close = chat_session_id.to_string();
        tokio::spawn(async move {
            match tokio::time::timeout(
                std::time::Duration::from_secs(CLOSE_TIMEOUT_SECS),
                child.wait(),
            )
            .await
            {
                Ok(Ok(_)) => {}
                _ => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                }
            }
            clear_session_closing(&csid_for_close).await;
            prune_session_runtime_lock(&csid_for_close).await;
        });
    }

    Ok(())
}

#[cfg(unix)]
async fn sweep_process_group(pgid: u32) {
    unsafe {
        libc::killpg(pgid as libc::pid_t, libc::SIGTERM);
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    unsafe {
        libc::killpg(pgid as libc::pid_t, libc::SIGKILL);
    }
}

/// Force kill remaining processes in the map and clear it.
/// Returns the list of session IDs that were in the map (for pid file cleanup).
async fn force_kill_all_sessions(map: &mut AgentProcessMap) -> Vec<String> {
    let session_ids: Vec<String> = map.keys().cloned().collect();
    for csid in &session_ids {
        if let Some(proc) = map.get_mut(csid) {
            #[cfg(unix)]
            if let Some(pg) = proc.pgid {
                unsafe {
                    libc::killpg(pg as libc::pid_t, libc::SIGKILL);
                }
            }
            #[cfg(not(unix))]
            {
                let _ = proc.child.kill().await;
            }
        }
    }
    map.clear();
    session_ids
}

pub async fn close_all_agent_sessions(
    app: &tauri::AppHandle,
    handles: &Arc<Mutex<AgentProcessMap>>,
) {
    // Send graceful close command to all sessions in a single lock
    {
        let mut map = handles.lock().await;
        let ids: Vec<String> = map.keys().cloned().collect();
        for csid in &ids {
            if let Some(proc) = map.get_mut(csid) {
                let _ = proc.stdin.write_all(b"{\"type\":\"close\"}\n").await;
                let _ = proc.stdin.flush().await;
            }
        }
    }

    // Wait for graceful shutdown
    tokio::time::sleep(std::time::Duration::from_secs(CLOSE_TIMEOUT_SECS)).await;

    // Force kill remaining processes
    let mut map = handles.lock().await;
    let session_ids = force_kill_all_sessions(&mut map).await;
    drop(map);

    // Remove all pid files
    #[cfg(unix)]
    if let Ok(data_dir) = resolve_data_dir(app) {
        for csid in &session_ids {
            remove_pgid(&data_dir, csid);
        }
    }

    #[cfg(not(unix))]
    let _ = app;
}

pub async fn set_agent_permission_mode(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
    permission_mode: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    set_agent_permission_mode_internal(
        session_store.inner(),
        handles.inner(),
        &data_dir,
        &chat_session_id,
        &permission_mode,
    )
    .await
}

/// `set_agent_permission_mode` の内部実装。Tauri コマンドから AppHandle 依存を切り離し、
/// 境界での invalid 値拒否（保存値・current_permission_mode・bridge stdin 不変）を
/// テストから直接検証できるようにする（Spec issues-947）。
pub(crate) async fn set_agent_permission_mode_internal(
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    data_dir: &Path,
    chat_session_id: &str,
    permission_mode: &str,
) -> Result<(), String> {
    // 境界で抽象モードを検証。対象外の値はセッション状態を変更せず bridge にも送らない。
    let pm =
        crate::permission::PermissionMode::parse(permission_mode).map_err(|e| e.to_string())?;

    // Persist to SessionStore（検証済みの抽象モード）
    session_store.update_permission_mode(data_dir, chat_session_id, pm.as_str())?;

    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(chat_session_id) {
            let data = build_set_mode_command_for_mode(pm, &proc.backend_id);
            proc.stdin
                .write_all(data.as_bytes())
                .await
                .map_err(|e| format!("Failed to write setMode: {e}"))?;
            proc.stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush setMode: {e}"))?;
            proc.current_permission_mode = pm.as_str().to_string();
        }
        // If no process exists, silently ignore (process not yet started)
    }

    Ok(())
}

/// `agent-models-updated` イベントの payload を組み立てる。
/// session 単位の available_models / selected_model を frontend へ同期するために使う。
pub(crate) fn build_agent_models_updated_payload(
    chat_session_id: &str,
    available_models: &[ModelInfo],
    selected_model: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "chat_session_id": chat_session_id,
        "available_models": available_models,
        "selected_model": selected_model,
    })
}

pub async fn set_agent_model(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<
        '_,
        Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    >,
    chat_session_id: String,
    model_id: String,
) -> Result<(), String> {
    set_agent_model_internal(
        &app,
        handles.inner(),
        session_store.inner(),
        Some(registry.inner()),
        &chat_session_id,
        model_id,
    )
    .await
}

pub(crate) async fn set_active_process_model(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    model_id: String,
) -> Result<(), String> {
    let data = build_set_model_command(&model_id);
    let mut map = handles.lock().await;
    if let Some(proc) = map.get_mut(chat_session_id) {
        proc.stdin
            .write_all(data.as_bytes())
            .await
            .map_err(|e| format!("Failed to write setModel: {e}"))?;
        proc.stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush setModel: {e}"))?;
        proc.selected_model = Some(model_id);
    }
    Ok(())
}

pub(crate) async fn set_agent_model_internal(
    app: &tauri::AppHandle,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    chat_session_id: &str,
    model_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(app)?;
    set_agent_model_internal_with_data_dir(
        Some(app),
        handles,
        session_store,
        registry,
        &data_dir,
        chat_session_id,
        model_id,
    )
    .await
}

/// `set_agent_model_internal` のテスト用バリエーション。
/// `tauri::AppHandle` の代わりに `data_dir` を直接受け取り、emit は AppHandle が
/// 渡された場合のみ行う。検証ロジックの単体テストに用いる。
pub(crate) async fn set_agent_model_internal_with_data_dir(
    app: Option<&tauri::AppHandle>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    data_dir: &Path,
    chat_session_id: &str,
    model_id: String,
) -> Result<(), String> {
    let meta = session_store
        .get_session_meta(data_dir, chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    let backend_id = meta
        .backend_id
        .clone()
        .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string());
    let resolved_model = match registry {
        Some(reg) => reg.resolve_model_entry(&model_id)?,
        None => ModelInfo::new(&backend_id, &model_id),
    };
    let target_backend_id = resolved_model.backend.clone();
    let target_model_id = resolved_model.model_id.clone();

    // モデルは必須。常に形式検証 + 固定リスト照合を通す（モデル未選択状態は廃止）。
    let model = target_model_id.as_str();
    crate::domain::agent_session::ModelId::parse(model)?;
    if target_backend_id != backend_id {
        if !can_change_session_backend_from_meta(&meta) {
            return Err(format!(
                "Cannot change backend after the first message has been sent: {chat_session_id}"
            ));
        }
        remove_stale_unstarted_agent_process(handles, data_dir, chat_session_id).await;
    }
    if let Some(reg) = registry {
        let session_models: Vec<String> =
            reg.config_models_for(&target_backend_id).map_err(|e| {
                log::warn!(
                "set_agent_model: backend '{target_backend_id}' の登録済みモデル一覧取得に失敗: {e}"
            );
                format!(
                    "バックエンド '{target_backend_id}' の登録済みモデル一覧を取得できません: {e}"
                )
            })?;
        if !session_models.iter().any(|v| v == model) {
            // 「未登録」を伝える前に、別バックエンドに登録されていないかを問い合わせる。
            // - Ok(Some(other)) かつ other != current backend: backend mismatch として返す
            // - Ok(Some(same)) / Ok(None): 当該 backend への未登録として返す
            // - Err: infrastructure 故障。warn を残して当該 backend への未登録として返す
            //   （別バックエンドに登録されているかは判定できないため、ヒントは付けない）
            match reg.resolve_backend_for_model(model) {
                Ok(Some(bid)) if bid != target_backend_id => {
                    return Err(format!(
                        "モデル '{model}' はバックエンド '{target_backend_id}' に登録されていません (別バックエンド '{bid}' に登録)"
                    ));
                }
                Ok(_) => {}
                Err(e) => {
                    log::warn!(
                        "set_agent_model: モデル '{model}' の所属バックエンド解決に失敗（未登録として扱う）: {e}"
                    );
                }
            }
            return Err(format!(
                "モデル '{model}' はバックエンド '{target_backend_id}' に登録されていません"
            ));
        }
    }

    // 1. Send setModel command to Bridge + update process state when the process is active.
    //    proc.available_models は config 単一 owner に追従させるため、active process が
    //    存在する場合も config 由来の最新値で同期する。
    //    infrastructure 故障時は Err を伝播し、proc キャッシュを空一覧で上書きしない。
    let models_from_config = available_models_for_backend(&target_backend_id, registry).map_err(|e| {
        log::warn!(
            "set_agent_model: backend '{target_backend_id}' のモデル一覧取得に失敗したため proc キャッシュ同期を中止: {e}"
        );
        format!("バックエンド '{target_backend_id}' のモデル一覧を取得できません: {e}")
    })?;
    sync_active_process_available_models(handles, chat_session_id, &models_from_config).await;
    set_active_process_model(handles, chat_session_id, target_model_id.clone()).await?;

    // 2. Persist metadata without loading message body.
    session_store.update_backend_selection(
        data_dir,
        chat_session_id,
        target_backend_id.clone(),
        Some(target_model_id.clone()),
    )?;

    // 3. Always emit event to keep frontend in sync.
    //    供給元は常に config.toml（registry 経由）に統一する。
    if let Some(app) = app {
        use tauri::Emitter;
        let _ = app.emit(
            "agent-models-updated",
            build_agent_models_updated_payload(
                chat_session_id,
                &models_from_config,
                Some(resolved_model.id.as_str()),
            ),
        );
    }

    Ok(())
}

/// active process の `available_models` キャッシュを config 由来の最新値で同期する。
/// 永続的なモデル一覧の owner は config.toml 単一であり、process 側のキャッシュは
/// emit 整合用にのみ維持する。
async fn sync_active_process_available_models(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    models: &[ModelInfo],
) {
    let mut map = handles.lock().await;
    if let Some(proc) = map.get_mut(chat_session_id) {
        proc.available_models = models.to_vec();
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn respond_agent_permission(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    registry: tauri::State<
        '_,
        Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    >,
    chat_session_id: String,
    request_id: String,
    behavior: String,
    message: Option<String>,
    updated_input: Option<String>,
) -> Result<(), String> {
    respond_agent_permission_internal(
        &app,
        session_store.inner(),
        handles.inner(),
        registry.inner(),
        chat_session_id,
        request_id,
        behavior,
        message,
        updated_input,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn respond_agent_permission_internal(
    app: &tauri::AppHandle,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    chat_session_id: String,
    request_id: String,
    behavior: String,
    message: Option<String>,
    updated_input: Option<String>,
) -> Result<(), String> {
    if behavior != "allow" && behavior != "deny" {
        return Err(format!("Invalid behavior: {behavior}"));
    }
    let mut result = serde_json::json!({ "behavior": behavior });
    if let Some(msg) = &message {
        result["message"] = serde_json::Value::String(msg.clone());
    }
    let answers_value = if let Some(input_json) = &updated_input {
        match serde_json::from_str::<serde_json::Value>(input_json) {
            Ok(parsed) => {
                result["updatedInput"] = parsed.clone();
                parsed.get("answers").cloned()
            }
            Err(e) => {
                log::warn!("Failed to parse updated_input JSON: {e}");
                None
            }
        }
    } else {
        None
    };
    let payload = serde_json::json!({
        "type": "permission_response",
        "request_id": request_id,
        "result": result,
    });
    let data = format!("{}\n", payload);

    let backend_id = {
        let map = handles.lock().await;
        map.get(&chat_session_id)
            .map(|proc| proc.backend_id.clone())
            .ok_or_else(|| format!("No active agent process for session {chat_session_id}"))?
    };

    if backend_id == CODEX_BACKEND_ID {
        let backend = registry
            .get(&backend_id)
            .ok_or_else(|| format!("Agent backend not found: {backend_id}"))?;
        backend
            .respond_permission(
                &crate::infrastructure::agent_session::runtime::SessionHandle {
                    chat_session_id: chat_session_id.clone(),
                    backend_id: backend_id.clone(),
                },
                crate::infrastructure::agent_session::runtime::PermissionResponse {
                    request_id: request_id.clone(),
                    behavior: behavior.clone(),
                    message: message.clone(),
                    updated_input: updated_input.clone(),
                },
            )
            .await?;
    }

    let did_transition_to_streaming;
    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(&chat_session_id) {
            if backend_id != CODEX_BACKEND_ID {
                proc.stdin
                    .write_all(data.as_bytes())
                    .await
                    .map_err(|e| format!("Failed to write permission response: {e}"))?;
                proc.stdin
                    .flush()
                    .await
                    .map_err(|e| format!("Failed to flush: {e}"))?;
            }

            // Apply the synchronous part of the permission response
            // (phase flip + permission part patch + force flush) via the
            // shared helper so production and unit tests exercise the same
            // ordering: flush must complete before the state-change emit
            // outside the lock.
            let effect = apply_respond_permission_locked(
                proc,
                &chat_session_id,
                &request_id,
                &behavior,
                answers_value.as_ref(),
                |mid, parts| emit_streaming_parts(app, &chat_session_id, mid, parts.to_vec()),
            );
            did_transition_to_streaming = effect.did_transition;

            // Resuming the turn: restart the per-turn auxiliary timer if it
            // has already exited (turn left Streaming when WaitingPermission
            // was entered). Idempotent — no-op if a timer is still alive.
            if did_transition_to_streaming {
                spawn_streaming_timer(app, handles, &chat_session_id, proc);
            }
        } else {
            return Err(format!(
                "No active agent process for session {chat_session_id}"
            ));
        }
    }

    // Emit state change only if we actually transitioned: WaitingPermission → Streaming
    if did_transition_to_streaming {
        emit_session_state_changed(app, &chat_session_id, TurnPhase::Streaming, None);
        notify_status_transition(
            app,
            session_store,
            &chat_session_id,
            TurnPhase::Streaming,
            None,
        );
    }

    Ok(())
}

#[allow(dead_code)]
pub(crate) struct ExternalBridgeMessageState {
    last_persist_time: Instant,
}

impl Default for ExternalBridgeMessageState {
    fn default() -> Self {
        Self {
            last_persist_time: Instant::now(),
        }
    }
}

#[allow(dead_code)]
pub(crate) async fn start_external_agent_turn_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    permission_mode: &str,
    streaming_message_id: &str,
) -> Result<(), String> {
    let canonical_permission_mode =
        crate::permission::PermissionMode::parse(permission_mode).map_err(|e| e.to_string())?;
    {
        let mut map = handles.lock().await;
        let proc = map
            .get_mut(chat_session_id)
            .ok_or_else(|| format!("No agent process for session {chat_session_id}"))?;
        proc.current_permission_mode = canonical_permission_mode.as_str().to_string();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some(streaming_message_id.to_string());
        proc.reset_streaming_state_for_new_turn();
        spawn_streaming_timer(app, handles, chat_session_id, proc);
    }
    emit_session_state_changed(app, chat_session_id, TurnPhase::Streaming, None);
    notify_status_transition(
        app,
        session_store,
        chat_session_id,
        TurnPhase::Streaming,
        None,
    );
    Ok(())
}

#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn register_external_agent_process<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    backend_id: String,
    child: Child,
    stdin: ChildStdin,
    #[cfg(unix)] pgid: Option<u32>,
    permission_mode: String,
    selected_model: Option<String>,
    sdk_session_id: Option<String>,
    context_carry_on_ready: Option<ContextCarryState>,
) -> Result<u64, String> {
    if let Err(err) = crate::permission::PermissionMode::parse(&permission_mode) {
        cleanup_unregistered_agent_process(
            child,
            #[cfg(unix)]
            pgid,
        )
        .await;
        return Err(err.to_string());
    }
    #[cfg(unix)]
    if let Some(pg) = pgid {
        let data_dir = match resolve_data_dir(app)
            .map_err(|e| format!("Failed to resolve data dir for session {chat_session_id}: {e}"))
        {
            Ok(data_dir) => data_dir,
            Err(err) => {
                cleanup_unregistered_agent_process(child, pgid).await;
                return Err(err);
            }
        };
        if let Err(err) = save_pgid(&data_dir, chat_session_id, pg) {
            cleanup_unregistered_agent_process(child, pgid).await;
            return Err(err);
        }
    }

    let gen_id = GENERATION_COUNTER.fetch_add(1, Ordering::SeqCst);
    {
        let mut map = handles.lock().await;
        map.insert(
            chat_session_id.to_string(),
            AgentProcess {
                stdin,
                backend_id,
                state: BridgeState::Initializing,
                turn_phase: TurnPhase::Idle,
                sdk_session_id,
                context_carry_on_ready,
                child,
                generation_id: gen_id,
                #[cfg(unix)]
                pgid,
                streaming_message_id: None,
                streaming_parts: Vec::new(),
                last_message_id: None,
                post_turn_base_untrusted_message_id: None,
                task_id_map: HashMap::new(),
                pending_messages: VecDeque::new(),
                current_permission_mode: permission_mode,
                available_models: Vec::new(),
                selected_model,
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
            },
        );
    }
    notify_status_transition(app, session_store, chat_session_id, TurnPhase::Idle, None);
    Ok(gen_id)
}

async fn cleanup_unregistered_agent_process(mut child: Child, #[cfg(unix)] pgid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pg) = pgid {
        sweep_process_group(pg).await;
    }
    if let Err(e) = child.kill().await {
        log::warn!("Failed to kill unregistered agent process: {e}");
    }
}

#[allow(dead_code)]
pub(crate) async fn close_external_agent_process<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
) -> Result<(), String> {
    mark_session_closing(chat_session_id).await;
    let removed = {
        let mut map = handles.lock().await;
        map.remove(chat_session_id)
    };

    let Some(mut proc) = removed else {
        clear_session_closing(chat_session_id).await;
        return Ok(());
    };

    #[cfg(unix)]
    {
        if let Some(pg) = proc.pgid {
            sweep_process_group(pg).await;
        } else if let Err(e) = proc.child.kill().await {
            log::warn!("Failed to kill external agent process {chat_session_id}: {e}");
        }
        if let Ok(data_dir) = resolve_data_dir(app) {
            remove_pgid(&data_dir, chat_session_id);
        }
    }
    #[cfg(not(unix))]
    if let Err(e) = proc.child.kill().await {
        log::warn!("Failed to kill external agent process {chat_session_id}: {e}");
    }

    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), proc.child.wait()).await;
    clear_session_closing(chat_session_id).await;
    prune_session_runtime_lock(chat_session_id).await;
    Ok(())
}

#[allow(dead_code)]
pub(crate) struct ExternalPendingTurn {
    pub queued_turn_id: String,
    pub worktree_path: String,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
    pub prompt: String,
    pub agent_message_id: String,
    pub images: Vec<ImageAttachment>,
    pub editor_context: Option<AgentEditorContext>,
}

#[allow(dead_code)]
pub(crate) async fn prepare_external_pending_message_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
) -> Result<Option<ExternalPendingTurn>, String> {
    let Some(pending) = take_pending_message(handles, chat_session_id).await else {
        return Ok(None);
    };

    let data_dir = match resolve_data_dir(app) {
        Ok(data_dir) => data_dir,
        Err(e) => {
            clear_pending_turn_starting(chat_session_id).await;
            return Err(format!("failed to resolve data dir: {e}"));
        }
    };
    let (human_msg, agent_msg, emit_consumed_messages) =
        match prepare_pending_turn_messages(session_store, &data_dir, chat_session_id, &pending) {
            Ok(messages) => messages,
            Err(e) => {
                clear_pending_turn_starting(chat_session_id).await;
                return Err(format!("failed to prepare pending messages: {e}"));
            }
        };
    let permission_profile_id = session_store
        .get_session_meta(&data_dir, chat_session_id)?
        .and_then(|meta| meta.permission_profile_id);

    if emit_consumed_messages {
        use tauri::Emitter;
        let _ = app.emit(
            "agent-pending-message-consumed",
            serde_json::json!({
                "chat_session_id": chat_session_id,
                "queued_turn_id": pending.id,
                "human_message": human_msg,
                "agent_message": agent_msg,
            }),
        );
    }

    let prompt = app
        .state::<crate::adaptor::controller::state::AppState>()
        .code_usecase
        .resolve_mentions_or_fallback(&pending.worktree_path, &pending.content, &pending.mentions);

    Ok(Some(ExternalPendingTurn {
        queued_turn_id: pending.id,
        worktree_path: pending.worktree_path,
        permission_mode: pending.permission_mode,
        plan_mode: pending.plan_mode,
        permission_profile_id,
        prompt,
        agent_message_id: agent_msg.id,
        images: pending.images,
        editor_context: pending.editor_context,
    }))
}

#[allow(dead_code)]
pub(crate) async fn finish_external_pending_message_turn_start(chat_session_id: &str) {
    clear_pending_turn_starting(chat_session_id).await;
}

fn workflow_final_text_parts(final_parts: &[MessagePart]) -> Vec<String> {
    final_parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

/// turn_complete 後の Workflow Engine 通知と pending message 消費を、
/// `session_runtime_lock` を保持しない経路で実施する共通ヘルパー。
///
/// engine 内で同 session への turn 再投入があると tokio Mutex の非再入性により
/// 再入デッドロックするため、呼び出し側は lock を保持してはならない。内部で
/// `std::thread::spawn + block_on` し、呼び出し元の lock スコープから切り離す。
///
/// `pending` は streaming 中にキューされた人間メッセージ（無ければ `None`）。
/// engine 通知後に消費する。Codex app-server は独自の pending キュー
/// (`start_next_app_server_pending_turn`) を持つため、その経路では `None` を渡す。
///
/// Claude（stdout 読み取りループ）と Codex/legacy（`handle_external_bridge_message`）
/// の両経路から呼ばれ、turn 完了 → workflow 進行の通知ロジックを一本化する。
#[allow(clippy::too_many_arguments)]
fn spawn_workflow_turn_complete_notification<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    session_store: Arc<SessionStore>,
    handles: Arc<Mutex<AgentProcessMap>>,
    chat_session_id: String,
    exit_code: i64,
    final_parts: Vec<MessagePart>,
    token_usage: Option<(u64, u64)>,
    pending: Option<PendingMessage>,
) {
    let workflow_runtime: Option<Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>> = app
        .try_state::<Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>>()
        .map(|s| Arc::clone(&s));
    let handle = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        handle.block_on(async move {
            if let Some(runtime) = workflow_runtime {
                if runtime.is_session_running(&chat_session_id).await {
                    let final_text_parts = workflow_final_text_parts(&final_parts);
                    let token_usage = token_usage.map(|(input_tokens, output_tokens)| {
                        crate::usecase::workflow::ports::WorkflowTurnTokenUsage {
                            input_tokens,
                            output_tokens,
                        }
                    });
                    let command = crate::usecase::workflow::ports::WorkflowTurnCompleteCommand {
                        chat_session_id: chat_session_id.clone(),
                        exit_code,
                        final_text_parts,
                        token_usage,
                    };
                    if let Err(e) = runtime.complete_turn(command).await {
                        log::error!("Workflow turn completion error for {chat_session_id}: {e}");
                    }
                }
            }
            if let Some(pending) = pending {
                start_pending_message_turn(
                    &app,
                    &handles,
                    &session_store,
                    &chat_session_id,
                    pending,
                )
                .await;
            }
        });
    });
}

#[allow(dead_code)]
pub(crate) async fn handle_external_bridge_message<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    mut msg: serde_json::Value,
    state: &mut ExternalBridgeMessageState,
) {
    use tauri::Emitter;

    msg["chat_session_id"] = serde_json::Value::String(chat_session_id.to_string());
    let defer_agent_session_id_persist_on_ready =
        take_defer_agent_session_id_persist_on_ready(&mut msg);
    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match msg_type {
        "session_ready" => {
            let (context_carry_on_ready, resume_mismatch) = {
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(chat_session_id) {
                    if proc.state == BridgeState::Initializing {
                        proc.state = BridgeState::Ready;
                    }
                    let ready_session_id = msg.get("session_id").and_then(|v| v.as_str());
                    let requested_resume_id = proc.sdk_session_id.clone();
                    let context_carry_on_ready = proc.context_carry_on_ready.take();
                    let resume_mismatch = session_ready_resume_mismatch(
                        context_carry_on_ready.as_ref(),
                        requested_resume_id.as_deref(),
                        ready_session_id,
                    );
                    if let Some(sid) = ready_session_id {
                        proc.sdk_session_id = Some(sid.to_string());
                        if !resume_mismatch && !defer_agent_session_id_persist_on_ready {
                            persist_agent_session_id(app, session_store, chat_session_id, sid);
                        }
                    }
                    (context_carry_on_ready, resume_mismatch)
                } else {
                    (None, false)
                }
            };
            if resume_mismatch {
                persist_resume_mismatch_for_reinject(app, session_store, chat_session_id);
                crash_agent_process_for_context_reinject(app, handles, chat_session_id).await;
                return;
            } else if let Some(context_carry) = context_carry_on_ready {
                persist_context_carry_state(app, session_store, chat_session_id, context_carry);
            }
            let _ = app.emit("agent-sdk-message", &msg);
        }
        "result" => {
            if let Some(token_usage) = token_usage_from_result_message(&msg) {
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(chat_session_id) {
                    proc.last_result_token_usage =
                        Some((token_usage.input_tokens, token_usage.output_tokens));
                    proc.latest_token_usage = Some(token_usage);
                }
            }
            let _ = app.emit("agent-sdk-message", &msg);
        }
        "turn_complete" => {
            let exit_code = msg.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0);
            let completed_session_id = msg
                .get("session_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let (effect, context_restore_failed_on_init) = {
                let _runtime_guard = acquire_session_runtime_lock(chat_session_id).await;
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(chat_session_id) {
                    let effect = run_turn_complete_transition_locked(
                        proc,
                        chat_session_id,
                        exit_code,
                        |mid, parts| {
                            emit_streaming_parts(app, chat_session_id, mid, parts.to_vec())
                        },
                    );
                    let context_restore_failed_on_init = !effect.was_streaming
                        && exit_code != 0
                        && proc.context_carry_on_ready.take().is_some();
                    (Some(effect), context_restore_failed_on_init)
                } else {
                    (None, false)
                }
            };

            let Some(effect) = effect else {
                return;
            };
            let TurnCompleteTransition {
                was_streaming,
                final_msg_id,
                final_parts,
                turn_token_usage,
                released_streaming_parts,
            } = effect;
            if was_streaming {
                if exit_code == 0 {
                    if let Some(sid) = completed_session_id.as_deref() {
                        persist_agent_session_id(app, session_store, chat_session_id, sid);
                    }
                }
                if let Some(ref mid) = final_msg_id {
                    if !final_parts.is_empty() {
                        let persisted = persist_streaming_parts(
                            session_store,
                            app,
                            chat_session_id,
                            mid,
                            &final_parts,
                            Some(now_timestamp()),
                        );
                        if persisted {
                            clear_post_turn_store_base_untrusted_for_message(
                                handles,
                                chat_session_id,
                                mid,
                            )
                            .await;
                        }
                    }
                }
                drop(released_streaming_parts);
                emit_session_state_changed(app, chat_session_id, TurnPhase::Idle, Some(exit_code));
                let override_state = if exit_code != 0 {
                    Some(crate::usecase::agent_session::session::SessionState::Error)
                } else {
                    None
                };
                notify_status_transition(
                    app,
                    session_store,
                    chat_session_id,
                    TurnPhase::Idle,
                    override_state,
                );
                // Codex app-server は独自の pending キュー
                // (`start_next_app_server_pending_turn`) で follow-up turn を起動するため、
                // ここでは pending を消費しない（legacy external bridge のみ消費する）。
                let pending = {
                    let is_legacy_bridge = {
                        let map = handles.lock().await;
                        map.get(chat_session_id)
                            .is_some_and(|proc| proc.backend_id != CODEX_BACKEND_ID)
                    };
                    if is_legacy_bridge {
                        take_pending_message(handles, chat_session_id).await
                    } else {
                        None
                    }
                };
                // Claude(stdout loop) と同じ共通ヘルパーで Workflow Engine へ通知する。
                // これが無いと Codex の turn 完了が engine に届かずワークフローが進まない。
                spawn_workflow_turn_complete_notification(
                    app.clone(),
                    Arc::clone(session_store),
                    Arc::clone(handles),
                    chat_session_id.to_string(),
                    exit_code,
                    final_parts,
                    turn_token_usage,
                    pending,
                );
            } else if exit_code != 0 {
                persist_context_carry_failed_after_init_error(
                    app,
                    session_store,
                    chat_session_id,
                    true,
                    context_restore_failed_on_init,
                );
            }
        }
        "error" => {
            let transition = {
                let _runtime_guard = acquire_session_runtime_lock(chat_session_id).await;
                let mut map = handles.lock().await;
                map.get_mut(chat_session_id).map(|proc| {
                    run_bridge_error_transition_locked(proc, chat_session_id, &msg, |mid, parts| {
                        emit_streaming_parts(app, chat_session_id, mid, parts.to_vec())
                    })
                })
            };
            let _ = app.emit("agent-sdk-message", &msg);

            let transition = transition.unwrap_or_default();
            let effect = transition.turn_complete;
            if effect.was_streaming {
                if let Some(ref mid) = effect.final_msg_id {
                    if !effect.final_parts.is_empty() {
                        let persisted = persist_streaming_parts(
                            session_store,
                            app,
                            chat_session_id,
                            mid,
                            &effect.final_parts,
                            Some(now_timestamp()),
                        );
                        if persisted {
                            clear_post_turn_store_base_untrusted_for_message(
                                handles,
                                chat_session_id,
                                mid,
                            )
                            .await;
                        }
                    }
                }
                emit_session_state_changed(app, chat_session_id, TurnPhase::Idle, Some(1));
                notify_status_transition(
                    app,
                    session_store,
                    chat_session_id,
                    TurnPhase::Idle,
                    Some(crate::usecase::agent_session::session::SessionState::Error),
                );
                let pending = {
                    let is_legacy_bridge = {
                        let map = handles.lock().await;
                        map.get(chat_session_id)
                            .is_some_and(|proc| proc.backend_id != CODEX_BACKEND_ID)
                    };
                    if is_legacy_bridge {
                        take_pending_message(handles, chat_session_id).await
                    } else {
                        None
                    }
                };
                spawn_workflow_turn_complete_notification(
                    app.clone(),
                    Arc::clone(session_store),
                    Arc::clone(handles),
                    chat_session_id.to_string(),
                    1,
                    effect.final_parts,
                    effect.turn_token_usage,
                    pending,
                );
            } else if transition.was_initializing {
                notify_status_transition(
                    app,
                    session_store,
                    chat_session_id,
                    TurnPhase::Idle,
                    Some(crate::usecase::agent_session::session::SessionState::Error),
                );
            }
            if transition.was_initializing
                || msg
                    .get("clear_session_id")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                || msg
                    .get("context_carry_failed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            {
                persist_context_carry_failed_after_init_error(
                    app,
                    session_store,
                    chat_session_id,
                    transition.was_initializing
                        || msg
                            .get("clear_session_id")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                    transition.context_restore_failed_on_init
                        || msg
                            .get("context_carry_failed")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                );
            }
        }
        _ => {
            let elapsed_persist = state.last_persist_time.elapsed().as_millis() as u64;
            let mut effect = accumulate_stream_or_post_turn_message(
                app,
                session_store,
                handles,
                chat_session_id,
                &msg,
                elapsed_persist,
            )
            .await;

            if effect.should_persist {
                if let Some(ref mid) = effect.emit_msg_id {
                    state.last_persist_time = Instant::now();
                    let persisted = persist_streaming_parts(
                        session_store,
                        app,
                        chat_session_id,
                        mid,
                        &effect.persist_parts,
                        None,
                    );
                    if persisted {
                        clear_post_turn_store_base_untrusted_for_message(
                            handles,
                            chat_session_id,
                            mid,
                        )
                        .await;
                    }
                }
            }
            drop(std::mem::take(&mut effect.released_streaming_parts));

            let permission_did_transition = if msg_type == "permission_request" {
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(chat_session_id) {
                    let effect = run_permission_request_transition_locked(
                        proc,
                        chat_session_id,
                        |mid, parts| {
                            emit_streaming_parts(app, chat_session_id, mid, parts.to_vec())
                        },
                    );
                    effect.did_transition
                } else {
                    false
                }
            } else {
                false
            };

            if should_forward_sdk_message(effect.accumulated, msg_type) {
                let _ = app.emit("agent-sdk-message", &msg);
            }
            if permission_did_transition {
                emit_session_state_changed(
                    app,
                    chat_session_id,
                    TurnPhase::WaitingPermission,
                    None,
                );
                notify_status_transition(
                    app,
                    session_store,
                    chat_session_id,
                    TurnPhase::WaitingPermission,
                    None,
                );
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageResponse {
    pub session: ChatSession,
    pub human_message: ChatMessage,
    pub agent_message: Option<ChatMessage>,
    pub queued_turn: Option<crate::usecase::agent_session::session::QueuedAgentTurn>,
    pub pending_queue: Vec<crate::usecase::agent_session::session::QueuedAgentTurn>,
    pub pending_queue_count: usize,
    pub sessions: Vec<SessionSummary>,
}

struct PreparedAgentTurn {
    session_id: String,
    backend_id: String,
    worktree_path: String,
    permission_mode: String,
    plan_mode: bool,
    prompt: String,
    agent_message_id: String,
    images: Vec<ImageAttachment>,
    editor_context: Option<AgentEditorContext>,
}

struct PreparedAgentSteer {
    session_id: String,
    backend_id: String,
    permission_mode: String,
    plan_mode: bool,
    prompt: String,
    steering_message_id: String,
    images: Vec<ImageAttachment>,
    editor_context: Option<AgentEditorContext>,
}

enum PreparedAgentRuntimeInput {
    Turn(PreparedAgentTurn),
    Steer(PreparedAgentSteer),
}

#[allow(clippy::too_many_arguments)]
async fn prepare_send_agent_message_internal(
    code_usecase: &crate::usecase::code_usecase::CodeUsecase,
    session_store: &Arc<SessionStore>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    data_dir: &Path,
    chat_session_id: Option<String>,
    worktree_path: String,
    content: String,
    permission_mode: crate::permission::PermissionMode,
    plan_mode: bool,
    backend_id: Option<String>,
    model_id: Option<String>,
    images: Option<Vec<ImageAttachment>>,
    mentions: Option<Vec<crate::domain::code::MentionReference>>,
    editor_context: Option<AgentEditorContext>,
) -> Result<(SendMessageResponse, Option<PreparedAgentRuntimeInput>), String> {
    let pm = permission_mode.as_str().to_string();
    let images = images.unwrap_or_default();
    let mentions = mentions.unwrap_or_default();

    // 1. Create or get session
    let session = if let Some(ref sid) = chat_session_id {
        let mut session = session_store
            .get_session_shell(data_dir, sid)?
            .ok_or_else(|| format!("Session not found: {sid}"))?;
        if !session.is_workflow_step_session() && session.worktree_path != worktree_path {
            return Err(session_target_rejected());
        }
        // 既存セッション分岐でも検証済み pm をセッション保存層に書き戻す。
        // 新規セッション分岐と対称化し、リモート UI で start → message とした場合に
        // 選択した permission_mode が ChatSession.permission_mode に反映されるようにする。
        if session.permission_mode != pm {
            session_store.update_permission_mode(data_dir, sid, &pm)?;
            session.permission_mode = pm.clone();
        }
        if session.plan_mode != plan_mode {
            session_store.update_plan_mode(data_dir, sid, plan_mode)?;
            session.plan_mode = plan_mode;
        }
        ensure_session_backend_selected(session_store, registry, data_dir, session)?
    } else {
        let resolved_model = match model_id.as_deref() {
            Some(model_id) => Some(registry.resolve_model_entry(model_id)?),
            None => None,
        };
        let requested_backend_id = resolved_model
            .as_ref()
            .map(|entry| entry.backend.clone())
            .or(backend_id);
        let resolved_backend_id = registry.resolve_backend_id(requested_backend_id)?;
        // 新規セッションは検証済み抽象モードを初回保存で確定する。
        // 既定値で save → update_permission_mode の二段階保存を行うと、途中失敗時に
        // 選択値ではない permission_mode で永続化されたセッションが残ってしまうため
        // （Spec issues-947: セッション保存層が permission_mode の正典）、生成 API を一本化する。
        // backend の登録済み初期モデルがあれば selected_model に永続化する（Spec issues-946）。
        crate::usecase::agent_session::session::create_session_with_model_and_plan_mode(
            session_store,
            registry,
            data_dir,
            &worktree_path,
            resolved_backend_id,
            permission_mode,
            resolved_model.map(|entry| entry.model_id),
            plan_mode,
        )?
    };
    let sid = session.id.clone();
    let session_worktree_path = session.worktree_path.clone();
    let session_backend_id = session
        .backend_id
        .clone()
        .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string());

    // 2. Compute human message parts.
    // 永続化のタイミングは busy 判定後の分岐で決める。キュー投入時は session に
    // 即時追加すると transcript とキューUI に二重表示されるため、ここでは追加せず
    // drain（start_pending_message_turn / prepare_external_pending_message_turn）で追加する。
    let human_parts = if images.is_empty() {
        None
    } else {
        let mut p: Vec<MessagePart> = Vec::new();
        if !content.is_empty() {
            p.push(MessagePart::Text {
                content: content.clone(),
                parent_tool_use_id: None,
            });
        }
        for img in &images {
            p.push(MessagePart::Image {
                data: img.data.clone(),
                media_type: img.media_type.clone(),
            });
        }
        Some(p)
    };
    let human_mentions = if mentions.is_empty() {
        None
    } else {
        Some(mentions.clone())
    };

    // 3. Check turn phase
    let (current_phase, current_state, has_pending_messages) = {
        let map = handles.lock().await;
        map.get(&sid)
            .map(|p| (p.turn_phase, p.state, !p.pending_messages.is_empty()))
            .unwrap_or((TurnPhase::Idle, BridgeState::Ready, false))
    };

    // Initializing だけでは active turn とみなさない。Claude bridge は最初の
    // prompt が渡されるまで session_ready を出さないため、復帰直後の idle な
    // Initializing process には初回発話を直接送る必要がある。
    let initializing_active_turn =
        current_state == BridgeState::Initializing && current_phase != TurnPhase::Idle;
    let active_turn_busy = current_phase == TurnPhase::Streaming
        || current_phase == TurnPhase::WaitingPermission
        || initializing_active_turn;
    let pending_turn_starting = is_pending_turn_starting(&sid).await;
    let pending_queue_busy = has_pending_messages || pending_turn_starting;
    let turn_busy = active_turn_busy || pending_queue_busy;

    let (human_message, agent_message, prepared_input, queued_turn) = if turn_busy {
        let can_steer_active_turn = if active_turn_busy && !pending_turn_starting {
            if let Some(backend) = registry.get(&session_backend_id) {
                let session_handle = crate::infrastructure::agent_session::runtime::SessionHandle {
                    chat_session_id: sid.clone(),
                    backend_id: session_backend_id.clone(),
                };
                backend.active_turn_steering_ready(&session_handle).await
            } else {
                false
            }
        } else {
            false
        };

        if can_steer_active_turn {
            // steer は即座にアクティブターンへ流し込むため、人間メッセージを永続化する。
            let human_message = add_message_internal(
                session_store,
                data_dir,
                &sid,
                MessageRole::Human,
                &content,
                human_parts.clone(),
                human_mentions.clone(),
            )?;
            let resolved_prompt = code_usecase.resolve_mentions_or_fallback(
                &session_worktree_path,
                &content,
                &mentions,
            );
            let steer = PreparedAgentSteer {
                session_id: sid.clone(),
                backend_id: session_backend_id.clone(),
                permission_mode: pm.clone(),
                plan_mode,
                prompt: resolved_prompt,
                steering_message_id: human_message.id.clone(),
                images: images.clone(),
                editor_context,
            };
            (
                human_message,
                None,
                Some(PreparedAgentRuntimeInput::Steer(steer)),
                None,
            )
        } else {
            // 4a. Queue pending message + interrupt
            // 人間メッセージはここでは永続化しない（transcript とキューUI の二重表示を
            // 避けるため）。drain 時に各 drain 関数が永続化する。response 用には
            // 非永続の ChatMessage を構築して返す。
            let pending = PendingMessage {
                id: uuid::Uuid::new_v4().to_string(),
                content: content.clone(),
                created_at: now_timestamp(),
                permission_mode: pm.clone(),
                plan_mode,
                images: images.clone(),
                worktree_path: session_worktree_path.clone(),
                mentions: mentions.clone(),
                editor_context: editor_context.clone(),
                existing_human_message_id: None,
                existing_agent_message_id: None,
            };
            let queued_turn = pending_message_to_queued_turn(&pending);
            let transient_human = ChatMessage {
                id: uuid::Uuid::new_v4().to_string(),
                role: MessageRole::Human,
                content: content.clone(),
                thinking: None,
                activities: None,
                parts: human_parts.clone(),
                timestamp: now_timestamp(),
                mentions: None,
            };
            {
                let mut map = handles.lock().await;
                let proc = map
                    .get_mut(&sid)
                    .ok_or_else(|| format!("No active agent process for session {sid}"))?;
                proc.pending_messages.push_back(pending);
            }
            if active_turn_busy && !pending_turn_starting {
                interrupt_active_agent_turn(handles, registry, &sid).await?;
            }
            (transient_human, None, None, Some(queued_turn))
        }
    } else {
        // 4b. Create human + agent message, start turn
        let human_message = add_message_internal(
            session_store,
            data_dir,
            &sid,
            MessageRole::Human,
            &content,
            human_parts.clone(),
            human_mentions.clone(),
        )?;
        let agent_msg = add_message_internal(
            session_store,
            data_dir,
            &sid,
            MessageRole::Agent,
            "",
            None,
            None,
        )?;
        let resolved_prompt =
            code_usecase.resolve_mentions_or_fallback(&session_worktree_path, &content, &mentions);
        let turn = PreparedAgentTurn {
            session_id: sid.clone(),
            backend_id: session
                .backend_id
                .clone()
                .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string()),
            worktree_path: session_worktree_path.clone(),
            permission_mode: pm.clone(),
            plan_mode,
            prompt: resolved_prompt,
            agent_message_id: agent_msg.id.clone(),
            images: images.clone(),
            editor_context,
        };
        (
            human_message,
            Some(agent_msg),
            Some(PreparedAgentRuntimeInput::Turn(turn)),
            None,
        )
    };

    // 5. Get updated session shell and list. Message bodies are returned through
    // human_message / agent_message and page APIs, not the session envelope.
    let updated_session = session_store
        .get_session_shell(data_dir, &sid)?
        .ok_or_else(|| format!("Session not found: {sid}"))?;
    let sessions = session_store.list_sessions(data_dir, &session_worktree_path)?;
    let pending_queue = {
        let map = handles.lock().await;
        map.get(&sid).map(pending_queue_view).unwrap_or_default()
    };

    Ok((
        SendMessageResponse {
            session: updated_session,
            human_message,
            agent_message,
            queued_turn,
            pending_queue_count: pending_queue.len(),
            pending_queue,
            sessions,
        },
        prepared_input,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn start_prepared_agent_turn(
    app: &tauri::AppHandle,
    session_store: &Arc<SessionStore>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    turn: PreparedAgentTurn,
) -> Result<(), String> {
    if turn.backend_id == CODEX_BACKEND_ID {
        let backend = registry
            .get(&turn.backend_id)
            .ok_or_else(|| format!("Agent backend not found: {}", turn.backend_id))?;
        return backend
            .send_message(
                &crate::infrastructure::agent_session::runtime::SessionHandle {
                    chat_session_id: turn.session_id,
                    backend_id: turn.backend_id,
                },
                crate::infrastructure::agent_session::runtime::AgentMessage {
                    content: turn.prompt,
                    streaming_message_id: turn.agent_message_id,
                    images: turn.images,
                    permission_mode: turn.permission_mode,
                    plan_mode: turn.plan_mode,
                    permission_profile_id: None,
                    editor_context: turn.editor_context,
                },
            )
            .await;
    }

    start_agent_turn(
        app,
        handles,
        session_store,
        &turn.session_id,
        &turn.worktree_path,
        &turn.permission_mode,
        turn.plan_mode,
        &turn.prompt,
        &turn.agent_message_id,
        &turn.images,
    )
    .await
}

async fn steer_prepared_agent_turn(
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    steer: PreparedAgentSteer,
) -> Result<(), String> {
    let backend = registry
        .get(&steer.backend_id)
        .ok_or_else(|| format!("Agent backend not found: {}", steer.backend_id))?;
    backend
        .steer_message(
            &crate::infrastructure::agent_session::runtime::SessionHandle {
                chat_session_id: steer.session_id,
                backend_id: steer.backend_id,
            },
            crate::infrastructure::agent_session::runtime::AgentMessage {
                content: steer.prompt,
                streaming_message_id: steer.steering_message_id,
                images: steer.images,
                permission_mode: steer.permission_mode,
                plan_mode: steer.plan_mode,
                permission_profile_id: None,
                editor_context: steer.editor_context,
            },
        )
        .await
}

/// Unified command to send a message: handles session creation, message persistence,
/// turn phase check (interrupt if streaming, start query if idle), and pending message queuing.
#[allow(clippy::too_many_arguments)]
pub async fn send_agent_message_internal(
    app: &tauri::AppHandle,
    session_store: &Arc<SessionStore>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: Option<String>,
    worktree_path: String,
    content: String,
    permission_mode: crate::permission::PermissionMode,
    plan_mode: bool,
    backend_id: Option<String>,
    model_id: Option<String>,
    images: Option<Vec<ImageAttachment>>,
    mentions: Option<Vec<crate::domain::code::MentionReference>>,
    editor_context: Option<AgentEditorContext>,
) -> Result<SendMessageResponse, String> {
    let lock_key = chat_session_id
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| format!("new-session:{worktree_path}"));
    let data_dir = resolve_data_dir(app)?;
    let code_usecase = Arc::clone(
        &app.state::<crate::adaptor::controller::state::AppState>()
            .code_usecase,
    );
    let (response, prepared_input) = {
        let _send_guard = acquire_session_runtime_lock(&lock_key).await;
        prepare_send_agent_message_internal(
            &code_usecase,
            session_store,
            registry,
            handles,
            &data_dir,
            chat_session_id,
            worktree_path,
            content,
            permission_mode,
            plan_mode,
            backend_id,
            model_id,
            images,
            mentions,
            editor_context,
        )
        .await?
    };

    if let Some(input) = prepared_input {
        match input {
            PreparedAgentRuntimeInput::Turn(turn) => {
                start_prepared_agent_turn(app, session_store, registry, handles, turn).await?;
            }
            PreparedAgentRuntimeInput::Steer(steer) => {
                steer_prepared_agent_turn(registry, steer).await?;
            }
        }
    }

    Ok(response)
}

#[allow(clippy::too_many_arguments)]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitSessionsResponse {
    pub sessions: Vec<SessionSummary>,
    pub active_session: Option<GetSessionResponse>,
    pub permission_mode: String,
    pub plan_mode: bool,
}

/// Unified command for session initialization: lists sessions, starts Bridge processes,
/// creates a new session if empty, returns sessions + active session.
pub async fn init_agent_sessions(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<
        '_,
        Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    >,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    open_tabs: tauri::State<'_, Arc<crate::usecase::agent_session::session::OpenTabRegistry>>,
    worktree_path: String,
) -> Result<InitSessionsResponse, String> {
    init_agent_sessions_internal(
        &app,
        session_store.inner(),
        registry.inner(),
        handles.inner(),
        open_tabs.inner(),
        worktree_path,
    )
    .await
}

pub(crate) async fn init_agent_sessions_internal<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &Arc<crate::usecase::agent_session::session::OpenTabRegistry>,
    worktree_path: String,
) -> Result<InitSessionsResponse, String> {
    let data_dir = resolve_data_dir(app)?;

    crate::adaptor::gateway::workflow::hydrate_open_workflow_step_tabs(
        session_store,
        &data_dir,
        &worktree_path,
        open_tabs,
    )?;
    let sessions = session_store.list_sessions(&data_dir, &worktree_path)?;

    if sessions.is_empty() {
        Ok(InitSessionsResponse {
            sessions,
            active_session: None,
            permission_mode: crate::permission::PermissionMode::Edit.as_str().to_string(),
            plan_mode: false,
        })
    } else {
        // spec issues-1023: workflow step として起動された chat session は free chat
        // tab bar 上に同格に並ばないため、初期 active session 候補からも除外する。
        // 候補が無い場合は active_session を None で返し、UI は空状態を描く。
        let active_candidate = pick_initial_active_session_candidate(&sessions);
        let active = if let Some(candidate) = active_candidate {
            get_session_internal(session_store, handles, Some(registry), app, &candidate.id).await?
        } else {
            None
        };
        let (permission_mode, plan_mode) = active
            .as_ref()
            .map(|response| {
                (
                    response.session.permission_mode.clone(),
                    response.session.plan_mode,
                )
            })
            .unwrap_or_else(|| {
                (
                    crate::permission::PermissionMode::Edit.as_str().to_string(),
                    false,
                )
            });

        Ok(InitSessionsResponse {
            sessions,
            active_session: active,
            permission_mode,
            plan_mode,
        })
    }
}

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct SlashCommandEntry {
    pub name: String,
    pub description: String,
    #[serde(
        rename = "argumentHint",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub argument_hint: Option<String>,
}

fn normalize_supported_command_name(raw: &str) -> String {
    raw.trim().trim_start_matches('/').to_string()
}

fn supported_commands_from_bridge_message(msg: &serde_json::Value) -> Vec<SlashCommandEntry> {
    let Some(commands) = msg.get("commands").and_then(|value| value.as_array()) else {
        return Vec::new();
    };

    commands
        .iter()
        .filter_map(|command| {
            let obj = command.as_object()?;
            let name = normalize_supported_command_name(obj.get("name")?.as_str()?);
            if name.is_empty() {
                return None;
            }
            let description = obj
                .get("description")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let argument_hint = obj
                .get("argumentHint")
                .or_else(|| obj.get("argument_hint"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            Some(SlashCommandEntry {
                name,
                description,
                argument_hint,
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub scope: String,
}

/// Parse SKILL.md frontmatter (delimited by `---`) and extract `name` / `description` fields.
fn parse_skill_frontmatter(content: &str) -> Option<(String, String)> {
    let mut lines = content.lines();
    // First line must be `---`
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(val) = trimmed.strip_prefix("name:") {
            name = Some(val.trim().to_string());
        } else if let Some(val) = trimmed.strip_prefix("description:") {
            description = Some(val.trim().to_string());
        }
    }
    Some((name.unwrap_or_default(), description.unwrap_or_default()))
}

fn normalized_scanner_backend_id(backend_id: Option<&str>) -> &'static str {
    match backend_id {
        Some(id) if id == CODEX_BACKEND_ID => CODEX_BACKEND_ID,
        _ => CLAUDE_BACKEND_ID,
    }
}

fn agent_skill_dirs_for_backend(
    cwd: &Path,
    backend_id: Option<&str>,
    home: Option<PathBuf>,
) -> Vec<(PathBuf, &'static str)> {
    let mut dirs = Vec::new();
    match normalized_scanner_backend_id(backend_id) {
        CODEX_BACKEND_ID => {
            if let Some(home) = home {
                dirs.push((home.join(".agents").join("skills"), "personal"));
            }
            dirs.push((cwd.join(".agents").join("skills"), "project"));
        }
        _ => {
            if let Some(home) = home {
                dirs.push((home.join(".claude").join("skills"), "personal"));
            }
            dirs.push((cwd.join(".claude").join("skills"), "project"));
        }
    }
    dirs
}

fn scan_agent_skills_inner(
    cwd: &Path,
    backend_id: Option<&str>,
    home: Option<PathBuf>,
) -> Vec<SkillEntry> {
    let mut skills = Vec::new();
    for (skills_dir, scope) in agent_skill_dirs_for_backend(cwd, backend_id, home) {
        if let Ok(entries) = std::fs::read_dir(skills_dir) {
            for entry in entries.flatten() {
                let skill_md = entry.path().join("SKILL.md");
                if !skill_md.is_file() {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&skill_md) {
                    if let Some((name, description)) = parse_skill_frontmatter(&content) {
                        if !name.is_empty() {
                            skills.push(SkillEntry {
                                name,
                                description,
                                scope: scope.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
    skills
}

pub(crate) fn filter_agent_skills_for_query(
    skills: Vec<SkillEntry>,
    query: Option<&str>,
    limit: Option<usize>,
) -> Vec<SkillEntry> {
    let needle = query.unwrap_or_default().trim().to_lowercase();
    let max_results = limit.unwrap_or(usize::MAX);
    skills
        .into_iter()
        .filter(|skill| {
            needle.is_empty()
                || skill.name.to_lowercase().contains(&needle)
                || skill.description.to_lowercase().contains(&needle)
        })
        .take(max_results)
        .collect()
}

pub async fn scan_agent_skills(
    cwd: String,
    backend_id: Option<String>,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SkillEntry>, String> {
    let home = dirs::home_dir();
    Ok(filter_agent_skills_for_query(
        scan_agent_skills_inner(&PathBuf::from(cwd), backend_id.as_deref(), home),
        query.as_deref(),
        limit,
    ))
}

// --- Image attachment support ---

#[derive(Default)]
struct BridgeInitOptions<'a> {
    system_prompt: Option<String>,
    selected_model: Option<&'a str>,
    restore_context: Option<&'a RestoreContextPayload>,
}

/// 抽象モード文字列 + backend_id を受け取り、バックエンド固有の init コマンドを構築する。
fn build_init_cmd(
    cwd: &str,
    permission_mode: &str,
    plan_mode: bool,
    session_id: &Option<String>,
    backend_id: &str,
    options: BridgeInitOptions<'_>,
) -> Result<serde_json::Value, String> {
    let pm =
        crate::permission::PermissionMode::parse(permission_mode).map_err(|e| e.to_string())?;
    let mut cmd = serde_json::json!({
        "type": "init",
        "cwd": cwd,
        "sessionId": session_id,
    });
    if let Some(obj) = cmd.as_object_mut() {
        for (k, v) in bridge_permission_fields(pm, backend_id, plan_mode) {
            obj.insert(k, v);
        }
    }
    if let Some(sp) = options.system_prompt {
        cmd["systemPrompt"] = serde_json::Value::String(sp);
    }
    if let Some(model) = options.selected_model {
        cmd["model"] = serde_json::Value::String(model.to_string());
    }
    if let Some(restore_context) = options.restore_context {
        if !restore_context.prompt_prefix.trim().is_empty() {
            cmd["restoreContext"] = serde_json::to_value(restore_context)
                .map_err(|e| format!("Failed to serialize restore context: {e}"))?;
        }
    }
    Ok(cmd)
}

fn build_message_cmd(prompt: &str, images: &[ImageAttachment]) -> serde_json::Value {
    if images.is_empty() {
        serde_json::json!({
            "type": "message",
            "prompt": prompt,
        })
    } else {
        let img_blocks: Vec<serde_json::Value> = images
            .iter()
            .map(|img| {
                serde_json::json!({
                    "data": img.data,
                    "mediaType": img.media_type,
                })
            })
            .collect();
        serde_json::json!({
            "type": "message",
            "prompt": prompt,
            "images": img_blocks,
        })
    }
}

/// Validate and encode an image from raw bytes.
/// Returns base64-encoded data and detected MIME type, or an error for unsupported formats.
fn validate_and_encode_image(bytes: &[u8]) -> Result<ImageAttachment, String> {
    let media_type = validate_image_bytes(bytes)?;

    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);

    Ok(ImageAttachment {
        data,
        media_type: media_type.to_string(),
    })
}

/// Tauri command: Validate image bytes and return base64-encoded image attachment.
/// Called from the frontend after D&D or paste events.
pub fn prepare_image_attachment(data: Vec<u8>) -> Result<ImageAttachment, String> {
    if data.is_empty() {
        return Err("Empty image data".to_string());
    }
    validate_and_encode_image(&data)
}

/// Tauri command: Read image files from paths and return base64-encoded attachments.
/// Called from the frontend when files are dropped via native drag-and-drop.
/// Non-image files are silently skipped.
pub async fn prepare_image_attachments_from_paths(
    paths: Vec<String>,
) -> Result<Vec<ImageAttachment>, String> {
    let mut attachments = Vec::new();
    for path in &paths {
        let data = tokio::fs::read(path)
            .await
            .map_err(|e| format!("Failed to read {}: {}", path, e))?;
        if data.is_empty() {
            continue;
        }
        if let Ok(attachment) = validate_and_encode_image(&data) {
            attachments.push(attachment);
        }
    }
    Ok(attachments)
}

/// Runtime lock acquired by the caller variant used by workflow step startup.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_agent_turn_internal_locked<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    cwd: &str,
    permission_mode: &str,
    prompt: &str,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(app)?;

    // Add human message
    let _human_msg = add_message_internal(
        session_store,
        &data_dir,
        chat_session_id,
        MessageRole::Human,
        prompt,
        None,
        None,
    )?;

    // Add empty agent message (will be filled by streaming)
    let agent_msg = add_message_internal(
        session_store,
        &data_dir,
        chat_session_id,
        MessageRole::Agent,
        "",
        None,
        None,
    )?;

    start_agent_turn_locked(
        app,
        handles,
        session_store,
        chat_session_id,
        cwd,
        permission_mode,
        false,
        prompt,
        &agent_msg.id,
        &[],
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::state::WorkflowExecutionState;
    use crate::adaptor::gateway::workflow::test_support::TestRuntimeKernel;
    use crate::infrastructure::agent_session::runtime::{
        AgentBackend, AgentBackendRegistry, AgentMessage, PermissionResponse, SessionConfig,
        SessionHandle,
    };
    use async_trait::async_trait;

    fn approved_fix_policy_output(policy: &str, review_step: &str) -> String {
        format!(
            r#"<workflow_output type="approved-fix-policy">{{"policy":"{policy}","review_step":"{review_step}"}}</workflow_output>"#
        )
    }

    fn pending_message_for_test(id: &str, content: &str, created_at: f64) -> PendingMessage {
        PendingMessage {
            id: id.to_string(),
            content: content.to_string(),
            created_at,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        }
    }

    fn test_pending_message(id: &str, content: &str) -> PendingMessage {
        pending_message_for_test(id, content, 1.0)
    }

    struct MockModelBackend {
        backend_id: String,
        #[allow(dead_code)]
        models: Vec<String>,
    }

    #[async_trait]
    impl AgentBackend for MockModelBackend {
        fn id(&self) -> &str {
            &self.backend_id
        }

        fn name(&self) -> &str {
            "Mock"
        }

        async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String> {
            Ok(SessionHandle {
                chat_session_id: config.chat_session_id,
                backend_id: self.backend_id.clone(),
            })
        }

        async fn send_message(
            &self,
            _session: &SessionHandle,
            _message: AgentMessage,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn interrupt(&self, _session: &SessionHandle) -> Result<(), String> {
            Ok(())
        }

        async fn respond_permission(
            &self,
            _session: &SessionHandle,
            _response: PermissionResponse,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn close_session(&self, _session: &SessionHandle) -> Result<(), String> {
            Ok(())
        }
    }

    struct MockSteeringBackend {
        backend_id: String,
    }

    #[async_trait]
    impl AgentBackend for MockSteeringBackend {
        fn id(&self) -> &str {
            &self.backend_id
        }

        fn name(&self) -> &str {
            "MockSteering"
        }

        async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String> {
            Ok(SessionHandle {
                chat_session_id: config.chat_session_id,
                backend_id: self.backend_id.clone(),
            })
        }

        async fn send_message(
            &self,
            _session: &SessionHandle,
            _message: AgentMessage,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn steer_message(
            &self,
            _session: &SessionHandle,
            _message: AgentMessage,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn active_turn_steering_ready(&self, _session: &SessionHandle) -> bool {
            true
        }

        async fn interrupt(&self, _session: &SessionHandle) -> Result<(), String> {
            panic!("steering-ready backend should not be interrupted")
        }

        async fn respond_permission(
            &self,
            _session: &SessionHandle,
            _response: PermissionResponse,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn close_session(&self, _session: &SessionHandle) -> Result<(), String> {
            Ok(())
        }
    }

    fn expect_prepared_turn(input: PreparedAgentRuntimeInput) -> PreparedAgentTurn {
        match input {
            PreparedAgentRuntimeInput::Turn(turn) => turn,
            PreparedAgentRuntimeInput::Steer(_) => {
                panic!("expected a prepared turn, got active-turn steer")
            }
        }
    }

    fn expect_prepared_steer(input: PreparedAgentRuntimeInput) -> PreparedAgentSteer {
        match input {
            PreparedAgentRuntimeInput::Steer(steer) => steer,
            PreparedAgentRuntimeInput::Turn(_) => {
                panic!("expected active-turn steer, got a prepared turn")
            }
        }
    }

    fn chat_session_for_spawn_info(session_id: &str) -> ChatSession {
        ChatSession {
            id: session_id.to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::usecase::agent_session::session::SessionState::Closed,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: Some("sdk-resume-id".to_string()),
            context_carry: Some(crate::usecase::agent_session::session::ContextCarryState::Resumed),
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: Some("sonnet".to_string()),
            backend_id: Some("mock".to_string()),
            workflow_step_session: true,
            workflow_step_context: None,
        }
    }

    #[test]
    fn persisted_spawn_info_uses_step_agent_session_id_for_resume() {
        let info = resolve_spawn_info(Some(chat_session_for_spawn_info("step")), None);

        assert_eq!(info.resume_sid.as_deref(), Some("sdk-resume-id"));
        assert_eq!(info.selected_model.as_deref(), Some("sonnet"));
        assert_eq!(info.backend_id, "mock");
        assert!(matches!(
            info.context_restore_plan,
            ContextRestorePlan::Resume { .. }
        ));
    }

    #[test]
    fn persist_agent_session_id_updates_session_store_on_ready() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = crate::test_support::build_session_store();
        let session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();

        persist_agent_session_id(&app.handle(), &store, &session.id, "sdk-ready");

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id.as_deref(), Some("sdk-ready"));
    }

    #[test]
    fn save_session_context_carry_returns_update_payload_when_state_changes() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::test_support::build_session_store();
        let mut session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.agent_session_id = Some("sdk-session".to_string());
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let update =
            save_session_context_carry(&store, temp.path(), &session.id, ContextCarryState::Failed)
                .unwrap()
                .unwrap();

        assert_eq!(update.chat_session_id, session.id);
        assert_eq!(update.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(update.context_carry, Some(ContextCarryState::Failed));
        assert!(update.updated_at >= session.updated_at);
        assert!(save_session_context_carry(
            &store,
            temp.path(),
            &session.id,
            ContextCarryState::Failed
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn context_carry_persistence_does_not_read_message_chunks() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::test_support::build_session_store();
        let session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        add_message_internal(
            &store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "hello",
            None,
            None,
        )
        .unwrap();
        let chunk = temp
            .path()
            .join("sessions")
            .join(&session.id)
            .join("messages")
            .join("1.json");
        std::fs::write(chunk, "{not valid json").unwrap();

        let update =
            save_session_context_carry(&store, temp.path(), &session.id, ContextCarryState::Failed)
                .unwrap()
                .unwrap();

        assert_eq!(update.context_carry, Some(ContextCarryState::Failed));
        let meta = store
            .get_session_meta(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.context_carry, Some(ContextCarryState::Failed));
    }

    #[test]
    fn session_ready_resume_mismatch_requires_requested_id_to_match_ready_id() {
        assert!(!session_ready_resume_mismatch(
            Some(&ContextCarryState::Reinjected),
            Some("resume-1"),
            Some("new-1")
        ));
        assert!(!session_ready_resume_mismatch(
            Some(&ContextCarryState::Resumed),
            Some("resume-1"),
            Some("resume-1")
        ));
        assert!(session_ready_resume_mismatch(
            Some(&ContextCarryState::Resumed),
            Some("resume-1"),
            Some("new-1")
        ));
        assert!(session_ready_resume_mismatch(
            Some(&ContextCarryState::Resumed),
            Some("resume-1"),
            None
        ));
    }

    #[test]
    fn defer_agent_session_id_persist_flag_is_internal() {
        let mut msg = serde_json::json!({
            "type": "session_ready",
            "session_id": "sdk-session",
        });
        msg[DEFER_AGENT_SESSION_ID_PERSIST_ON_READY] = serde_json::Value::Bool(true);

        assert!(take_defer_agent_session_id_persist_on_ready(&mut msg));
        assert!(msg.get(DEFER_AGENT_SESSION_ID_PERSIST_ON_READY).is_none());

        let mut msg_without_flag = serde_json::json!({ "type": "session_ready" });
        assert!(!take_defer_agent_session_id_persist_on_ready(
            &mut msg_without_flag
        ));
    }

    #[tokio::test]
    async fn external_session_ready_can_defer_agent_session_id_persistence() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.backend_id = CODEX_BACKEND_ID.to_string();
        proc.state = BridgeState::Initializing;
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        let mut ready_message = serde_json::json!({
            "type": "session_ready",
            "session_id": "new-codex-thread",
        });
        ready_message[DEFER_AGENT_SESSION_ID_PERSIST_ON_READY] = serde_json::Value::Bool(true);

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            ready_message,
            &mut state,
        )
        .await;

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id, None);
        let removed = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed {
            assert_eq!(proc.sdk_session_id.as_deref(), Some("new-codex-thread"));
            assert_eq!(proc.state, BridgeState::Ready);
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn external_turn_complete_persists_successful_session_id() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.backend_id = CODEX_BACKEND_ID.to_string();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "turn_complete",
                "session_id": "new-codex-thread",
                "exit_code": 0,
            }),
            &mut state,
        )
        .await;

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id.as_deref(), Some("new-codex-thread"));
        let removed = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed {
            assert_eq!(proc.state, BridgeState::Ready);
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn external_session_ready_mismatch_prepares_reinject_and_crashes_process() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.agent_session_id = Some("stale-sdk-session".to_string());
        session.context_carry = Some(ContextCarryState::Resumed);
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Initializing;
        proc.sdk_session_id = Some("stale-sdk-session".to_string());
        proc.context_carry_on_ready = Some(ContextCarryState::Resumed);
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "session_ready",
                "session_id": "new-sdk-session"
            }),
            &mut state,
        )
        .await;

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id, None);
        assert_eq!(loaded.context_carry, None);
        let removed = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed {
            assert_eq!(proc.sdk_session_id, None);
            assert_eq!(proc.context_carry_on_ready, None);
            assert_eq!(proc.state, BridgeState::Crashed);
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn session_ready_streaming_resume_mismatch_requeues_current_turn_for_reinject() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.agent_session_id = Some("stale-sdk-session".to_string());
        session.context_carry = Some(ContextCarryState::Resumed);
        session.messages = vec![
            ChatMessage {
                id: "prior-human".to_string(),
                role: MessageRole::Human,
                content: "remember alpha".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 1.0,
                mentions: None,
            },
            ChatMessage {
                id: "prior-agent".to_string(),
                role: MessageRole::Agent,
                content: "alpha is set".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 2.0,
                mentions: None,
            },
            ChatMessage {
                id: "current-human".to_string(),
                role: MessageRole::Human,
                content: "what was it?".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 3.0,
                mentions: None,
            },
            ChatMessage {
                id: "current-agent".to_string(),
                role: MessageRole::Agent,
                content: String::new(),
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 4.0,
                mentions: None,
            },
        ];
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.sdk_session_id = Some("stale-sdk-session".to_string());
        proc.context_carry_on_ready = Some(ContextCarryState::Resumed);
        proc.streaming_message_id = Some("current-agent".to_string());
        handles.lock().await.insert(session.id.clone(), proc);

        let candidate = {
            let mut map = handles.lock().await;
            let proc = map.get_mut(&session.id).unwrap();
            let context_carry_on_ready = proc.context_carry_on_ready.take();
            assert!(session_ready_resume_mismatch(
                context_carry_on_ready.as_ref(),
                proc.sdk_session_id.as_deref(),
                Some("new-sdk-session"),
            ));
            streaming_turn_requeue_candidate(proc).expect("streaming candidate")
        };
        assert!(
            requeue_streaming_turn_for_resume_mismatch(
                &app.handle(),
                &handles,
                &store,
                &session.id,
                candidate,
            )
            .await
        );
        persist_resume_mismatch_for_reinject(&app.handle(), &store, &session.id);
        crash_agent_process_for_context_reinject(&app.handle(), &handles, &session.id).await;

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id, None);
        assert_eq!(loaded.context_carry, None);
        let ContextRestorePlan::Reinject { payload } =
            context_restore_plan_for_session_before_turn(Some(&loaded), "current-agent")
        else {
            panic!("expected reinject plan before current turn");
        };
        assert!(payload.prompt_prefix.contains("remember alpha"));
        assert!(!payload.prompt_prefix.contains("what was it?"));

        let mut proc = handles.lock().await.remove(&session.id).unwrap();
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(proc.sdk_session_id, None);
        assert_eq!(proc.pending_messages.len(), 1);
        let pending = proc.pending_messages.pop_front().unwrap();
        assert_eq!(pending.content, "what was it?");
        assert_eq!(
            pending.existing_human_message_id.as_deref(),
            Some("current-human")
        );
        assert_eq!(
            pending.existing_agent_message_id.as_deref(),
            Some("current-agent")
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn initializing_resume_mismatch_has_no_streaming_requeue_candidate() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Initializing;
        proc.sdk_session_id = Some("stale-sdk-session".to_string());
        proc.context_carry_on_ready = Some(ContextCarryState::Resumed);

        assert!(session_ready_resume_mismatch(
            proc.context_carry_on_ready.as_ref(),
            proc.sdk_session_id.as_deref(),
            Some("new-sdk-session"),
        ));
        assert!(streaming_turn_requeue_candidate(&proc).is_none());
        let _ = proc.child.kill().await;
    }

    #[test]
    fn persist_context_carry_failed_can_force_failed_before_success_state() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = crate::test_support::build_session_store();
        let mut session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.agent_session_id = Some("stale-sdk-session".to_string());
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        persist_context_carry_failed_after_init_error(
            &app.handle(),
            &store,
            &session.id,
            true,
            true,
        );

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id, None);
        assert_eq!(
            loaded.context_carry,
            Some(crate::usecase::agent_session::session::ContextCarryState::Failed)
        );
    }

    #[tokio::test]
    async fn bridge_eof_initializing_pending_context_carry_persists_failed() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.agent_session_id = Some("stale-sdk-session".to_string());
        session.context_carry = Some(ContextCarryState::Resumed);
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Initializing;
        proc.generation_id = 42;
        proc.sdk_session_id = Some("stale-sdk-session".to_string());
        proc.context_carry_on_ready = Some(ContextCarryState::Resumed);
        handles.lock().await.insert(session.id.clone(), proc);

        let (was_initializing, context_carry_failed_after_init_error) = {
            let mut map = handles.lock().await;
            let proc = map.get_mut(&session.id).unwrap();
            let generation_matches = proc.generation_id == 42;
            let transition = run_bridge_eof_crash_transition_locked(
                generation_matches,
                proc,
                &session.id,
                |_mid, _parts| (true, true),
            );
            (
                transition.was_initializing,
                transition.context_restore_failed_on_init,
            )
        };
        assert!(was_initializing);
        assert!(context_carry_failed_after_init_error);
        persist_context_carry_failed_after_init_error(
            &app.handle(),
            &store,
            &session.id,
            true,
            true,
        );

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.agent_session_id, None);
        assert_eq!(loaded.context_carry, Some(ContextCarryState::Failed));
        let mut proc = handles.lock().await.remove(&session.id).unwrap();
        assert_eq!(proc.context_carry_on_ready, None);
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn bridge_eof_initializing_without_pending_context_carry_does_not_persist_failed() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.agent_session_id = Some("existing-sdk-session".to_string());
        session.context_carry = None;
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Initializing;
        proc.generation_id = 7;
        proc.sdk_session_id = Some("existing-sdk-session".to_string());
        proc.context_carry_on_ready = None;
        handles.lock().await.insert(session.id.clone(), proc);

        let context_carry_failed_after_init_error = {
            let mut map = handles.lock().await;
            let proc = map.get_mut(&session.id).unwrap();
            let generation_matches = proc.generation_id == 7;
            let transition = run_bridge_eof_crash_transition_locked(
                generation_matches,
                proc,
                &session.id,
                |_mid, _parts| (true, true),
            );
            assert!(transition.was_initializing);
            transition.context_restore_failed_on_init
        };
        assert!(!context_carry_failed_after_init_error);
        if context_carry_failed_after_init_error {
            persist_context_carry_failed_after_init_error(
                &app.handle(),
                &store,
                &session.id,
                true,
                true,
            );
        }

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.agent_session_id.as_deref(),
            Some("existing-sdk-session")
        );
        assert_eq!(loaded.context_carry, None);
        let mut proc = handles.lock().await.remove(&session.id).unwrap();
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn ensure_runtime_for_turn_spawns_at_most_once_for_concurrent_sends() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let spawn_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let session_id = "step-session".to_string();

        let first = ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            move || async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                spawn_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        });
        let second = ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            move || async move {
                spawn_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        });

        let (first, second) = tokio::join!(first, second);
        first.unwrap();
        second.unwrap();

        assert_eq!(spawn_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(handles.lock().await.contains_key("step-session"));
    }

    #[tokio::test]
    async fn ensure_runtime_for_turn_removes_partial_runtime_when_spawn_fails() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "step-session-spawn-fail".to_string();

        let result = ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            move || async move {
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Err("spawn failed".to_string())
            }
        })
        .await;

        assert_eq!(result.unwrap_err(), "spawn failed");
        assert!(!handles.lock().await.contains_key("step-session-spawn-fail"));
    }

    #[tokio::test]
    async fn ensure_runtime_for_turn_spawns_when_ready_idle_child_exited_before_eof() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "ready-idle-exited-before-eof".to_string();
        let spawn_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut ready = make_test_agent_process();
        ready.state = BridgeState::Ready;
        ready.turn_phase = TurnPhase::Idle;
        ready
            .pending_messages
            .push_back(test_pending_message("queued-after-result", "continue"));
        ready.child.start_kill().unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        handles.lock().await.insert(session_id.clone(), ready);

        ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            move || async move {
                spawn_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        })
        .await
        .unwrap();

        assert_eq!(spawn_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        let mut proc = handles
            .lock()
            .await
            .remove("ready-idle-exited-before-eof")
            .unwrap();
        assert_eq!(proc.pending_messages.len(), 1);
        assert_eq!(
            proc.pending_messages.pop_front().unwrap().id,
            "queued-after-result"
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn ensure_runtime_for_turn_spawns_after_ready_eof_eviction() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "ready-eof-evicted".to_string();
        let spawn_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut ready = make_test_agent_process();
        let _ = ready.child.kill().await;
        ready.state = BridgeState::Ready;
        ready.turn_phase = TurnPhase::Idle;
        ready.generation_id = 42;
        handles.lock().await.insert(session_id.clone(), ready);

        {
            let mut map = handles.lock().await;
            let should_evict = {
                let proc = map.get_mut(&session_id).unwrap();
                let generation_matches = proc.generation_id == 42;
                let transition = run_bridge_eof_crash_transition_locked(
                    generation_matches,
                    proc,
                    &session_id,
                    |_mid, _parts| (true, true),
                );
                let should_evict = transition.should_evict;
                assert!(should_evict);
                should_evict
            };
            if should_evict {
                let removed = retire_ready_eof_runtime_locked(&mut map, &session_id);
                assert!(removed);
            }
            assert!(!map.contains_key(&session_id));
        }

        ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            move || async move {
                spawn_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        })
        .await
        .unwrap();

        assert_eq!(spawn_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(handles.lock().await.contains_key("ready-eof-evicted"));
        let mut spawned = handles.lock().await.remove("ready-eof-evicted").unwrap();
        let _ = spawned.child.kill().await;
    }

    #[tokio::test]
    async fn ready_eof_with_pending_queue_preserves_pending_when_respawning() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "ready-eof-pending".to_string();
        let mut ready = make_test_agent_process();
        let _ = ready.child.kill().await;
        ready.state = BridgeState::Ready;
        ready.turn_phase = TurnPhase::Idle;
        ready.generation_id = 7;
        ready
            .pending_messages
            .push_back(test_pending_message("queued-2", "second pending"));
        ready
            .pending_messages
            .push_back(test_pending_message("queued-3", "third pending"));
        handles.lock().await.insert(session_id.clone(), ready);

        {
            let mut map = handles.lock().await;
            let proc = map.get_mut(&session_id).unwrap();
            let generation_matches = proc.generation_id == 7;
            let transition = run_bridge_eof_crash_transition_locked(
                generation_matches,
                proc,
                &session_id,
                |_mid, _parts| (true, true),
            );
            assert!(transition.should_evict);
            let removed = retire_ready_eof_runtime_locked(&mut map, &session_id);
            assert!(!removed);

            let proc = map.get(&session_id).unwrap();
            assert_eq!(proc.state, BridgeState::Crashed);
            assert_eq!(proc.turn_phase, TurnPhase::Idle);
            assert_eq!(proc.pending_messages.len(), 2);
        }

        ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            move || async move {
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        })
        .await
        .unwrap();

        let mut proc = handles.lock().await.remove("ready-eof-pending").unwrap();
        let pending_ids: Vec<&str> = proc
            .pending_messages
            .iter()
            .map(|pending| pending.id.as_str())
            .collect();
        assert_eq!(pending_ids, vec!["queued-2", "queued-3"]);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn ensure_runtime_for_turn_preserves_pending_when_replacing_crashed_runtime() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "crashed-with-pending".to_string();
        let mut crashed = make_test_agent_process();
        let _ = crashed.child.kill().await;
        crashed.state = BridgeState::Crashed;
        crashed.pending_messages.push_back(PendingMessage {
            id: "queued-before-crash".to_string(),
            content: "continue after reinject".to_string(),
            created_at: 1.0,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        });
        handles.lock().await.insert(session_id.clone(), crashed);

        ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            move || async move {
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        })
        .await
        .unwrap();

        let mut proc = handles.lock().await.remove("crashed-with-pending").unwrap();
        assert_eq!(proc.pending_messages.len(), 1);
        let pending = proc.pending_messages.pop_front().unwrap();
        assert_eq!(pending.id, "queued-before-crash");
        assert_eq!(pending.content, "continue after reinject");
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn timed_out_recovery_preserves_remaining_pending_messages_for_replacement() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "timeout-with-multiple-pending".to_string();
        let mut timed_out = make_test_agent_process();
        let _ = timed_out.child.kill().await;
        timed_out.generation_id = 42;
        timed_out.turn_seq = 7;
        timed_out.state = BridgeState::Crashed;
        timed_out.turn_phase = TurnPhase::Idle;
        timed_out
            .pending_messages
            .push_back(pending_message_for_test(
                "queued-after-timeout-1",
                "first remaining",
                1.0,
            ));
        timed_out
            .pending_messages
            .push_back(pending_message_for_test(
                "queued-after-timeout-2",
                "second remaining",
                2.0,
            ));
        handles.lock().await.insert(session_id.clone(), timed_out);

        {
            let mut map = handles.lock().await;
            let (remove_pid_file, _sweep_pgid) =
                mark_timed_out_bridge_for_recovery_locked(&mut map, &session_id, 42, 7, None);
            assert!(remove_pid_file);
            assert!(
                map.contains_key(&session_id),
                "timeout recovery must retain crashed runtime so pending queue can be preserved"
            );
        }

        ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            move || async move {
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        })
        .await
        .unwrap();

        let mut proc = handles
            .lock()
            .await
            .remove("timeout-with-multiple-pending")
            .unwrap();
        let pending_ids = proc
            .pending_messages
            .iter()
            .map(|pending| pending.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            pending_ids,
            vec!["queued-after-timeout-1", "queued-after-timeout-2"]
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn ensure_runtime_for_turn_spawns_fresh_runtime_after_timeout_crash() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "timed-out-runtime".to_string();
        let mut timed_out = make_test_agent_process();
        let _ = timed_out.child.kill().await;
        timed_out.state = BridgeState::Crashed;
        timed_out.turn_phase = TurnPhase::Idle;
        handles.lock().await.insert(session_id.clone(), timed_out);
        let spawn_count = Arc::new(AtomicUsize::new(0));

        ensure_runtime_for_turn(&handles, &session_id, {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            move || async move {
                spawn_count.fetch_add(1, Ordering::SeqCst);
                handles
                    .lock()
                    .await
                    .insert(session_id, make_test_agent_process());
                Ok(())
            }
        })
        .await
        .unwrap();

        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);
        let mut proc = handles.lock().await.remove("timed-out-runtime").unwrap();
        assert_eq!(proc.state, BridgeState::Ready);
        let _ = proc.child.kill().await;
    }

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

    #[tokio::test]
    async fn enqueue_pending_delta_accumulates_parts_and_bytes() {
        let mut proc = make_streaming_test_process();
        let delta = vec![
            MessagePart::Text {
                content: "abcde".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Thinking {
                content: "fg".to_string(),
                parent_tool_use_id: None,
            },
        ];
        enqueue_pending_delta(&mut proc, &delta);
        assert_eq!(proc.pending_stream_part_count, 2);
        assert_eq!(proc.pending_stream_bytes, "abcde".len() + "fg".len());
    }

    #[tokio::test]
    async fn streaming_interval_elapsed_is_true_before_any_emit() {
        let proc = make_streaming_test_process();
        assert!(
            streaming_interval_elapsed(&proc),
            "first emit must not wait for an interval"
        );
    }

    #[tokio::test]
    async fn streaming_interval_elapsed_is_false_within_interval() {
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());
        assert!(
            !streaming_interval_elapsed(&proc),
            "successive emit within {}ms must wait",
            STREAMING_EMIT_INTERVAL_MS
        );
    }

    #[tokio::test]
    async fn pending_exceeds_threshold_triggers_on_part_count() {
        let mut proc = make_streaming_test_process();
        for _ in 0..STREAMING_PENDING_PART_LIMIT {
            enqueue_pending_delta(
                &mut proc,
                &[MessagePart::Text {
                    content: "x".to_string(),
                    parent_tool_use_id: None,
                }],
            );
        }
        assert!(pending_exceeds_threshold(&proc));
    }

    #[tokio::test]
    async fn pending_exceeds_threshold_triggers_on_byte_count() {
        let mut proc = make_streaming_test_process();
        proc.pending_stream_bytes = STREAMING_PENDING_BYTE_LIMIT;
        assert!(pending_exceeds_threshold(&proc));
    }

    #[tokio::test]
    async fn pending_exceeds_threshold_returns_false_when_below_both_caps() {
        let mut proc = make_streaming_test_process();
        proc.pending_stream_bytes = STREAMING_PENDING_BYTE_LIMIT - 1;
        proc.pending_stream_part_count = 1;
        assert!(!pending_exceeds_threshold(&proc));
    }

    #[tokio::test]
    async fn streaming_interval_elapsed_is_true_after_interval() {
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at =
            Some(Instant::now() - Duration::from_millis(STREAMING_EMIT_INTERVAL_MS + 5));
        assert!(streaming_interval_elapsed(&proc));
    }

    #[tokio::test]
    async fn prepare_streaming_flush_is_none_when_buffer_is_empty() {
        let proc = make_streaming_test_process();
        // 空バッファでは flush 準備が None になり、emit を発火しない。
        assert!(prepare_streaming_flush(&proc).is_none());
    }

    #[tokio::test]
    async fn prepare_streaming_flush_consolidates_cumulative_parts() {
        let mut proc = make_streaming_test_process();
        proc.streaming_parts.push(MessagePart::Text {
            content: "Hel".to_string(),
            parent_tool_use_id: None,
        });
        proc.streaming_parts.push(MessagePart::Text {
            content: "lo".to_string(),
            parent_tool_use_id: None,
        });
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "lo".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        assert_eq!(snapshot.parts.len(), 1);
        match &snapshot.parts[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "Hello"),
            _ => panic!("expected consolidated Text part"),
        }
        assert_eq!(snapshot.buffer_len, 1);
        assert_eq!(snapshot.pending_bytes, "lo".len());
    }

    #[tokio::test]
    async fn apply_streaming_emit_result_clears_pending_on_success() {
        let mut proc = make_streaming_test_process();
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "abc".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);
        assert!(ok);
        assert_eq!(proc.pending_stream_part_count, 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        assert!(proc.last_stream_emit_at.is_some());
    }

    #[tokio::test]
    async fn apply_streaming_emit_result_records_interval_on_second_success() {
        let _guard = crate::other::telemetry::lock_test_telemetry();
        crate::other::telemetry::reset_test_metrics();
        crate::other::telemetry::set_performance_configured(true);
        crate::other::telemetry::set_performance_enabled(true);
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now() - Duration::from_millis(25));
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "abc".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");

        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);

        assert!(ok);
        let records = crate::other::telemetry::test_metric_records();
        assert!(records.iter().any(|record| {
            record.name == "releash.agent_stream.emit_interval_ms" && record.value >= 25.0
        }));
        crate::other::telemetry::reset_test_metrics();
    }

    #[tokio::test]
    async fn apply_streaming_emit_result_does_not_record_interval_on_first_success() {
        let _guard = crate::other::telemetry::lock_test_telemetry();
        crate::other::telemetry::reset_test_metrics();
        crate::other::telemetry::set_performance_configured(true);
        crate::other::telemetry::set_performance_enabled(true);
        let mut proc = make_streaming_test_process();
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "abc".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");

        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);

        assert!(ok);
        assert!(!crate::other::telemetry::test_metric_records()
            .iter()
            .any(|record| record.name == "releash.agent_stream.emit_interval_ms"));
        crate::other::telemetry::reset_test_metrics();
    }

    #[tokio::test]
    async fn apply_streaming_emit_result_retains_pending_on_failure() {
        let mut proc = make_streaming_test_process();
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "abc".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        // Tauri failed / WS ok
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, false, true);
        assert!(!ok);
        assert_eq!(proc.pending_stream_part_count, 1);
        assert_eq!(proc.pending_stream_bytes, "abc".len());
        assert!(
            proc.last_stream_emit_at.is_none(),
            "last_emit_at must not advance on failure"
        );
    }

    #[tokio::test]
    async fn apply_streaming_emit_result_does_not_record_interval_on_failure() {
        let _guard = crate::other::telemetry::lock_test_telemetry();
        crate::other::telemetry::reset_test_metrics();
        crate::other::telemetry::set_performance_configured(true);
        crate::other::telemetry::set_performance_enabled(true);
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now() - Duration::from_millis(25));
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "abc".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");

        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, false, true);

        assert!(!ok);
        assert!(!crate::other::telemetry::test_metric_records()
            .iter()
            .any(|record| record.name == "releash.agent_stream.emit_interval_ms"));
        crate::other::telemetry::reset_test_metrics();
    }

    #[tokio::test]
    async fn apply_streaming_emit_result_retains_when_both_channels_fail() {
        let mut proc = make_streaming_test_process();
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "abc".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, false, false);
        assert!(!ok);
        assert_eq!(proc.pending_stream_part_count, 1);
        assert!(proc.last_stream_emit_at.is_none());
    }

    #[tokio::test]
    async fn next_flush_after_partial_failure_re_sends_full_cumulative_parts() {
        // チャネルの片方だけが失敗した場合、次 flush は累積 streaming_parts 全体を
        // 両チャネル向けに同一ペイロードで再送する（spec: 累積置換型）。
        let mut proc = make_streaming_test_process();
        proc.streaming_parts.push(MessagePart::Text {
            content: "Hel".to_string(),
            parent_tool_use_id: None,
        });
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "Hel".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let first = prepare_streaming_flush(&proc).expect("first snapshot");
        apply_streaming_emit_result(&mut proc, "csid", "mid", &first, true, false);

        // 失敗後に次 delta が到着。
        proc.streaming_parts.push(MessagePart::Text {
            content: "lo".to_string(),
            parent_tool_use_id: None,
        });
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "lo".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let second = prepare_streaming_flush(&proc).expect("second snapshot");
        assert_eq!(second.parts.len(), 1);
        match &second.parts[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "Hello"),
            _ => panic!("expected consolidated Text"),
        }
    }

    #[tokio::test]
    async fn pending_can_overflow_thresholds_while_delivery_fails() {
        // 上限到達状態で配信失敗しても、追加 delta はバッファに保持される（ソフト上限）。
        let mut proc = make_streaming_test_process();
        // streaming_parts にも同等の cumulative を入れて prepare_streaming_flush が
        // snapshot を返せる状態にする。
        for _ in 0..STREAMING_PENDING_PART_LIMIT {
            let part = MessagePart::Text {
                content: "x".to_string(),
                parent_tool_use_id: None,
            };
            proc.streaming_parts.push(part.clone());
            enqueue_pending_delta(&mut proc, std::slice::from_ref(&part));
        }
        assert!(pending_exceeds_threshold(&proc));

        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, false, true);

        let before = proc.pending_stream_part_count;
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "extra".to_string(),
                parent_tool_use_id: None,
            }],
        );
        assert_eq!(proc.pending_stream_part_count, before + 1);
    }

    #[tokio::test]
    async fn reset_streaming_state_for_new_turn_clears_all_coalescing_state() {
        // 前ターン残骸 (pending / last_emit_at / streaming_parts / last_message_id)
        // が新ターン開始時に確実にクリアされる。
        let mut proc = make_streaming_test_process();
        proc.streaming_parts.push(MessagePart::Text {
            content: "old".to_string(),
            parent_tool_use_id: None,
        });
        proc.pending_stream_part_count = 1;
        proc.pending_stream_bytes = 32;
        proc.last_stream_emit_at = Some(Instant::now());
        proc.last_message_id = Some("old".to_string());
        proc.post_turn_base_untrusted_message_id = Some("old".to_string());
        proc.task_id_map
            .insert("task".to_string(), "tool".to_string());

        proc.reset_streaming_state_for_new_turn();

        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_part_count, 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        assert!(proc.last_stream_emit_at.is_none());
        assert!(proc.last_message_id.is_none());
        assert!(proc.post_turn_base_untrusted_message_id.is_none());
        assert!(proc.task_id_map.is_empty());

        // 新ターン直後は最初の emit が即時 flush される (= interval elapsed).
        assert!(streaming_interval_elapsed(&proc));
    }

    #[tokio::test]
    async fn second_flush_after_success_is_noop_until_new_delta() {
        // 強制 flush が同じ契機で連続呼ばれても、二重配信は起きない。
        let mut proc = make_streaming_test_process();
        proc.streaming_parts.push(MessagePart::Text {
            content: "Hello".to_string(),
            parent_tool_use_id: None,
        });
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "Hello".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        assert!(apply_streaming_emit_result(
            &mut proc, "csid", "mid", &snapshot, true, true,
        ));

        assert!(prepare_streaming_flush(&proc).is_none(), "no double emit");
    }

    #[tokio::test]
    async fn forced_flush_continues_after_failure_for_state_transition() {
        // Spec: 強制配信が失敗しても後続の状態遷移は続行する。
        // apply_streaming_emit_result は false を返すだけで panic / abort せず、
        // 呼び出し元は戻り値を見ずに後続処理へ進められる。
        let mut proc = make_streaming_test_process();
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "tail".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        // 失敗を返してもパニックしない（= 状態遷移を続行できる）。
        let _ = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, false, false);
        // pending は保持され、次の契機で再試行可能。
        assert!(proc.pending_stream_part_count > 0);
    }

    #[tokio::test]
    async fn coalescing_first_delta_flushes_immediately() {
        // 初回 delta: last_stream_emit_at が None なので interval elapsed=true、
        // should_flush=true、flush_streaming で pending がクリアされる。
        let mut proc = make_streaming_test_process();
        let delta = vec![MessagePart::Text {
            content: "first".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);

        assert!(should_flush_per_delta(&proc, &delta, false));
        let snapshot = prepare_streaming_flush(&proc).expect("first emit must flush");
        apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);

        assert!(proc.pending_stream_part_count == 0);
        assert!(proc.last_stream_emit_at.is_some());
    }

    #[tokio::test]
    async fn coalescing_within_interval_does_not_flush() {
        // 配信直後（last_stream_emit_at=now）で続く delta が来ても、
        // 件数・byte 上限・tool event のいずれも当たらなければ flush しない。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());
        let delta = vec![MessagePart::Text {
            content: "tick".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);

        assert!(!should_flush_per_delta(&proc, &delta, false));
        // pending は保持されたまま（次の契機まで蓄積される）。
        assert_eq!(proc.pending_stream_part_count, 1);
    }

    #[tokio::test]
    async fn coalescing_after_interval_flushes_accumulated_buffer() {
        // 直前配信から interval を超えて経過した状態で次 delta が来ると、
        // 既に溜まっている pending と新規 delta をまとめて flush する。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at =
            Some(Instant::now() - Duration::from_millis(STREAMING_EMIT_INTERVAL_MS + 5));
        let earlier = MessagePart::Text {
            content: "ear".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(earlier.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&earlier));

        let new_delta = vec![MessagePart::Text {
            content: "lier".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(new_delta.clone());
        enqueue_pending_delta(&mut proc, &new_delta);

        assert!(should_flush_per_delta(&proc, &new_delta, false));
        let snapshot = prepare_streaming_flush(&proc).expect("must flush");
        // consolidated 後は 1 個の Text に統合される。
        assert_eq!(snapshot.parts.len(), 1);
        match &snapshot.parts[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "earlier"),
            _ => panic!("expected consolidated Text"),
        }
        apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);
        assert!(proc.pending_stream_part_count == 0);
    }

    #[tokio::test]
    async fn coalescing_count_limit_forces_flush_within_interval() {
        // pending parts が件数上限に達していれば、interval 未経過でも force flush。
        // production 経路と同じ流れを踏ませる: enqueue_pending_delta で上限まで
        // 蓄積 → 新規 delta が到着 → flush snapshot に新規 delta が含まれる →
        // apply 成功で pending が空に戻る。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());
        for _ in 0..STREAMING_PENDING_PART_LIMIT {
            let part = MessagePart::Text {
                content: "x".to_string(),
                parent_tool_use_id: None,
            };
            proc.streaming_parts.push(part.clone());
            enqueue_pending_delta(&mut proc, std::slice::from_ref(&part));
        }
        assert!(pending_exceeds_threshold(&proc));

        // 新規 delta を production と同じ手順で蓄積する。
        let next = vec![MessagePart::Text {
            content: "y".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(next.clone());
        enqueue_pending_delta(&mut proc, &next);

        assert!(!streaming_interval_elapsed(&proc));
        assert!(should_flush_per_delta(&proc, &next, false));

        let snapshot =
            prepare_streaming_flush(&proc).expect("count-limit flush must produce snapshot");
        // consolidate 後は全 Text が 1 個に統合され、末尾は新規 delta の "y"。
        assert_eq!(snapshot.parts.len(), 1);
        match snapshot
            .parts
            .last()
            .expect("snapshot has at least one part")
        {
            MessagePart::Text { content, .. } => {
                assert!(
                    content.ends_with('y'),
                    "consolidated tail should be the new delta"
                );
            }
            _ => panic!("expected consolidated Text part"),
        }
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);
        assert!(ok);
        assert!(proc.pending_stream_part_count == 0);
        assert_eq!(proc.pending_stream_bytes, 0);
    }

    #[tokio::test]
    async fn coalescing_byte_limit_forces_flush_within_interval() {
        // pending bytes が byte 上限に達していれば、interval 未経過でも force flush。
        // ハードコード値ではなく実装定数 STREAMING_PENDING_BYTE_LIMIT から算出する。
        // production 経路と同じ流れ: 上限相当の chunk を enqueue → 新規 delta
        // 到着 → flush snapshot に新規 delta が含まれる → apply 成功で pending 空。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());
        let chunk = "z".repeat(STREAMING_PENDING_BYTE_LIMIT);
        let part = MessagePart::Text {
            content: chunk,
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(part.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&part));
        assert!(pending_exceeds_threshold(&proc));

        let next = vec![MessagePart::Text {
            content: "n".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(next.clone());
        enqueue_pending_delta(&mut proc, &next);

        assert!(!streaming_interval_elapsed(&proc));
        assert!(should_flush_per_delta(&proc, &next, false));

        let snapshot =
            prepare_streaming_flush(&proc).expect("byte-limit flush must produce snapshot");
        assert_eq!(snapshot.parts.len(), 1);
        match snapshot
            .parts
            .last()
            .expect("snapshot has at least one part")
        {
            MessagePart::Text { content, .. } => {
                assert!(
                    content.ends_with('n'),
                    "consolidated tail should be the new delta"
                );
            }
            _ => panic!("expected consolidated Text part"),
        }
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);
        assert!(ok);
        assert!(proc.pending_stream_part_count == 0);
        assert_eq!(proc.pending_stream_bytes, 0);
    }

    #[tokio::test]
    async fn coalescing_tool_event_forces_flush_within_interval() {
        // tool start / end は interval 未経過でも即 flush（UI に古いフレームを残さない）。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());

        let delta_tool_use = vec![MessagePart::ToolUse {
            id: "tool-1".to_string(),
            tool: "Bash".to_string(),
            input: serde_json::json!({}),
            parent_tool_use_id: None,
        }];
        assert!(!streaming_interval_elapsed(&proc));
        assert!(should_flush_per_delta(&proc, &delta_tool_use, false));

        let delta_tool_result = vec![MessagePart::ToolResult {
            tool_use_id: Some("tool-1".to_string()),
            content: "ok".to_string(),
            is_error: false,
            parent_tool_use_id: None,
        }];
        assert!(should_flush_per_delta(&proc, &delta_tool_result, false));
    }

    #[tokio::test]
    async fn tool_event_flushes_pending_text_through_production_path() {
        // Spec (Rule: ターン完了・状態遷移時には未配信バッファを強制配信する,
        //  Examples ツール実行の開始 / 終了):
        //   未配信 text が pending に積まれている状態で ToolUse / ToolResult
        //   delta が到着すると、interval 未経過でも force flush され、
        //   pending text + tool event が同一の cumulative payload として
        //   emit され、emit 成功で pending が clear される。
        //
        // 本テストは production 経路 (enqueue_pending_delta →
        // prepare_streaming_flush → apply_streaming_emit_result) を最初から
        // 最後まで通し、ToolUse / ToolResult 双方について同じ流れを検証する。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());

        // 1) interval 未経過で未配信 text を pending に蓄積する。
        let pending_text = MessagePart::Text {
            content: "before-tool".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(pending_text.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&pending_text));
        assert!(!streaming_interval_elapsed(&proc));
        assert_eq!(proc.pending_stream_part_count, 1);

        // 2) ToolUse delta が到着 → production と同じ手順で enqueue。
        let tool_use_delta = vec![MessagePart::ToolUse {
            id: "tool-1".to_string(),
            tool: "Bash".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(tool_use_delta.clone());
        enqueue_pending_delta(&mut proc, &tool_use_delta);

        // tool event は interval 未経過でも force flush。
        assert!(!streaming_interval_elapsed(&proc));
        assert!(should_flush_per_delta(&proc, &tool_use_delta, false));

        // 3) prepare → emit (success) → apply で pending が clear される。
        let snapshot = prepare_streaming_flush(&proc).expect("tool start must produce snapshot");
        // cumulative payload には pending text + ToolUse が同一 emit で含まれる。
        assert_eq!(snapshot.parts.len(), 2);
        match &snapshot.parts[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "before-tool"),
            other => panic!("first cumulative part must be pending Text, got {other:?}"),
        }
        match &snapshot.parts[1] {
            MessagePart::ToolUse { id, tool, .. } => {
                assert_eq!(id, "tool-1");
                assert_eq!(tool, "Bash");
            }
            other => panic!("second cumulative part must be ToolUse, got {other:?}"),
        }
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);
        assert!(ok, "tool start emit must succeed → pending cleared");
        assert_eq!(proc.pending_stream_part_count, 0);
        assert_eq!(proc.pending_stream_bytes, 0);

        // 4) 続いて ToolResult delta が到着 → 同じく force flush。
        //    last_stream_emit_at は直前の apply で now() に更新されている。
        let tool_result_delta = vec![MessagePart::ToolResult {
            tool_use_id: Some("tool-1".to_string()),
            content: "ok".to_string(),
            is_error: false,
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(tool_result_delta.clone());
        enqueue_pending_delta(&mut proc, &tool_result_delta);

        assert!(!streaming_interval_elapsed(&proc));
        assert!(should_flush_per_delta(&proc, &tool_result_delta, false));

        let snapshot2 = prepare_streaming_flush(&proc).expect("tool end must produce snapshot");
        // 累積 payload は text + ToolUse + ToolResult の 3 件を含む。
        assert_eq!(snapshot2.parts.len(), 3);
        assert!(matches!(
            snapshot2.parts.last(),
            Some(MessagePart::ToolResult { content, .. }) if content == "ok"
        ));
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot2, true, true);
        assert!(ok, "tool end emit must succeed → pending cleared");
        assert_eq!(proc.pending_stream_part_count, 0);
        assert_eq!(proc.pending_stream_bytes, 0);
    }

    #[tokio::test]
    async fn timer_flushes_when_pending_and_interval_elapsed() {
        // 本番の補助 timer (`spawn_streaming_timer`) は `run_streaming_timer_tick`
        // を毎 tick 呼ぶ。テストも同じ helper を直接呼び、pending と
        // last_stream_emit_at の更新まで含めた挙動を検証する。
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at =
            Some(Instant::now() - Duration::from_millis(STREAMING_EMIT_INTERVAL_MS + 5));
        let part = MessagePart::Text {
            content: "silent".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(part.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&part));

        let mut emitted = Vec::new();
        let tick_effect = run_streaming_timer_tick(&mut proc, "csid", |mid, parts| {
            emitted.push((mid.to_string(), parts.to_vec()));
            (true, true)
        });

        assert!(
            tick_effect.keep_running,
            "still streaming → timer continues"
        );
        assert!(tick_effect.released_streaming_parts.is_empty());
        assert_eq!(emitted.len(), 1, "timer must call emit exactly once");
        assert_eq!(emitted[0].0, "m1");
        assert_eq!(emitted[0].1.len(), 1);
        assert_eq!(
            proc.pending_stream_part_count, 0,
            "pending cleared on success"
        );
        assert!(
            proc.last_stream_emit_at.is_some(),
            "last_stream_emit_at updated on success"
        );
    }

    #[tokio::test]
    async fn timer_skips_when_pending_but_interval_not_elapsed() {
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at = Some(Instant::now());
        let part = MessagePart::Text {
            content: "fresh".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(part.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&part));

        let mut emitted = false;
        let tick_effect = run_streaming_timer_tick(&mut proc, "csid", |_mid, _parts| {
            emitted = true;
            (true, true)
        });
        assert!(tick_effect.keep_running);
        assert!(tick_effect.released_streaming_parts.is_empty());
        assert!(!emitted, "interval not elapsed → timer must not flush");
        assert_eq!(proc.pending_stream_part_count, 1);
    }

    #[tokio::test]
    async fn timer_skips_when_pending_empty_even_if_interval_elapsed() {
        let mut proc = make_streaming_test_process();
        proc.last_stream_emit_at =
            Some(Instant::now() - Duration::from_millis(STREAMING_EMIT_INTERVAL_MS + 5));
        assert!(streaming_interval_elapsed(&proc));
        assert_eq!(proc.pending_stream_part_count, 0);

        let mut emitted = false;
        let tick_effect = run_streaming_timer_tick(&mut proc, "csid", |_mid, _parts| {
            emitted = true;
            (true, true)
        });
        // pending=0 & still Streaming → continue running but no flush this tick.
        assert!(tick_effect.keep_running);
        assert!(tick_effect.released_streaming_parts.is_empty());
        assert!(!emitted);
    }

    #[tokio::test]
    async fn timer_exits_when_turn_ended_and_buffer_empty() {
        // turn 終了 (state != Streaming) かつ pending が空になった時点で timer は
        // ループを終了させるべき。これを `run_streaming_timer_tick` の戻り値で表現する。
        let mut proc = make_streaming_test_process();
        proc.state = BridgeState::Ready;
        proc.last_stream_emit_at =
            Some(Instant::now() - Duration::from_millis(STREAMING_EMIT_INTERVAL_MS + 5));
        assert_eq!(proc.pending_stream_part_count, 0);

        let tick_effect = run_streaming_timer_tick(&mut proc, "csid", |_mid, _parts| (true, true));
        assert!(
            !tick_effect.keep_running,
            "turn ended (state != Streaming) and buffer empty → timer must exit"
        );
        assert!(tick_effect.released_streaming_parts.is_empty());
    }

    #[tokio::test]
    async fn timer_drains_pending_even_after_turn_ended() {
        // turn 終了直後でも pending が残っていれば drain し、成功時は
        // 完了済み streaming buffer を解放して timer を終了する。
        let mut proc = make_streaming_test_process();
        proc.state = BridgeState::Ready;
        proc.last_stream_emit_at =
            Some(Instant::now() - Duration::from_millis(STREAMING_EMIT_INTERVAL_MS + 5));
        let part = MessagePart::Text {
            content: "tail".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(part.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&part));

        let mut emitted = 0usize;
        let tick_effect = run_streaming_timer_tick(&mut proc, "csid", |_mid, _parts| {
            emitted += 1;
            (true, true)
        });
        assert!(
            !tick_effect.keep_running,
            "turn ended and pending drained → timer exits immediately"
        );
        assert_eq!(tick_effect.released_streaming_parts, vec![part]);
        assert_eq!(emitted, 1, "tail content flushed before exit");
        assert_eq!(proc.pending_stream_part_count, 0);
        assert!(proc.streaming_parts.is_empty());
    }

    #[tokio::test]
    async fn streaming_timer_decision_continue_when_generation_matches_and_streaming() {
        let proc = make_streaming_test_process();
        assert_eq!(
            streaming_timer_decision(&proc, proc.generation_id),
            TimerDecision::Continue
        );
    }

    #[tokio::test]
    async fn streaming_timer_decision_break_keep_flag_on_generation_mismatch() {
        // 新しい turn (generation_id 更新) が同じ csid を再利用したケース。
        // 既存 timer は自分の captured generation と一致しないので flag を残して
        // 終了する (新 timer が flag を所有しているため触らない)。
        let mut proc = make_streaming_test_process();
        let captured = proc.generation_id;
        proc.generation_id = captured.wrapping_add(1);
        proc.streaming_timer_active = true;
        assert_eq!(
            streaming_timer_decision(&proc, captured),
            TimerDecision::BreakKeepFlag
        );
    }

    #[tokio::test]
    async fn streaming_timer_decision_break_clear_flag_on_crash() {
        // 同一 generation で Crashed に遷移したら drain 不要なので flag を解放して
        // 終了する。
        let mut proc = make_streaming_test_process();
        proc.state = BridgeState::Crashed;
        assert_eq!(
            streaming_timer_decision(&proc, proc.generation_id),
            TimerDecision::BreakClearFlag
        );
    }

    #[tokio::test]
    async fn try_mark_streaming_timer_active_marks_idle_and_returns_true() {
        let mut proc = make_streaming_test_process();
        assert!(!proc.streaming_timer_active);
        assert!(try_mark_streaming_timer_active(&mut proc));
        assert!(proc.streaming_timer_active);
    }

    #[tokio::test]
    async fn try_mark_streaming_timer_active_returns_false_when_already_active() {
        // Duplicate spawn no-op: 同じ turn で 2 回目の spawn_streaming_timer を
        // 呼んでも flag は既に true なので false を返し新 task を起こさない。
        let mut proc = make_streaming_test_process();
        proc.streaming_timer_active = true;
        assert!(!try_mark_streaming_timer_active(&mut proc));
        assert!(proc.streaming_timer_active, "flag must remain set");
    }

    #[tokio::test]
    async fn forced_flush_emits_pending_before_state_transition_inputs() {
        // 強制 flush の呼び出し元（turn_complete / permission_request / error /
        // tool start/end）は、まず flush_streaming で pending を排出してから
        // state 遷移用の値（emit_session_state_changed の引数等）を組み立てる。
        // 本テストは「flush 完了後に pending が空になっている」ことを通じて、
        // 後続の状態通知が flush 済みデータの後で発火することを担保する。
        let mut proc = make_streaming_test_process();
        let delta = vec![MessagePart::Text {
            content: "tail-before-state".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);

        // forced flush の中身: snapshot → emit (mocked success) → apply
        let snapshot = prepare_streaming_flush(&proc).expect("pending must yield snapshot");
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, true, true);
        assert!(
            ok,
            "forced flush succeeded → pending cleared before state emit"
        );
        assert!(proc.pending_stream_part_count == 0);

        // この時点で呼び出し元が emit_session_state_changed を発火する。pending は
        // 既にクリアされているので、状態通知より前にストリーム emit が完了している。
    }

    #[tokio::test]
    async fn forced_flush_is_noop_when_no_pending_avoiding_double_delivery() {
        // 直前の強制 flush で pending を空にしている状態で再度同じ契機 (e.g. error 経路
        // と直後の EOF 経路) が forced flush を呼んでも、prepare_streaming_flush が
        // None を返すため二重配信は発生しない。
        let mut proc = make_streaming_test_process();
        proc.streaming_parts.push(MessagePart::Text {
            content: "once".to_string(),
            parent_tool_use_id: None,
        });
        enqueue_pending_delta(
            &mut proc,
            &[MessagePart::Text {
                content: "once".to_string(),
                parent_tool_use_id: None,
            }],
        );
        let snapshot = prepare_streaming_flush(&proc).expect("first snapshot");
        assert!(apply_streaming_emit_result(
            &mut proc, "csid", "mid", &snapshot, true, true,
        ));

        // 二度目の forced flush は no-op になる。
        assert!(prepare_streaming_flush(&proc).is_none());
    }

    #[tokio::test]
    async fn forced_flush_failure_does_not_block_followup_processing() {
        // Spec: 強制配信が失敗しても後続の状態遷移は続行する。失敗時 pending と
        // last_stream_emit_at は保持され、apply は false を返すのみ（panic しない）。
        let mut proc = make_streaming_test_process();
        let delta = vec![MessagePart::Text {
            content: "kept".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);
        let snapshot = prepare_streaming_flush(&proc).expect("snapshot");
        let ok = apply_streaming_emit_result(&mut proc, "csid", "mid", &snapshot, false, false);
        assert!(!ok);
        // 呼び出し元はここから後続 (emit_session_state_changed 等) に進める。
        // pending と last_stream_emit_at は次の契機での再試行のため保持される。
        assert_eq!(proc.pending_stream_part_count, 1);
        assert!(proc.last_stream_emit_at.is_none());
    }

    /// 呼び出し元経路で発火する 2 種類の emit を順序付きで記録するテスト用イベント。
    /// 実コードの `emit_streaming_parts` と `emit_session_state_changed` は
    /// `tauri::AppHandle` 直叩きでユニットテストから直接観測できないため、
    /// 呼び出し元ロジックをミラーした下記ヘルパで両 emit を同じ Vec に
    /// 記録し、ストリーム emit が state emit より先に来ることを確認する。
    #[derive(Debug, PartialEq)]
    enum RecordedEmit {
        StreamingFlush {
            parts_count: usize,
            tail_text: Option<String>,
        },
        StateChanged {
            phase: TurnPhase,
            exit_code: Option<i64>,
        },
    }

    /// Build a recording emit closure that pushes a `StreamingFlush` event
    /// for each cumulative payload it observes. Shared by the
    /// `permission_request` / `turn_complete` order tests so they exercise
    /// the same `flush_streaming_before_transition` helper the production
    /// stdout reader uses, instead of mirroring the prepare/apply sequence.
    fn recording_emit<'a>(
        events: &'a mut Vec<RecordedEmit>,
    ) -> impl FnMut(&str, &[MessagePart]) -> (bool, bool) + 'a {
        |_mid, parts| {
            events.push(RecordedEmit::StreamingFlush {
                parts_count: parts.len(),
                tail_text: match parts.last() {
                    Some(MessagePart::Text { content, .. })
                    | Some(MessagePart::Thinking { content, .. })
                    | Some(MessagePart::Error { content, .. }) => Some(content.clone()),
                    _ => None,
                },
            });
            (true, true)
        }
    }

    /// Drive the production `permission_request` lock-block via
    /// `run_permission_request_transition_locked` — the same helper the
    /// production stdout reader calls. This guarantees that any drift in the
    /// flush → state-mutation order would be caught here. The post-lock state
    /// emit (production: `emit_session_state_changed` outside the lock) is
    /// simulated by pushing a `StateChanged` event after the helper returns.
    fn drive_permission_request_path(
        proc: &mut AgentProcess,
        chat_session_id: &str,
        events: &mut Vec<RecordedEmit>,
    ) -> bool {
        let effect =
            run_permission_request_transition_locked(proc, chat_session_id, recording_emit(events));
        if effect.did_transition {
            events.push(RecordedEmit::StateChanged {
                phase: TurnPhase::WaitingPermission,
                exit_code: None,
            });
        }
        effect.did_transition
    }

    /// Drive the production `turn_complete` lock-block via
    /// `run_turn_complete_transition_locked` — the same helper the production
    /// stdout reader calls. State emit outside the lock is mirrored as a
    /// pushed `StateChanged` event so the ordering invariant is asserted on
    /// the event sequence.
    fn drive_turn_complete_path(
        proc: &mut AgentProcess,
        chat_session_id: &str,
        exit_code: i64,
        events: &mut Vec<RecordedEmit>,
    ) {
        let effect = run_turn_complete_transition_locked(
            proc,
            chat_session_id,
            exit_code,
            recording_emit(events),
        );
        if effect.was_streaming {
            events.push(RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(exit_code),
            });
        }
    }

    #[test]
    fn liveness_marks_streaming_stale_after_last_progress_timeout() {
        let now = Instant::now();
        let timeout = evaluate_turn_liveness(
            TurnPhase::Streaming,
            Some(now - Duration::from_secs(STALE_TIMEOUT_SECS + 1)),
            now - Duration::from_secs(STALE_TIMEOUT_SECS + 1),
            now,
        );

        assert_eq!(timeout, Some(TurnLivenessTimeout::Stale));
    }

    #[test]
    fn liveness_keeps_streaming_alive_when_progress_is_recent() {
        let now = Instant::now();
        let timeout = evaluate_turn_liveness(
            TurnPhase::Streaming,
            Some(now - Duration::from_secs(STALE_TIMEOUT_SECS - 1)),
            now - Duration::from_secs(STALE_TIMEOUT_SECS + 10),
            now,
        );

        assert_eq!(timeout, None);
    }

    #[test]
    fn liveness_keeps_waiting_permission_alive_after_long_wait() {
        let now = Instant::now();
        let since = now - Duration::from_secs(3600);

        assert_eq!(
            evaluate_turn_liveness(TurnPhase::WaitingPermission, None, since, now),
            None
        );
    }

    #[test]
    fn liveness_keeps_idle_alive() {
        let now = Instant::now();

        assert_eq!(
            evaluate_turn_liveness(TurnPhase::Idle, None, now - Duration::from_secs(3600), now),
            None
        );
    }

    #[tokio::test]
    async fn touch_liveness_resets_streaming_stale_clock() {
        let mut proc = make_streaming_test_process();
        let stale_base = Instant::now() - Duration::from_secs(STALE_TIMEOUT_SECS + 1);
        proc.last_progress_at = Some(stale_base);
        proc.turn_phase_since = stale_base;

        assert_eq!(
            turn_watchdog_decision(&proc, proc.generation_id, proc.turn_seq, Instant::now()),
            TurnWatchdogDecision::Timeout(TurnLivenessTimeout::Stale)
        );

        proc.touch_liveness();
        assert_eq!(
            turn_watchdog_decision(&proc, proc.generation_id, proc.turn_seq, Instant::now()),
            TurnWatchdogDecision::Continue
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn timeout_finalize_rechecks_liveness_and_continues_when_progress_arrives_after_decision()
    {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        proc.turn_watchdog_active = true;
        let captured_gen_id = proc.generation_id;
        let captured_turn_seq = proc.turn_seq;
        let stale_base = Instant::now() - Duration::from_secs(STALE_TIMEOUT_SECS + 1);
        proc.last_progress_at = Some(stale_base);
        proc.turn_phase_since = stale_base;

        let decision_now = Instant::now();
        assert_eq!(
            turn_watchdog_decision(&proc, captured_gen_id, captured_turn_seq, decision_now),
            TurnWatchdogDecision::Timeout(TurnLivenessTimeout::Stale)
        );

        proc.last_progress_at = Some(decision_now);
        let mut events = Vec::new();
        let transition = run_timeout_finalize_transition_locked(
            &mut proc,
            "csid",
            captured_gen_id,
            captured_turn_seq,
            decision_now,
            recording_emit(&mut events),
        );

        assert!(transition.effect.is_none());
        assert!(transition.continue_watchdog);
        assert_eq!(proc.state, BridgeState::Streaming);
        assert_eq!(proc.turn_phase, TurnPhase::Streaming);
        assert!(proc.turn_watchdog_active);
        assert!(proc.streaming_parts.is_empty());
        assert!(events.is_empty());
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn finalize_timeout_adds_error_part_and_completes_as_failure() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        let partial = MessagePart::Text {
            content: "partial response".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(partial.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&partial));
        let mut events = Vec::new();

        let effect = finalize_turn_as_timeout_locked(
            &mut proc,
            "csid",
            TurnLivenessTimeout::Stale,
            recording_emit(&mut events),
        );

        assert!(effect.was_streaming);
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(effect.final_msg_id.as_deref(), Some("m1"));
        assert!(effect.final_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content == "partial response"
        )));
        assert!(effect.final_parts.iter().any(|part| matches!(
            part,
            MessagePart::Error { content, .. } if content == STALE_ERROR_MESSAGE
        )));
        assert_eq!(proc.pending_stream_part_count, 0);
        assert!(!proc.turn_watchdog_active);
        assert_eq!(events.len(), 1);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn late_turn_complete_after_timeout_does_not_restore_ready_state() {
        let mut proc = make_streaming_test_process();
        proc.begin_turn_liveness();
        let _ = finalize_turn_as_timeout_locked(
            &mut proc,
            "csid",
            TurnLivenessTimeout::Stale,
            |_mid, _parts| (true, true),
        );

        let effect =
            run_turn_complete_transition_locked(&mut proc, "csid", 0, |_mid, _parts| (true, true));

        assert!(!effect.was_streaming);
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn timed_out_recovery_interrupt_is_scoped_to_captured_turn() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.generation_id = 2;
        proc.turn_seq = 7;
        handles.lock().await.insert("csid".to_string(), proc);

        let sent = write_bridge_command_for_captured_turn(
            &handles,
            "csid",
            1,
            6,
            serde_json::json!({ "type": "interrupt" }),
        )
        .await
        .unwrap();

        assert!(!sent, "must not interrupt a later bridge turn");
        let mut proc = handles.lock().await.remove("csid").unwrap();
        let _ = proc.child.kill().await;
    }

    /// Drive the production `respond_agent_permission` lock-block via
    /// `apply_respond_permission_locked`. State emit outside the lock is
    /// mirrored as a pushed `StateChanged(Streaming)` event after the helper
    /// returns so the order assertion mirrors production.
    fn drive_respond_permission_path(
        proc: &mut AgentProcess,
        chat_session_id: &str,
        request_id: &str,
        behavior: &str,
        answers_value: Option<&serde_json::Value>,
        events: &mut Vec<RecordedEmit>,
    ) -> bool {
        let effect = apply_respond_permission_locked(
            proc,
            chat_session_id,
            request_id,
            behavior,
            answers_value,
            recording_emit(events),
        );
        if effect.did_transition {
            events.push(RecordedEmit::StateChanged {
                phase: TurnPhase::Streaming,
                exit_code: None,
            });
        }
        effect.did_transition
    }

    #[tokio::test]
    async fn permission_request_emits_pending_before_state_change() {
        // Spec (Rule: ターン完了・状態遷移時には未配信バッファを強制配信する):
        //   ストリーミング → 権限待ち の遷移時、未配信 delta が state 通知より
        //   前にフロントエンドへ配信されること。
        let mut proc = make_streaming_test_process();
        let delta = vec![MessagePart::Text {
            content: "tail-before-perm".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);

        let mut events = Vec::new();
        let transitioned = drive_permission_request_path(&mut proc, "csid", &mut events);
        assert!(transitioned);

        assert_eq!(events.len(), 2, "both emits must fire");
        match &events[0] {
            RecordedEmit::StreamingFlush {
                parts_count,
                tail_text,
            } => {
                assert_eq!(*parts_count, 1);
                assert_eq!(tail_text.as_deref(), Some("tail-before-perm"));
            }
            other => panic!("first emit must be StreamingFlush, got {other:?}"),
        }
        assert_eq!(
            events[1],
            RecordedEmit::StateChanged {
                phase: TurnPhase::WaitingPermission,
                exit_code: None,
            }
        );
        assert!(proc.pending_stream_part_count == 0);
        assert_eq!(proc.turn_phase, TurnPhase::WaitingPermission);
    }

    #[tokio::test]
    async fn permission_request_without_pending_skips_streaming_emit() {
        // pending が空のとき、state 通知のみが発火し、ストリーム emit は
        // 起きない (prepare_streaming_flush が None → no-op)。
        let mut proc = make_streaming_test_process();

        let mut events = Vec::new();
        assert!(drive_permission_request_path(
            &mut proc,
            "csid",
            &mut events,
        ));

        assert_eq!(
            events,
            vec![RecordedEmit::StateChanged {
                phase: TurnPhase::WaitingPermission,
                exit_code: None,
            }]
        );
    }

    #[tokio::test]
    async fn turn_complete_emits_pending_before_state_change() {
        // Spec: ターン完了時に未配信バッファを強制配信する。
        // streaming emit が state emit (Idle) より前に観測される。
        let mut proc = make_streaming_test_process();
        let delta = vec![MessagePart::Text {
            content: "tail-before-idle".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);

        let mut events = Vec::new();
        drive_turn_complete_path(&mut proc, "csid", 0, &mut events);

        assert_eq!(events.len(), 2);
        match &events[0] {
            RecordedEmit::StreamingFlush {
                parts_count,
                tail_text,
            } => {
                assert_eq!(*parts_count, 1);
                assert_eq!(tail_text.as_deref(), Some("tail-before-idle"));
            }
            other => panic!("first emit must be StreamingFlush, got {other:?}"),
        }
        assert_eq!(
            events[1],
            RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(0),
            }
        );
        assert!(proc.pending_stream_part_count == 0);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(proc.state, BridgeState::Ready);
    }

    #[tokio::test]
    async fn turn_complete_with_nonzero_exit_code_still_flushes_before_state() {
        // 失敗終了 (exit_code != 0) でも emit 順序は同じ: streaming → state。
        let mut proc = make_streaming_test_process();
        let delta = vec![MessagePart::Text {
            content: "tail-error".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(delta.clone());
        enqueue_pending_delta(&mut proc, &delta);

        let mut events = Vec::new();
        drive_turn_complete_path(&mut proc, "csid", 1, &mut events);

        assert!(matches!(events[0], RecordedEmit::StreamingFlush { .. }));
        assert_eq!(
            events[1],
            RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(1),
            }
        );
        assert_eq!(proc.state, BridgeState::Crashed);
    }

    #[tokio::test]
    async fn turn_complete_releases_streaming_parts_after_final_snapshot() {
        let mut proc = make_streaming_test_process();
        proc.task_id_map
            .insert("background-1".to_string(), "tool-1".to_string());
        let raw_parts = vec![
            MessagePart::Text {
                content: "hello".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: " world".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "echo ok" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        proc.streaming_parts.extend(raw_parts.clone());
        enqueue_pending_delta(&mut proc, &raw_parts);

        let effect =
            run_turn_complete_transition_locked(&mut proc, "csid", 0, |_mid, _parts| (true, true));

        assert!(effect.was_streaming);
        assert_eq!(effect.final_msg_id.as_deref(), Some("m1"));
        assert_eq!(effect.final_parts, consolidate_parts_from_slice(&raw_parts));
        assert_eq!(effect.released_streaming_parts, raw_parts);
        assert_eq!(proc.state, BridgeState::Ready);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(proc.last_message_id.as_deref(), Some("m1"));
        assert_eq!(
            proc.post_turn_base_untrusted_message_id.as_deref(),
            Some("m1")
        );
        clear_post_turn_store_base_untrusted_after_persist_success(&mut proc, "m1");
        assert!(proc.post_turn_base_untrusted_message_id.is_none());
        assert_eq!(
            proc.task_id_map.get("background-1").map(String::as_str),
            Some("tool-1")
        );
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_part_count, 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        assert!(proc.last_stream_emit_at.is_none());
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_skips_stale_store_base_when_final_persist_failed() {
        let mut proc = make_streaming_test_process();
        let fresh_parts = vec![
            MessagePart::Text {
                content: "fresh base".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "date" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        proc.streaming_parts.extend(fresh_parts.clone());
        enqueue_pending_delta(&mut proc, &fresh_parts);

        let complete_effect =
            run_turn_complete_transition_locked(&mut proc, "csid", 0, |_mid, _parts| (true, true));

        assert_eq!(complete_effect.final_msg_id.as_deref(), Some("m1"));
        assert_eq!(
            proc.post_turn_base_untrusted_message_id.as_deref(),
            Some("m1"),
            "simulates the final persist failing and leaving the store base stale"
        );
        assert!(proc.streaming_parts.is_empty());

        let stale_store_base = vec![
            MessagePart::Text {
                content: "stale base".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "date" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let msg = post_turn_tool_result_message("tool-1", "must-not-overwrite");
        let mut emitted = false;

        let post_turn_effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _parts| {
                emitted = true;
                (true, true)
            },
            Some(("m1".to_string(), stale_store_base)),
        );

        assert!(post_turn_effect.accumulated);
        assert!(post_turn_effect.emit_msg_id.is_none());
        assert!(!post_turn_effect.should_persist);
        assert!(post_turn_effect.persist_parts.is_empty());
        assert!(!emitted);
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(
            proc.post_turn_base_untrusted_message_id.as_deref(),
            Some("m1")
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn turn_complete_emit_failure_retains_retry_state_until_timer_drains() {
        let mut proc = make_streaming_test_process();
        let raw_parts = vec![MessagePart::Text {
            content: "tail".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(raw_parts.clone());
        enqueue_pending_delta(&mut proc, &raw_parts);

        let effect =
            run_turn_complete_transition_locked(&mut proc, "csid", 0, |_mid, _parts| (false, true));

        assert!(effect.was_streaming);
        assert_eq!(effect.final_msg_id.as_deref(), Some("m1"));
        assert_eq!(effect.final_parts, consolidate_parts_from_slice(&raw_parts));
        assert!(effect.released_streaming_parts.is_empty());
        assert_eq!(proc.state, BridgeState::Ready);
        assert_eq!(proc.last_message_id.as_deref(), Some("m1"));
        assert_eq!(proc.pending_stream_part_count, 1);
        assert_eq!(proc.streaming_parts, raw_parts);
        assert!(
            proc.last_stream_emit_at.is_none(),
            "failed emit must remain retryable"
        );

        let mut emitted: Vec<(String, Vec<MessagePart>)> = Vec::new();
        let tick_effect = run_streaming_timer_tick(&mut proc, "csid", |mid, parts| {
            emitted.push((mid.to_string(), parts.to_vec()));
            (true, true)
        });

        assert!(!tick_effect.keep_running);
        assert_eq!(tick_effect.released_streaming_parts, raw_parts.clone());
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].0, "m1");
        assert_eq!(emitted[0].1, consolidate_parts_from_slice(&raw_parts));
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_part_count, 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn turn_complete_nonzero_exit_code_emit_failure_releases_after_final_snapshot() {
        let mut proc = make_streaming_test_process();
        let raw_parts = vec![MessagePart::Text {
            content: "tail-before-crash".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(raw_parts.clone());
        enqueue_pending_delta(&mut proc, &raw_parts);
        let mut emit_attempts = 0usize;

        let effect = run_turn_complete_transition_locked(&mut proc, "csid", 1, |_mid, _parts| {
            emit_attempts += 1;
            (false, true)
        });

        assert!(effect.was_streaming);
        assert_eq!(effect.final_msg_id.as_deref(), Some("m1"));
        assert_eq!(effect.final_parts, consolidate_parts_from_slice(&raw_parts));
        assert_eq!(effect.released_streaming_parts, raw_parts);
        assert_eq!(emit_attempts, 1);
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(proc.last_message_id.as_deref(), Some("m1"));
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_part_count, 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        let _ = proc.child.kill().await;
    }

    fn post_turn_tool_result_message(tool_use_id: &str, content: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "user",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": content,
                        "is_error": false
                    }
                ]
            }
        })
    }

    #[tokio::test]
    async fn post_turn_permission_mode_notification_skips_reseed_and_persist() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some("m1".to_string());
        let msg = serde_json::json!({
            "type": "system",
            "permissionMode": "acceptEdits"
        });
        let mut emitted = false;

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _parts| {
                emitted = true;
                (true, true)
            },
            None,
        );

        assert!(!effect.accumulated);
        assert!(effect.post_turn_reseed_message_id.is_none());
        assert!(effect.emit_msg_id.is_none());
        assert!(!effect.should_persist);
        assert!(effect.persist_parts.is_empty());
        assert!(!emitted);
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_part_count, 0);
        assert!(should_forward_sdk_message(effect.accumulated, "system"));
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_partless_system_notification_skips_reseed_emit_and_persist() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some("m1".to_string());
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "hook_started",
            "hook_name": "SessionEnd",
            "hook_event": "StopSession",
            "hook_id": "hook-001"
        });
        let mut emitted = false;

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _parts| {
                emitted = true;
                (true, true)
            },
            None,
        );

        assert!(effect.accumulated);
        assert!(effect.post_turn_reseed_message_id.is_none());
        assert!(effect.emit_msg_id.is_none());
        assert!(!effect.should_persist);
        assert!(effect.persist_parts.is_empty());
        assert!(!emitted);
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_part_count, 0);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_reseed_preserves_cumulative_payload_and_releases_again() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some("m1".to_string());
        let base_parts = vec![
            MessagePart::Text {
                content: "base".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "date" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let delta_part = MessagePart::ToolResult {
            content: "done".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
        };
        let expected_parts = consolidate_parts_from_slice(
            &base_parts
                .iter()
                .cloned()
                .chain(std::iter::once(delta_part))
                .collect::<Vec<_>>(),
        );
        let msg = post_turn_tool_result_message("tool-1", "done");
        let mut emitted: Vec<Vec<MessagePart>> = Vec::new();

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, parts| {
                emitted.push(parts.to_vec());
                (true, true)
            },
            Some(("m1".to_string(), base_parts.clone())),
        );

        assert!(effect.accumulated);
        assert_eq!(effect.emit_msg_id.as_deref(), Some("m1"));
        assert!(effect.should_persist);
        assert_eq!(effect.persist_parts, expected_parts);
        assert_eq!(effect.released_streaming_parts, expected_parts.clone());
        assert_eq!(emitted, vec![expected_parts]);
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_part_count, 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_emit_failure_requests_timer_restart_when_idle() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some("m1".to_string());
        assert!(!proc.streaming_timer_active);
        let base_parts = vec![
            MessagePart::Text {
                content: "base".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "date" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let delta_part = MessagePart::ToolResult {
            content: "done".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
        };
        let expected_parts = consolidate_parts_from_slice(
            &base_parts
                .iter()
                .cloned()
                .chain(std::iter::once(delta_part))
                .collect::<Vec<_>>(),
        );
        let msg = post_turn_tool_result_message("tool-1", "done");
        let mut emitted_attempts = 0;

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, parts| {
                emitted_attempts += 1;
                assert_eq!(parts, expected_parts.as_slice());
                (false, true)
            },
            Some(("m1".to_string(), base_parts)),
        );

        assert!(effect.accumulated);
        assert_eq!(effect.emit_msg_id.as_deref(), Some("m1"));
        assert!(effect.should_persist);
        assert_eq!(effect.persist_parts, expected_parts);
        assert!(effect.start_streaming_timer);
        assert!(effect.released_streaming_parts.is_empty());
        assert_eq!(emitted_attempts, 1);
        assert_eq!(proc.pending_stream_part_count, 1);
        assert!(!proc.streaming_parts.is_empty());
        assert!(
            proc.last_stream_emit_at.is_none(),
            "failed post-turn emit must remain retryable"
        );

        let mut retry_payloads: Vec<(String, Vec<MessagePart>)> = Vec::new();
        let tick_effect = run_streaming_timer_tick(&mut proc, "csid", |mid, parts| {
            retry_payloads.push((mid.to_string(), parts.to_vec()));
            (true, true)
        });

        assert!(!tick_effect.keep_running);
        assert_eq!(tick_effect.released_streaming_parts, expected_parts.clone());
        assert_eq!(retry_payloads, vec![("m1".to_string(), expected_parts)]);
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_part_count, 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_reseed_retry_persists_old_message_when_new_turn_started_after_base_load() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some("old-message".to_string());
        let msg = post_turn_tool_result_message("tool-1", "late");
        let mut emitted: Vec<(String, Vec<MessagePart>)> = Vec::new();

        let first_effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _parts| panic!("first pass must request a store reseed before emitting"),
            None,
        );

        assert!(!first_effect.accumulated);
        assert_eq!(
            first_effect.post_turn_reseed_message_id.as_deref(),
            Some("old-message")
        );
        assert!(emitted.is_empty());

        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("new-message".to_string());
        proc.reset_streaming_state_for_new_turn();
        let new_turn_parts = vec![MessagePart::Text {
            content: "new turn".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts = new_turn_parts.clone();
        proc.task_id_map
            .insert("new-task".to_string(), "new-tool".to_string());

        let stale_base_parts = vec![
            MessagePart::Text {
                content: "old base".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "date" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let expected_parts = consolidate_parts_from_slice(
            &stale_base_parts
                .iter()
                .cloned()
                .chain(std::iter::once(MessagePart::ToolResult {
                    content: "late".to_string(),
                    is_error: false,
                    tool_use_id: Some("tool-1".to_string()),
                    parent_tool_use_id: None,
                }))
                .collect::<Vec<_>>(),
        );
        let second_effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |mid, parts| {
                emitted.push((mid.to_string(), parts.to_vec()));
                (true, true)
            },
            Some(("old-message".to_string(), stale_base_parts)),
        );

        assert!(second_effect.accumulated);
        assert_eq!(second_effect.emit_msg_id.as_deref(), Some("old-message"));
        assert!(second_effect.should_persist);
        assert_eq!(second_effect.persist_parts, expected_parts);
        assert!(!second_effect.start_streaming_timer);
        assert_eq!(
            emitted,
            vec![(
                "old-message".to_string(),
                second_effect.persist_parts.clone()
            )]
        );
        assert_eq!(proc.state, BridgeState::Streaming);
        assert_eq!(proc.streaming_message_id.as_deref(), Some("new-message"));
        assert!(proc.last_message_id.is_none());
        assert_eq!(proc.streaming_parts, new_turn_parts);
        assert_eq!(proc.pending_stream_part_count, 0);
        assert_eq!(
            proc.task_id_map.get("new-task").map(String::as_str),
            Some("new-tool")
        );
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_reseed_retry_applies_status_to_old_message_from_loaded_task_map() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("new-message".to_string());
        let new_turn_parts = vec![MessagePart::Text {
            content: "new turn".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts = new_turn_parts.clone();
        proc.task_id_map
            .insert("new-task".to_string(), "new-tool".to_string());

        let base_parts = vec![MessagePart::ToolResult {
            content: "Async agent launched successfully.\nagentId: old-task (internal ID)"
                .to_string(),
            is_error: false,
            tool_use_id: Some("old-tool".to_string()),
            parent_tool_use_id: None,
        }];
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "task_updated",
            "task_id": "old-task",
            "patch": {"status": "completed", "summary": "done"}
        });

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _parts| (true, true),
            Some(("old-message".to_string(), base_parts.clone())),
        );

        assert!(effect.accumulated);
        assert_eq!(effect.emit_msg_id.as_deref(), Some("old-message"));
        assert!(effect.should_persist);
        assert!(matches!(
            effect.persist_parts.last(),
            Some(MessagePart::TaskStatus {
                task_tool_use_id,
                status,
                summary,
                ..
            }) if task_tool_use_id == "old-tool"
                && status == "completed"
                && summary.as_deref() == Some("done")
        ));
        assert_eq!(proc.streaming_message_id.as_deref(), Some("new-message"));
        assert_eq!(proc.streaming_parts, new_turn_parts);
        assert_eq!(
            proc.task_id_map.get("new-task").map(String::as_str),
            Some("new-tool")
        );
        assert!(!proc.task_id_map.contains_key("old-task"));
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_reseed_retry_persist_payload_restores_old_message_without_duplication() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let message_id = "old-message";
        let base_parts = vec![
            MessagePart::Text {
                content: "old base".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "date" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let (content, thinking, activities) = parts_to_legacy(&base_parts);
        session.messages.push(ChatMessage {
            id: message_id.to_string(),
            role: MessageRole::Agent,
            content,
            thinking,
            activities,
            parts: Some(base_parts.clone()),
            timestamp: 10.0,
            mentions: None,
        });
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("new-message".to_string());
        let new_turn_parts = vec![MessagePart::Text {
            content: "new turn".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts = new_turn_parts.clone();
        let msg = post_turn_tool_result_message("tool-1", "late");
        let mut emitted = Vec::new();

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |mid, parts| {
                emitted.push((mid.to_string(), parts.to_vec()));
                (true, true)
            },
            Some((message_id.to_string(), base_parts.clone())),
        );

        assert!(effect.should_persist);
        let persisted = persist_streaming_parts(
            &store,
            &app.handle(),
            &session.id,
            message_id,
            &effect.persist_parts,
            None,
        );
        assert!(persisted);

        let expected_parts = vec![
            base_parts[0].clone(),
            base_parts[1].clone(),
            MessagePart::ToolResult {
                content: "late".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
            },
        ];
        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        let loaded_message = loaded
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .expect("old agent message persisted");
        assert_eq!(
            loaded_message.parts.as_deref(),
            Some(expected_parts.as_slice())
        );
        assert_eq!(proc.streaming_message_id.as_deref(), Some("new-message"));
        assert_eq!(proc.streaming_parts, new_turn_parts);
        assert_eq!(emitted, vec![(message_id.to_string(), expected_parts)]);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn external_post_turn_events_reseed_from_store_without_duplication() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let message_id = "agent-message-1";
        let base_parts = vec![
            MessagePart::Text {
                content: "base".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "date" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let (content, thinking, activities) = parts_to_legacy(&base_parts);
        session.messages.push(ChatMessage {
            id: message_id.to_string(),
            role: MessageRole::Agent,
            content,
            thinking,
            activities,
            parts: Some(base_parts.clone()),
            timestamp: 10.0,
            mentions: None,
        });
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.backend_id = CODEX_BACKEND_ID.to_string();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some(message_id.to_string());
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            post_turn_tool_result_message("tool-1", "first"),
            &mut state,
        )
        .await;

        {
            let map = handles.lock().await;
            let proc = map.get(&session.id).unwrap();
            assert!(proc.streaming_parts.is_empty());
        }
        let first_expected = vec![
            base_parts[0].clone(),
            base_parts[1].clone(),
            MessagePart::ToolResult {
                content: "first".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
            },
        ];
        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        let loaded_message = loaded
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .expect("agent message persisted");
        assert_eq!(
            loaded_message.parts.as_deref(),
            Some(first_expected.as_slice())
        );

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            post_turn_tool_result_message("tool-1", " second"),
            &mut state,
        )
        .await;

        let final_expected = vec![
            base_parts[0].clone(),
            base_parts[1].clone(),
            MessagePart::ToolResult {
                content: "first second".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
            },
        ];
        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        let loaded_message = loaded
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .expect("agent message persisted");
        assert_eq!(
            loaded_message.parts.as_deref(),
            Some(final_expected.as_slice())
        );
        let mut proc = handles.lock().await.remove(&session.id).unwrap();
        assert!(proc.streaming_parts.is_empty());
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_reseed_failure_skips_accumulate_and_persist_payload() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;
        proc.last_message_id = Some("m1".to_string());
        let msg = post_turn_tool_result_message("tool-1", "should-not-write");
        let mut emitted = false;

        let effect = accumulate_stream_or_post_turn_message_locked(
            &mut proc,
            "csid",
            &msg,
            0,
            |_mid, _parts| {
                emitted = true;
                (true, true)
            },
            None,
        );

        assert!(!effect.accumulated);
        assert_eq!(effect.post_turn_reseed_message_id.as_deref(), Some("m1"));
        assert!(effect.emit_msg_id.is_none());
        assert!(!effect.should_persist);
        assert!(effect.persist_parts.is_empty());
        assert!(!emitted);
        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_part_count, 0);
        let _ = proc.child.kill().await;
    }

    #[test]
    fn consolidated_post_turn_base_matches_raw_retained_payload() {
        let raw_base = vec![
            MessagePart::Text {
                content: "hel".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "lo".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({}),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Thinking {
                content: "thin".to_string(),
                parent_tool_use_id: Some("tool-1".to_string()),
            },
            MessagePart::Thinking {
                content: "king".to_string(),
                parent_tool_use_id: Some("tool-1".to_string()),
            },
        ];
        let post_turn_delta = vec![
            MessagePart::Text {
                content: " done".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolResult {
                content: "ok".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
            },
        ];

        let old_payload = consolidate_parts_from_slice(
            &raw_base
                .iter()
                .cloned()
                .chain(post_turn_delta.iter().cloned())
                .collect::<Vec<_>>(),
        );
        let new_payload = consolidate_parts_from_slice(
            &consolidate_parts_from_slice(&raw_base)
                .into_iter()
                .chain(post_turn_delta)
                .collect::<Vec<_>>(),
        );

        assert_eq!(new_payload, old_payload);
    }

    /// Build a streaming-test process with one pending `Permission` part in
    /// the streaming buffer matching `request_id`. Used by the
    /// respond_permission tests to mimic the production state at the moment
    /// `respond_agent_permission` runs.
    fn make_process_waiting_for_permission(request_id: &str) -> AgentProcess {
        let mut proc = make_streaming_test_process();
        proc.turn_phase = TurnPhase::WaitingPermission;
        proc.streaming_parts.push(MessagePart::Permission {
            request: serde_json::json!({ "request_id": request_id }),
            status: "pending".to_string(),
            answers: None,
            parent_tool_use_id: None,
        });
        proc
    }

    #[tokio::test]
    async fn respond_permission_orders_flush_then_state_change() {
        // Spec (Rule: ターン完了・状態遷移時には未配信バッファを強制配信する):
        //   権限待ち → ストリーミング への遷移時、Permission part 更新を
        //   含む強制 flush が state 通知より前に観測されること。
        let mut proc = make_process_waiting_for_permission("req-1");
        let mut events = Vec::new();
        let transitioned =
            drive_respond_permission_path(&mut proc, "csid", "req-1", "allow", None, &mut events);
        assert!(transitioned);

        assert_eq!(events.len(), 2, "flush emit then state emit");
        match &events[0] {
            RecordedEmit::StreamingFlush { parts_count, .. } => {
                assert!(*parts_count >= 1);
            }
            other => panic!("first emit must be StreamingFlush, got {other:?}"),
        }
        assert_eq!(
            events[1],
            RecordedEmit::StateChanged {
                phase: TurnPhase::Streaming,
                exit_code: None,
            }
        );
        assert_eq!(proc.turn_phase, TurnPhase::Streaming);
        assert!(proc.pending_stream_part_count == 0);
        // Permission part status was updated in place.
        let updated = proc
            .streaming_parts
            .iter()
            .find_map(|p| match p {
                MessagePart::Permission { status, .. } => Some(status.clone()),
                _ => None,
            })
            .expect("permission part present");
        assert_eq!(updated, "allowed");
    }

    #[tokio::test]
    async fn respond_permission_no_transition_when_not_waiting() {
        // 直前に WaitingPermission でなかった場合、state は変更されず、
        // 後続の state-changed emit も発火しないこと。
        let mut proc = make_process_waiting_for_permission("req-1");
        proc.turn_phase = TurnPhase::Streaming; // not WaitingPermission

        let mut events = Vec::new();
        let transitioned =
            drive_respond_permission_path(&mut proc, "csid", "req-1", "deny", None, &mut events);
        assert!(
            !transitioned,
            "no transition when proc was not in WaitingPermission"
        );

        // StateChanged は events に積まれていないこと。
        assert!(!events
            .iter()
            .any(|e| matches!(e, RecordedEmit::StateChanged { .. })));
        assert_eq!(proc.turn_phase, TurnPhase::Streaming);
    }

    /// Drive the production `bridge error` lock-block via
    /// `run_bridge_error_transition_locked`, then mirror the post-lock
    /// `emit_session_state_changed`.
    fn drive_bridge_error_path(
        proc: &mut AgentProcess,
        chat_session_id: &str,
        error_message: &str,
        events: &mut Vec<RecordedEmit>,
    ) -> BridgeErrorTransition {
        let msg = serde_json::json!({
            "type": "error",
            "message": error_message,
        });
        let transition =
            run_bridge_error_transition_locked(proc, chat_session_id, &msg, recording_emit(events));
        if transition.turn_complete.was_streaming {
            events.push(RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(1),
            });
        }
        transition
    }

    /// Drive the production `EOF crash` lock-block via
    /// `run_bridge_eof_crash_transition_locked`, then push a
    /// `StateChanged(Idle)` event to mirror `emit_session_state_changed`.
    fn drive_bridge_eof_crash_path(
        proc: &mut AgentProcess,
        chat_session_id: &str,
        events: &mut Vec<RecordedEmit>,
    ) {
        let transition = run_bridge_eof_crash_transition_locked(
            true,
            proc,
            chat_session_id,
            recording_emit(events),
        );
        if transition.turn_complete.was_streaming {
            events.push(RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(-1),
            });
        }
    }

    #[tokio::test]
    async fn bridge_error_persist_success_clears_untrusted_and_allows_post_turn_update() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let message_id = "m1";
        let empty_parts: Vec<MessagePart> = Vec::new();
        let (content, thinking, activities) = parts_to_legacy(&empty_parts);
        session.messages.push(ChatMessage {
            id: message_id.to_string(),
            role: MessageRole::Agent,
            content,
            thinking,
            activities,
            parts: Some(empty_parts),
            timestamp: 10.0,
            mentions: None,
        });
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_streaming_test_process();
        proc.backend_id = CODEX_BACKEND_ID.to_string();
        let streaming_parts = vec![
            MessagePart::Text {
                content: "before error".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({ "cmd": "date" }),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
        ];
        proc.streaming_parts.extend(streaming_parts.clone());
        enqueue_pending_delta(&mut proc, &streaming_parts);
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "error",
                "message": "bridge reported failure",
            }),
            &mut state,
        )
        .await;

        {
            let map = handles.lock().await;
            let proc = map.get(&session.id).unwrap();
            assert!(proc.post_turn_base_untrusted_message_id.is_none());
            assert_eq!(proc.last_message_id.as_deref(), Some(message_id));
        }

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            post_turn_tool_result_message("tool-1", "post-turn result"),
            &mut state,
        )
        .await;

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        let loaded_message = loaded
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .unwrap();
        assert!(loaded_message
            .parts
            .as_deref()
            .unwrap()
            .iter()
            .any(|part| matches!(
                part,
                MessagePart::ToolResult { content, .. } if content == "post-turn result"
            )));

        let removed_proc = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed_proc {
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn bridge_error_persist_failure_keeps_untrusted() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_streaming_test_process();
        proc.backend_id = CODEX_BACKEND_ID.to_string();
        let streaming_parts = vec![MessagePart::Text {
            content: "before error".to_string(),
            parent_tool_use_id: None,
        }];
        proc.streaming_parts.extend(streaming_parts.clone());
        enqueue_pending_delta(&mut proc, &streaming_parts);
        handles.lock().await.insert(session.id.clone(), proc);
        let mut state = ExternalBridgeMessageState::default();

        handle_external_bridge_message(
            &app.handle(),
            &store,
            &handles,
            &session.id,
            serde_json::json!({
                "type": "error",
                "message": "bridge reported failure",
            }),
            &mut state,
        )
        .await;

        {
            let map = handles.lock().await;
            let proc = map.get(&session.id).unwrap();
            assert_eq!(
                proc.post_turn_base_untrusted_message_id.as_deref(),
                Some("m1")
            );
        }

        let removed_proc = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed_proc {
            let _ = proc.child.kill().await;
        }
    }

    #[tokio::test]
    async fn bridge_error_emits_pending_before_state_change() {
        // Spec (Rule: ターン完了・状態遷移時には未配信バッファを強制配信する,
        //  Examples ストリーミング → クラッシュ):
        //   Bridge から error メッセージを受信したクラッシュ経路では、
        //   未配信 delta + 合成 error part が同一 cumulative payload として
        //   state 通知 (Idle) より前にフロントエンドへ配信されること。
        let mut proc = make_streaming_test_process();
        // 未配信 text が pending に残っている状態でクラッシュが起こる。
        let pending_text = MessagePart::Text {
            content: "tail-before-crash".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(pending_text.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&pending_text));

        let mut events = Vec::new();
        let transition =
            drive_bridge_error_path(&mut proc, "csid", "bridge reported failure", &mut events);

        assert_eq!(events.len(), 2, "flush emit then state emit");
        assert!(transition.turn_complete.was_streaming);
        assert!(transition
            .turn_complete
            .final_parts
            .iter()
            .any(|part| matches!(
                part,
                MessagePart::Error { content, .. } if content == "Error: bridge reported failure"
            )));
        match &events[0] {
            RecordedEmit::StreamingFlush {
                parts_count,
                tail_text,
            } => {
                // cumulative: pending Text + 合成 Error
                assert_eq!(*parts_count, 2);
                assert_eq!(
                    tail_text.as_deref(),
                    Some("Error: bridge reported failure"),
                    "tail must be the synthetic error part"
                );
            }
            other => panic!("first emit must be StreamingFlush, got {other:?}"),
        }
        assert_eq!(
            events[1],
            RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(1),
            }
        );
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(proc.pending_stream_part_count, 0);
    }

    #[tokio::test]
    async fn bridge_eof_crash_emits_pending_before_state_change() {
        // Spec (Rule: ターン完了・状態遷移時には未配信バッファを強制配信する,
        //  Examples ストリーミング → クラッシュ):
        //   Bridge process EOF クラッシュ経路では、未配信 delta + 合成 error
        //   part が同一 cumulative payload として state 通知 (Idle) より前に
        //   フロントエンドへ配信されること。
        let mut proc = make_streaming_test_process();
        let pending_text = MessagePart::Text {
            content: "tail-before-eof".to_string(),
            parent_tool_use_id: None,
        };
        proc.streaming_parts.push(pending_text.clone());
        enqueue_pending_delta(&mut proc, std::slice::from_ref(&pending_text));

        let mut events = Vec::new();
        drive_bridge_eof_crash_path(&mut proc, "csid", &mut events);

        assert_eq!(events.len(), 2, "flush emit then state emit");
        match &events[0] {
            RecordedEmit::StreamingFlush {
                parts_count,
                tail_text,
            } => {
                // cumulative: pending Text + EOF transition が積んだ Error。
                assert_eq!(*parts_count, 2);
                assert!(
                    tail_text
                        .as_deref()
                        .unwrap_or("")
                        .contains("Bridge process exited unexpectedly"),
                    "tail must be the synthetic EOF error part, got {tail_text:?}"
                );
            }
            other => panic!("first emit must be StreamingFlush, got {other:?}"),
        }
        assert_eq!(
            events[1],
            RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(-1),
            }
        );
        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert_eq!(proc.pending_stream_part_count, 0);
    }

    #[tokio::test]
    async fn respond_permission_continues_on_emit_failure() {
        // Spec L157「強制配信が失敗しても後続の状態遷移は続行する」:
        //  emit 失敗 (tauri_ok=false) でも did_transition は true のまま返り、
        //  呼び出し側 (production: emit_session_state_changed) は続行できる。
        let mut proc = make_process_waiting_for_permission("req-1");

        let effect = apply_respond_permission_locked(
            &mut proc,
            "csid",
            "req-1",
            "allow",
            None,
            |_mid, _parts| (false, false), // emit failure on both channels
        );
        assert!(
            effect.did_transition,
            "transition must still be reported so caller emits state-change"
        );
        assert_eq!(proc.turn_phase, TurnPhase::Streaming);
        // Pending is retained for next-flush retry (Spec L108-113).
        assert!(proc.pending_stream_part_count >= 1);
        assert!(proc.last_stream_emit_at.is_none());
    }

    #[test]
    fn delta_has_tool_event_detects_tool_use_and_tool_result() {
        assert!(delta_has_tool_event(&[MessagePart::ToolUse {
            id: "1".to_string(),
            tool: "Bash".to_string(),
            input: serde_json::json!({}),
            parent_tool_use_id: None,
        }]));
        assert!(delta_has_tool_event(&[MessagePart::ToolResult {
            tool_use_id: Some("1".to_string()),
            content: "ok".to_string(),
            is_error: false,
            parent_tool_use_id: None,
        }]));
        assert!(!delta_has_tool_event(&[MessagePart::Text {
            content: "plain".to_string(),
            parent_tool_use_id: None,
        }]));
        assert!(!delta_has_tool_event(&[]));
    }

    fn make_streaming_test_process() -> AgentProcess {
        // Standalone, non-running AgentProcess used purely to exercise the
        // coalescing helpers. Stdin/child are tied to a `cat` subprocess so
        // the struct is well-formed. Must run inside a Tokio runtime.
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("m1".to_string());
        proc
    }

    #[tokio::test]
    async fn bridge_eof_crash_adds_error_part_for_streaming_message() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("message-1".to_string());
        proc.streaming_parts.push(MessagePart::Text {
            content: "partial".to_string(),
            parent_tool_use_id: None,
        });

        let transition =
            run_bridge_eof_crash_transition_locked(true, &mut proc, "csid", |_mid, _parts| {
                (true, true)
            });

        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert!(transition.turn_complete.was_streaming);
        assert_eq!(
            transition.turn_complete.final_msg_id.as_deref(),
            Some("message-1")
        );
        assert_eq!(transition.turn_complete.final_parts.len(), 2);
        assert!(transition
            .sdk_error_message
            .as_deref()
            .unwrap()
            .contains("mock"));
        assert!(!transition.should_evict);
        assert!(matches!(
            &transition.turn_complete.final_parts[1],
            MessagePart::Error { content, .. }
                if content.contains("Bridge process exited unexpectedly")
        ));
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn bridge_eof_crash_marks_initializing_without_streaming_part() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Initializing;
        proc.turn_phase = TurnPhase::Idle;

        let transition =
            run_bridge_eof_crash_transition_locked(true, &mut proc, "csid", |_mid, _parts| {
                (true, true)
            });

        assert_eq!(proc.state, BridgeState::Crashed);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert!(transition.was_initializing);
        assert!(!transition.should_evict);
        assert!(transition.turn_complete.final_parts.is_empty());
        assert!(transition.sdk_error_message.is_some());
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn bridge_eof_ready_idle_requests_eviction_without_error() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;

        let transition =
            run_bridge_eof_crash_transition_locked(true, &mut proc, "csid", |_mid, _parts| {
                (true, true)
            });

        // Ready/Idle EOF leaves the state untouched but flags the runtime for eviction.
        assert_eq!(proc.state, BridgeState::Ready);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert!(!transition.was_initializing);
        assert!(transition.should_evict);
        assert!(transition.turn_complete.final_parts.is_empty());
        assert!(transition.sdk_error_message.is_none());
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn bridge_eof_generation_mismatch_does_not_evict_or_mutate() {
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Ready;
        proc.turn_phase = TurnPhase::Idle;

        let transition =
            run_bridge_eof_crash_transition_locked(false, &mut proc, "csid", |_mid, _parts| {
                (true, true)
            });

        assert_eq!(proc.state, BridgeState::Ready);
        assert_eq!(proc.turn_phase, TurnPhase::Idle);
        assert!(!transition.should_evict);
        assert!(!transition.was_initializing);
        assert!(transition.turn_complete.final_parts.is_empty());
        assert!(transition.sdk_error_message.is_none());
        let _ = proc.child.kill().await;
    }

    #[test]
    fn available_models_for_backend_reads_from_config_via_registry() {
        let mut cfg = crate::adaptor::gateway::app_config::ReleashConfig::default();
        cfg.agents.claude.models = vec!["mock-model".to_string()];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
            cfg,
            tmp.path().to_path_buf(),
        ));

        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: CLAUDE_BACKEND_ID.to_string(),
            models: vec![],
        }));
        registry.set_config(config);
        let registry = Arc::new(registry);

        let models = available_models_for_backend(CLAUDE_BACKEND_ID, Some(&registry)).unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "claude:mock-model");
        assert_eq!(models[0].model_id, "mock-model");
    }

    #[test]
    fn available_models_for_backend_propagates_registry_error() {
        // registry に config が未紐付けの状態では Err が返り、空配列で潰れない。
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: CLAUDE_BACKEND_ID.to_string(),
            models: vec![],
        }));
        let registry = Arc::new(registry);
        let err = available_models_for_backend(CLAUDE_BACKEND_ID, Some(&registry));
        assert!(err.is_err(), "config 未紐付けは Err として伝播する");
    }

    #[test]
    fn available_models_for_backend_returns_empty_without_registry() {
        // registry が無い経路（テスト等）は Ok(empty) として扱う。
        let models = available_models_for_backend(CLAUDE_BACKEND_ID, None).unwrap();
        assert!(models.is_empty());
    }

    #[tokio::test]
    async fn prepared_send_accepts_already_validated_workflow_step_session() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock".to_string(),
            models: vec![],
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let worktree_path = "/repo".to_string();
        let mut step_session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock".to_string()),
        )
        .unwrap();
        step_session.workflow_step_session = true;
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &step_session)
            .unwrap();
        let parent_session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock".to_string()),
        )
        .unwrap();

        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &parent_session)
            .unwrap();

        let result = prepare_send_agent_message_internal(
            &crate::adaptor::controller::wiring::build_code_usecase(),
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(step_session.id.clone()),
            "/different-request-worktree".to_string(),
            "continue completed step".to_string(),
            crate::permission::PermissionMode::Edit,
            false,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        let (_response, prepared_turn) = result.unwrap();
        let prepared_turn = prepared_turn.expect("workflow step message starts a turn");
        let prepared_turn = expect_prepared_turn(prepared_turn);
        assert_eq!(prepared_turn.worktree_path, "/repo");
        let saved = session_store
            .load_full_session_for_restore(data_dir.path(), &step_session.id)
            .unwrap()
            .unwrap();
        assert!(
            saved
                .messages
                .iter()
                .any(|message| message.content == "continue completed step"),
            "workflow command validation happens before bridge turn preparation"
        );
    }

    fn workflow_state_for_runtime_test(
        session_id: &str,
    ) -> crate::domain::workflow::WorkflowStateSnapshot {
        crate::domain::workflow::WorkflowStateSnapshot {
            execution_id: "exec-runtime".to_string(),
            workflow_name: "wf".to_string(),
            state: crate::domain::workflow::WorkflowExecutionState::Running,
            current_step_index: 0,
            current_step_name: "step".to_string(),
            current_session_id: Some(session_id.to_string()),
            total_steps: 1,
            step_history: Vec::new(),
            step_execution_counts: HashMap::new(),
            workflow_definition: crate::domain::workflow::WorkflowDefinition {
                variables: Default::default(),
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                nodes: vec![],
            },
            total_token_usage: crate::domain::workflow::TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![],
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 1.0,
            updated_at: 1.0,
        }
    }

    #[tokio::test]
    async fn prepared_send_to_regular_session_does_not_change_workflow_step_runtime() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock".to_string(),
            models: vec![],
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let open_tabs = crate::usecase::agent_session::session::OpenTabRegistry::default();
        let worktree_path = "/repo".to_string();
        let regular_session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock".to_string()),
        )
        .unwrap();
        let mut step_session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock".to_string()),
        )
        .unwrap();
        step_session.workflow_step_session = true;
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &step_session)
            .unwrap();
        handles
            .lock()
            .await
            .insert(step_session.id.clone(), make_test_agent_process());

        let before =
            crate::adaptor::gateway::workflow::build_workflow_state_projection_from_snapshot(
                workflow_state_for_runtime_test(&step_session.id),
                Some(&handles),
                Some(&open_tabs),
            )
            .await;

        let (_response, prepared_turn) = prepare_send_agent_message_internal(
            &crate::adaptor::controller::wiring::build_code_usecase(),
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(regular_session.id),
            worktree_path,
            "regular chat".to_string(),
            crate::permission::PermissionMode::Edit,
            false,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(prepared_turn.is_some());
        assert!(handles.lock().await.contains_key(&step_session.id));
        let after =
            crate::adaptor::gateway::workflow::build_workflow_state_projection_from_snapshot(
                workflow_state_for_runtime_test(&step_session.id),
                Some(&handles),
                Some(&open_tabs),
            )
            .await;
        assert_eq!(
            before.runtime_states[&step_session.id].runtime_active,
            after.runtime_states[&step_session.id].runtime_active
        );
    }

    #[tokio::test]
    async fn prepare_send_persists_selected_permission_mode_for_new_session() {
        // Spec issues-947: 新規セッション作成時、選択された抽象モードがそのまま
        // ChatSession.permission_mode に保存される（PreparedAgentTurn と乖離しない）。
        // モデル未選択状態は廃止されたため、新規セッションは backend の既定モデル解決を
        // 必要とする。fixed_models を持つ実 backend registry を使う。
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let worktree_path = "/repo".to_string();

        let (response, prepared_turn) = prepare_send_agent_message_internal(
            &crate::adaptor::controller::wiring::build_code_usecase(),
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            None,
            worktree_path.clone(),
            "hi".to_string(),
            crate::permission::PermissionMode::Ask,
            false,
            Some(CLAUDE_BACKEND_ID.to_string()),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let prepared_turn = prepared_turn.expect("new session should start a turn");
        let prepared_turn = expect_prepared_turn(prepared_turn);
        assert_eq!(prepared_turn.permission_mode, "ask");
        assert_eq!(response.session.permission_mode, "ask");
        let saved = session_store
            .load_full_session_for_restore(data_dir.path(), &response.session.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.permission_mode, "ask");
    }

    #[tokio::test]
    async fn prepare_send_existing_session_uses_meta_and_append_without_hydrating_body() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock".to_string(),
            models: vec![],
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = ChatSession {
            id: session_id.clone(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "old-message".to_string(),
                role: MessageRole::Human,
                content: "old body".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: crate::usecase::agent_session::session::SessionState::Active,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            selected_model: None,
            permission_profile_id: None,
            backend_id: Some("mock".to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
        };
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &session)
            .unwrap();
        std::fs::write(
            data_dir
                .path()
                .join("sessions")
                .join(&session_id)
                .join("messages")
                .join("1.json"),
            "{",
        )
        .unwrap();

        let (response, prepared_turn) = prepare_send_agent_message_internal(
            &crate::adaptor::controller::wiring::build_code_usecase(),
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(session_id.clone()),
            "/repo".to_string(),
            "new prompt".to_string(),
            crate::permission::PermissionMode::Ask,
            true,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(prepared_turn.is_some());
        assert!(response.session.messages.is_empty());
        assert_eq!(response.session.permission_mode, "ask");
        assert!(response.session.plan_mode);
        assert_eq!(response.human_message.content, "new prompt");
        assert!(response.agent_message.is_some());
        let page = session_store
            .get_session_page(data_dir.path(), &session_id, None, 2)
            .unwrap()
            .unwrap();
        assert_eq!(
            page.messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                response.human_message.id.as_str(),
                response.agent_message.as_ref().unwrap().id.as_str(),
            ]
        );
    }

    #[tokio::test]
    async fn prepared_turn_carries_codex_backend_for_runtime_dispatch() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let (_response, prepared_turn) = prepare_send_agent_message_internal(
            &crate::adaptor::controller::wiring::build_code_usecase(),
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            None,
            "/repo".to_string(),
            "hello codex".to_string(),
            crate::permission::PermissionMode::Edit,
            false,
            Some(CODEX_BACKEND_ID.to_string()),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let prepared_turn = prepared_turn.expect("new codex session should start a turn");
        let prepared_turn = expect_prepared_turn(prepared_turn);
        assert_eq!(prepared_turn.backend_id, CODEX_BACKEND_ID);
        assert_eq!(prepared_turn.prompt, "hello codex");
    }

    #[tokio::test]
    async fn initializing_idle_runtime_prepares_first_turn_instead_of_queueing() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock".to_string(),
            models: vec![],
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let worktree_path = "/repo".to_string();
        let session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock".to_string()),
        )
        .unwrap();
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Initializing;
        proc.turn_phase = TurnPhase::Idle;
        handles.lock().await.insert(session.id.clone(), proc);

        let (response, prepared_input) = prepare_send_agent_message_internal(
            &crate::adaptor::controller::wiring::build_code_usecase(),
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(session.id.clone()),
            worktree_path,
            "first restored turn".to_string(),
            crate::permission::PermissionMode::Edit,
            false,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(response.queued_turn.is_none());
        assert!(response.agent_message.is_some());
        assert!(response.pending_queue.is_empty());
        let prepared_turn = expect_prepared_turn(prepared_input.expect("first turn should start"));
        assert_eq!(prepared_turn.session_id, session.id);
        assert_eq!(prepared_turn.prompt, "first restored turn");
        let mut proc = handles.lock().await.remove(&session.id).unwrap();
        assert!(proc.pending_messages.is_empty());
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn busy_send_uses_active_turn_steer_when_backend_is_ready() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockSteeringBackend {
            backend_id: "steer".to_string(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let worktree_path = "/repo".to_string();
        let session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("steer".to_string()),
        )
        .unwrap();
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        handles.lock().await.insert(session.id.clone(), proc);

        let (response, prepared_input) = prepare_send_agent_message_internal(
            &crate::adaptor::controller::wiring::build_code_usecase(),
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(session.id.clone()),
            worktree_path.clone(),
            "/status".to_string(),
            crate::permission::PermissionMode::Edit,
            false,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(response.agent_message.is_none());
        assert!(response.queued_turn.is_none());
        assert!(response.pending_queue.is_empty());
        let steer = expect_prepared_steer(prepared_input.expect("busy send should steer"));
        assert_eq!(steer.session_id, session.id);
        assert_eq!(steer.backend_id, "steer");
        assert_eq!(steer.prompt, "/status");
        assert_eq!(steer.steering_message_id, response.human_message.id);
        let pending_count = handles
            .lock()
            .await
            .get(&session.id)
            .map(|proc| proc.pending_messages.len())
            .unwrap_or_default();
        assert_eq!(pending_count, 0);
    }

    #[tokio::test]
    async fn workflow_step_send_with_stopped_runtime_prepares_single_resume_turn() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock".to_string(),
            models: vec![],
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let worktree_path = "/repo".to_string();
        let mut step_session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock".to_string()),
        )
        .unwrap();
        step_session.workflow_step_session = true;
        step_session.agent_session_id = Some("sdk-session".to_string());
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &step_session)
            .unwrap();

        let (_response, prepared_turn) = prepare_send_agent_message_internal(
            &crate::adaptor::controller::wiring::build_code_usecase(),
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(step_session.id.clone()),
            worktree_path.clone(),
            "resume step".to_string(),
            crate::permission::PermissionMode::Edit,
            false,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let prepared_turn = prepared_turn.expect("stopped workflow step should resume on send");
        let prepared_turn = expect_prepared_turn(prepared_turn);
        assert_eq!(prepared_turn.session_id, step_session.id);
        assert_eq!(prepared_turn.worktree_path, worktree_path);
        assert_eq!(prepared_turn.prompt, "resume step");
        assert!(
            handles.lock().await.is_empty(),
            "preparation must not leave a half-started runtime before turn start"
        );
    }

    #[tokio::test]
    async fn turn_start_requires_existing_session_meta() {
        let data_dir = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(
                data_dir.path().to_path_buf(),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let err = start_agent_turn(
            app.handle(),
            &handles,
            &session_store,
            "missing-turn",
            "/repo",
            "edit",
            false,
            "hello",
            "agent-msg-1",
            &[],
        )
        .await
        .unwrap_err();
        assert_eq!(err, "Session not found: missing-turn");

        let err = start_agent_turn_locked(
            app.handle(),
            &handles,
            &session_store,
            "missing-locked-turn",
            "/repo",
            "edit",
            false,
            "hello",
            "agent-msg-2",
            &[],
        )
        .await
        .unwrap_err();
        assert_eq!(err, "Session not found: missing-locked-turn");
        assert!(handles.lock().await.is_empty());
    }

    #[tokio::test]
    async fn stopped_workflow_step_turn_start_spawns_resume_runtime_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "stopped-step".to_string();
        let spawn_count = Arc::new(AtomicUsize::new(0));

        start_agent_turn_with_runtime_spawner(
            None::<&tauri::AppHandle>,
            None,
            &handles,
            &session_id,
            "edit",
            "resume step",
            "agent-msg-1",
            &[],
            {
                let handles = Arc::clone(&handles);
                let session_id = session_id.clone();
                let spawn_count = Arc::clone(&spawn_count);
                move || async move {
                    spawn_count.fetch_add(1, Ordering::SeqCst);
                    handles
                        .lock()
                        .await
                        .insert(session_id, make_test_agent_process());
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);
        let map = handles.lock().await;
        let proc = map.get(&session_id).expect("runtime was started");
        assert_eq!(proc.state, BridgeState::Streaming);
        assert_eq!(proc.turn_phase, TurnPhase::Streaming);
        assert_eq!(proc.streaming_message_id.as_deref(), Some("agent-msg-1"));
    }

    #[tokio::test]
    async fn running_workflow_step_turn_start_reuses_existing_runtime_without_spawn() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "running-step".to_string();
        handles
            .lock()
            .await
            .insert(session_id.clone(), make_test_agent_process());
        let spawn_count = Arc::new(AtomicUsize::new(0));

        start_agent_turn_with_runtime_spawner(
            None::<&tauri::AppHandle>,
            None,
            &handles,
            &session_id,
            "edit",
            "continue step",
            "agent-msg-2",
            &[],
            {
                let spawn_count = Arc::clone(&spawn_count);
                move || async move {
                    spawn_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(spawn_count.load(Ordering::SeqCst), 0);
        assert_eq!(handles.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn workflow_step_turn_start_holds_session_runtime_lock_until_message_write() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "locked-turn-start".to_string();
        let spawn_count = Arc::new(AtomicUsize::new(0));
        let guard = acquire_session_runtime_lock(&session_id).await;

        let start = {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            tokio::spawn(async move {
                start_agent_turn_with_runtime_spawner(
                    None::<&tauri::AppHandle>,
                    None,
                    &handles,
                    &session_id,
                    "edit",
                    "resume step",
                    "agent-msg-locked",
                    &[],
                    {
                        let handles = Arc::clone(&handles);
                        let session_id = session_id.clone();
                        let spawn_count = Arc::clone(&spawn_count);
                        move || async move {
                            spawn_count.fetch_add(1, Ordering::SeqCst);
                            handles
                                .lock()
                                .await
                                .insert(session_id, make_test_agent_process());
                            Ok(())
                        }
                    },
                )
                .await
            })
        };

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(spawn_count.load(Ordering::SeqCst), 0);
        assert!(handles.lock().await.is_empty());

        drop(guard);
        start.await.unwrap().unwrap();

        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);
        let map = handles.lock().await;
        let proc = map.get(&session_id).expect("runtime was started");
        assert_eq!(
            proc.streaming_message_id.as_deref(),
            Some("agent-msg-locked")
        );
    }

    #[tokio::test]
    async fn concurrent_workflow_step_turn_start_spawns_runtime_at_most_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "concurrent-step".to_string();
        let spawn_count = Arc::new(AtomicUsize::new(0));

        let start_one = {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            async move {
                start_agent_turn_with_runtime_spawner(
                    None::<&tauri::AppHandle>,
                    None,
                    &handles,
                    &session_id,
                    "edit",
                    "first",
                    "agent-msg-1",
                    &[],
                    {
                        let handles = Arc::clone(&handles);
                        let session_id = session_id.clone();
                        let spawn_count = Arc::clone(&spawn_count);
                        move || async move {
                            spawn_count.fetch_add(1, Ordering::SeqCst);
                            handles
                                .lock()
                                .await
                                .insert(session_id, make_test_agent_process());
                            Ok(())
                        }
                    },
                )
                .await
            }
        };
        let start_two = {
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let spawn_count = Arc::clone(&spawn_count);
            async move {
                start_agent_turn_with_runtime_spawner(
                    None::<&tauri::AppHandle>,
                    None,
                    &handles,
                    &session_id,
                    "edit",
                    "second",
                    "agent-msg-2",
                    &[],
                    {
                        let handles = Arc::clone(&handles);
                        let session_id = session_id.clone();
                        let spawn_count = Arc::clone(&spawn_count);
                        move || async move {
                            spawn_count.fetch_add(1, Ordering::SeqCst);
                            handles
                                .lock()
                                .await
                                .insert(session_id, make_test_agent_process());
                            Ok(())
                        }
                    },
                )
                .await
            }
        };

        let (first, second) = tokio::join!(start_one, start_two);

        first.unwrap();
        second.unwrap();
        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);
        assert_eq!(handles.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn get_session_returns_registry_models_without_process() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();

        let mut cfg = crate::adaptor::gateway::app_config::ReleashConfig::default();
        cfg.agents.claude.models = vec!["mock-model".to_string()];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
            cfg,
            tmp.path().to_path_buf(),
        ));

        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: CLAUDE_BACKEND_ID.to_string(),
            models: vec![],
        }));
        registry.set_config(config);
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let response = get_session_internal_with_data_dir(
            &session_store,
            &handles,
            Some(&registry),
            temp.path(),
            &session.id,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(response.available_models.len(), 1);
        assert_eq!(response.available_models[0].model_id, "mock-model");
    }

    #[tokio::test]
    async fn set_session_backend_updates_unstarted_session_and_models() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.selected_model = Some("old-model".to_string());
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let mut cfg = crate::adaptor::gateway::app_config::ReleashConfig::default();
        cfg.agents.claude.models = vec!["a-model".to_string()];
        cfg.agents.codex.models = vec!["b-model".to_string()];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
            cfg,
            tmp.path().to_path_buf(),
        ));

        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: CLAUDE_BACKEND_ID.to_string(),
            models: vec![],
        }));
        registry.register(Arc::new(MockModelBackend {
            backend_id: CODEX_BACKEND_ID.to_string(),
            models: vec![],
        }));
        registry.set_config(config);
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let response = set_session_backend_internal(
            &session_store,
            &registry,
            &handles,
            temp.path(),
            &session.id,
            CODEX_BACKEND_ID.to_string(),
        )
        .await
        .unwrap();

        assert_eq!(
            response.session.backend_id,
            Some(CODEX_BACKEND_ID.to_string())
        );
        // backend 切替後は新 backend の既定モデル（一覧先頭）へ解決される。
        assert_eq!(
            response.session.selected_model,
            Some("codex:b-model".to_string())
        );
        assert_eq!(response.available_models.len(), 1);
        assert_eq!(response.available_models[0].model_id, "b-model");
    }

    #[tokio::test]
    async fn set_session_backend_rejects_session_with_messages() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some("mock-a".to_string()),
        )
        .unwrap();
        add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "hello",
            None,
            None,
        )
        .unwrap();
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-a".to_string(),
            models: Vec::new(),
        }));
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-b".to_string(),
            models: Vec::new(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let result = set_session_backend_internal(
            &session_store,
            &registry,
            &handles,
            temp.path(),
            &session.id,
            "mock-b".to_string(),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn set_session_backend_rejects_session_with_agent_session_id() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some("mock-a".to_string()),
        )
        .unwrap();
        session.agent_session_id = Some("sdk-session".to_string());
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-a".to_string(),
            models: Vec::new(),
        }));
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-b".to_string(),
            models: Vec::new(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let result = set_session_backend_internal(
            &session_store,
            &registry,
            &handles,
            temp.path(),
            &session.id,
            "mock-b".to_string(),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn set_session_backend_rejects_invalid_backend_id() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some("mock-a".to_string()),
        )
        .unwrap();
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-a".to_string(),
            models: Vec::new(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let result = set_session_backend_internal(
            &session_store,
            &registry,
            &handles,
            temp.path(),
            &session.id,
            "missing".to_string(),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn approval_chat_adjustment_send_path_keeps_session_state() {
        let worktree = tempfile::tempdir().unwrap();
        let worktree_path = worktree.path().to_string_lossy().to_string();
        let engine = Arc::new(TestRuntimeKernel::new_for_test());
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock-a".to_string()),
        )
        .unwrap();
        add_message_internal(
            &session_store,
            data_dir.path(),
            &session.id,
            MessageRole::Agent,
            &approved_fix_policy_output("Old policy.", "code_review_parallel"),
            None,
            None,
        )
        .unwrap();
        let before = engine
            .insert_test_approval_execution(
                &worktree_path,
                &session.id,
                WorkflowExecutionState::WaitingApproval,
            )
            .await;

        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-a".to_string(),
            models: Vec::new(),
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut child = tokio::process::Command::new("cat")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        handles.lock().await.insert(
            session.id.clone(),
            AgentProcess {
                stdin,
                backend_id: "mock-a".to_string(),
                state: BridgeState::Ready,
                turn_phase: TurnPhase::Idle,
                sdk_session_id: None,
                context_carry_on_ready: None,
                child,
                generation_id: 0,
                #[cfg(unix)]
                pgid: None,
                streaming_message_id: None,
                streaming_parts: Vec::new(),
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
            },
        );

        let (response, prepared_turn) = prepare_send_agent_message_internal(
            &crate::adaptor::controller::wiring::build_code_usecase(),
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(session.id.clone()),
            worktree_path.clone(),
            "Narrow the policy to reviewed findings.".to_string(),
            crate::permission::PermissionMode::Edit,
            false,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let agent = response.agent_message.unwrap();
        let prepared_turn = prepared_turn.unwrap();
        let prepared_turn = expect_prepared_turn(prepared_turn);
        assert_eq!(prepared_turn.session_id, session.id);
        assert_eq!(
            prepared_turn.prompt,
            "Narrow the policy to reviewed findings."
        );
        {
            let mut map = handles.lock().await;
            let proc = map.get_mut(&session.id).unwrap();
            proc.streaming_parts = vec![MessagePart::Text {
                content: approved_fix_policy_output(
                    "Latest adjusted policy.",
                    "code_review_parallel",
                ),
                parent_tool_use_id: None,
            }];
        }
        {
            let mut saved = session_store
                .load_full_session_for_restore(data_dir.path(), &session.id)
                .unwrap()
                .unwrap();
            let latest_policy =
                approved_fix_policy_output("Latest adjusted policy.", "code_review_parallel");
            let msg = saved
                .messages
                .iter_mut()
                .find(|msg| msg.id == agent.id)
                .unwrap();
            msg.content = latest_policy.clone();
            msg.parts = Some(vec![MessagePart::Text {
                content: latest_policy,
                parent_tool_use_id: None,
            }]);
            session_store
                .save_full_session_for_migration_or_restore(data_dir.path(), &saved)
                .unwrap();
        }

        assert_eq!(response.human_message.role, MessageRole::Human);
        assert_eq!(agent.role, MessageRole::Agent);
        let after_send = engine.get_state(&worktree_path).await.unwrap();
        assert_eq!(after_send.execution_id, before.execution_id);
        assert_eq!(after_send.current_step_name, before.current_step_name);
        assert_eq!(
            after_send.current_session_id.as_deref(),
            Some(session.id.as_str())
        );
        assert_eq!(after_send.state, WorkflowExecutionState::WaitingApproval);

        let saved = session_store
            .load_full_session_for_restore(data_dir.path(), &session.id)
            .unwrap()
            .unwrap();
        let latest_agent = saved
            .messages
            .iter()
            .rev()
            .find(|msg| msg.role == MessageRole::Agent)
            .unwrap();
        // [08] このテストの責務は「send path で session が破壊されず維持されること」のみに限定する。
        // typed structured output の contract 検証（typed な step_outputs 更新の保証）は
        // SubmitOutput を経由する CLI/API 経路でしか発生しないため、本テストでは保証しない。
        // contract 検証経路の回帰テストは domain contract の単体テスト群および
        // SubmitOutput の CLI/API 経路テストで別途カバーする。
        // ここでは「session の最新 Agent メッセージが上書きされて存在すること」のみ確認する。
        assert_eq!(latest_agent.id, agent.id);

        let removed_proc = handles.lock().await.remove(&session.id);
        if let Some(mut proc) = removed_proc {
            let _ = proc.child.kill().await;
        }
    }

    #[test]
    fn ensure_session_backend_selected_saves_default_for_missing_backend_id() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = crate::test_support::build_session_store();
        let session = create_session_internal(&session_store, temp.path(), "/repo", None).unwrap();
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock-default".to_string(),
            models: Vec::new(),
        }));

        let updated =
            ensure_session_backend_selected(&session_store, &registry, temp.path(), session)
                .unwrap();

        assert_eq!(updated.backend_id, Some("mock-default".to_string()));
        let persisted = session_store
            .load_full_session_for_restore(temp.path(), &updated.id)
            .unwrap()
            .unwrap();
        assert_eq!(persisted.backend_id, Some("mock-default".to_string()));
    }

    /// spec issues-1023: workflow step として起動された chat session は free chat
    /// tab bar 上に同格に並ばないため、`init_agent_sessions` の active 候補からも
    /// 除外される。本テストは候補選択 helper を 3 シナリオで検証する:
    /// - 先頭が workflow step でも active にならない（free chat があればそれが active）
    /// - 全てが workflow step の場合は active は None
    /// - 通常 chat のみのときは先頭が active になる
    #[test]
    fn pick_initial_active_session_candidate_excludes_workflow_step_sessions() {
        fn make(id: &str, workflow_step: bool) -> SessionSummary {
            SessionSummary {
                id: id.to_string(),
                worktree_path: "/repo".to_string(),
                state: crate::usecase::agent_session::session::SessionState::Idle,
                created_at: 1.0,
                updated_at: 1.0,
                first_message: String::new(),
                message_count: 0,
                agent_session_id: None,
                context_carry: None,
                permission_mode: "edit".to_string(),
                plan_mode: false,
                permission_profile_id: None,
                backend_id: Some("claude".to_string()),
                workflow_step_session: workflow_step,
                workflow_step_context: None,
            }
        }

        // 先頭が workflow step だが後ろに free chat がある: free chat が active になる
        let sessions = vec![make("step-1", true), make("chat-1", false)];
        let picked = pick_initial_active_session_candidate(&sessions);
        assert_eq!(picked.map(|s| s.id.as_str()), Some("chat-1"));

        // 全て workflow step: active 候補 None
        let only_steps = vec![make("step-1", true), make("step-2", true)];
        assert!(pick_initial_active_session_candidate(&only_steps).is_none());

        // 通常 chat のみ: 先頭が active
        let only_chats = vec![make("chat-1", false), make("chat-2", false)];
        assert_eq!(
            pick_initial_active_session_candidate(&only_chats).map(|s| s.id.as_str()),
            Some("chat-1")
        );

        // 空: None
        assert!(pick_initial_active_session_candidate(&[]).is_none());
    }

    #[tokio::test]
    async fn init_agent_sessions_returns_active_latest_page_without_starting_processes() {
        let temp = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_builder()
            .manage(crate::app_data_dir::TestDataDir(temp.path().to_path_buf()))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "hello",
            None,
            None,
        )
        .unwrap();

        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let open_tabs =
            Arc::new(crate::usecase::agent_session::session::OpenTabRegistry::default());

        let response = init_agent_sessions_internal(
            &app.handle(),
            &session_store,
            &registry,
            &handles,
            &open_tabs,
            "/repo".to_string(),
        )
        .await
        .unwrap();

        assert_eq!(response.sessions.len(), 1);
        let active = response
            .active_session
            .expect("active shell should be returned");
        assert_eq!(active.session.id, session.id);
        assert_eq!(active.session.messages.len(), 1);
        assert_eq!(active.session.messages[0].content, "hello");
        assert_eq!(
            active.initial_page,
            Some(InitialSessionPage {
                next_cursor: None,
                has_more: false,
                total_count: 1,
            })
        );
        assert!(handles.lock().await.is_empty());
    }

    #[tokio::test]
    async fn agent_step_runtime_busy_tracks_streaming_permission_and_pending_message() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_streaming_test_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Idle;
        handles.lock().await.insert("step".to_string(), proc);
        assert!(is_agent_step_runtime_busy(&handles, "step").await);

        {
            let mut map = handles.lock().await;
            let proc = map.get_mut("step").unwrap();
            proc.state = BridgeState::Ready;
            proc.turn_phase = TurnPhase::WaitingPermission;
        }
        assert!(is_agent_step_runtime_busy(&handles, "step").await);

        {
            let mut map = handles.lock().await;
            let proc = map.get_mut("step").unwrap();
            proc.turn_phase = TurnPhase::Idle;
            proc.pending_messages.push_back(PendingMessage {
                id: "queued-1".to_string(),
                content: "next".to_string(),
                created_at: 1.0,
                permission_mode: "edit".to_string(),
                plan_mode: false,
                images: Vec::new(),
                worktree_path: "/repo".to_string(),
                mentions: Vec::new(),
                editor_context: None,
                existing_human_message_id: None,
                existing_agent_message_id: None,
            });
        }
        assert!(is_agent_step_runtime_busy(&handles, "step").await);
        assert!(agent_session_has_pending_message(&handles, "step").await);

        {
            let mut map = handles.lock().await;
            let proc = map.get_mut("step").unwrap();
            proc.pending_messages.clear();
        }
        assert!(!is_agent_step_runtime_busy(&handles, "step").await);
        assert!(!agent_session_has_pending_message(&handles, "step").await);
    }

    #[tokio::test]
    async fn pending_turn_starting_keeps_step_runtime_busy_after_pending_is_taken() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.pending_messages.push_back(PendingMessage {
            id: "queued-1".to_string(),
            content: "next".to_string(),
            created_at: 1.0,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        });
        handles
            .lock()
            .await
            .insert("step-pending".to_string(), proc);

        let pending = take_pending_message(&handles, "step-pending").await;

        assert!(pending.is_some());
        assert!(!agent_session_has_pending_message(&handles, "step-pending").await);
        assert!(is_agent_step_runtime_busy(&handles, "step-pending").await);

        clear_pending_turn_starting("step-pending").await;
        assert!(!is_agent_step_runtime_busy(&handles, "step-pending").await);
    }

    #[tokio::test]
    async fn pending_messages_are_consumed_fifo_without_overwriting_later_turns() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.pending_messages.push_back(PendingMessage {
            id: "queued-1".to_string(),
            content: "first".to_string(),
            created_at: 1.0,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        });
        proc.pending_messages.push_back(PendingMessage {
            id: "queued-2".to_string(),
            content: "second".to_string(),
            created_at: 2.0,
            permission_mode: "ask".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        });
        handles.lock().await.insert("queued".to_string(), proc);

        let first = take_pending_message(&handles, "queued").await.unwrap();
        assert_eq!(first.content, "first");
        assert_eq!(first.permission_mode, "edit");
        assert!(agent_session_has_pending_message(&handles, "queued").await);

        clear_pending_turn_starting("queued").await;
        let second = take_pending_message(&handles, "queued").await.unwrap();
        assert_eq!(second.content, "second");
        assert_eq!(second.permission_mode, "ask");
        assert!(!agent_session_has_pending_message(&handles, "queued").await);

        clear_pending_turn_starting("queued").await;
    }

    #[tokio::test]
    async fn cancel_agent_queued_turn_removes_only_requested_pending_turn() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.pending_messages.push_back(PendingMessage {
            id: "keep".to_string(),
            content: "first".to_string(),
            created_at: 1.0,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        });
        proc.pending_messages.push_back(PendingMessage {
            id: "drop".to_string(),
            content: "second".to_string(),
            created_at: 2.0,
            permission_mode: "ask".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        });
        handles.lock().await.insert("queued".to_string(), proc);

        let response = cancel_agent_queued_turn_internal(&handles, "queued", Some("drop"))
            .await
            .unwrap();

        assert_eq!(response.canceled_count, 1);
        assert_eq!(response.pending_queue_count, 1);
        assert_eq!(response.pending_queue[0].id, "keep");
        assert_eq!(response.pending_queue[0].content_preview, "first");
    }

    #[test]
    fn pending_turn_start_failure_log_is_redacted() {
        let log_line = pending_turn_start_failed_log_message();

        assert!(log_line.contains("code=pending_turn_start_failed"));
        assert!(log_line.contains("message=failed_to_start_pending_turn"));
        assert!(!log_line.contains("agent-session-secret"));
        assert!(!log_line.contains("queued message body"));
        assert!(!log_line.contains("/private/worktree/path"));
    }

    #[tokio::test]
    async fn cleanup_then_pending_resume_leaves_one_runtime_without_double_spawn() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "step-cleanup-resume".to_string();
        handles
            .lock()
            .await
            .insert(session_id.clone(), make_test_agent_process());
        mark_pending_turn_starting(&session_id).await;
        let close_count = Arc::new(AtomicUsize::new(0));
        let spawn_count = Arc::new(AtomicUsize::new(0));

        {
            let _guard = acquire_session_runtime_lock(&session_id).await;
            if handles.lock().await.remove(&session_id).is_some() {
                close_count.fetch_add(1, Ordering::SeqCst);
            }
        }
        {
            let _guard = acquire_session_runtime_lock(&session_id).await;
            ensure_runtime_for_turn(&handles, &session_id, {
                let handles = Arc::clone(&handles);
                let session_id = session_id.clone();
                let spawn_count = Arc::clone(&spawn_count);
                move || async move {
                    spawn_count.fetch_add(1, Ordering::SeqCst);
                    handles
                        .lock()
                        .await
                        .insert(session_id, make_test_agent_process());
                    Ok(())
                }
            })
            .await
            .unwrap();
        }
        clear_pending_turn_starting(&session_id).await;

        assert_eq!(close_count.load(Ordering::SeqCst), 1);
        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);
        assert!(handles.lock().await.contains_key(&session_id));
    }

    #[tokio::test]
    async fn session_runtime_lock_is_pruned_after_last_guard_drops() {
        {
            let _guard = acquire_session_runtime_lock("lock-prune-test").await;
            assert!(
                crate::infrastructure::agent_session::runtime::runtime_coordinator::session_runtime_lock_exists(
                    "lock-prune-test"
                )
                .await
            );
        }

        for _ in 0..10 {
            if !crate::infrastructure::agent_session::runtime::runtime_coordinator::session_runtime_lock_exists("lock-prune-test")
                .await
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("runtime lock was not pruned after guard drop");
    }

    #[tokio::test]
    async fn session_runtime_lock_serializes_same_step_operations() {
        let guard = acquire_session_runtime_lock("same-step-lock-test").await;
        let waiter = tokio::spawn(async {
            let _guard = acquire_session_runtime_lock("same-step-lock-test").await;
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!waiter.is_finished());
        drop(guard);
        waiter.await.unwrap();
    }

    #[tokio::test]
    async fn spawn_session_guard_serializes_same_step_spawns() {
        let guard = acquire_spawn_session_guard("same-step-spawn-test").await;
        let waiter = tokio::spawn(async {
            let _guard = acquire_spawn_session_guard("same-step-spawn-test").await;
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!waiter.is_finished());
        drop(guard);
        waiter.await.unwrap();
    }

    #[tokio::test]
    async fn closing_session_marker_blocks_until_close_finishes() {
        mark_session_closing("same-step-close-test").await;
        mark_session_closing("same-step-close-test").await;
        let waiter = tokio::spawn(async {
            wait_until_session_close_finished("same-step-close-test").await;
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!waiter.is_finished());
        clear_session_closing("same-step-close-test").await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!waiter.is_finished());
        clear_session_closing("same-step-close-test").await;
        waiter.await.unwrap();
    }

    #[test]
    fn init_command_format() {
        // 抽象モード "edit" を Claude バックエンド向けに変換すると acceptEdits になる。
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &Some("sess-abc".to_string()),
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(cmd["type"], "init");
        assert_eq!(cmd["cwd"], "/repo");
        assert_eq!(cmd["permissionMode"], "acceptEdits");
        assert_eq!(cmd["sessionId"], "sess-abc");
    }

    #[test]
    fn set_mode_command_format() {
        let data =
            build_set_mode_command_for_backend("full", CLAUDE_BACKEND_ID).expect("valid mode");
        let cmd: serde_json::Value = serde_json::from_str(data.trim()).unwrap();
        assert_eq!(cmd["type"], "setMode");
        assert_eq!(cmd["permissionMode"], "bypassPermissions");
    }

    #[test]
    fn set_model_command_format() {
        let result = build_set_model_command("claude-opus");
        let cmd: serde_json::Value = serde_json::from_str(result.trim()).unwrap();
        assert_eq!(cmd["type"], "setModel");
        assert_eq!(cmd["modelId"], "claude-opus");
    }

    #[test]
    fn init_command_without_session_id() {
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert!(cmd["sessionId"].is_null());
    }

    #[test]
    fn message_command_format() {
        let prompt = "Hello, agent!";
        let cmd = serde_json::json!({
            "type": "message",
            "prompt": prompt,
        });
        assert_eq!(cmd["type"], "message");
        assert_eq!(cmd["prompt"], "Hello, agent!");
    }

    #[test]
    fn dev_bridge_path_points_to_src_tauri_resources() {
        let path = dev_bridge_path(CLAUDE_BACKEND_ID).unwrap();
        assert!(
            path.ends_with("src-tauri/resources/claude-sdk-bridge.mjs"),
            "dev_bridge_path should end with src-tauri/resources/claude-sdk-bridge.mjs, got: {}",
            path.display()
        );
    }

    #[test]
    fn dev_bridge_path_file_exists() {
        let path = dev_bridge_path(CLAUDE_BACKEND_ID).unwrap();
        assert!(
            path.exists(),
            "Bridge script should exist at {}, but it does not",
            path.display()
        );
    }

    #[test]
    fn dev_bridge_path_rejects_codex_legacy_bridge() {
        let err = dev_bridge_path(CODEX_BACKEND_ID).unwrap_err();
        assert!(err.contains("app-server"));
    }

    #[test]
    fn bridge_script_names_returns_claude_bridge_only() {
        assert_eq!(
            bridge_script_names(CLAUDE_BACKEND_ID).unwrap(),
            (
                "claude-sdk-bridge.mjs",
                "generated/bridges/claude-sdk-bridge.bundled.mjs"
            )
        );
        assert!(bridge_script_names(CODEX_BACKEND_ID).is_err());
    }

    #[tokio::test]
    async fn permission_request_message_is_parseable() {
        let json_str = r#"{"type":"permission_request","request_id":"abc-123","tool_name":"Edit","input":{},"tool_use_id":"toolu_001"}"#;
        let msg: serde_json::Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            msg.get("type").and_then(|v| v.as_str()),
            Some("permission_request")
        );
        assert_eq!(
            msg.get("request_id").and_then(|v| v.as_str()),
            Some("abc-123")
        );
        assert_eq!(msg.get("tool_name").and_then(|v| v.as_str()), Some("Edit"));
    }

    #[test]
    fn permission_response_payload_format() {
        let request_id = "req-123";
        let behavior = "allow";
        let message: Option<String> = None;
        let mut result = serde_json::json!({ "behavior": behavior });
        if let Some(msg) = &message {
            result["message"] = serde_json::Value::String(msg.clone());
        }
        let payload = serde_json::json!({
            "type": "permission_response",
            "request_id": request_id,
            "result": result,
        });
        assert_eq!(payload["type"], "permission_response");
        assert_eq!(payload["request_id"], "req-123");
        assert_eq!(payload["result"]["behavior"], "allow");
        assert!(payload["result"].get("message").is_none());
    }

    #[test]
    fn permission_response_payload_with_updated_input() {
        let request_id = "req-789";
        let behavior = "allow";
        let message: Option<String> = None;
        let updated_input = Some(r#"{"questions":[],"answers":{"Q":"A"}}"#.to_string());
        let mut result = serde_json::json!({ "behavior": behavior });
        if let Some(msg) = &message {
            result["message"] = serde_json::Value::String(msg.clone());
        }
        if let Some(input_json) = &updated_input {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(input_json) {
                result["updatedInput"] = parsed;
            }
        }
        let payload = serde_json::json!({
            "type": "permission_response",
            "request_id": request_id,
            "result": result,
        });
        assert_eq!(payload["result"]["behavior"], "allow");
        assert_eq!(payload["result"]["updatedInput"]["answers"]["Q"], "A");
        assert!(payload["result"].get("message").is_none());
    }

    #[test]
    fn behavior_validation_rejects_invalid_values() {
        let valid = ["allow", "deny"];
        let invalid = ["Allow", "ALLOW", "reject", "", "maybe"];
        for v in valid {
            assert!(v == "allow" || v == "deny");
        }
        for v in invalid {
            assert!(v != "allow" && v != "deny");
        }
    }

    #[test]
    fn permission_response_payload_with_deny_message() {
        let request_id = "req-456";
        let behavior = "deny";
        let message = Some("User denied".to_string());
        let mut result = serde_json::json!({ "behavior": behavior });
        if let Some(msg) = &message {
            result["message"] = serde_json::Value::String(msg.clone());
        }
        let payload = serde_json::json!({
            "type": "permission_response",
            "request_id": request_id,
            "result": result,
        });
        assert_eq!(payload["result"]["behavior"], "deny");
        assert_eq!(payload["result"]["message"], "User denied");
    }

    #[tokio::test]
    async fn node_subprocess_stdout_is_readable_as_ndjson() {
        let mock_script = r#"
            process.stdout.write(JSON.stringify({type:"system",session_id:"test-sid"}) + "\n");
            process.stdout.write(JSON.stringify({type:"stream_event",event:{type:"content_block_delta",delta:{type:"text_delta",text:"hello"}}}) + "\n");
            process.stdout.write(JSON.stringify({type:"result",subtype:"success",session_id:"test-sid"}) + "\n");
        "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(mock_script)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();

        let mut messages: Vec<serde_json::Value> = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty() {
                continue;
            }
            let msg: serde_json::Value =
                serde_json::from_str(&line).unwrap_or_else(|_| panic!("Failed to parse: {line}"));
            messages.push(msg);
        }

        let status = child.wait().await.unwrap();
        assert!(status.success(), "node process should exit 0");
        assert_eq!(messages.len(), 3, "Should have 3 messages");

        assert_eq!(
            messages[0].get("session_id").and_then(|v| v.as_str()),
            Some("test-sid")
        );

        let event = &messages[1]["event"];
        assert_eq!(event["type"].as_str(), Some("content_block_delta"));
        assert_eq!(event["delta"]["type"].as_str(), Some("text_delta"));
        assert_eq!(event["delta"]["text"].as_str(), Some("hello"));

        assert_eq!(messages[2]["type"].as_str(), Some("result"));
        assert_eq!(messages[2]["subtype"].as_str(), Some("success"));
    }

    #[tokio::test]
    async fn bridge_stdin_command_protocol_roundtrip() {
        use tokio::io::AsyncWriteExt;

        // Simulate the bridge's stdin protocol: init → message handling
        // Uses an inline script that mirrors the bridge's command parsing.
        let test_script = r#"
            let stdinBuffer = "";
            const commands = [];
            process.stdin.setEncoding("utf8");
            process.stdin.on("data", (chunk) => {
                stdinBuffer += chunk;
                const lines = stdinBuffer.split("\n");
                stdinBuffer = lines.pop();
                for (const line of lines) {
                    if (!line.trim()) continue;
                    try {
                        commands.push(JSON.parse(line));
                    } catch {}
                }
            });
            process.stdin.on("end", () => {
                process.stdout.write(JSON.stringify({ received: commands }) + "\n");
            });
        "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(test_script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        // Send init and message commands
        let init_cmd =
            serde_json::json!({"type": "init", "cwd": "/tmp", "permissionMode": "acceptEdits"});
        let msg_cmd = serde_json::json!({"type": "message", "prompt": "hello"});
        let close_cmd = serde_json::json!({"type": "close"});

        stdin
            .write_all(format!("{}\n{}\n{}\n", init_cmd, msg_cmd, close_cmd).as_bytes())
            .await
            .unwrap();
        drop(stdin); // Close stdin to trigger "end" event

        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();
        let line = tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
            .await
            .expect("Timeout")
            .unwrap()
            .unwrap();

        let result: serde_json::Value = serde_json::from_str(&line).unwrap();
        let received = result["received"].as_array().unwrap();
        assert_eq!(received.len(), 3);
        assert_eq!(received[0]["type"], "init");
        assert_eq!(received[1]["type"], "message");
        assert_eq!(received[1]["prompt"], "hello");
        assert_eq!(received[2]["type"], "close");

        let status = child.wait().await.unwrap();
        assert!(status.success());
    }

    #[tokio::test]
    async fn bridge_sets_can_use_tool_for_interactive_tools_in_accept_edits_mode() {
        let test_script = r#"
            const permissionMode = "acceptEdits";
            const INTERACTIVE_TOOLS = ["AskUserQuestion", "EnterPlanMode", "ExitPlanMode"];
            let canUseToolSet = false;

            if (permissionMode !== "bypassPermissions") {
                canUseToolSet = true;
            }

            const result = {
                permissionMode,
                canUseToolSet,
                interactiveToolsHandled: canUseToolSet,
            };
            process.stdout.write(JSON.stringify(result) + "\n");
        "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(test_script)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();
        let line = lines.next_line().await.unwrap().unwrap();
        let result: serde_json::Value = serde_json::from_str(&line).unwrap();

        let status = child.wait().await.unwrap();
        assert!(status.success());

        assert!(
            result["interactiveToolsHandled"].as_bool().unwrap(),
            "acceptEdits mode should set canUseTool for interactive tools. Result: {}",
            result
        );
    }

    #[tokio::test]
    async fn bridge_sets_can_use_tool_for_interactive_tools_in_plan_mode() {
        let test_script = r#"
            const permissionMode = "plan";
            const INTERACTIVE_TOOLS = ["AskUserQuestion", "EnterPlanMode", "ExitPlanMode"];
            let canUseToolSet = false;

            if (permissionMode !== "bypassPermissions") {
                canUseToolSet = true;
            }

            const result = {
                permissionMode,
                canUseToolSet,
                interactiveToolsHandled: canUseToolSet,
            };
            process.stdout.write(JSON.stringify(result) + "\n");
        "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(test_script)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();
        let line = lines.next_line().await.unwrap().unwrap();
        let result: serde_json::Value = serde_json::from_str(&line).unwrap();

        let status = child.wait().await.unwrap();
        assert!(status.success());

        assert!(
            result["interactiveToolsHandled"].as_bool().unwrap(),
            "plan mode should set canUseTool for interactive tools. Result: {}",
            result
        );
    }

    #[tokio::test]
    async fn bridge_exit_plan_mode_permission_response_roundtrip() {
        use tokio::io::AsyncWriteExt;

        let test_script = r#"
            const pendingPermissions = new Map();

            process.stdin.setEncoding('utf8');
            let buffer = '';
            process.stdin.on('data', (chunk) => {
                buffer += chunk;
                const lines = buffer.split('\n');
                buffer = lines.pop();
                for (const line of lines) {
                    if (!line.trim()) continue;
                    try {
                        const cmd = JSON.parse(line);
                        if (cmd.type === 'permission_response') {
                            const pending = pendingPermissions.get(cmd.request_id);
                            if (pending) {
                                pendingPermissions.delete(cmd.request_id);
                                const result = cmd.result;
                                if (result.behavior === 'allow' && !result.updatedInput) {
                                    result.updatedInput = pending.input;
                                }
                                pending.resolve(result);
                            }
                        }
                    } catch {}
                }
            });

            const requestId = 'req-exit-001';
            const toolInput = {
                allowedPrompts: [{ tool: 'Bash', prompt: 'run tests' }],
                pushToRemote: false,
            };

            const resultPromise = new Promise((resolve) => {
                pendingPermissions.set(requestId, { resolve, input: toolInput });
            });

            process.stdout.write(JSON.stringify({
                type: 'permission_request',
                request_id: requestId,
                tool_name: 'ExitPlanMode',
                input: toolInput,
                tool_use_id: 'toolu_exit_001',
            }) + '\n');

            resultPromise.then((result) => {
                process.stdout.write(JSON.stringify({
                    type: 'canUseTool_resolved',
                    tool_name: 'ExitPlanMode',
                    result: result,
                    result_keys: Object.keys(result).sort(),
                    result_json: JSON.stringify(result),
                }) + '\n');
                process.exit(0);
            });
        "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(test_script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();

        let request_line = lines.next_line().await.unwrap().unwrap();
        let request: serde_json::Value = serde_json::from_str(&request_line).unwrap();
        assert_eq!(request["type"], "permission_request");
        assert_eq!(request["tool_name"], "ExitPlanMode");

        let behavior = "allow";
        let message: Option<String> = None;
        let updated_input: Option<String> = None;
        let mut result = serde_json::json!({ "behavior": behavior });
        if let Some(msg) = &message {
            result["message"] = serde_json::Value::String(msg.clone());
        }
        if let Some(input_json) = &updated_input {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(input_json) {
                result["updatedInput"] = parsed;
            }
        }
        let response = serde_json::json!({
            "type": "permission_response",
            "request_id": request["request_id"].as_str().unwrap(),
            "result": result,
        });
        let data = format!("{}\n", response);
        stdin.write_all(data.as_bytes()).await.unwrap();
        stdin.flush().await.unwrap();

        let resolved_line =
            tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
                .await
                .expect("Timeout waiting for resolved line")
                .unwrap()
                .unwrap();
        let resolved: serde_json::Value = serde_json::from_str(&resolved_line).unwrap();

        assert_eq!(resolved["type"], "canUseTool_resolved");
        assert_eq!(resolved["tool_name"], "ExitPlanMode");

        let can_use_tool_result = &resolved["result"];
        assert_eq!(
            can_use_tool_result["behavior"], "allow",
            "behavior should be 'allow'"
        );
        assert!(
            can_use_tool_result.get("updatedInput").is_some(),
            "updatedInput must be present in allow response (required by CLI Zod schema)"
        );
        assert_eq!(
            can_use_tool_result["updatedInput"]["allowedPrompts"][0]["tool"], "Bash",
            "updatedInput should contain the original tool input"
        );

        let status = child.wait().await.unwrap();
        assert!(status.success());
    }

    #[test]
    fn turn_complete_message_parsing() {
        let msg_str = r#"{"type":"turn_complete","session_id":"sess-123","exit_code":0}"#;
        let msg: serde_json::Value = serde_json::from_str(msg_str).unwrap();
        assert_eq!(msg["type"], "turn_complete");
        assert_eq!(msg["exit_code"], 0);
        assert_eq!(msg["session_id"], "sess-123");
    }

    #[test]
    fn turn_complete_with_error() {
        let msg_str = r#"{"type":"turn_complete","session_id":"sess-123","exit_code":1}"#;
        let msg: serde_json::Value = serde_json::from_str(msg_str).unwrap();
        assert_eq!(msg["exit_code"], 1);
    }

    #[test]
    fn session_ready_message_parsing() {
        let msg_str = r#"{"type":"session_ready","session_id":"sess-456"}"#;
        let msg: serde_json::Value = serde_json::from_str(msg_str).unwrap();
        assert_eq!(msg["type"], "session_ready");
        assert_eq!(msg["session_id"], "sess-456");
    }

    #[test]
    fn parse_skill_frontmatter_valid() {
        let content = "---\nname: review\ndescription: Code review tool\n---\nBody here";
        let (name, desc) = parse_skill_frontmatter(content).unwrap();
        assert_eq!(name, "review");
        assert_eq!(desc, "Code review tool");
    }

    #[test]
    fn parse_skill_frontmatter_missing_fields() {
        let content = "---\ntitle: something\n---\n";
        let (name, desc) = parse_skill_frontmatter(content).unwrap();
        assert_eq!(name, "");
        assert_eq!(desc, "");
    }

    #[test]
    fn parse_skill_frontmatter_no_opening_delimiter() {
        let content = "name: review\n---\n";
        assert!(parse_skill_frontmatter(content).is_none());
    }

    #[test]
    fn parse_skill_frontmatter_empty_content() {
        assert!(parse_skill_frontmatter("").is_none());
    }

    #[test]
    fn scan_agent_skills_switches_directories_by_backend() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let cwd = tmp.path().join("repo");
        let claude_skill = cwd.join(".claude").join("skills").join("claude-review");
        std::fs::create_dir_all(&claude_skill).unwrap();
        std::fs::write(
            claude_skill.join("SKILL.md"),
            "---\nname: claude-review\ndescription: Claude review\n---\nBody",
        )
        .unwrap();
        let codex_skill = cwd.join(".agents").join("skills").join("codex-review");
        std::fs::create_dir_all(&codex_skill).unwrap();
        std::fs::write(
            codex_skill.join("SKILL.md"),
            "---\nname: codex-review\ndescription: Codex review\n---\nBody",
        )
        .unwrap();

        let claude = scan_agent_skills_inner(&cwd, Some(CLAUDE_BACKEND_ID), Some(home.clone()));
        let codex = scan_agent_skills_inner(&cwd, Some(CODEX_BACKEND_ID), Some(home));

        assert!(claude.iter().any(|skill| skill.name == "claude-review"));
        assert!(!claude.iter().any(|skill| skill.name == "codex-review"));
        let codex_skill = codex
            .iter()
            .find(|skill| skill.name == "codex-review")
            .expect("Codex project skill should be included");
        assert_eq!(codex_skill.scope, "project");
        assert!(!codex.iter().any(|skill| skill.name == "claude-review"));
    }

    #[test]
    fn scan_agent_skills_preserves_duplicate_codex_skill_names_across_scopes() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let cwd = tmp.path().join("repo");
        let personal_skill = home.join(".agents").join("skills").join("shared-review");
        std::fs::create_dir_all(&personal_skill).unwrap();
        std::fs::write(
            personal_skill.join("SKILL.md"),
            "---\nname: shared-review\ndescription: Personal review\n---\nBody",
        )
        .unwrap();
        let repo_skill = cwd.join(".agents").join("skills").join("shared-review");
        std::fs::create_dir_all(&repo_skill).unwrap();
        std::fs::write(
            repo_skill.join("SKILL.md"),
            "---\nname: shared-review\ndescription: Repo review\n---\nBody",
        )
        .unwrap();

        let codex = scan_agent_skills_inner(&cwd, Some(CODEX_BACKEND_ID), Some(home));

        let matches = codex
            .iter()
            .filter(|skill| skill.name == "shared-review")
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].description, "Personal review");
        assert_eq!(matches[0].scope, "personal");
        assert_eq!(matches[1].description, "Repo review");
        assert_eq!(matches[1].scope, "project");
    }

    #[tokio::test]
    async fn scan_agent_skills_returns_project_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join(".claude").join("skills").join("reviewer");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: reviewer\ndescription: Review focused changes\n---\nBody",
        )
        .unwrap();

        let result = scan_agent_skills(tmp.path().to_string_lossy().to_string(), None, None, None)
            .await
            .unwrap();

        let skill = result.iter().find(|skill| skill.name == "reviewer");
        assert!(skill.is_some(), "project skill should be included");
        let skill = skill.unwrap();
        assert_eq!(skill.description, "Review focused changes");
        assert_eq!(skill.scope, "project");
    }

    #[test]
    fn scan_agent_skills_filters_by_query_and_limit_in_rust() {
        let skills = vec![
            SkillEntry {
                name: "review".to_string(),
                description: "Review code changes".to_string(),
                scope: "project".to_string(),
            },
            SkillEntry {
                name: "docs".to_string(),
                description: "Write documentation".to_string(),
                scope: "personal".to_string(),
            },
            SkillEntry {
                name: "diagram".to_string(),
                description: "Document architecture diagrams".to_string(),
                scope: "project".to_string(),
            },
        ];

        let result = filter_agent_skills_for_query(skills.clone(), Some("doc"), Some(1));

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "docs");

        let result = filter_agent_skills_for_query(skills, Some("review"), Some(20));

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "review");
    }

    // --- append_to_parts tests ---

    #[test]
    fn test_append_pushes_separate_parts() {
        let mut parts = vec![];
        append_to_parts(&mut parts, "text", "Hello", None);
        append_to_parts(&mut parts, "text", " world", None);
        assert_eq!(parts.len(), 2);
        match &parts[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "Hello"),
            _ => panic!("expected Text"),
        }
        match &parts[1] {
            MessagePart::Text { content, .. } => assert_eq!(content, " world"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_append_no_merge_different_type() {
        let mut parts = vec![];
        append_to_parts(&mut parts, "text", "Hello", None);
        append_to_parts(&mut parts, "thinking", "hmm", None);
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], MessagePart::Text { .. }));
        assert!(matches!(&parts[1], MessagePart::Thinking { .. }));
    }

    #[test]
    fn test_append_no_merge_different_parent() {
        let mut parts = vec![];
        append_to_parts(&mut parts, "text", "main", None);
        append_to_parts(&mut parts, "text", "sub", Some("parent1".to_string()));
        assert_eq!(parts.len(), 2);
    }

    // --- extract_tool_result_content tests ---

    #[test]
    fn test_extract_string_content() {
        let content = serde_json::json!("file contents here");
        assert_eq!(extract_tool_result_content(&content), "file contents here");
    }

    #[test]
    fn test_extract_array_content() {
        let content = serde_json::json!([
            {"type": "text", "text": "line1"},
            {"type": "text", "text": "line2"}
        ]);
        assert_eq!(extract_tool_result_content(&content), "line1\nline2");
    }

    #[test]
    fn test_extract_empty_on_other() {
        let content = serde_json::json!(42);
        assert_eq!(extract_tool_result_content(&content), "");
    }

    // --- accumulate_sdk_message tests ---

    #[test]
    fn post_turn_base_requirement_matches_accumulate_sdk_message() {
        struct Case {
            name: &'static str,
            msg: serde_json::Value,
            initial_parts: Vec<MessagePart>,
            task_id_map: HashMap<String, String>,
            expected: PostTurnBaseRequirement,
        }

        let mut mapped_task_ids = HashMap::new();
        mapped_task_ids.insert("task-1".to_string(), "tool-1".to_string());

        let compaction_in_progress = vec![MessagePart::SystemNotification {
            notification_type: SystemNotificationType::Compaction,
            status: "in_progress".to_string(),
            label: "Compacting conversation...".to_string(),
            detail: None,
            hook_id: None,
        }];

        let cases = vec![
            Case {
                name: "stream_event text_delta",
                msg: serde_json::json!({
                    "type": "stream_event",
                    "event": {
                        "type": "content_block_delta",
                        "delta": {"type": "text_delta", "text": "hello"}
                    }
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "stream_event thinking_delta",
                msg: serde_json::json!({
                    "type": "stream_event",
                    "event": {
                        "type": "content_block_delta",
                        "delta": {"type": "thinking_delta", "thinking": "thinking"}
                    }
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "stream_event unsupported delta",
                msg: serde_json::json!({
                    "type": "stream_event",
                    "event": {
                        "type": "content_block_delta",
                        "delta": {"type": "input_json_delta", "partial_json": "{}"}
                    }
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::NotAccumulated,
            },
            Case {
                name: "assistant tool_use",
                msg: serde_json::json!({
                    "type": "assistant",
                    "message": {
                        "content": [{
                            "type": "tool_use",
                            "name": "Read",
                            "input": {"file_path": "/tmp/file"},
                            "id": "tool-1"
                        }]
                    }
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "assistant TodoWrite snapshot",
                msg: serde_json::json!({
                    "type": "assistant",
                    "message": {
                        "content": [{
                            "type": "tool_use",
                            "name": "TodoWrite",
                            "input": {"todos": [{"content": "ship", "status": "pending"}]},
                            "id": "tool-1"
                        }]
                    }
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "assistant TodoWrite without items",
                msg: serde_json::json!({
                    "type": "assistant",
                    "message": {
                        "content": [{
                            "type": "tool_use",
                            "name": "TodoWrite",
                            "input": {},
                            "id": "tool-1"
                        }]
                    }
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "assistant without tool_use",
                msg: serde_json::json!({
                    "type": "assistant",
                    "message": {"content": [{"type": "text", "text": "ignored"}]}
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "user tool_result",
                msg: post_turn_tool_result_message("tool-1", "done"),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "user without tool_result",
                msg: serde_json::json!({
                    "type": "user",
                    "message": {"content": [{"type": "text", "text": "ignored"}]}
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "todo_list_snapshot with items",
                msg: serde_json::json!({
                    "type": "todo_list_snapshot",
                    "items": [{"text": "ship", "completed": false}]
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "todo_list_snapshot without items",
                msg: serde_json::json!({"type": "todo_list_snapshot"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "permission_denied",
                msg: serde_json::json!({
                    "type": "permission_denied",
                    "tool_name": "Edit",
                    "tool_use_id": "tool-1",
                    "request_id": "req-1"
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "permission_request",
                msg: serde_json::json!({
                    "type": "permission_request",
                    "request_id": "req-1",
                    "tool_name": "Edit"
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system task_started",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "task_started",
                    "tool_use_id": "tool-1",
                    "description": "start"
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system task_notification",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "task_notification",
                    "tool_use_id": "tool-1",
                    "status": "completed"
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system task_progress",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "task_progress",
                    "tool_use_id": "tool-1",
                    "description": "progress"
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system task_updated mapped",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "task_updated",
                    "task_id": "task-1",
                    "patch": {"status": "completed"}
                }),
                initial_parts: vec![],
                task_id_map: mapped_task_ids.clone(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system task_updated without mapping",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "task_updated",
                    "task_id": "missing",
                    "patch": {"status": "completed"}
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system init",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "init",
                    "session_id": "session-1"
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::NotAccumulated,
            },
            Case {
                name: "system compact_boundary new part",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "compact_boundary",
                    "compact_metadata": {"trigger": "manual", "pre_summary_token_count": 10}
                }),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system compact_boundary update",
                msg: serde_json::json!({
                    "type": "system",
                    "subtype": "compact_boundary",
                    "compact_metadata": {"trigger": "auto", "pre_summary_token_count": 20}
                }),
                initial_parts: compaction_in_progress,
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system hook_started",
                msg: serde_json::json!({"type": "system", "subtype": "hook_started"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system hook_progress",
                msg: serde_json::json!({"type": "system", "subtype": "hook_progress"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system hook_response",
                msg: serde_json::json!({"type": "system", "subtype": "hook_response"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system files_persisted",
                msg: serde_json::json!({"type": "system", "subtype": "files_persisted"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system local_command_output",
                msg: serde_json::json!({"type": "system", "subtype": "local_command_output"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system codex_realtime",
                msg: serde_json::json!({"type": "system", "subtype": "codex_realtime"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::AccumulatedWithoutParts,
            },
            Case {
                name: "system status compacting",
                msg: serde_json::json!({"type": "system", "status": "compacting"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::RequiresBase,
            },
            Case {
                name: "system unknown",
                msg: serde_json::json!({"type": "system", "subtype": "unknown"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::NotAccumulated,
            },
            Case {
                name: "error",
                msg: serde_json::json!({"type": "error", "message": "boom"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::NotAccumulated,
            },
            Case {
                name: "unknown type",
                msg: serde_json::json!({"type": "unknown"}),
                initial_parts: vec![],
                task_id_map: HashMap::new(),
                expected: PostTurnBaseRequirement::NotAccumulated,
            },
        ];

        for case in cases {
            let requirement =
                post_turn_base_requirement_for_empty_buffer(&case.msg, &case.task_id_map);
            assert_eq!(requirement, case.expected, "classifier: {}", case.name);

            let mut parts = case.initial_parts.clone();
            let before_parts = parts.clone();
            let mut task_id_map = case.task_id_map.clone();
            let (accumulated, _updated_parts) =
                accumulate_sdk_message(&case.msg, &mut parts, &mut task_id_map);
            let parts_changed = parts != before_parts;
            let expected_shape = match requirement {
                PostTurnBaseRequirement::RequiresBase => (true, true),
                PostTurnBaseRequirement::AccumulatedWithoutParts => (true, false),
                PostTurnBaseRequirement::NotAccumulated => (false, false),
            };
            assert_eq!(
                (accumulated, parts_changed),
                expected_shape,
                "accumulate shape: {}",
                case.name
            );
        }
    }

    #[test]
    fn test_accumulate_text_delta() {
        let msg = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "text_delta", "text": "Hello"}
            }
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "Hello"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_accumulate_thinking_delta() {
        let msg = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "thinking_delta", "thinking": "Let me think"}
            }
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::Thinking { content, .. } => assert_eq!(content, "Let me think"),
            _ => panic!("expected Thinking"),
        }
    }

    #[test]
    fn test_accumulation_liveness_tracks_visible_part_changes() {
        let msg = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "text_delta", "text": "Hello"}
            }
        });
        let mut parts = vec![];
        let accumulation =
            accumulate_sdk_message_with_liveness(&msg, &mut parts, &mut HashMap::new());

        assert!(accumulation.handled);
        assert!(accumulation.liveness);
        assert_eq!(parts.len(), 1);
    }

    #[test]
    fn test_accumulate_tool_use() {
        let msg = serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "name": "Read",
                    "input": {"file_path": "/src/main.rs"},
                    "id": "toolu_001"
                }]
            }
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::ToolUse { tool, id, .. } => {
                assert_eq!(tool, "Read");
                assert_eq!(id, "toolu_001");
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn test_accumulate_todo_snapshot_accepts_empty_items() {
        let msg = serde_json::json!({
            "type": "todo_list_snapshot",
            "items": []
        });
        let mut parts = vec![MessagePart::TodoListSnapshot {
            items: vec![crate::usecase::agent_session::session::TodoListItem {
                text: "old todo".to_string(),
                completed: false,
            }],
        }];

        let (handled, updated) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());

        assert!(handled);
        assert!(updated.is_none());
        let snapshot = parts
            .iter()
            .find_map(|part| match part {
                MessagePart::TodoListSnapshot { items } => Some(items),
                _ => None,
            })
            .expect("snapshot should be present");
        assert!(snapshot.is_empty());
    }

    #[test]
    fn test_extract_todo_items_rejects_missing_or_non_array_items() {
        assert!(extract_todo_items(&serde_json::json!({})).is_none());
        assert!(extract_todo_items(&serde_json::json!({ "items": "not-array" })).is_none());
    }

    #[test]
    fn test_accumulate_tool_result() {
        let msg = serde_json::json!({
            "type": "user",
            "message": {
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_001",
                    "content": "file contents",
                    "is_error": false
                }]
            }
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::ToolResult {
                content,
                is_error,
                tool_use_id,
                ..
            } => {
                assert_eq!(content, "file contents");
                assert!(!is_error);
                assert_eq!(tool_use_id.as_deref(), Some("toolu_001"));
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn test_accumulate_permission_request() {
        let msg = serde_json::json!({
            "type": "permission_request",
            "request_id": "req-1",
            "tool_name": "Edit"
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert_eq!(parts.len(), 1);
        assert!(matches!(&parts[0], MessagePart::Permission { status, .. } if status == "pending"));
    }

    #[test]
    fn test_should_forward_sdk_message() {
        // Non-accumulated (meta events) → always forward
        assert!(should_forward_sdk_message(false, "session_ready"));
        assert!(should_forward_sdk_message(false, "error"));
        // Accumulated → NOT forward (delta emit only)
        assert!(!should_forward_sdk_message(true, "assistant"));
        assert!(!should_forward_sdk_message(true, "stream_event"));
        // permission_request → accumulated=true but still forward
        assert!(should_forward_sdk_message(true, "permission_request"));
    }

    #[test]
    fn supported_commands_from_bridge_message_normalizes_sdk_commands() {
        let msg = serde_json::json!({
            "type": "supported_commands",
            "commands": [
                {
                    "name": "/compact",
                    "description": "Compact context",
                    "argumentHint": "[instructions]"
                },
                {
                    "name": "status",
                    "description": "Show status",
                    "argument_hint": ""
                },
                {
                    "name": "   ",
                    "description": "ignored"
                }
            ]
        });

        let commands = supported_commands_from_bridge_message(&msg);
        assert_eq!(
            commands,
            vec![
                SlashCommandEntry {
                    name: "compact".to_string(),
                    description: "Compact context".to_string(),
                    argument_hint: Some("[instructions]".to_string()),
                },
                SlashCommandEntry {
                    name: "status".to_string(),
                    description: "Show status".to_string(),
                    argument_hint: None,
                },
            ]
        );
    }

    #[test]
    fn test_accumulate_error() {
        let msg = serde_json::json!({
            "type": "error",
            "message": "Something went wrong"
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(!handled);
        assert!(
            parts.is_empty(),
            "error is forwarded to dedicated handlers; empty-buffer post-turn must not persist it"
        );
        let error_part = sdk_error_part_from_message(&msg);
        match error_part {
            MessagePart::Error { content, .. } => assert!(content.contains("Something went wrong")),
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn test_accumulate_task_status() {
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "task_started",
            "tool_use_id": "task1",
            "description": "Searching"
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::TaskStatus {
                task_tool_use_id,
                status,
                description,
                ..
            } => {
                assert_eq!(task_tool_use_id, "task1");
                assert_eq!(status, "started");
                assert_eq!(description.as_deref(), Some("Searching"));
            }
            _ => panic!("expected TaskStatus"),
        }
    }

    #[test]
    fn test_extract_agent_id() {
        // Task tool format: "agentId: <id>"
        assert_eq!(
            extract_agent_id("Async agent launched successfully.\nagentId: a72ca50 (internal ID)"),
            Some("a72ca50")
        );
        assert_eq!(
            extract_agent_id("agentId: abc-123_def"),
            Some("abc-123_def")
        );
        // Bash tool format: "with ID: <id>"
        assert_eq!(
            extract_agent_id(
                "Command running in background with ID: b8625ae. Output is being written to: /tmp/tasks/b8625ae.output"
            ),
            Some("b8625ae")
        );
        assert_eq!(
            extract_agent_id("with ID: task-abc_123"),
            Some("task-abc_123")
        );
        // No match
        assert_eq!(extract_agent_id("no agent id here"), None);
        assert_eq!(extract_agent_id("agentId: "), None);
        assert_eq!(extract_agent_id("with ID: "), None);
    }

    #[test]
    fn test_task_notification_resolves_tool_use_id_from_map() {
        // Step 1: tool_result with agentId populates the map
        let tool_result_msg = serde_json::json!({
            "type": "user",
            "message": {
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_abc123",
                    "content": [{
                        "type": "text",
                        "text": "Async agent launched successfully.\nagentId: task42 (internal ID)"
                    }]
                }]
            }
        });
        let mut parts = vec![];
        let mut task_id_map = HashMap::new();
        accumulate_sdk_message(&tool_result_msg, &mut parts, &mut task_id_map);
        assert_eq!(task_id_map.get("task42"), Some(&"toolu_abc123".to_string()));

        // Step 2: task_notification without tool_use_id resolves from map
        let notification_msg = serde_json::json!({
            "type": "system",
            "subtype": "task_notification",
            "task_id": "task42",
            "status": "completed",
            "summary": "Agent completed"
        });
        accumulate_sdk_message(&notification_msg, &mut parts, &mut task_id_map);
        let task_status = parts
            .iter()
            .find(|p| matches!(p, MessagePart::TaskStatus { status, .. } if status == "completed"))
            .expect("should have a completed TaskStatus");
        match task_status {
            MessagePart::TaskStatus {
                task_tool_use_id, ..
            } => {
                assert_eq!(task_tool_use_id, "toolu_abc123");
            }
            _ => panic!("expected TaskStatus"),
        }
    }

    #[test]
    fn test_task_notification_without_map_entry_stays_empty() {
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "task_notification",
            "task_id": "unknown_task",
            "status": "completed"
        });
        let mut parts = vec![];
        let mut task_id_map = HashMap::new();
        accumulate_sdk_message(&msg, &mut parts, &mut task_id_map);
        match &parts[0] {
            MessagePart::TaskStatus {
                task_tool_use_id, ..
            } => {
                assert_eq!(task_tool_use_id, "");
            }
            _ => panic!("expected TaskStatus"),
        }
    }

    #[test]
    fn test_task_updated_in_place_returns_updated_delta() {
        let mut parts = vec![MessagePart::TaskStatus {
            task_tool_use_id: "toolu_task".to_string(),
            status: "started".to_string(),
            description: Some("Initial".to_string()),
            summary: None,
        }];
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "task_updated",
            "tool_use_id": "toolu_task",
            "patch": {
                "status": "completed",
                "summary": "Done"
            }
        });

        let (handled, updated) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());

        assert!(handled);
        let updated = updated.expect("in-place update should be returned as delta");
        assert_eq!(updated.len(), 1);
        match (&parts[0], &updated[0]) {
            (
                MessagePart::TaskStatus {
                    status,
                    description,
                    summary,
                    ..
                },
                MessagePart::TaskStatus {
                    status: delta_status,
                    description: delta_description,
                    summary: delta_summary,
                    ..
                },
            ) => {
                assert_eq!(status, "completed");
                assert_eq!(description.as_deref(), Some("Initial"));
                assert_eq!(summary.as_deref(), Some("Done"));
                assert_eq!(delta_status, "completed");
                assert_eq!(delta_description.as_deref(), Some("Initial"));
                assert_eq!(delta_summary.as_deref(), Some("Done"));
            }
            _ => panic!("expected TaskStatus"),
        }
    }

    // --- SystemNotification accumulate tests ---

    #[test]
    fn test_accumulate_compaction_start() {
        let msg = serde_json::json!({
            "type": "system",
            "status": "compacting"
        });
        let mut parts = vec![];
        let (handled, updated) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert!(updated.is_none());
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::SystemNotification {
                notification_type,
                status,
                label,
                detail,
                hook_id,
            } => {
                assert_eq!(*notification_type, SystemNotificationType::Compaction);
                assert_eq!(status, "in_progress");
                assert_eq!(label, "Compacting conversation...");
                assert_eq!(*detail, None);
                assert_eq!(*hook_id, None);
            }
            _ => panic!("expected SystemNotification"),
        }
    }

    #[test]
    fn test_accumulate_compaction_complete_updates_existing() {
        let mut parts = vec![MessagePart::SystemNotification {
            notification_type: SystemNotificationType::Compaction,
            status: "in_progress".to_string(),
            label: "Compacting conversation...".to_string(),
            detail: None,
            hook_id: None,
        }];
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "compact_boundary",
            "compact_metadata": {
                "trigger": "auto",
                "pre_summary_token_count": 50000
            }
        });
        let (handled, updated) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert!(updated.is_some());
        let updated_parts = updated.unwrap();
        assert_eq!(updated_parts.len(), 1);
        // Verify the part was updated in-place
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::SystemNotification {
                notification_type,
                status,
                label,
                detail,
                ..
            } => {
                assert_eq!(*notification_type, SystemNotificationType::Compaction);
                assert_eq!(status, "completed");
                assert_eq!(label, "Conversation compacted");
                assert!(detail.as_ref().unwrap().contains("trigger=auto"));
                assert!(detail.as_ref().unwrap().contains("50000 tokens"));
            }
            _ => panic!("expected SystemNotification"),
        }
    }

    #[test]
    fn test_accumulate_compaction_complete_without_start() {
        let mut parts = vec![];
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "compact_boundary",
            "compact_metadata": {
                "trigger": "manual",
                "pre_summary_token_count": 10000
            }
        });
        let (handled, updated) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert!(updated.is_none()); // No existing part to update, new one pushed
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::SystemNotification { status, label, .. } => {
                assert_eq!(status, "completed");
                assert_eq!(label, "Conversation compacted");
            }
            _ => panic!("expected SystemNotification"),
        }
    }

    #[test]
    fn test_accumulate_removed_system_subtypes_are_ignored() {
        for msg in [
            serde_json::json!({
                "type": "system",
                "subtype": "hook_started",
                "hook_name": "SessionEnd",
                "hook_event": "StopSession",
                "hook_id": "hook-001"
            }),
            serde_json::json!({
                "type": "system",
                "subtype": "hook_response",
                "hook_id": "hook-001",
                "outcome": "success",
                "exit_code": 0
            }),
            serde_json::json!({
                "type": "system",
                "subtype": "files_persisted",
                "filePaths": ["CLAUDE.md", "src/main.rs"]
            }),
            serde_json::json!({
                "type": "system",
                "subtype": "local_command_output",
                "content": "npm test output here"
            }),
            serde_json::json!({
                "type": "system",
                "subtype": "codex_realtime",
                "notification_type": "codex_realtime",
                "status": "in_progress",
                "label": "Codex realtime started",
                "detail": "thread=thr_123, version=v2"
            }),
        ] {
            let mut parts = vec![];
            let (handled, updated) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
            assert!(handled);
            assert!(updated.is_none());
            assert!(parts.is_empty());
        }
    }

    #[test]
    fn test_accumulation_liveness_ignores_removed_system_subtypes() {
        for msg in [
            serde_json::json!({
                "type": "system",
                "subtype": "hook_started",
                "hook_name": "SessionEnd",
                "hook_event": "StopSession",
                "hook_id": "hook-001"
            }),
            serde_json::json!({
                "type": "system",
                "subtype": "hook_progress",
                "hook_id": "hook-001",
                "message": "running"
            }),
            serde_json::json!({
                "type": "system",
                "subtype": "hook_response",
                "hook_id": "hook-001",
                "outcome": "success",
                "exit_code": 0
            }),
            serde_json::json!({
                "type": "system",
                "subtype": "files_persisted",
                "filePaths": ["CLAUDE.md", "src/main.rs"]
            }),
            serde_json::json!({
                "type": "system",
                "subtype": "local_command_output",
                "content": "npm test output here"
            }),
            serde_json::json!({
                "type": "system",
                "subtype": "codex_realtime",
                "notification_type": "codex_realtime",
                "status": "in_progress",
                "label": "Codex realtime started",
                "detail": "thread=thr_123, version=v2"
            }),
        ] {
            let mut parts = vec![];
            let accumulation =
                accumulate_sdk_message_with_liveness(&msg, &mut parts, &mut HashMap::new());

            assert!(accumulation.handled);
            assert!(!accumulation.liveness);
            assert!(accumulation.updated_parts.is_none());
            assert!(parts.is_empty());
        }
    }

    #[test]
    fn test_accumulation_liveness_accepts_explicit_progress_notifications() {
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "task_updated",
            "task_id": "task-001",
            "patch": {"status": "progress", "summary": "still running"}
        });
        let mut parts = vec![];
        let accumulation =
            accumulate_sdk_message_with_liveness(&msg, &mut parts, &mut HashMap::new());

        assert!(accumulation.handled);
        assert!(accumulation.liveness);
        assert!(accumulation.updated_parts.is_none());
        assert!(parts.is_empty());
    }

    #[test]
    fn test_accumulate_init_not_handled() {
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "init",
            "session_id": "sess-123"
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(!handled);
        assert!(parts.is_empty());
    }

    #[test]
    fn test_accumulate_permission_mode_status_not_handled() {
        let msg = serde_json::json!({
            "type": "system",
            "permissionMode": "acceptEdits"
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(!handled);
        assert!(parts.is_empty());
    }

    // --- consolidate_parts tests ---

    #[test]
    fn test_consolidate_merges_consecutive_text() {
        let parts = vec![
            MessagePart::Text {
                content: "Hello".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: " world".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let result = consolidate_parts_from_slice(&parts);
        assert_eq!(result.len(), 1);
        match &result[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "Hello world"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_consolidate_no_merge_different_types() {
        let parts = vec![
            MessagePart::Text {
                content: "Hello".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Thinking {
                content: "hmm".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let result = consolidate_parts_from_slice(&parts);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_consolidate_no_merge_different_parent() {
        let parts = vec![
            MessagePart::Text {
                content: "main".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "sub".to_string(),
                parent_tool_use_id: Some("parent1".to_string()),
            },
        ];
        let result = consolidate_parts_from_slice(&parts);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_consolidate_preserves_non_text_parts() {
        let parts = vec![
            MessagePart::Text {
                content: "Hello".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Read".to_string(),
                input: serde_json::json!({}),
                id: "t1".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "World".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let result = consolidate_parts_from_slice(&parts);
        assert_eq!(result.len(), 3);
        assert!(matches!(&result[0], MessagePart::Text { content, .. } if content == "Hello"));
        assert!(matches!(&result[1], MessagePart::ToolUse { .. }));
        assert!(matches!(&result[2], MessagePart::Text { content, .. } if content == "World"));
    }

    #[test]
    fn test_consolidate_merges_multiple_consecutive_chunks() {
        let parts = vec![
            MessagePart::Text {
                content: "a".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "b".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "c".to_string(),
                parent_tool_use_id: None,
            },
        ];
        let result = consolidate_parts_from_slice(&parts);
        assert_eq!(result.len(), 1);
        match &result[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "abc"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn tool_result_append_update_returns_delta_while_cumulative_stays_full() {
        let mut parts = vec![MessagePart::ToolResult {
            content: "hello".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
        }];

        let delta = push_or_update_tool_result(
            &mut parts,
            " world".to_string(),
            false,
            Some("tool-1".to_string()),
            None,
        )
        .expect("existing tool result should return a delta marker");

        assert!(matches!(
            &parts[0],
            MessagePart::ToolResult { content, .. } if content == "hello world"
        ));
        assert!(matches!(
            delta,
            MessagePart::ToolResult { content, .. } if content == " world"
        ));
        assert_eq!(consolidate_parts_from_slice(&parts), parts);
    }

    #[test]
    fn workflow_final_text_parts_extracts_only_text_in_order() {
        let parts = vec![
            MessagePart::Text {
                content: "one".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolResult {
                content: "ignored".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "two".to_string(),
                parent_tool_use_id: Some("tool-1".to_string()),
            },
        ];

        assert_eq!(
            workflow_final_text_parts(&parts),
            vec!["one".to_string(), "two".to_string()]
        );
    }

    #[test]
    fn claude_flag_round_trip_via_permission_flags_module() {
        use crate::infrastructure::agent_session::runtime::permission_flags::{
            claude_flag_from_mode, mode_from_claude_flag,
        };
        use crate::permission::PermissionMode;
        for (abstract_mode, expected_flag) in [
            (PermissionMode::Ask, "default"),
            (PermissionMode::Edit, "acceptEdits"),
            (PermissionMode::Full, "bypassPermissions"),
        ] {
            assert_eq!(claude_flag_from_mode(abstract_mode), expected_flag);
            assert_eq!(mode_from_claude_flag(expected_flag), Some(abstract_mode));
        }
        // "plan" は廃止語彙のため抽象モードに戻せない（None）。
        assert!(mode_from_claude_flag("plan").is_none());
    }

    // --- Image attachment tests ---

    #[test]
    fn detect_image_mime_jpeg() {
        let bytes = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(detect_image_mime(&bytes), Some("image/jpeg"));
    }

    #[test]
    fn detect_image_mime_png() {
        let bytes = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_image_mime(&bytes), Some("image/png"));
    }

    #[test]
    fn detect_image_mime_gif() {
        let bytes = [0x47, 0x49, 0x46, 0x38, 0x39, 0x61];
        assert_eq!(detect_image_mime(&bytes), Some("image/gif"));
    }

    #[test]
    fn detect_image_mime_webp() {
        let bytes = [
            0x52, 0x49, 0x46, 0x46, // RIFF
            0x00, 0x00, 0x00, 0x00, // size
            0x57, 0x45, 0x42, 0x50, // WEBP
        ];
        assert_eq!(detect_image_mime(&bytes), Some("image/webp"));
    }

    #[test]
    fn detect_image_mime_unknown() {
        let bytes = [0x00, 0x01, 0x02, 0x03];
        assert_eq!(detect_image_mime(&bytes), None);
    }

    #[test]
    fn detect_image_mime_too_short() {
        let bytes = [0xFF, 0xD8];
        assert_eq!(detect_image_mime(&bytes), None);
    }

    #[test]
    fn validate_and_encode_image_jpeg() {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
        bytes.extend_from_slice(&[0x00; 100]); // pad
        let result = validate_and_encode_image(&bytes).unwrap();
        assert_eq!(result.media_type, "image/jpeg");
        assert!(!result.data.is_empty());
    }

    #[test]
    fn validate_and_encode_image_png() {
        let mut bytes = vec![0x89, 0x50, 0x4E, 0x47];
        bytes.extend_from_slice(&[0x00; 100]);
        let result = validate_and_encode_image(&bytes).unwrap();
        assert_eq!(result.media_type, "image/png");
    }

    #[test]
    fn validate_and_encode_image_rejects_unknown() {
        let bytes = vec![0x00, 0x01, 0x02, 0x03, 0x04];
        let result = validate_and_encode_image(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported"));
    }

    #[test]
    fn prepare_image_attachment_empty_data() {
        let result = prepare_image_attachment(vec![]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty"));
    }

    #[test]
    fn prepare_image_attachment_valid_png() {
        let mut bytes = vec![0x89, 0x50, 0x4E, 0x47];
        bytes.extend_from_slice(&[0x00; 100]);
        let result = prepare_image_attachment(bytes).unwrap();
        assert_eq!(result.media_type, "image/png");
    }

    #[test]
    fn prepare_image_attachment_rejects_text_file() {
        let bytes = b"Hello, world!".to_vec();
        let result = prepare_image_attachment(bytes);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn prepare_image_attachments_from_paths_valid_png() {
        let dir = tempfile::tempdir().unwrap();
        let png_path = dir.path().join("test.png");
        let mut png_bytes = vec![0x89, 0x50, 0x4E, 0x47];
        png_bytes.extend_from_slice(&[0x00; 100]);
        tokio::fs::write(&png_path, &png_bytes).await.unwrap();

        let result =
            prepare_image_attachments_from_paths(vec![png_path.to_string_lossy().to_string()])
                .await
                .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].media_type, "image/png");
    }

    #[tokio::test]
    async fn prepare_image_attachments_from_paths_skips_non_image() {
        let dir = tempfile::tempdir().unwrap();
        let txt_path = dir.path().join("readme.txt");
        tokio::fs::write(&txt_path, b"Hello, world!").await.unwrap();

        let result =
            prepare_image_attachments_from_paths(vec![txt_path.to_string_lossy().to_string()])
                .await
                .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn prepare_image_attachments_from_paths_skips_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let empty_path = dir.path().join("empty.png");
        tokio::fs::write(&empty_path, b"").await.unwrap();

        let result =
            prepare_image_attachments_from_paths(vec![empty_path.to_string_lossy().to_string()])
                .await
                .unwrap();
        assert!(result.is_empty());
    }

    /// spec issues-1022 "Agent process environment contract": user system_prompt が
    /// 未指定でも、Releash CLI の long help が必ず注入されること。
    #[test]
    fn compose_system_prompt_none_returns_only_cli_help() {
        let composed = super::compose_system_prompt(None).expect("must return Some");
        let help = crate::cli::render_long_help();
        assert_eq!(composed, help);
    }

    /// user system_prompt 指定時は、user prompt の後ろに CLI help を append する。
    #[test]
    fn compose_system_prompt_some_appends_cli_help() {
        let composed = super::compose_system_prompt(Some("user prompt".to_string()))
            .expect("must return Some");
        let help = crate::cli::render_long_help();
        assert!(
            composed.starts_with("user prompt\n\n"),
            "composed must start with user prompt: {composed}"
        );
        assert!(
            composed.ends_with(help),
            "composed must end with CLI help: {composed}"
        );
    }

    /// 空文字の user system_prompt は None と同じ扱いとし、CLI help のみを返す。
    #[test]
    fn compose_system_prompt_empty_string_treated_as_none() {
        let composed = super::compose_system_prompt(Some(String::new())).expect("must return Some");
        let help = crate::cli::render_long_help();
        assert_eq!(composed, help);
    }

    #[test]
    fn build_init_cmd_without_system_prompt_for_claude() {
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(cmd["type"], "init");
        assert_eq!(cmd["cwd"], "/repo");
        assert_eq!(cmd["permissionMode"], "acceptEdits");
        assert!(cmd["sessionId"].is_null());
        assert!(cmd.get("systemPrompt").is_none());
    }

    #[test]
    fn build_init_cmd_includes_model_when_selected() {
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions {
                selected_model: Some("claude-sonnet-4-5"),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(cmd["model"], "claude-sonnet-4-5");
    }

    #[test]
    fn build_init_cmd_omits_model_when_unset() {
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();

        assert!(cmd.get("model").is_none());
    }

    #[test]
    fn build_init_cmd_with_system_prompt_for_claude() {
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &Some("prev-session".to_string()),
            CLAUDE_BACKEND_ID,
            BridgeInitOptions {
                system_prompt: Some("You are a coder.".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(cmd["type"], "init");
        assert_eq!(cmd["cwd"], "/repo");
        assert_eq!(cmd["permissionMode"], "acceptEdits");
        assert_eq!(cmd["sessionId"], "prev-session");
        assert_eq!(cmd["systemPrompt"], "You are a coder.");
    }

    #[test]
    fn build_init_cmd_includes_restore_context_for_reinjection() {
        let payload = RestoreContextPayload {
            prompt_prefix: "restored prefix".to_string(),
        };
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions {
                restore_context: Some(&payload),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(cmd["sessionId"].is_null());
        assert_eq!(cmd["restoreContext"]["promptPrefix"], "restored prefix");
        assert!(cmd["restoreContext"].get("messages").is_none());
    }

    #[test]
    fn build_init_cmd_omits_empty_restore_context_prefix() {
        let payload = RestoreContextPayload {
            prompt_prefix: "  ".to_string(),
        };
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions {
                restore_context: Some(&payload),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(cmd.get("restoreContext").is_none());
    }

    #[test]
    fn build_init_cmd_full_for_claude_emits_bypass_permissions() {
        let cmd = build_init_cmd(
            "/repo",
            "full",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(cmd["permissionMode"], "bypassPermissions");
        assert!(cmd.get("systemPrompt").is_none());
    }

    #[test]
    fn build_init_cmd_ask_for_claude_emits_default() {
        let cmd = build_init_cmd(
            "/repo",
            "ask",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(cmd["permissionMode"], "default");
    }

    #[test]
    fn build_init_cmd_for_codex_emits_sandbox_and_approval() {
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CODEX_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(cmd["type"], "init");
        assert_eq!(cmd["sandboxMode"], "workspace-write");
        assert_eq!(cmd["approvalPolicy"], "on-request");
        // Codex 用 init には permissionMode は載らない（バックエンド固有フラグのみ）
        assert!(cmd.get("permissionMode").is_none());
    }

    #[test]
    fn build_init_cmd_for_codex_ask_and_full() {
        let ask = build_init_cmd(
            "/repo",
            "ask",
            false,
            &None,
            CODEX_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(ask["sandboxMode"], "read-only");
        assert_eq!(ask["approvalPolicy"], "on-request");
        let full = build_init_cmd(
            "/repo",
            "full",
            false,
            &None,
            CODEX_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(full["sandboxMode"], "danger-full-access");
        assert_eq!(full["approvalPolicy"], "never");
    }

    #[test]
    fn build_init_cmd_rejects_invalid_abstract_mode() {
        assert!(
            build_init_cmd(
                "/repo",
                "acceptEdits",
                false,
                &None,
                CLAUDE_BACKEND_ID,
                BridgeInitOptions::default()
            )
            .is_err(),
            "legacy claude flag must be rejected at the boundary"
        );
        assert!(build_init_cmd(
            "/repo",
            "plan",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default()
        )
        .is_err());
        assert!(build_init_cmd(
            "/repo",
            "",
            false,
            &None,
            CODEX_BACKEND_ID,
            BridgeInitOptions::default()
        )
        .is_err());
    }

    /// spawn_bridge_process が spawn 前にパーミッションモードを検証することの担保。
    /// 本テストは spawn 前の `PermissionMode::parse` 早期 return を直接利用するためのスモークテスト。
    #[test]
    fn pre_spawn_permission_validation_smoke() {
        // 抽象モード以外は早期に弾かれる契約を確認する。
        for invalid in ["acceptEdits", "bypassPermissions", "plan", "default", ""] {
            assert!(
                crate::permission::PermissionMode::parse(invalid).is_err(),
                "spawn 前の検証は '{invalid}' を弾く必要がある"
            );
        }
        for valid in ["ask", "edit", "full"] {
            assert!(crate::permission::PermissionMode::parse(valid).is_ok());
        }
    }

    #[test]
    fn build_set_mode_command_emits_claude_flag() {
        let data =
            build_set_mode_command_for_backend("edit", CLAUDE_BACKEND_ID).expect("valid mode");
        let cmd: serde_json::Value = serde_json::from_str(data.trim()).unwrap();
        assert_eq!(cmd["type"], "setMode");
        assert_eq!(cmd["permissionMode"], "acceptEdits");
        assert!(cmd.get("approvalPolicy").is_none());
        assert!(cmd.get("sandboxMode").is_none());
    }

    #[test]
    fn build_set_mode_command_emits_codex_flags() {
        let data =
            build_set_mode_command_for_backend("full", CODEX_BACKEND_ID).expect("valid mode");
        let cmd: serde_json::Value = serde_json::from_str(data.trim()).unwrap();
        assert_eq!(cmd["type"], "setMode");
        assert_eq!(cmd["sandboxMode"], "danger-full-access");
        assert_eq!(cmd["approvalPolicy"], "never");
        assert!(cmd.get("permissionMode").is_none());
    }

    #[test]
    fn build_set_mode_command_rejects_legacy_value() {
        for legacy in ["acceptEdits", "bypassPermissions", "plan", "default", ""] {
            assert!(
                build_set_mode_command_for_backend(legacy, CLAUDE_BACKEND_ID).is_err(),
                "legacy '{legacy}' must be rejected"
            );
        }
    }

    fn chat_session_for_permission_test(session_id: &str, permission: &str) -> ChatSession {
        ChatSession {
            id: session_id.to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::usecase::agent_session::session::SessionState::Active,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: permission.to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some("mock".to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
        }
    }

    /// Spec issues-947: bridge stdin への書き込みを観測するために、stdout 側を pipe で
    /// 開いた `cat` を spawn し、process が複製した stdout を返す。`cat` は stdin を
    /// stdout にエコーするので、stdout が空 == bridge への書き込みなし、を観測できる。
    fn make_test_agent_process_with_stdout() -> (AgentProcess, tokio::process::ChildStdout) {
        use std::process::Stdio;
        let mut child = tokio::process::Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn cat test process");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let proc = AgentProcess {
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
            streaming_parts: Vec::new(),
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
        };
        (proc, stdout)
    }

    #[tokio::test]
    async fn sync_pre_turn_settings_does_not_send_set_model() {
        use tokio::io::AsyncReadExt;

        let (mut proc, mut stdout) = make_test_agent_process_with_stdout();
        proc.backend_id = CLAUDE_BACKEND_ID.to_string();
        proc.selected_model = Some("claude-opus".to_string());

        proc.sync_pre_turn_settings("edit")
            .await
            .expect("pre-turn settings must sync");

        let AgentProcess {
            stdin, mut child, ..
        } = proc;
        drop(stdin);

        let mut output = String::new();
        tokio::time::timeout(Duration::from_secs(5), stdout.read_to_string(&mut output))
            .await
            .expect("stdout read must complete")
            .expect("stdout must be readable");
        let _ = child.wait().await;

        let commands = output
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("json command"))
            .collect::<Vec<_>>();

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["type"], "setMode");
        assert_eq!(commands[0]["permissionMode"], "acceptEdits");
        assert!(
            commands
                .iter()
                .all(|cmd| cmd.get("type").and_then(|value| value.as_str()) != Some("setModel")),
            "pre-turn sync must not send setModel: {commands:?}"
        );
    }

    #[tokio::test]
    async fn set_agent_permission_mode_internal_rejects_invalid_without_mutating_state() {
        use tokio::io::AsyncReadExt;

        // Spec issues-947: 外部境界（set_agent_permission_mode 相当）で invalid 値を受けたとき、
        // 保存値・current_permission_mode・bridge stdin のいずれも変化させない。
        // bridge stdin の不変は、stdout を pipe で開いた `cat` を bridge process に見立てて
        // 「invalid を拒否した後で stdin を閉じ、stdout の echo が空である」ことで観測する。
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = chat_session_for_permission_test(&session_id, "edit");
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &session)
            .unwrap();

        let (proc, mut stdout) = make_test_agent_process_with_stdout();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        handles.lock().await.insert(session_id.clone(), proc);

        for invalid in [
            "acceptEdits",
            "bypassPermissions",
            "plan",
            "default",
            "unknown",
            "",
        ] {
            let err = set_agent_permission_mode_internal(
                &session_store,
                &handles,
                data_dir.path(),
                &session_id,
                invalid,
            )
            .await
            .err()
            .unwrap_or_else(|| panic!("invalid '{invalid}' must be rejected"));
            assert!(
                err.contains("ask, edit, full"),
                "invalid '{invalid}' must include allowed list, got: {err}"
            );

            // 保存値が変わらない。
            let saved = session_store
                .load_full_session_for_restore(data_dir.path(), &session_id)
                .unwrap()
                .unwrap();
            assert_eq!(
                saved.permission_mode, "edit",
                "persisted permission_mode must remain unchanged for '{invalid}'"
            );

            // current_permission_mode（ランタイム）も変わらない。
            let map = handles.lock().await;
            let proc = map.get(&session_id).expect("agent process retained");
            assert_eq!(
                proc.current_permission_mode, "edit",
                "current_permission_mode must remain unchanged for '{invalid}'"
            );
        }

        // 全ての invalid 入力を試した後、bridge stdin への書き込みなしを直接観測する。
        // Map から AgentProcess を取り出して child を kill し、stdin を drop することで
        // `cat` が EOF を読み取って終了し、stdout 読み取りが完了する。`cat` は受け取った
        // バイトをそのまま echo するため、stdout が空 == bridge stdin 未書き込み。
        let mut proc = handles.lock().await.remove(&session_id).unwrap();
        let _ = proc.child.kill().await;
        drop(proc.stdin);
        let mut buf = Vec::new();
        stdout
            .read_to_end(&mut buf)
            .await
            .expect("read stdout to EOF");
        assert!(
            buf.is_empty(),
            "no bytes must be written to bridge stdin for invalid permission modes, got: {:?}",
            String::from_utf8_lossy(&buf)
        );
    }

    #[tokio::test]
    async fn set_agent_permission_mode_internal_persists_valid_abstract_mode() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = chat_session_for_permission_test(&session_id, "edit");
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &session)
            .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        set_agent_permission_mode_internal(
            &session_store,
            &handles,
            data_dir.path(),
            &session_id,
            "ask",
        )
        .await
        .expect("valid abstract mode must be accepted");

        let saved = session_store
            .load_full_session_for_restore(data_dir.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.permission_mode, "ask");
    }

    #[tokio::test]
    async fn prepare_send_persists_selected_modes_for_existing_session() {
        // Spec issues-947: 既存セッションに対する送信時にも、検証済み permission_mode が
        // 異なれば ChatSession.permission_mode に書き戻される（保存層が単一の正典）。
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: "mock".to_string(),
            models: vec![],
        }));
        let registry = Arc::new(registry);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let session_id = uuid::Uuid::new_v4().to_string();
        let session = chat_session_for_permission_test(&session_id, "edit");
        session_store
            .save_full_session_for_migration_or_restore(data_dir.path(), &session)
            .unwrap();

        let (response, _prepared_turn) = prepare_send_agent_message_internal(
            &crate::adaptor::controller::wiring::build_code_usecase(),
            &session_store,
            &registry,
            &handles,
            data_dir.path(),
            Some(session_id.clone()),
            "/repo".to_string(),
            "hello".to_string(),
            crate::permission::PermissionMode::Ask,
            true,
            Some("mock".to_string()),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(response.session.permission_mode, "ask");
        assert!(response.session.plan_mode);
        let saved = session_store
            .load_full_session_for_restore(data_dir.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.permission_mode, "ask");
        assert!(saved.plan_mode);
    }

    #[test]
    fn build_message_cmd_text_only() {
        let cmd = build_message_cmd("hello", &[]);
        assert_eq!(cmd["type"], "message");
        assert_eq!(cmd["prompt"], "hello");
        assert!(cmd.get("images").is_none());
    }

    #[test]
    fn build_message_cmd_with_images() {
        let images = vec![ImageAttachment {
            data: "base64data".to_string(),
            media_type: "image/png".to_string(),
        }];
        let cmd = build_message_cmd("check this", &images);
        assert_eq!(cmd["type"], "message");
        assert_eq!(cmd["prompt"], "check this");
        let imgs = cmd["images"].as_array().unwrap();
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0]["data"], "base64data");
        assert_eq!(imgs[0]["mediaType"], "image/png");
    }

    #[cfg(unix)]
    mod process_group_tests {
        use super::*;
        use std::os::unix::process::CommandExt as _;

        /// PID belonging to a process guaranteed not to exist. PIDs on Linux
        /// and macOS are capped well below this value, so `kill(pid, 0)` will
        /// always return ESRCH.
        const DEAD_OWNER_PID: u32 = 999_999_999;

        /// Helper: serialize a PidFileV1 payload to the given path.
        fn write_pid_file_v1(path: &Path, payload: &PidFileV1) {
            std::fs::write(path, serde_json::to_string(payload).unwrap()).unwrap();
        }

        #[test]
        fn save_and_remove_pgid() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();

            save_pgid(app_data_dir, "session-1", 12345).unwrap();

            let pid_file = pids_dir(app_data_dir).join("session-1.pid");
            assert!(pid_file.exists());
            let contents = std::fs::read_to_string(&pid_file).unwrap();
            let parsed: PidFileV1 = serde_json::from_str(&contents).unwrap();
            assert_eq!(parsed.version, 1);
            assert_eq!(parsed.pgid, 12345);
            assert_eq!(parsed.owner_app_pid, std::process::id());

            remove_pgid(app_data_dir, "session-1");
            assert!(!pid_file.exists());
        }

        #[test]
        fn save_pgid_writes_owner_app_pid_and_start_time() {
            // issue #1024: PID files must identify their owning Releash
            // instance so cleanup can distinguish self-orphans from files
            // belonging to a different live instance.
            let tmp = tempfile::tempdir().unwrap();
            save_pgid(tmp.path(), "owner-test", 42_424).unwrap();
            let contents =
                std::fs::read_to_string(pids_dir(tmp.path()).join("owner-test.pid")).unwrap();
            let parsed: PidFileV1 = serde_json::from_str(&contents).unwrap();
            assert_eq!(parsed.owner_app_pid, std::process::id());
            assert!(
                parsed.owner_start_time > 0,
                "owner_start_time should be populated on supported platforms"
            );
            // The recorded start_time must match what we can read back now.
            let live_start = get_process_start_time(std::process::id()).unwrap();
            assert_eq!(parsed.owner_start_time, live_start);
        }

        #[test]
        fn save_pgid_rejects_path_traversal() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();

            assert!(save_pgid(app_data_dir, "../escape", 12345).is_err());
            assert!(save_pgid(app_data_dir, "a/b", 12345).is_err());
            assert!(save_pgid(app_data_dir, "", 12345).is_err());
            assert!(save_pgid(app_data_dir, "valid-session-id", 12345).is_ok());
        }

        #[test]
        fn cleanup_orphan_processes_removes_stale_pid_files() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            // Owner pid is guaranteed dead → cleanup proceeds and removes the
            // file even though the pgid itself doesn't refer to a live group.
            let pid_file = dir.join("stale-session.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid: 999_999_999,
                    owner_app_pid: DEAD_OWNER_PID,
                    owner_start_time: 0,
                },
            );
            assert!(pid_file.exists());

            cleanup_orphan_processes(app_data_dir);

            // PID file should be removed
            assert!(!pid_file.exists());
        }

        #[test]
        fn cleanup_orphan_processes_handles_empty_dir() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            cleanup_orphan_processes(app_data_dir);
        }

        #[test]
        fn cleanup_orphan_processes_handles_no_dir() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path().join("nonexistent");

            cleanup_orphan_processes(&app_data_dir);
        }

        #[test]
        fn cleanup_orphan_processes_ignores_non_pid_files() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            let other_file = dir.join("notes.txt");
            std::fs::write(&other_file, "not a pid").unwrap();

            cleanup_orphan_processes(app_data_dir);

            assert!(other_file.exists());
        }

        /// Spawn a process in a new process group via setsid(), verify it
        /// becomes a process group leader (pgid == pid), then verify that
        /// killpg terminates the entire group.
        #[test]
        fn setsid_creates_new_process_group_leader() {
            use std::process::Command;

            let child = unsafe {
                Command::new("sleep")
                    .arg("999")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .pre_exec(|| {
                        if libc::setsid() == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    })
                    .spawn()
                    .unwrap()
            };

            let pid = child.id() as libc::pid_t;

            // After setsid(), the child's PGID should equal its PID
            let pgid = unsafe { libc::getpgid(pid) };
            assert_eq!(
                pgid, pid,
                "setsid child should be its own process group leader"
            );

            // killpg should successfully terminate the group
            let ret = unsafe { libc::killpg(pid, libc::SIGKILL) };
            assert_eq!(ret, 0, "killpg should succeed");

            // Reap the child
            let mut child = child;
            let _ = child.wait();

            // Verify process is gone
            let alive = unsafe { libc::kill(pid, 0) };
            assert_ne!(alive, 0, "process should be terminated");
        }

        /// Verify killpg kills grandchild processes within the same group.
        #[test]
        fn killpg_kills_grandchild_processes() {
            use std::process::Command;

            // Spawn a shell that itself spawns a grandchild (sleep).
            // Both shell and sleep will be in the new process group.
            let child = unsafe {
                Command::new("sh")
                    .args(["-c", "sleep 999 & wait"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .pre_exec(|| {
                        if libc::setsid() == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    })
                    .spawn()
                    .unwrap()
            };

            let pgid = child.id() as libc::pid_t;

            // Give the grandchild time to spawn
            std::thread::sleep(std::time::Duration::from_millis(200));

            // Kill the entire process group
            let ret = unsafe { libc::killpg(pgid, libc::SIGKILL) };
            assert_eq!(ret, 0, "killpg should succeed");

            // Reap the child
            let mut child = child;
            let _ = child.wait();

            // Verify no processes remain in this group
            std::thread::sleep(std::time::Duration::from_millis(100));
            let group_alive = unsafe { libc::killpg(pgid, 0) };
            assert_ne!(
                group_alive, 0,
                "no processes should remain in the killed group"
            );
        }

        /// Spawn `sleep 999` in its own process group (setsid). Returns the
        /// `Child` and the pgid (== child PID after setsid).
        fn spawn_setsid_sleep() -> (std::process::Child, libc::pid_t) {
            use std::process::Command;
            let child = unsafe {
                Command::new("sleep")
                    .arg("999")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .pre_exec(|| {
                        if libc::setsid() == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    })
                    .spawn()
                    .unwrap()
            };
            let pgid = child.id() as libc::pid_t;
            (child, pgid)
        }

        #[test]
        fn cleanup_orphan_processes_kills_alive_process_group() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            let (mut child, pgid) = spawn_setsid_sleep();

            // Owner is a dead PID, so cleanup treats this as a self-orphan and
            // must terminate the bridge group + delete the file.
            let pid_file = dir.join("alive-session.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid,
                    owner_app_pid: DEAD_OWNER_PID,
                    owner_start_time: 0,
                },
            );

            assert_eq!(
                unsafe { libc::killpg(pgid, 0) },
                0,
                "process group should be alive before cleanup"
            );

            cleanup_orphan_processes(app_data_dir);

            // Reap the child to clear zombie state from process table.
            // cleanup_orphan_processes sends SIGTERM/SIGKILL via killpg, but without
            // wait() the child becomes a zombie and killpg(pgid, 0) still returns 0.
            let _ = child.wait();

            let still_alive = unsafe { libc::killpg(pgid, 0) };
            assert_ne!(
                still_alive, 0,
                "process group should be terminated after cleanup"
            );
            assert!(!pid_file.exists());
        }

        #[test]
        fn cleanup_skips_pid_file_owned_by_live_other_instance() {
            // issue #1024: A PID file whose owner_app_pid points at a live
            // process with matching start_time belongs to a different,
            // currently-running Releash instance. Cleanup must not touch it.
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            // Stand-in for "another Releash instance": use the current test
            // process itself as the owner — it is alive and its start_time
            // matches what get_process_start_time returns.
            let owner_pid = std::process::id();
            let owner_start_time = get_process_start_time(owner_pid).unwrap();

            // Stand-in for the bridge process group owned by that instance.
            let (mut bridge, pgid) = spawn_setsid_sleep();

            let pid_file = dir.join("foreign.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid,
                    owner_app_pid: owner_pid,
                    owner_start_time,
                },
            );

            cleanup_orphan_processes(app_data_dir);

            assert!(
                pid_file.exists(),
                "PID file owned by a live instance must be left in place"
            );
            assert_eq!(
                unsafe { libc::killpg(pgid, 0) },
                0,
                "bridge process group of a live instance must not be killed"
            );

            // Tear down the helper process so it doesn't outlive the test run.
            unsafe {
                libc::killpg(pgid, libc::SIGKILL);
            }
            let _ = bridge.wait();
        }

        #[test]
        fn cleanup_kills_when_owner_pid_was_reused() {
            // owner_app_pid is alive (we point it at ourselves) but the
            // recorded owner_start_time disagrees with reality → PID has been
            // reused. The file is stale and must be cleaned up.
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            let (mut child, pgid) = spawn_setsid_sleep();

            let pid_file = dir.join("reused.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid,
                    owner_app_pid: std::process::id(),
                    owner_start_time: 0, // definitely != real start_time
                },
            );

            cleanup_orphan_processes(app_data_dir);
            let _ = child.wait();

            assert!(!pid_file.exists());
            assert_ne!(
                unsafe { libc::killpg(pgid, 0) },
                0,
                "stale-owner bridge group should be terminated"
            );
        }

        #[test]
        fn cleanup_skips_legacy_numeric_pid_files() {
            // Files written by older builds had no owner info, so cleanup
            // cannot prove the owner is dead. They must be left in place
            // (conservative) — they get either overwritten on next save or
            // cleaned up manually by the developer.
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            let pid_file = dir.join("legacy.pid");
            std::fs::write(&pid_file, "999999999").unwrap();

            cleanup_orphan_processes(app_data_dir);

            assert!(pid_file.exists());
        }

        #[test]
        fn cleanup_orphan_processes_ignores_invalid_pgid_zero() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            // pgid=0 would target the caller's own group — must be rejected.
            let pid_file = dir.join("bad-zero.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid: 0,
                    owner_app_pid: DEAD_OWNER_PID,
                    owner_start_time: 0,
                },
            );

            cleanup_orphan_processes(app_data_dir);

            assert!(!pid_file.exists());
        }

        #[test]
        fn cleanup_orphan_processes_ignores_invalid_pgid_one() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            // pgid=1 (init) — must be rejected.
            let pid_file = dir.join("bad-one.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid: 1,
                    owner_app_pid: DEAD_OWNER_PID,
                    owner_start_time: 0,
                },
            );

            cleanup_orphan_processes(app_data_dir);

            assert!(!pid_file.exists());
        }

        #[test]
        fn cleanup_orphan_processes_ignores_negative_pgid() {
            let tmp = tempfile::tempdir().unwrap();
            let app_data_dir = tmp.path();
            let dir = pids_dir(app_data_dir);
            std::fs::create_dir_all(&dir).unwrap();

            let pid_file = dir.join("bad-negative.pid");
            write_pid_file_v1(
                &pid_file,
                &PidFileV1 {
                    version: 1,
                    pgid: -1,
                    owner_app_pid: DEAD_OWNER_PID,
                    owner_start_time: 0,
                },
            );

            cleanup_orphan_processes(app_data_dir);

            assert!(!pid_file.exists());
        }

        fn make_dummy_agent_process(
            child: tokio::process::Child,
            stdin: tokio::process::ChildStdin,
            pgid: Option<u32>,
        ) -> AgentProcess {
            AgentProcess {
                stdin,
                backend_id: CLAUDE_BACKEND_ID.to_string(),
                state: BridgeState::Initializing,
                turn_phase: TurnPhase::Idle,
                sdk_session_id: None,
                context_carry_on_ready: None,
                child,
                generation_id: 1,
                pgid,
                streaming_message_id: None,
                streaming_parts: Vec::new(),
                last_message_id: None,
                post_turn_base_untrusted_message_id: None,
                task_id_map: HashMap::new(),
                pending_messages: VecDeque::new(),
                current_permission_mode: "ask".to_string(),
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

        #[tokio::test]
        async fn set_active_process_model_updates_selected_model() {
            let mut cmd = tokio::process::Command::new("cat");
            cmd.stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let mut child = cmd.spawn().unwrap();
            let stdin = child.stdin.take().unwrap();
            let pgid = child.id();

            let handles = Arc::new(Mutex::new(HashMap::new()));
            {
                let mut map = handles.lock().await;
                let mut proc = make_dummy_agent_process(child, stdin, pgid);
                proc.available_models = vec![ModelInfo::new(CODEX_BACKEND_ID, "gpt-5.4")];
                proc.selected_model = Some("old-model".to_string());
                map.insert("session-1".to_string(), proc);
            }

            set_active_process_model(&handles, "session-1", "gpt-5.4".to_string())
                .await
                .unwrap();

            {
                let map = handles.lock().await;
                let proc = map.get("session-1").unwrap();
                assert_eq!(proc.selected_model, Some("gpt-5.4".to_string()));
                // available_models は process キャッシュとして変更されないこと（owner は config）。
                assert_eq!(proc.available_models[0].model_id, "gpt-5.4");
            }

            let mut map = handles.lock().await;
            force_kill_all_sessions(&mut map).await;
        }

        #[tokio::test]
        async fn set_active_process_model_inactive_session_is_ok() {
            let handles = Arc::new(Mutex::new(HashMap::new()));

            // 該当 session が無くてもエラーにせず Ok(()) を返す（active 不在は no-op）。
            set_active_process_model(&handles, "missing", "gpt-5.4".to_string())
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn set_session_backend_removes_stale_unstarted_process() {
            let temp = tempfile::tempdir().unwrap();
            let session_store = Arc::new(crate::test_support::build_session_store());
            let session = create_session_internal(
                &session_store,
                temp.path(),
                "/repo",
                Some(CLAUDE_BACKEND_ID.to_string()),
            )
            .unwrap();

            let mut cfg = crate::adaptor::gateway::app_config::ReleashConfig::default();
            cfg.agents.codex.models = vec!["b-model".to_string()];
            let tmp = tempfile::NamedTempFile::new().unwrap();
            let config = Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
                cfg,
                tmp.path().to_path_buf(),
            ));

            let mut registry = AgentBackendRegistry::new();
            registry.register(Arc::new(MockModelBackend {
                backend_id: CLAUDE_BACKEND_ID.to_string(),
                models: Vec::new(),
            }));
            registry.register(Arc::new(MockModelBackend {
                backend_id: CODEX_BACKEND_ID.to_string(),
                models: Vec::new(),
            }));
            registry.set_config(config);
            let registry = Arc::new(registry);

            let mut cmd = tokio::process::Command::new("sleep");
            cmd.arg("999")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let mut child = cmd.spawn().unwrap();
            let stdin = child.stdin.take().unwrap();
            let pid = child.id();
            save_pgid(temp.path(), &session.id, pid.unwrap()).unwrap();

            let handles = Arc::new(Mutex::new(HashMap::new()));
            {
                let mut map = handles.lock().await;
                map.insert(
                    session.id.clone(),
                    make_dummy_agent_process(child, stdin, pid),
                );
            }

            let response = set_session_backend_internal(
                &session_store,
                &registry,
                &handles,
                temp.path(),
                &session.id,
                CODEX_BACKEND_ID.to_string(),
            )
            .await
            .unwrap();

            assert_eq!(
                response.session.backend_id,
                Some(CODEX_BACKEND_ID.to_string())
            );
            assert_eq!(response.available_models[0].model_id, "b-model");
            assert!(handles.lock().await.get(&session.id).is_none());
            assert!(!pids_dir(temp.path())
                .join(format!("{}.pid", session.id))
                .exists());
        }

        /// Spawn processes with setsid into AgentProcessMap, then verify
        /// force_kill_all_sessions actually terminates them.
        #[tokio::test]
        async fn force_kill_all_sessions_clears_map_and_kills_processes() {
            let mut map: AgentProcessMap = HashMap::new();
            let mut pids: Vec<u32> = Vec::new();

            for id in ["sess-a", "sess-b"] {
                let mut cmd = tokio::process::Command::new("sleep");
                cmd.arg("999")
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                unsafe {
                    cmd.pre_exec(|| {
                        if libc::setsid() == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
                let mut child = cmd.spawn().unwrap();
                let stdin = child.stdin.take().unwrap();
                let pid = child.id();
                if let Some(p) = pid {
                    pids.push(p);
                }
                map.insert(id.to_string(), make_dummy_agent_process(child, stdin, pid));
            }

            assert_eq!(map.len(), 2);

            let returned_ids = force_kill_all_sessions(&mut map).await;

            assert!(map.is_empty());
            assert_eq!(returned_ids.len(), 2);
            assert!(returned_ids.contains(&"sess-a".to_string()));
            assert!(returned_ids.contains(&"sess-b".to_string()));

            // Give processes time to be reaped
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            // Verify all processes are actually dead
            for pid in &pids {
                let alive = unsafe { libc::kill(*pid as libc::pid_t, 0) };
                assert_ne!(
                    alive, 0,
                    "process {pid} should be terminated after force_kill_all_sessions"
                );
            }
        }

        #[tokio::test]
        async fn force_kill_all_sessions_handles_empty_map() {
            let mut map: AgentProcessMap = HashMap::new();

            let returned_ids = force_kill_all_sessions(&mut map).await;

            assert!(map.is_empty());
            assert!(returned_ids.is_empty());
        }
    }

    // --- set_agent_model_internal: 仕様の中核 Rule の回帰防止テスト ---

    fn make_test_registry_with_models(
        claude_models: &[&str],
        codex_models: &[&str],
    ) -> Arc<AgentBackendRegistry> {
        let mut cfg = crate::adaptor::gateway::app_config::ReleashConfig::default();
        cfg.agents.claude.models = claude_models.iter().map(|s| s.to_string()).collect();
        cfg.agents.codex.models = codex_models.iter().map(|s| s.to_string()).collect();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
            cfg,
            tmp.path().to_path_buf(),
        ));
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: CLAUDE_BACKEND_ID.to_string(),
            models: vec![],
        }));
        registry.register(Arc::new(MockModelBackend {
            backend_id: CODEX_BACKEND_ID.to_string(),
            models: vec![],
        }));
        registry.set_config(config);
        Arc::new(registry)
    }

    #[tokio::test]
    async fn set_agent_model_accepts_registered_model() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_test_registry_with_models(&["claude-4"], &["gpt-5"]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "claude-4".to_string(),
        )
        .await
        .unwrap();

        let updated = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some("claude-4".to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_does_not_read_message_chunks() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "hello",
            None,
            None,
        )
        .unwrap();
        let chunk = temp
            .path()
            .join("sessions")
            .join(&session.id)
            .join("messages")
            .join("1.json");
        std::fs::write(chunk, "{not valid json").unwrap();
        let registry = make_test_registry_with_models(&["claude-4"], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "claude-4".to_string(),
        )
        .await
        .unwrap();

        let updated = session_store
            .get_session_meta(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some("claude-4".to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_preserves_surrounding_whitespace() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let model = "  claude-4  ";
        let registry = make_test_registry_with_models(&[model], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            model.to_string(),
        )
        .await
        .unwrap();

        let updated = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some(model.to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_unregistered_model() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.selected_model = Some("existing".to_string());
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let registry = make_test_registry_with_models(&["claude-4"], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let err = set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "unknown".to_string(),
        )
        .await;
        assert!(err.is_err());

        // 拒否時は selected_model を維持
        let after = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, Some("existing".to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_allows_other_backend_model_before_first_message() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_test_registry_with_models(&["claude-4"], &["gpt-5"]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "gpt-5".to_string(),
        )
        .await
        .unwrap();

        let after = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.backend_id.as_deref(), Some(CODEX_BACKEND_ID));
        assert_eq!(after.selected_model.as_deref(), Some("gpt-5"));
    }

    #[tokio::test]
    async fn get_session_applies_runtime_streaming_overlay_to_latest_page() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            Some(vec![MessagePart::Text {
                content: "persisted".to_string(),
                parent_tool_use_id: None,
            }]),
            None,
        )
        .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        {
            let mut proc = make_test_agent_process();
            proc.state = BridgeState::Streaming;
            proc.turn_phase = TurnPhase::Streaming;
            proc.streaming_message_id = Some(agent_message.id.clone());
            proc.streaming_parts = vec![MessagePart::Text {
                content: "streaming".to_string(),
                parent_tool_use_id: None,
            }];
            handles.lock().await.insert(session.id.clone(), proc);
        }

        let response = get_session_internal_with_data_dir(
            &session_store,
            &handles,
            Some(&make_fixed_model_registry()),
            temp.path(),
            &session.id,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(
            response.session.messages[0].parts,
            Some(vec![MessagePart::Text {
                content: "streaming".to_string(),
                parent_tool_use_id: None,
            }])
        );
        assert_eq!(
            response.initial_page,
            Some(InitialSessionPage {
                next_cursor: None,
                has_more: false,
                total_count: 1,
            })
        );
        let mut map = handles.lock().await;
        force_kill_all_sessions(&mut map).await;
    }

    #[tokio::test]
    async fn get_session_reads_only_latest_page_for_large_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let storage =
            Arc::new(crate::adaptor::gateway::agent_session::FileSessionStorage::default());
        let session_store = Arc::new(SessionStore::new(storage.clone()));
        let total_messages = INITIAL_SESSION_PAGE_LIMIT + 25;
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = ChatSession {
            id: session_id.clone(),
            worktree_path: "/repo".to_string(),
            messages: (0..total_messages)
                .map(|index| ChatMessage {
                    id: format!("m{index}"),
                    role: MessageRole::Human,
                    content: format!("message {index}"),
                    thinking: None,
                    activities: None,
                    parts: None,
                    timestamp: 1000.0 + index as f64,
                    mentions: None,
                })
                .collect(),
            state: crate::usecase::agent_session::session::SessionState::Idle,
            created_at: 1000.0,
            updated_at: 2000.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: Some("selected-model".to_string()),
            backend_id: Some(CLAUDE_BACKEND_ID.to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
        };
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();
        storage.reset_message_read_count();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let response = get_session_internal_with_data_dir(
            &session_store,
            &handles,
            None,
            temp.path(),
            &session_id,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(response.session.id, session_id);
        assert_eq!(
            response.session.selected_model,
            Some(crate::domain::agent_session::model_entry_id(
                CLAUDE_BACKEND_ID,
                "selected-model",
            ))
        );
        assert_eq!(
            response.turn_phase,
            crate::usecase::agent_session::status::TurnPhase::Idle
        );
        assert_eq!(response.session.messages.len(), INITIAL_SESSION_PAGE_LIMIT);
        assert_eq!(response.session.messages[0].id, "m25");
        assert_eq!(
            response.session.messages[INITIAL_SESSION_PAGE_LIMIT - 1].id,
            format!("m{}", total_messages - 1)
        );
        assert_eq!(
            response.initial_page,
            Some(InitialSessionPage {
                next_cursor: Some(PageCursor(26)),
                has_more: true,
                total_count: total_messages,
            })
        );
        assert_eq!(storage.message_read_count(), INITIAL_SESSION_PAGE_LIMIT);
    }

    #[tokio::test]
    async fn get_session_page_applies_runtime_streaming_overlay() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            Some(vec![MessagePart::Text {
                content: "persisted".to_string(),
                parent_tool_use_id: None,
            }]),
            None,
        )
        .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        {
            let mut proc = make_test_agent_process();
            proc.state = BridgeState::Streaming;
            proc.turn_phase = TurnPhase::Streaming;
            proc.streaming_message_id = Some(agent_message.id.clone());
            proc.streaming_parts = vec![MessagePart::Text {
                content: "streaming".to_string(),
                parent_tool_use_id: None,
            }];
            handles.lock().await.insert(session.id.clone(), proc);
        }

        let page = get_session_page_internal_with_data_dir(
            &session_store,
            &handles,
            temp.path(),
            &session.id,
            None,
            10,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(
            page.messages[0].parts,
            Some(vec![MessagePart::Text {
                content: "streaming".to_string(),
                parent_tool_use_id: None,
            }])
        );
        let mut map = handles.lock().await;
        force_kill_all_sessions(&mut map).await;
    }

    #[tokio::test]
    async fn set_agent_model_rejects_backend_change_after_first_message() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "hello",
            None,
            None,
        )
        .unwrap();
        let registry = make_test_registry_with_models(&["claude-4"], &["gpt-5"]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let err = set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "gpt-5".to_string(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("Cannot change backend"));

        let after = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.backend_id.as_deref(), Some(CLAUDE_BACKEND_ID));
        assert_eq!(after.selected_model, None);
    }

    #[tokio::test]
    async fn set_agent_model_rejects_empty_model() {
        // モデルは必須。空文字は形式不正として登録判定の前に拒否する。
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.selected_model = Some("claude-4".to_string());
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let registry = make_test_registry_with_models(&["claude-4"], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let err = set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            String::new(),
        )
        .await;
        assert!(err.is_err());

        // 拒否時は既存の selected_model を維持
        let after = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, Some("claude-4".to_string()));
    }

    // --- set_agent_model: 実 backend の固定リスト検証 ---

    fn make_fixed_model_registry() -> Arc<AgentBackendRegistry> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
            crate::adaptor::gateway::app_config::ReleashConfig::default(),
            tmp.path().to_path_buf(),
        ));
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(
            crate::infrastructure::agent_session::runtime::claude::ClaudeBackend::new(),
        ));
        registry.register(Arc::new(
            crate::infrastructure::agent_session::runtime::codex::CodexBackend::new(),
        ));
        registry.set_config(config);
        Arc::new(registry)
    }

    #[tokio::test]
    async fn set_agent_model_accepts_claude_fixed_model() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let model = crate::domain::agent_session::CLAUDE_FIXED_MODELS[0].to_string();

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            model.clone(),
        )
        .await
        .unwrap();

        let updated = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some(model));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_model_outside_claude_fixed_list() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let err = set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "not-a-fixed-claude-model".to_string(),
        )
        .await;
        assert!(err.is_err());

        let after = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, None);
    }

    #[tokio::test]
    async fn set_agent_model_accepts_codex_fixed_model() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let model = crate::domain::agent_session::CODEX_FIXED_MODELS[0].to_string();

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            model.clone(),
        )
        .await
        .unwrap();

        let updated = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some(model));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_model_outside_codex_fixed_list() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let err = set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "not-a-fixed-codex-model".to_string(),
        )
        .await;
        assert!(err.is_err());

        let after = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, None);
    }

    #[test]
    fn build_agent_models_updated_payload_emits_event_contract_fields() {
        let models = vec![
            ModelInfo::new(CLAUDE_BACKEND_ID, "a"),
            ModelInfo::new(CLAUDE_BACKEND_ID, "b"),
        ];
        let payload = build_agent_models_updated_payload("sess-1", &models, Some("a"));

        assert_eq!(payload["chat_session_id"], "sess-1");
        let candidates = payload["available_models"]
            .as_array()
            .expect("available_models is array");
        let values: Vec<String> = candidates
            .iter()
            .map(|v| v["modelId"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(values, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(candidates[0]["id"], "claude:a");
        assert_eq!(candidates[0]["displayName"], "a");
        assert_eq!(candidates[0]["backend"], "claude");
        assert_eq!(payload["selected_model"], "a");
    }

    #[test]
    fn build_agent_models_updated_payload_carries_selected_model_non_null() {
        // モデル未選択状態は廃止。set_agent_model は常に非 null の selected_model を emit する。
        let payload =
            build_agent_models_updated_payload("sess-2", &[], Some("claude:claude-opus-4-8"));
        assert_eq!(payload["selected_model"], "claude:claude-opus-4-8");
        assert_eq!(payload["available_models"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn token_usage_from_result_message_preserves_context_window_metadata() {
        let usage = token_usage_from_result_message(&serde_json::json!({
            "type": "result",
            "modelUsage": {
                "codex": {
                    "inputTokens": 12,
                    "outputTokens": 34,
                    "totalTokens": 1234,
                    "contextWindowTokens": 200000
                }
            }
        }))
        .expect("usage should be parsed");

        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.output_tokens, 34);
        assert_eq!(usage.total_tokens, Some(1234));
        assert_eq!(usage.context_window_tokens, Some(200000));
    }

    // --- get_persisted_spawn_info: 新規未起動セッションと選択解除後の区別 ---

    fn make_chat_session_for_spawn(
        agent_session_id: Option<String>,
        selected_model: Option<String>,
        backend_id: &str,
    ) -> ChatSession {
        ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::usecase::agent_session::session::SessionState::Active,
            created_at: 0.0,
            updated_at: 0.0,
            agent_session_id,
            context_carry: None,
            permission_mode: "acceptEdits".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model,
            backend_id: Some(backend_id.to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
        }
    }

    #[test]
    fn resolve_spawn_info_without_registry_keeps_none() {
        // registry 未指定（テスト等）では selected_model=None は None のまま。
        let session = make_chat_session_for_spawn(None, None, CODEX_BACKEND_ID);
        let info = resolve_spawn_info(Some(session), None);
        assert_eq!(info.resume_sid, None);
        assert_eq!(info.selected_model, None);
        assert_eq!(info.backend_id, CODEX_BACKEND_ID.to_string());
    }

    #[test]
    fn resolve_spawn_info_resolves_none_to_default_with_registry() {
        // モデル未選択状態は廃止。selected_model=None は registry の既定モデルへ解決する。
        let registry = make_fixed_model_registry();
        let session = make_chat_session_for_spawn(None, None, CODEX_BACKEND_ID);
        let info = resolve_spawn_info(Some(session), Some(&registry));
        assert_eq!(
            info.selected_model,
            Some(crate::domain::agent_session::CODEX_FIXED_MODELS[0].to_string())
        );
    }

    #[test]
    fn resolve_spawn_info_preserves_existing_selected_model() {
        // 永続化済みの selected_model はそのまま採用する（既定で上書きしない）。
        let registry = make_fixed_model_registry();
        let session = make_chat_session_for_spawn(
            None,
            Some(crate::domain::agent_session::CODEX_FIXED_MODELS[1].to_string()),
            CODEX_BACKEND_ID,
        );
        let info = resolve_spawn_info(Some(session), Some(&registry));
        assert_eq!(
            info.selected_model,
            Some(crate::domain::agent_session::CODEX_FIXED_MODELS[1].to_string())
        );
    }

    #[test]
    fn resolve_spawn_info_uses_default_backend_when_session_missing() {
        // 永続化セッションが存在しない場合は新規セッション扱い。
        // backend_id は claude（既定）にフォールバックし、selected_model も claude の既定へ解決する。
        let registry = make_fixed_model_registry();
        let info = resolve_spawn_info(None, Some(&registry));
        assert_eq!(
            info.selected_model,
            Some(crate::domain::agent_session::CLAUDE_FIXED_MODELS[0].to_string())
        );
        assert_eq!(info.backend_id, CLAUDE_BACKEND_ID.to_string());
    }

    // --- get_session: active process が居ても config 由来を返す ---

    #[tokio::test]
    async fn get_session_returns_config_derived_available_models_even_with_active_process() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_test_registry_with_models(&[], &["from-config"]);

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        // active process の stale キャッシュ
        {
            let mut map = handles.lock().await;
            let mut proc = make_test_agent_process();
            proc.backend_id = CODEX_BACKEND_ID.to_string();
            proc.available_models = vec![ModelInfo::new(CODEX_BACKEND_ID, "stale-from-process")];
            proc.latest_token_usage = Some(TokenUsage {
                input_tokens: 1200,
                output_tokens: 34,
                total_tokens: None,
                context_window_tokens: None,
            });
            map.insert(session.id.clone(), proc);
        }

        let response = get_session_internal_with_data_dir(
            &session_store,
            &handles,
            Some(&registry),
            temp.path(),
            &session.id,
        )
        .await
        .unwrap()
        .expect("session should exist");

        let values: Vec<String> = response
            .available_models
            .into_iter()
            .map(|m| m.model_id)
            .collect();
        assert_eq!(values, vec!["from-config".to_string()]);
        assert_eq!(
            response.latest_token_usage,
            Some(TokenUsage {
                input_tokens: 1200,
                output_tokens: 34,
                total_tokens: None,
                context_window_tokens: None,
            })
        );

        let mut map = handles.lock().await;
        force_kill_all_sessions(&mut map).await;
    }

    #[tokio::test]
    async fn get_session_page_returns_latest_token_usage_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "hello",
            None,
            None,
        )
        .unwrap();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        {
            let mut proc = make_test_agent_process();
            proc.latest_token_usage = Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: Some(15),
                context_window_tokens: Some(200_000),
            });
            handles.lock().await.insert(session.id.clone(), proc);
        }

        let page = get_session_page_internal_with_data_dir(
            &session_store,
            &handles,
            temp.path(),
            &session.id,
            None,
            10,
        )
        .await
        .unwrap()
        .expect("page should exist");

        assert_eq!(
            page.latest_token_usage,
            Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: Some(15),
                context_window_tokens: Some(200_000),
            })
        );
        assert_eq!(page.message_metadata[0].message_id, page.messages[0].id);

        let mut map = handles.lock().await;
        force_kill_all_sessions(&mut map).await;
    }

    #[tokio::test]
    async fn get_session_resolves_none_selected_model_to_default() {
        // spec: モデル未選択状態は廃止。selected_model=None の既存セッションを get_session
        // すると、応答の selected_model は backend の既定モデル（固定リスト先頭）へ解決される。
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        // 旧フォーマット（未選択）を模して None を永続化する。
        session.selected_model = None;
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let response = get_session_internal_with_data_dir(
            &session_store,
            &handles,
            Some(&registry),
            temp.path(),
            &session.id,
        )
        .await
        .unwrap()
        .expect("session should exist");

        assert_eq!(
            response.session.selected_model,
            Some(crate::domain::agent_session::model_entry_id(
                CLAUDE_BACKEND_ID,
                crate::domain::agent_session::CLAUDE_FIXED_MODELS[0],
            ))
        );
    }

    #[tokio::test]
    async fn get_session_errors_when_default_model_unresolvable() {
        // 契約: 応答の selected_model は常に非 null。registry が在りつつ既定モデルへ解決
        // できない場合（fixed_models 無し + config 空）、フィールド脱落を防ぐため Err を返す。
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.selected_model = None;
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        // claude/codex とも fixed_models を持たない mock backend + 空 config → 既定モデル無し。
        let registry = make_test_registry_with_models(&[], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let result = get_session_internal_with_data_dir(
            &session_store,
            &handles,
            Some(&registry),
            temp.path(),
            &session.id,
        )
        .await;

        assert!(result.is_err());
    }

    #[test]
    fn resolve_selected_model_for_response_keeps_existing_value() {
        let registry = make_test_registry_with_models(&[], &[]);
        let resolved = resolve_selected_model_for_response(
            Some("explicit-model".to_string()),
            CLAUDE_BACKEND_ID,
            Some(&registry),
        )
        .unwrap();
        assert_eq!(resolved, Some("explicit-model".to_string()));
    }

    #[test]
    fn resolve_selected_model_for_response_resolves_default_when_none() {
        let registry = make_fixed_model_registry();
        let resolved =
            resolve_selected_model_for_response(None, CLAUDE_BACKEND_ID, Some(&registry)).unwrap();
        assert_eq!(
            resolved,
            Some(crate::domain::agent_session::CLAUDE_FIXED_MODELS[0].to_string())
        );
    }

    #[test]
    fn resolve_selected_model_for_response_errors_when_unresolvable() {
        let registry = make_test_registry_with_models(&[], &[]);
        let result = resolve_selected_model_for_response(None, CLAUDE_BACKEND_ID, Some(&registry));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_selected_model_for_response_keeps_none_without_registry() {
        let resolved = resolve_selected_model_for_response(None, CLAUDE_BACKEND_ID, None).unwrap();
        assert_eq!(resolved, None);
    }

    #[tokio::test]
    async fn set_agent_model_syncs_active_process_available_models_from_config() {
        // active process が居る状態で set_agent_model を呼ぶと、proc.available_models が
        // config 由来の最新値で同期される（spec: モデル選択候補は config 単一 owner、
        // process キャッシュは emit 整合用にのみ維持）。
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_test_registry_with_models(&["claude-4", "haiku"], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        {
            let mut map = handles.lock().await;
            let mut proc = make_test_agent_process();
            proc.backend_id = CLAUDE_BACKEND_ID.to_string();
            // process cache が stale な状態
            proc.available_models = vec![ModelInfo::new(CLAUDE_BACKEND_ID, "stale")];
            map.insert(session.id.clone(), proc);
        }

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "claude-4".to_string(),
        )
        .await
        .unwrap();

        // proc.available_models が registry/config 由来の最新値に同期される
        let map = handles.lock().await;
        let proc = map.get(&session.id).unwrap();
        let values: Vec<String> = proc
            .available_models
            .iter()
            .map(|m| m.model_id.clone())
            .collect();
        assert_eq!(values, vec!["claude-4".to_string(), "haiku".to_string()]);
        // selected_model も反映される
        assert_eq!(proc.selected_model, Some("claude-4".to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_invalid_format_before_registry_check() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_test_registry_with_models(&["claude-4"], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        // 制御文字（形式不正）は登録判定に進む前に拒否
        let err = set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "bad\u{0001}model".to_string(),
        )
        .await;
        assert!(err.is_err());
    }

    /// spec issues-1022 "Agent process environment contract":
    /// agent process には自分の chat_session_id が `RELEASH_SESSION_ID` env として渡る。
    /// pure helper 単位で固定し、spawn_bridge_process の経路改修で env 注入が漏れた場合に
    /// 即座に検知できるようにする。
    #[test]
    fn session_specific_env_overrides_includes_releash_session_id() {
        let env = session_specific_env_overrides("my-session-id", None);
        let session_id = env
            .iter()
            .find_map(|(k, v)| (*k == "RELEASH_SESSION_ID").then_some(v.as_str()));
        assert_eq!(
            session_id,
            Some("my-session-id"),
            "agent process must receive its chat_session_id as RELEASH_SESSION_ID env"
        );
    }

    /// helper は受け取った文字列を env 値としてそのまま返す。入力検証 (空文字等) は
    /// spawn_bridge_process の呼び出し側 (Tauri command / WS handler) で行われる責務であり、
    /// helper はその境界を越えて値を加工しないことを固定する。
    #[test]
    fn session_specific_env_overrides_passes_through_value_verbatim() {
        let env = session_specific_env_overrides("", None);
        let session_id = env
            .iter()
            .find_map(|(k, v)| (*k == "RELEASH_SESSION_ID").then_some(v.as_str()));
        assert_eq!(session_id, Some(""));
    }

    /// spec issues-1022 "Agent process environment contract":
    /// base_branch が解決できた場合 (= Some) は `RELEASH_BASE_BRANCH` env が渡る。
    /// reviewer agent が `git diff "$RELEASH_BASE_BRANCH"...HEAD` で今回差分のみを
    /// 対象化できるようにする境界の固定。
    #[test]
    fn session_specific_env_overrides_includes_releash_base_branch_when_resolved() {
        let env = session_specific_env_overrides("sid", Some("main"));
        let base = env
            .iter()
            .find_map(|(k, v)| (*k == "RELEASH_BASE_BRANCH").then_some(v.as_str()));
        assert_eq!(
            base,
            Some("main"),
            "agent process must receive base branch as RELEASH_BASE_BRANCH env when resolved"
        );
    }

    /// base_branch が解決できない場合 (None) は `RELEASH_BASE_BRANCH` env を立てない。
    /// 空文字を立ててしまうと `git diff "$RELEASH_BASE_BRANCH"...HEAD` が
    /// `git diff ...HEAD` になり仕様外の挙動を起こすため、env 自体を立てないことを固定する。
    #[test]
    fn session_specific_env_overrides_omits_releash_base_branch_when_none() {
        let env = session_specific_env_overrides("sid", None);
        assert!(
            !env.iter().any(|(k, _)| *k == "RELEASH_BASE_BRANCH"),
            "RELEASH_BASE_BRANCH must not be set when base branch cannot be resolved"
        );
    }

    #[test]
    fn claude_bridge_watchdog_env_uses_existing_native_levers() {
        let env = claude_bridge_watchdog_env_overrides();
        let keys: Vec<&str> = env.iter().map(|(key, _)| *key).collect();

        assert_eq!(
            keys,
            vec![
                "CLAUDE_STREAM_IDLE_TIMEOUT_MS",
                "CLAUDE_ENABLE_STREAM_WATCHDOG",
                "CLAUDE_ENABLE_BYTE_WATCHDOG",
                "CLAUDE_CODE_MAX_RETRIES",
                "API_TIMEOUT_MS",
            ]
        );
        assert!(!keys.contains(&"CLAUDE_CODE_STREAM_CLOSE_TIMEOUT"));
        assert_eq!(
            env.iter()
                .find_map(|(key, value)| (*key == "CLAUDE_ENABLE_STREAM_WATCHDOG")
                    .then_some(value.as_str())),
            Some("1")
        );
        assert_eq!(
            env.iter()
                .find_map(|(key, value)| (*key == "CLAUDE_STREAM_IDLE_TIMEOUT_MS")
                    .then_some(value.as_str())),
            Some("180000")
        );
    }
}
