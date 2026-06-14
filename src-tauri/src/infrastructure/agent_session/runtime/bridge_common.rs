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
use crate::usecase::agent_session::session::{
    add_message_internal, now_timestamp, ChatMessage, ChatSession, GetSessionResponse, MessagePart,
    MessageRole, SessionStore, SessionSummary, TokenUsage,
};

pub(crate) use crate::infrastructure::agent_session::runtime::runtime_coordinator::acquire_session_runtime_lock;

pub const CLAUDE_BACKEND_ID: &str = "claude";
pub const CODEX_BACKEND_ID: &str = "codex";

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
    pub images: Vec<ImageAttachment>,
    pub worktree_path: String,
    pub mentions: Vec<crate::domain::code::MentionReference>,
    pub editor_context: Option<AgentEditorContext>,
}

pub struct AgentProcess {
    pub stdin: ChildStdin,
    pub backend_id: String,
    pub state: BridgeState,
    pub turn_phase: TurnPhase,
    pub sdk_session_id: Option<String>,
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
    /// Consumed by turn_complete handler and passed to WorkflowEngine.
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
        child,
        generation_id: 0,
        #[cfg(unix)]
        pgid: None,
        streaming_message_id: None,
        streaming_parts: Vec::new(),
        last_message_id: None,
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

const PERSIST_INTERVAL_MS: u64 = 1000;
const BRIDGE_EOF_ERROR_MESSAGE: &str = "Bridge process exited unexpectedly.";

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
        self.task_id_map.clear();
    }

    /// Write setMode + setModel commands to the Bridge stdin before a turn starts.
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

        // モデルは spawn 時に lazy 解決され常に非 null だが、フィールド型は互換のため
        // Option のまま。万一 None の場合は setModel を送らず Bridge 既定に委ねる。
        if let Some(model) = self.selected_model.as_deref() {
            let model_data = build_set_model_command(model);
            self.stdin
                .write_all(model_data.as_bytes())
                .await
                .map_err(|e| format!("Failed to write setModel: {e}"))?;
            self.stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush setModel: {e}"))?;
        }

        Ok(())
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
) {
    let data_dir = match resolve_data_dir(app) {
        Ok(d) => d,
        Err(e) => {
            log::warn!(
                "Failed to resolve data dir for streaming persist (session {chat_session_id}): {e}"
            );
            return;
        }
    };
    let mut session = match session_store.get_session(&data_dir, chat_session_id) {
        Ok(Some(s)) => s,
        Ok(None) => {
            log::warn!("Session not found for streaming persist: {chat_session_id}");
            return;
        }
        Err(e) => {
            log::warn!(
                "Failed to get session for streaming persist (session {chat_session_id}): {e}"
            );
            return;
        }
    };
    if let Some(msg) = session.messages.iter_mut().find(|m| m.id == message_id) {
        let (content, thinking, activities) =
            crate::usecase::agent_session::session::parts_to_legacy(parts);
        msg.content = content;
        msg.thinking = thinking;
        msg.activities = activities;
        msg.parts = Some(parts.to_vec());
        session.updated_at = now_timestamp();
        if let Err(e) = session_store.save_session(&data_dir, &session) {
            log::warn!("Failed to persist streaming parts for session {chat_session_id}: {e}");
        }
    }
}

fn emit_session_state_changed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    chat_session_id: &str,
    turn_phase: TurnPhase,
    exit_code: Option<i64>,
) {
    use tauri::Emitter;
    let _ = app.emit(
        "agent-session-state-changed",
        serde_json::json!({
            "chat_session_id": chat_session_id,
            "turn_phase": turn_phase,
            "exit_code": exit_code,
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
        MessagePart::SystemNotification {
            notification_type,
            status,
            label,
            detail,
            hook_id,
        } => crate::protocol::AgentStreamPartMsg::SystemNotification {
            notification_type,
            status,
            label,
            detail,
            hook_id,
        },
        MessagePart::Image { data, media_type } => {
            crate::protocol::AgentStreamPartMsg::Image { data, media_type }
        }
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
        MessagePart::SystemNotification {
            notification_type,
            status,
            label,
            detail,
            hook_id,
        } => {
            notification_type.len()
                + status.len()
                + label.len()
                + detail.as_ref().map(|s| s.len()).unwrap_or(0)
                + hook_id.as_ref().map(|s| s.len()).unwrap_or(0)
        }
        MessagePart::Image { data, media_type } => data.len() + media_type.len(),
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
    let parts = consolidate_parts(proc.streaming_parts.clone());
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
        proc.last_stream_emit_at = Some(Instant::now());
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
) where
    F: FnMut(&[MessagePart]) -> (bool, bool),
{
    let Some(snapshot) = prepare_streaming_flush(proc) else {
        return;
    };
    let (tauri_ok, ws_ok) = emit(&snapshot.parts);
    apply_streaming_emit_result(
        proc,
        chat_session_id,
        message_id,
        &snapshot,
        tauri_ok,
        ws_ok,
    );
}

/// Attempt to emit the cumulative `streaming_parts` payload. No-op when the
/// pending buffer is empty (prevents idle-tick re-delivery and double-flush
/// from forced-flush paths). On success, clears pending and updates
/// `last_stream_emit_at`. On failure, retains both so the next flush retries.
fn flush_streaming<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    proc: &mut AgentProcess,
    chat_session_id: &str,
    message_id: &str,
) {
    force_flush_pending_streaming(proc, chat_session_id, message_id, |parts| {
        emit_streaming_parts(app, chat_session_id, message_id, parts.to_vec())
    });
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
    force_flush_pending_streaming(proc, chat_session_id, &mid, |parts| {
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
    raw_parts: Vec<MessagePart>,
    turn_token_usage: Option<(u64, u64)>,
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
    let was_streaming = flush_streaming_before_transition(proc, chat_session_id, emit_stream);
    proc.state = if exit_code == 0 {
        BridgeState::Ready
    } else {
        BridgeState::Crashed
    };
    proc.turn_phase = TurnPhase::Idle;
    let turn_token_usage = proc.last_result_token_usage.take();
    let raw_parts = proc.streaming_parts.clone();
    let final_msg_id = proc.streaming_message_id.take();
    if final_msg_id.is_some() {
        proc.last_message_id.clone_from(&final_msg_id);
    }
    TurnCompleteTransition {
        was_streaming,
        final_msg_id,
        raw_parts,
        turn_token_usage,
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

/// One iteration of the auxiliary timer loop. Bound to a single process by
/// the caller (generation_id / state checks happen above this helper). The
/// emit closure mirrors `force_flush_pending_streaming` so tests can drive
/// the same code path the production timer uses.
///
/// Returns `true` when the timer should continue running this turn, and
/// `false` when the loop should exit (turn is over and the buffer has been
/// fully drained).
fn run_streaming_timer_tick<F>(proc: &mut AgentProcess, chat_session_id: &str, mut emit: F) -> bool
where
    F: FnMut(&str, &[MessagePart]) -> (bool, bool),
{
    let pending = proc.pending_stream_part_count > 0;
    let streaming = proc.state == BridgeState::Streaming;
    if !pending && !streaming {
        // Turn ended and the buffer is empty — timer has nothing left to do.
        return false;
    }
    if !pending || !streaming_interval_elapsed(proc) {
        return true;
    }
    let Some(mid) = proc
        .streaming_message_id
        .clone()
        .or_else(|| proc.last_message_id.clone())
    else {
        return true;
    };
    force_flush_pending_streaming(proc, chat_session_id, &mid, |parts| emit(&mid, parts));
    true
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
struct BridgeEofCrashEffect {
    was_streaming: bool,
    was_initializing: bool,
    message_id: Option<String>,
    error_delta: Vec<MessagePart>,
    persisted_parts: Vec<MessagePart>,
    sdk_error_message: Option<String>,
}

fn apply_bridge_eof_crash(
    generation_matches: bool,
    state: &mut BridgeState,
    turn_phase: &mut TurnPhase,
    streaming_message_id: Option<&str>,
    streaming_parts: &mut Vec<MessagePart>,
    backend_id: &str,
) -> BridgeEofCrashEffect {
    if !generation_matches {
        return BridgeEofCrashEffect::default();
    }

    let was_streaming = *state == BridgeState::Streaming;
    let was_initializing = *state == BridgeState::Initializing;
    let mut effect = BridgeEofCrashEffect {
        was_streaming,
        was_initializing,
        message_id: streaming_message_id.map(str::to_string),
        error_delta: Vec::new(),
        persisted_parts: Vec::new(),
        sdk_error_message: None,
    };

    if was_streaming || was_initializing {
        effect.sdk_error_message = Some(format!("{backend_id}: {BRIDGE_EOF_ERROR_MESSAGE}"));
    }

    if was_streaming {
        let part = MessagePart::Error {
            content: format!("Error: {BRIDGE_EOF_ERROR_MESSAGE}"),
            parent_tool_use_id: None,
        };
        streaming_parts.push(part.clone());
        effect.error_delta.push(part);
        effect.persisted_parts = consolidate_parts(streaming_parts.clone());
    }

    if was_streaming || was_initializing {
        *state = BridgeState::Crashed;
        *turn_phase = TurnPhase::Idle;
    }

    effect
}

/// 状態遷移時に AgentStatusCenter へ通知し、必要に応じて Webhook 送信を行う統一エントリ。
/// session_store から ChatSession を引いて worktree_path / SessionState を取得する。
/// `session_state_override` を渡すと、ストア値より優先される（Bridge crash 時など）。
pub(crate) fn notify_status_transition<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    turn_phase: TurnPhase,
    session_state_override: Option<crate::usecase::agent_session::session::SessionState>,
) {
    use crate::config::AppConfig;
    use crate::focus_tracker::FocusTracker;
    use crate::usecase::agent_session::status::{
        current_timestamp, AgentStatusCenter, SessionStatus, TurnPhaseRepr,
    };

    let data_dir = match resolve_data_dir(app) {
        Ok(d) => d,
        Err(_) => return,
    };
    let session = match session_store.get_session(&data_dir, chat_session_id) {
        Ok(Some(s)) => s,
        _ => return,
    };
    let worktree_path = session.worktree_path.clone();
    let session_state = session_state_override.unwrap_or_else(|| session.state.clone());

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

    // Webhook 送信（Slack/Discord）
    if let (Some(cfg_state), Some(ft_state)) = (
        app.try_state::<Arc<AppConfig>>(),
        app.try_state::<Arc<parking_lot::Mutex<FocusTracker>>>(),
    ) {
        if let Ok(cfg) = cfg_state.get_config() {
            let notify = cfg.server.notify.clone();
            let url = notify.webhook_url.clone();
            if !url.is_empty() && crate::webhook::should_notify(&notify, &agent_state, &ft_state) {
                let agent_state_msg = crate::protocol::AgentState::from(agent_state.clone());
                let sync = crate::protocol::AgentStateSync {
                    worktree_path: worktree_path.clone(),
                    state: agent_state_msg,
                    exit_code: None,
                    timestamp: current_timestamp(),
                    session_id: Some(chat_session_id.to_string()),
                    pty_id: None,
                };
                tokio::spawn(async move {
                    crate::webhook::send_webhook(&url, &sync).await;
                });
            }
        }
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
    let saved_session = match session_store.get_session(&data_dir, chat_session_id) {
        Ok(session) => session,
        Err(e) => {
            log::error!(
                "Failed to read saved session for SDK permissionMode notification \
                 (chat_session_id={chat_session_id}): {e}"
            );
            return;
        }
    };
    let Some(session) = saved_session else {
        log::error!(
            "Saved session not found for SDK permissionMode notification \
             (chat_session_id={chat_session_id})"
        );
        return;
    };
    let canonical_mode = match crate::permission::PermissionMode::parse(&session.permission_mode) {
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
    for (k, v) in bridge_permission_fields(pm, backend_id) {
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
) -> Vec<(String, serde_json::Value)> {
    use crate::infrastructure::agent_session::runtime::permission_flags::{
        claude_flag_from_mode, codex_approval_policy_from_mode, codex_sandbox_mode_from_mode,
    };
    if backend_id == CODEX_BACKEND_ID {
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
            serde_json::Value::String(claude_flag_from_mode(pm).to_string()),
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
/// merged same-type runs is performed by `consolidate_parts` when generating
/// emit/persist payloads.
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
fn consolidate_parts(parts: Vec<MessagePart>) -> Vec<MessagePart> {
    let mut result: Vec<MessagePart> = Vec::with_capacity(parts.len());
    for part in parts {
        match (&part, result.last_mut()) {
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
            _ => {
                result.push(part);
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

/// Returns true if the message should be forwarded as agent-sdk-message.
/// Non-accumulated messages (meta events) are always forwarded.
/// permission_request is accumulated (for streaming delta) but ALSO forwarded
/// for SET_PENDING_PERMISSION dispatch on the frontend.
fn should_forward_sdk_message(accumulated: bool, msg_type: &str) -> bool {
    !accumulated || msg_type == "permission_request"
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

/// Parse SDK message and accumulate into streaming_parts.
/// Returns (accumulated, updated_parts):
/// - accumulated: true if the message was handled and should NOT be forwarded as agent-sdk-message.
/// - updated_parts: Some(parts) when an existing part was updated in-place (e.g. compaction/hook completion).
///   These must be emitted as delta since they are not captured by the `parts[prev_len..]` diff.
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
                            parts.push(MessagePart::ToolUse {
                                tool,
                                input,
                                id,
                                parent_tool_use_id: parent_tool_use_id.clone(),
                            });
                        }
                    }
                }
            }
            (true, None)
        }
        "user" => {
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
                            parts.push(MessagePart::ToolResult {
                                content: content_str,
                                is_error,
                                tool_use_id,
                                parent_tool_use_id: parent_tool_use_id.clone(),
                            });
                        }
                    }
                }
            }
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
                            if notification_type == "compaction" && status == "in_progress" {
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
                            notification_type: "compaction".to_string(),
                            status: "completed".to_string(),
                            label: "Conversation compacted".to_string(),
                            detail: Some(detail),
                            hook_id: None,
                        });
                        (true, None)
                    }
                }
                "hook_started" => {
                    let hook_name = msg
                        .get("hook_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let hook_event = msg
                        .get("hook_event")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let hook_id = msg
                        .get("hook_id")
                        .and_then(|v| v.as_str())
                        .filter(|id| !id.is_empty())
                        .map(|id| id.to_string());
                    parts.push(MessagePart::SystemNotification {
                        notification_type: "hook".to_string(),
                        status: "in_progress".to_string(),
                        label: format!("{hook_name} ({hook_event})"),
                        detail: None,
                        hook_id: hook_id.clone(),
                    });
                    (true, None)
                }
                "hook_response" => {
                    let hook_id = msg
                        .get("hook_id")
                        .and_then(|v| v.as_str())
                        .filter(|id| !id.is_empty());
                    let outcome = msg
                        .get("outcome")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let exit_code = msg
                        .get("exit_code")
                        .and_then(|v| v.as_i64())
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let new_status = if outcome == "error" {
                        "error"
                    } else {
                        "completed"
                    };
                    let detail = format!("outcome={outcome}, exit_code={exit_code}");

                    // Walk parts in reverse to find the matching hook_started notification
                    let mut updated_part = None;
                    for part in parts.iter_mut().rev() {
                        if let MessagePart::SystemNotification {
                            notification_type,
                            status,
                            detail: d,
                            hook_id: hid,
                            ..
                        } = part
                        {
                            if notification_type == "hook"
                                && status == "in_progress"
                                && hid.as_deref() == hook_id
                            {
                                *status = new_status.to_string();
                                *d = Some(detail.clone());
                                updated_part = Some(part.clone());
                                break;
                            }
                        }
                    }
                    if let Some(p) = updated_part {
                        (true, Some(vec![p]))
                    } else {
                        // No matching hook_started found, add a standalone completed entry
                        let hook_name = msg
                            .get("hook_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let hook_event = msg
                            .get("hook_event")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        parts.push(MessagePart::SystemNotification {
                            notification_type: "hook".to_string(),
                            status: new_status.to_string(),
                            label: format!("{hook_name} ({hook_event})"),
                            detail: Some(detail),
                            hook_id: hook_id.map(|id| id.to_string()),
                        });
                        (true, None)
                    }
                }
                "files_persisted" => {
                    let file_paths = msg
                        .get("filePaths")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    parts.push(MessagePart::SystemNotification {
                        notification_type: "files_persisted".to_string(),
                        status: "completed".to_string(),
                        label: "Files persisted".to_string(),
                        detail: if file_paths.is_empty() {
                            None
                        } else {
                            Some(file_paths)
                        },
                        hook_id: None,
                    });
                    (true, None)
                }
                "local_command_output" => {
                    let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let truncated = if content.chars().count() > 200 {
                        // Truncate at char boundary
                        match content.char_indices().nth(200) {
                            Some((byte_pos, _)) => format!("{}…", &content[..byte_pos]),
                            None => content.to_string(),
                        }
                    } else {
                        content.to_string()
                    };
                    parts.push(MessagePart::SystemNotification {
                        notification_type: "local_command_output".to_string(),
                        status: "completed".to_string(),
                        label: "Command output".to_string(),
                        detail: if truncated.is_empty() {
                            None
                        } else {
                            Some(truncated)
                        },
                        hook_id: None,
                    });
                    (true, None)
                }
                "codex_realtime" => {
                    let notification_type = msg
                        .get("notification_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("codex_realtime");
                    let status = msg
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("in_progress");
                    let label = msg
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Codex realtime");
                    let detail = msg
                        .get("detail")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    parts.push(MessagePart::SystemNotification {
                        notification_type: notification_type.to_string(),
                        status: status.to_string(),
                        label: label.to_string(),
                        detail,
                        hook_id: None,
                    });
                    (true, None)
                }
                _ => {
                    // Check for status=compacting (subtype may be empty/"" for status messages)
                    let status = msg.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    if status == "compacting" {
                        parts.push(MessagePart::SystemNotification {
                            notification_type: "compaction".to_string(),
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
        "error" => {
            let error_text = msg
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            parts.push(MessagePart::Error {
                content: format!("Error: {}", error_text),
                parent_tool_use_id,
            });
            (false, None) // Still forward for handleBridgeError
        }
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
fn session_specific_env_overrides(
    chat_session_id: &str,
    base_branch: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut env = vec![("RELEASH_SESSION_ID", chat_session_id.to_string())];
    if let Some(b) = base_branch {
        env.push(("RELEASH_BASE_BRANCH", b.to_string()));
    }
    env
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
    selected_model: Option<String>,
    system_prompt: Option<String>,
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
        &session_id,
        composed_system_prompt,
        &backend_id,
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
                child,
                generation_id: gen_id,
                #[cfg(unix)]
                pgid,
                streaming_message_id: None,
                streaming_parts: Vec::new(),
                last_message_id: None,
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
                        let mut map = handles_stdout.lock().await;
                        if let Some(proc) = map.get_mut(&csid_stdout) {
                            // Only transition to Ready if still Initializing (not already Streaming)
                            if proc.state == BridgeState::Initializing {
                                proc.state = BridgeState::Ready;
                            }
                            if let Some(sid) = msg.get("session_id").and_then(|v| v.as_str()) {
                                proc.sdk_session_id = Some(sid.to_string());
                            }
                        }
                        let _ = app_stdout.emit("agent-sdk-message", &msg);
                    }
                    "session_cleared" => {
                        {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                proc.sdk_session_id = None;
                            }
                        }
                        if let Ok(data_dir) = resolve_data_dir(&app_stdout) {
                            if let Ok(Some(mut session)) =
                                session_store_clone.get_session(&data_dir, &csid_stdout)
                            {
                                if session.agent_session_id.is_some() {
                                    session.agent_session_id = None;
                                    session.updated_at = now_timestamp();
                                    let _ = session_store_clone.save_session(&data_dir, &session);
                                }
                            }
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
                        // 区間に限定する。engine.on_turn_complete や pending message
                        // 消費は lock を保持しない経路で行い、それらが必要に応じ
                        // 自前で lock を取得する設計とする（再入デッドロックを防ぐ）。
                        let was_streaming;
                        let raw_parts;
                        let final_msg_id;
                        let turn_token_usage;
                        {
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
                                was_streaming = effect.was_streaming;
                                turn_token_usage = effect.turn_token_usage;
                                raw_parts = effect.raw_parts;
                                final_msg_id = effect.final_msg_id;

                                // User turn succeeded: persist agent_session_id to SessionStore
                                if was_streaming && exit_code == 0 {
                                    if let Some(sid) = &proc.sdk_session_id {
                                        if let Ok(data_dir) = resolve_data_dir(&app_stdout) {
                                            if let Ok(Some(mut session)) = session_store_clone
                                                .get_session(&data_dir, &csid_stdout)
                                            {
                                                session.agent_session_id = Some(sid.to_string());
                                                session.updated_at = now_timestamp();
                                                let _ = session_store_clone
                                                    .save_session(&data_dir, &session);
                                            }
                                        }
                                    }
                                }
                            } else {
                                was_streaming = false;
                                raw_parts = Vec::new();
                                final_msg_id = None;
                                turn_token_usage = None;
                            }
                            // _runtime_guard はこのスコープを抜けて drop される
                        }

                        // Consolidate text/thinking chunks outside lock
                        let final_parts = consolidate_parts(raw_parts);

                        // Final persist of streaming buffer
                        if was_streaming {
                            if let Some(ref mid) = final_msg_id {
                                if !final_parts.is_empty() {
                                    persist_streaming_parts(
                                        &session_store_clone,
                                        &app_stdout,
                                        &csid_stdout,
                                        mid,
                                        &final_parts,
                                    );
                                }
                            }
                        }

                        // Resume failure (error during init) → clear stale agent_session_id
                        if !was_streaming && exit_code != 0 {
                            if let Ok(data_dir) = resolve_data_dir(&app_stdout) {
                                if let Ok(Some(mut session)) =
                                    session_store_clone.get_session(&data_dir, &csid_stdout)
                                {
                                    if session.agent_session_id.is_some() {
                                        session.agent_session_id = None;
                                        session.updated_at = now_timestamp();
                                        let _ =
                                            session_store_clone.save_session(&data_dir, &session);
                                    }
                                }
                            }
                        }

                        // Emit state change only for user turns (was Streaming)
                        if was_streaming {
                            emit_session_state_changed(
                                &app_stdout,
                                &csid_stdout,
                                TurnPhase::Idle,
                                Some(exit_code),
                            );
                            // AgentStatusCenter にも通知（exit_code 非0 なら Error 扱い）
                            let override_state = if exit_code != 0 {
                                Some(crate::usecase::agent_session::session::SessionState::Error)
                            } else {
                                None
                            };
                            notify_status_transition(
                                &app_stdout,
                                &session_store_clone,
                                &csid_stdout,
                                TurnPhase::Idle,
                                override_state,
                            );

                            // Workflow engine への通知と pending message 消費は
                            // session_runtime_lock を保持しない経路で実施する。
                            // 各経路は必要に応じて自前で lock を取得する（spawn-if-needed
                            // / close ガード）ため、ここで lock を保持してはならない
                            // （engine 内で同 session への turn 再投入があると再入デッドロック）。
                            {
                                use tauri::Manager;
                                let wf_engine: Option<
                                    Arc<crate::workflow::engine::WorkflowEngine>,
                                > = app_stdout
                                    .try_state::<Arc<crate::workflow::engine::WorkflowEngine>>()
                                    .map(|s| Arc::clone(&s));
                                let pending =
                                    take_pending_message(&handles_stdout, &csid_stdout).await;
                                let app_wf = app_stdout.clone();
                                let ss_wf = Arc::clone(&session_store_clone);
                                let h_wf = Arc::clone(&handles_stdout);
                                let csid_wf = csid_stdout.clone();
                                let parts_wf = final_parts.clone();
                                let token_usage_wf = turn_token_usage;
                                let handle = tokio::runtime::Handle::current();
                                std::thread::spawn(move || {
                                    handle.block_on(async move {
                                        if let Some(engine) = wf_engine {
                                            if engine.is_running(&csid_wf).await {
                                                match crate::app_data_dir::resolve_data_dir(&app_wf) {
                                                    Ok(data_dir) => {
                                                        let store =
                                                            crate::workflow::pending_command::PendingCommandStore::new(
                                                                &data_dir,
                                                            );
                                                        crate::workflow::pending_command_watcher::process_pending_submit_output_pickup(
                                                            &app_wf, &store,
                                                        )
                                                        .await;
                                                    }
                                                    Err(e) => {
                                                        log::warn!(
                                                            "pending SubmitOutput pickup skipped for {}: resolve_data_dir failed: {e}",
                                                            csid_wf
                                                        );
                                                    }
                                                }
                                                if let Err(e) = engine
                                                    .on_turn_complete(
                                                        &app_wf, &ss_wf, &h_wf, &csid_wf,
                                                        exit_code, &parts_wf,
                                                        token_usage_wf,
                                                    )
                                                    .await
                                                {
                                                    log::error!(
                                                        "Workflow on_turn_complete error for {}: {e}",
                                                        csid_wf
                                                    );
                                                }
                                            }
                                        }
                                        if let Some(pending) = pending {
                                            start_pending_message_turn(
                                                &app_wf, &h_wf, &ss_wf, &csid_wf, pending,
                                            )
                                            .await;
                                        }
                                    });
                                });
                            }
                        }
                    }
                    "error" => {
                        let error_msg = msg
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown bridge error");
                        log::error!("Bridge error [{}]: {}", csid_stdout, error_msg);

                        // Accumulate the error part, enqueue it for emission, and
                        // force-flush so the UI surfaces the failure immediately.
                        {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                if proc.state == BridgeState::Streaming {
                                    let prev_len = proc.streaming_parts.len();
                                    accumulate_sdk_message(
                                        &msg,
                                        &mut proc.streaming_parts,
                                        &mut proc.task_id_map,
                                    );
                                    let delta: Vec<MessagePart> =
                                        proc.streaming_parts[prev_len..].to_vec();
                                    let mid = proc.streaming_message_id.clone();
                                    if !delta.is_empty() {
                                        enqueue_pending_delta(proc, &delta);
                                    }
                                    if let Some(ref mid) = mid {
                                        flush_streaming(&app_stdout, proc, &csid_stdout, mid);
                                    }
                                }
                            }
                        }

                        let _ = app_stdout.emit("agent-sdk-message", &msg);

                        // Transition to Crashed for both Streaming and Initializing states
                        let (was_streaming, was_initializing) = {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                let ws = proc.state == BridgeState::Streaming;
                                let wi = proc.state == BridgeState::Initializing;
                                if ws || wi {
                                    proc.state = BridgeState::Crashed;
                                    proc.turn_phase = TurnPhase::Idle;
                                }
                                (ws, wi)
                            } else {
                                (false, false)
                            }
                        };
                        if was_streaming {
                            emit_session_state_changed(
                                &app_stdout,
                                &csid_stdout,
                                TurnPhase::Idle,
                                Some(1),
                            );
                        }
                        // Bridge crash: AgentStatusCenter に Error として通知
                        if was_streaming || was_initializing {
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
                        {
                            if let Ok(data_dir) = resolve_data_dir(&app_stdout) {
                                if let Ok(Some(mut session)) =
                                    session_store_clone.get_session(&data_dir, &csid_stdout)
                                {
                                    if session.agent_session_id.is_some() {
                                        session.agent_session_id = None;
                                        session.updated_at = now_timestamp();
                                        let _ =
                                            session_store_clone.save_session(&data_dir, &session);
                                    }
                                }
                            }
                        }
                    }
                    _ => {
                        // Accumulate into streaming buffer, enqueue delta into the
                        // coalescing buffer, and flush when warranted. We hold the
                        // lock across the flush so the emit observes consistent
                        // state with `streaming_parts`.
                        let (accumulated, emit_msg_id, should_persist, raw_persist_parts) = {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                let in_streaming = proc.state == BridgeState::Streaming
                                    && proc.streaming_message_id.is_some();
                                let post_turn = !in_streaming && proc.last_message_id.is_some();

                                if !in_streaming && !post_turn {
                                    (false, None, false, Vec::new())
                                } else {
                                    let prev_len = proc.streaming_parts.len();
                                    let (acc, updated_parts) = accumulate_sdk_message(
                                        &msg,
                                        &mut proc.streaming_parts,
                                        &mut proc.task_id_map,
                                    );
                                    if !acc {
                                        (false, None, false, Vec::new())
                                    } else {
                                        let mut delta: Vec<MessagePart> =
                                            proc.streaming_parts[prev_len..].to_vec();
                                        if let Some(up) = updated_parts {
                                            delta.extend(up);
                                        }
                                        let mid = if in_streaming {
                                            proc.streaming_message_id.clone()
                                        } else {
                                            proc.last_message_id.clone()
                                        };

                                        enqueue_pending_delta(proc, &delta);

                                        // Flush triggers: in-stream uses
                                        // interval + threshold; post-turn events
                                        // (background tasks) are flushed eagerly.
                                        if should_flush_per_delta(proc, &delta, post_turn) {
                                            if let Some(ref mid) = mid {
                                                flush_streaming(
                                                    &app_stdout,
                                                    proc,
                                                    &csid_stdout,
                                                    mid,
                                                );
                                            }
                                        }

                                        let now = Instant::now();
                                        let elapsed_persist =
                                            now.duration_since(last_persist_time).as_millis()
                                                as u64;
                                        let should_persist =
                                            post_turn || elapsed_persist >= PERSIST_INTERVAL_MS;
                                        let raw_persist_parts = if should_persist {
                                            proc.streaming_parts.clone()
                                        } else {
                                            Vec::new()
                                        };
                                        (true, mid, should_persist, raw_persist_parts)
                                    }
                                }
                            } else {
                                (false, None, false, Vec::new())
                            }
                        };

                        // Periodic persist (1s interval) — consolidate outside lock
                        if should_persist {
                            if let Some(ref mid) = emit_msg_id {
                                last_persist_time = Instant::now();
                                let persist_parts = consolidate_parts(raw_persist_parts);
                                persist_streaming_parts(
                                    &session_store_clone,
                                    &app_stdout,
                                    &csid_stdout,
                                    mid,
                                    &persist_parts,
                                );
                            }
                        }

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
                        if should_forward_sdk_message(accumulated, msg_type) {
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
        let effect = {
            let mut map = handles_stdout.lock().await;
            if let Some(proc) = map.get_mut(&csid_stdout) {
                let streaming_message_id = proc.streaming_message_id.clone();
                let generation_matches = proc.generation_id == captured_gen_id;
                let backend_id = proc.backend_id.clone();
                let effect = apply_bridge_eof_crash(
                    generation_matches,
                    &mut proc.state,
                    &mut proc.turn_phase,
                    streaming_message_id.as_deref(),
                    &mut proc.streaming_parts,
                    &backend_id,
                );
                // Enqueue the synthetic crash delta into the coalescing buffer and
                // force-flush so the UI sees the error before the Idle transition.
                if let (Some(mid), false) =
                    (streaming_message_id.as_ref(), effect.error_delta.is_empty())
                {
                    enqueue_pending_delta(proc, &effect.error_delta);
                    flush_streaming(&app_stdout, proc, &csid_stdout, mid);
                }
                effect
            } else {
                BridgeEofCrashEffect::default()
            }
        };
        if let Some(message) = effect.sdk_error_message.as_deref() {
            let _ = app_stdout.emit(
                "agent-sdk-message",
                serde_json::json!({
                    "type": "error",
                    "message": message,
                    "chat_session_id": &csid_stdout,
                }),
            );
        }
        if let Some(message_id) = effect.message_id.as_deref() {
            if !effect.persisted_parts.is_empty() {
                persist_streaming_parts(
                    &session_store_clone,
                    &app_stdout,
                    &csid_stdout,
                    message_id,
                    &effect.persisted_parts,
                );
            }
        }
        if effect.was_streaming {
            emit_session_state_changed(&app_stdout, &csid_stdout, TurnPhase::Idle, Some(-1));
            notify_status_transition(
                &app_stdout,
                &session_store_clone,
                &csid_stdout,
                TurnPhase::Idle,
                Some(crate::usecase::agent_session::session::SessionState::Error),
            );
        } else if effect.was_initializing {
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
            let keep_running = run_streaming_timer_tick(proc, &csid_timer, |mid, parts| {
                emit_streaming_parts(&app_timer, &csid_timer, mid, parts.to_vec())
            });
            if !keep_running {
                proc.streaming_timer_active = false;
                break;
            }
        }
    });
}

pub(crate) async fn get_session_internal(
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    app: &tauri::AppHandle,
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
    let session = session_store.get_session(data_dir, session_id)?;
    match session {
        None => Ok(None),
        Some(mut session) => {
            let (turn_phase, raw_parts, streaming_mid, pending_queue, latest_token_usage) = {
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
                            proc.streaming_parts.clone(),
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
                    let parts = consolidate_parts(raw_parts);
                    if !parts.is_empty() {
                        if let Some(msg) = session.messages.iter_mut().find(|m| m.id == *mid) {
                            msg.parts = Some(parts);
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
            session.selected_model = resolve_selected_model_for_response(
                session.selected_model.take(),
                &backend_id,
                registry,
            )?;

            Ok(Some(GetSessionResponse {
                session,
                turn_phase: turn_phase.into(),
                available_models: available_models.into_iter().map(Into::into).collect(),
                pending_queue_count: pending_queue.len(),
                pending_queue,
                latest_token_usage,
            }))
        }
    }
}

fn can_change_session_backend(session: &ChatSession) -> bool {
    session.messages.is_empty() && session.agent_session_id.is_none()
}

/// spec issues-1023: 初期 active 候補は workflow step として起動された session を
/// 除外し、free chat（`workflow_step_session == false`）の先頭を採用する。free chat が
/// 1 件もない場合は active 候補無し（`None`）で、UI は空状態を描く。
fn pick_initial_active_session_candidate(sessions: &[SessionSummary]) -> Option<&SessionSummary> {
    sessions.iter().find(|s| !s.workflow_step_session)
}

fn should_start_agent_process_for_summary(session: &SessionSummary) -> bool {
    !session.workflow_step_session
        && (session.message_count > 0 || session.agent_session_id.is_some())
}

fn ensure_session_backend_selected(
    session_store: &SessionStore,
    registry: &crate::infrastructure::agent_session::runtime::AgentBackendRegistry,
    data_dir: &Path,
    mut session: ChatSession,
) -> Result<ChatSession, String> {
    if session.backend_id.is_none() {
        session.backend_id = Some(registry.resolve_default_id()?);
        session.updated_at = now_timestamp();
        session_store.save_session(data_dir, &session)?;
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
    let mut session = session_store
        .get_session(data_dir, chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;

    if !can_change_session_backend(&session) {
        return Err(format!(
            "Cannot change backend after the first message has been sent: {chat_session_id}"
        ));
    }

    session.selected_model = Some(registry.default_model_for(&resolved_backend_id)?);
    session.backend_id = Some(resolved_backend_id);
    session.updated_at = now_timestamp();
    session_store.save_session(data_dir, &session)?;
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
) -> Result<(Option<String>, Option<String>, String), String> {
    let data_dir = resolve_data_dir(app)?;
    let persisted = session_store.get_session(&data_dir, chat_session_id)?;
    let registry =
        app.try_state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>();
    Ok(resolve_spawn_info(persisted, registry.as_deref()))
}

/// 永続化セッションから spawn 情報を組み立てる純粋関数。
///
/// `selected_model == None` は registry の既定モデルへ解決する（モデル未選択状態は廃止）。
/// registry 未指定（テスト等）では `None` のままとする。
pub(crate) fn resolve_spawn_info(
    persisted: Option<ChatSession>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
) -> (Option<String>, Option<String>, String) {
    let (resume_sid, selected_model, backend_id) = persisted_spawn_info_from_session(persisted);
    let selected_model = resolve_selected_model(selected_model, &backend_id, registry);
    (resume_sid, selected_model, backend_id)
}

fn persisted_spawn_info_from_session(
    session: Option<ChatSession>,
) -> (Option<String>, Option<String>, String) {
    session
        .map(|s| {
            (
                s.agent_session_id,
                s.selected_model,
                s.backend_id
                    .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string()),
            )
        })
        .unwrap_or((None, None, CLAUDE_BACKEND_ID.to_string()))
}

pub(crate) async fn start_agent_session_internal<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    cwd: &str,
    permission_mode: Option<String>,
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
            let session = session_store
                .get_session(&data_dir, chat_session_id)?
                .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
            crate::permission::PermissionMode::parse(&session.permission_mode)
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

    let (resume_sid, selected_model, backend_id) =
        get_persisted_spawn_info(app, session_store, chat_session_id)?;

    if backend_id == CODEX_BACKEND_ID {
        let backend = codex_backend_from_app(app)?;
        backend
            .start_session(SessionConfig {
                chat_session_id: chat_session_id.to_string(),
                cwd: cwd.to_string(),
                permission_mode: Some(resolved_permission_mode),
                permission_profile_id: None,
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
        backend_id,
        resume_sid,
        cwd,
        resolved_permission_mode,
        selected_model,
        system_prompt,
    )
    .await
}

/// Core logic for starting a new agent turn: spawn Bridge if needed, send prompt.
/// Used by send_agent_message and pending message consumption.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_agent_turn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    cwd: &str,
    permission_mode: &str,
    prompt: &str,
    streaming_message_id: &str,
    images: &[ImageAttachment],
) -> Result<(), String> {
    let (_, _, backend_id) = get_persisted_spawn_info(app, session_store, chat_session_id)?;
    if backend_id == CODEX_BACKEND_ID {
        return start_codex_backend_turn(
            app,
            chat_session_id,
            permission_mode,
            prompt,
            streaming_message_id,
            images,
        )
        .await;
    }

    start_agent_turn_with_runtime_spawner(
        Some(app),
        handles,
        chat_session_id,
        permission_mode,
        prompt,
        streaming_message_id,
        images,
        || async {
            wait_until_session_close_finished(chat_session_id).await;
            let (resume_sid, selected_model, backend_id) =
                get_persisted_spawn_info(app, session_store, chat_session_id)?;

            spawn_bridge_process(
                app,
                handles,
                session_store,
                chat_session_id,
                backend_id,
                resume_sid,
                cwd,
                permission_mode.to_string(),
                selected_model,
                None,
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
    prompt: &str,
    streaming_message_id: &str,
    images: &[ImageAttachment],
) -> Result<(), String> {
    let (_, _, backend_id) = get_persisted_spawn_info(app, session_store, chat_session_id)?;
    if backend_id == CODEX_BACKEND_ID {
        return start_codex_backend_turn(
            app,
            chat_session_id,
            permission_mode,
            prompt,
            streaming_message_id,
            images,
        )
        .await;
    }

    start_agent_turn_with_runtime_spawner_locked(
        Some(app),
        handles,
        chat_session_id,
        permission_mode,
        prompt,
        streaming_message_id,
        images,
        || async {
            let (resume_sid, selected_model, backend_id) =
                get_persisted_spawn_info(app, session_store, chat_session_id)?;

            spawn_bridge_process(
                app,
                handles,
                session_store,
                chat_session_id,
                backend_id,
                resume_sid,
                cwd,
                permission_mode.to_string(),
                selected_model,
                None,
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
                permission_profile_id: None,
                editor_context: None,
            },
        )
        .await
}

#[allow(clippy::too_many_arguments)]
async fn start_agent_turn_with_runtime_spawner<R: tauri::Runtime, F, Fut>(
    app: Option<&tauri::AppHandle<R>>,
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
    let needs_spawn = {
        let mut map = handles.lock().await;
        match map.get(chat_session_id) {
            None => true,
            Some(proc) if proc.state == BridgeState::Crashed => {
                map.remove(chat_session_id);
                true
            }
            _ => false,
        }
    };

    if !needs_spawn {
        return Ok(());
    }

    let _spawn_guard = acquire_spawn_session_guard(chat_session_id).await;
    let needs_spawn_after_wait = {
        let mut map = handles.lock().await;
        match map.get(chat_session_id) {
            None => true,
            Some(proc) if proc.state == BridgeState::Crashed => {
                map.remove(chat_session_id);
                true
            }
            _ => false,
        }
    };
    if needs_spawn_after_wait {
        if let Err(e) = spawn_runtime().await {
            handles.lock().await.remove(chat_session_id);
            return Err(e);
        }
    }
    Ok(())
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

    // 人間メッセージを drain 時に永続化する（enqueue 時は二重表示防止のため追加しない）。
    let human_parts = pending_human_parts(&pending);
    let human_mentions = if pending.mentions.is_empty() {
        None
    } else {
        Some(pending.mentions.clone())
    };
    let human_msg = match add_message_internal(
        session_store,
        &data_dir,
        chat_session_id,
        MessageRole::Human,
        &pending.content,
        human_parts,
        human_mentions,
    ) {
        Ok(msg) => msg,
        Err(e) => {
            log::error!("consume_pending_message: failed to add human message: {e}");
            clear_pending_turn_starting(chat_session_id).await;
            return;
        }
    };
    let agent_msg = match add_message_internal(
        session_store,
        &data_dir,
        chat_session_id,
        MessageRole::Agent,
        "",
        None,
        None,
    ) {
        Ok(msg) => msg,
        Err(e) => {
            log::error!("consume_pending_message: failed to add agent message: {e}");
            clear_pending_turn_starting(chat_session_id).await;
            return;
        }
    };

    // 3. Emit event so UI can update with the new human + agent messages
    {
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
    let mut session = session_store
        .get_session(data_dir, chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    let backend_id = session
        .backend_id
        .clone()
        .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string());

    // モデルは必須。常に形式検証 + 固定リスト照合を通す（モデル未選択状態は廃止）。
    let model = model_id.as_str();
    crate::domain::agent_session::ModelId::parse(model)?;
    if let Some(reg) = registry {
        let session_models: Vec<String> = reg.config_models_for(&backend_id).map_err(|e| {
            log::warn!(
                "set_agent_model: backend '{backend_id}' の登録済みモデル一覧取得に失敗: {e}"
            );
            format!("バックエンド '{backend_id}' の登録済みモデル一覧を取得できません: {e}")
        })?;
        if !session_models.iter().any(|v| v == model) {
            // 「未登録」を伝える前に、別バックエンドに登録されていないかを問い合わせる。
            // - Ok(Some(other)) かつ other != current backend: backend mismatch として返す
            // - Ok(Some(same)) / Ok(None): 当該 backend への未登録として返す
            // - Err: infrastructure 故障。warn を残して当該 backend への未登録として返す
            //   （別バックエンドに登録されているかは判定できないため、ヒントは付けない）
            match reg.resolve_backend_for_model(model) {
                Ok(Some(bid)) if bid != backend_id => {
                    return Err(format!(
                        "モデル '{model}' はバックエンド '{backend_id}' に登録されていません (別バックエンド '{bid}' に登録)"
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
                "モデル '{model}' はバックエンド '{backend_id}' に登録されていません"
            ));
        }
    }

    // 1. Send setModel command to Bridge + update process state when the process is active.
    //    proc.available_models は config 単一 owner に追従させるため、active process が
    //    存在する場合も config 由来の最新値で同期する。
    //    infrastructure 故障時は Err を伝播し、proc キャッシュを空一覧で上書きしない。
    let models_from_config = available_models_for_backend(&backend_id, registry).map_err(|e| {
        log::warn!(
            "set_agent_model: backend '{backend_id}' のモデル一覧取得に失敗したため proc キャッシュ同期を中止: {e}"
        );
        format!("バックエンド '{backend_id}' のモデル一覧を取得できません: {e}")
    })?;
    sync_active_process_available_models(handles, chat_session_id, &models_from_config).await;
    set_active_process_model(handles, chat_session_id, model_id.clone()).await?;

    // 2. Persist to ChatSession
    session.selected_model = Some(model_id.clone());
    session.updated_at = now_timestamp();
    session_store.save_session(data_dir, &session)?;

    // 3. Always emit event to keep frontend in sync.
    //    供給元は常に config.toml（registry 経由）に統一する。
    if let Some(app) = app {
        use tauri::Emitter;
        let _ = app.emit(
            "agent-models-updated",
            build_agent_models_updated_payload(
                chat_session_id,
                &models_from_config,
                Some(model_id.as_str()),
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
) -> Result<u64, String> {
    crate::permission::PermissionMode::parse(&permission_mode).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    if let Some(pg) = pgid {
        let data_dir = resolve_data_dir(app).map_err(|e| {
            format!("Failed to resolve data dir for session {chat_session_id}: {e}")
        })?;
        save_pgid(&data_dir, chat_session_id, pg)?;
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
                child,
                generation_id: gen_id,
                #[cfg(unix)]
                pgid,
                streaming_message_id: None,
                streaming_parts: Vec::new(),
                last_message_id: None,
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
            },
        );
    }
    notify_status_transition(app, session_store, chat_session_id, TurnPhase::Idle, None);
    Ok(gen_id)
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
    // 人間メッセージを drain 時に永続化する（enqueue 時は二重表示防止のため追加しない）。
    // Claude 経路 (start_pending_message_turn) と同じ扱い。
    let human_parts = pending_human_parts(&pending);
    let human_mentions = if pending.mentions.is_empty() {
        None
    } else {
        Some(pending.mentions.clone())
    };
    let human_msg = match add_message_internal(
        session_store,
        &data_dir,
        chat_session_id,
        MessageRole::Human,
        &pending.content,
        human_parts,
        human_mentions,
    ) {
        Ok(msg) => msg,
        Err(e) => {
            clear_pending_turn_starting(chat_session_id).await;
            return Err(format!("failed to add pending human message: {e}"));
        }
    };
    let agent_msg = match add_message_internal(
        session_store,
        &data_dir,
        chat_session_id,
        MessageRole::Agent,
        "",
        None,
        None,
    ) {
        Ok(msg) => msg,
        Err(e) => {
            clear_pending_turn_starting(chat_session_id).await;
            return Err(format!("failed to add pending agent message: {e}"));
        }
    };
    let permission_profile_id = session_store
        .get_session(&data_dir, chat_session_id)?
        .and_then(|session| session.permission_profile_id);

    {
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
    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match msg_type {
        "session_ready" => {
            let mut map = handles.lock().await;
            if let Some(proc) = map.get_mut(chat_session_id) {
                if proc.state == BridgeState::Initializing {
                    proc.state = BridgeState::Ready;
                }
                if let Some(sid) = msg.get("session_id").and_then(|v| v.as_str()) {
                    proc.sdk_session_id = Some(sid.to_string());
                }
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
            let effect = {
                let _runtime_guard = acquire_session_runtime_lock(chat_session_id).await;
                let mut map = handles.lock().await;
                map.get_mut(chat_session_id).map(|proc| {
                    run_turn_complete_transition_locked(
                        proc,
                        chat_session_id,
                        exit_code,
                        |mid, parts| {
                            emit_streaming_parts(app, chat_session_id, mid, parts.to_vec())
                        },
                    )
                })
            };

            let Some(effect) = effect else {
                return;
            };
            let final_parts = consolidate_parts(effect.raw_parts);
            if effect.was_streaming {
                if let Some(ref mid) = effect.final_msg_id {
                    if !final_parts.is_empty() {
                        persist_streaming_parts(
                            session_store,
                            app,
                            chat_session_id,
                            mid,
                            &final_parts,
                        );
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
                let should_consume_pending_with_legacy_bridge = {
                    let map = handles.lock().await;
                    map.get(chat_session_id)
                        .is_some_and(|proc| proc.backend_id != CODEX_BACKEND_ID)
                };
                if should_consume_pending_with_legacy_bridge {
                    if let Some(pending) = take_pending_message(handles, chat_session_id).await {
                        start_pending_message_turn(
                            app,
                            handles,
                            session_store,
                            chat_session_id,
                            pending,
                        )
                        .await;
                    }
                }
            }
        }
        "error" => {
            {
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(chat_session_id) {
                    if proc.state == BridgeState::Streaming {
                        let prev_len = proc.streaming_parts.len();
                        accumulate_sdk_message(
                            &msg,
                            &mut proc.streaming_parts,
                            &mut proc.task_id_map,
                        );
                        let delta: Vec<MessagePart> = proc.streaming_parts[prev_len..].to_vec();
                        let mid = proc.streaming_message_id.clone();
                        if !delta.is_empty() {
                            enqueue_pending_delta(proc, &delta);
                        }
                        if let Some(ref mid) = mid {
                            flush_streaming(app, proc, chat_session_id, mid);
                        }
                    }
                    if proc.state == BridgeState::Streaming
                        || proc.state == BridgeState::Initializing
                    {
                        proc.state = BridgeState::Crashed;
                        proc.turn_phase = TurnPhase::Idle;
                    }
                }
            }
            let _ = app.emit("agent-sdk-message", &msg);
            emit_session_state_changed(app, chat_session_id, TurnPhase::Idle, Some(1));
            notify_status_transition(
                app,
                session_store,
                chat_session_id,
                TurnPhase::Idle,
                Some(crate::usecase::agent_session::session::SessionState::Error),
            );
        }
        _ => {
            let (accumulated, emit_msg_id, should_persist, raw_persist_parts) = {
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(chat_session_id) {
                    let in_streaming =
                        proc.state == BridgeState::Streaming && proc.streaming_message_id.is_some();
                    let post_turn = !in_streaming && proc.last_message_id.is_some();

                    if !in_streaming && !post_turn {
                        (false, None, false, Vec::new())
                    } else {
                        let prev_len = proc.streaming_parts.len();
                        let (acc, updated_parts) = accumulate_sdk_message(
                            &msg,
                            &mut proc.streaming_parts,
                            &mut proc.task_id_map,
                        );
                        if !acc {
                            (false, None, false, Vec::new())
                        } else {
                            let mut delta: Vec<MessagePart> =
                                proc.streaming_parts[prev_len..].to_vec();
                            if let Some(up) = updated_parts {
                                delta.extend(up);
                            }
                            let mid = if in_streaming {
                                proc.streaming_message_id.clone()
                            } else {
                                proc.last_message_id.clone()
                            };
                            enqueue_pending_delta(proc, &delta);
                            if should_flush_per_delta(proc, &delta, post_turn) {
                                if let Some(ref mid) = mid {
                                    flush_streaming(app, proc, chat_session_id, mid);
                                }
                            }
                            let elapsed_persist =
                                state.last_persist_time.elapsed().as_millis() as u64;
                            let should_persist =
                                post_turn || elapsed_persist >= PERSIST_INTERVAL_MS;
                            let raw_persist_parts = if should_persist {
                                proc.streaming_parts.clone()
                            } else {
                                Vec::new()
                            };
                            (true, mid, should_persist, raw_persist_parts)
                        }
                    }
                } else {
                    (false, None, false, Vec::new())
                }
            };

            if should_persist {
                if let Some(ref mid) = emit_msg_id {
                    state.last_persist_time = Instant::now();
                    let persist_parts = consolidate_parts(raw_persist_parts);
                    persist_streaming_parts(
                        session_store,
                        app,
                        chat_session_id,
                        mid,
                        &persist_parts,
                    );
                }
            }

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

            if should_forward_sdk_message(accumulated, msg_type) {
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
    prompt: String,
    agent_message_id: String,
    images: Vec<ImageAttachment>,
    editor_context: Option<AgentEditorContext>,
}

struct PreparedAgentSteer {
    session_id: String,
    backend_id: String,
    permission_mode: String,
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
    backend_id: Option<String>,
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
            .get_session(data_dir, sid)?
            .ok_or_else(|| format!("Session not found: {sid}"))?;
        if !session.workflow_step_session && session.worktree_path != worktree_path {
            return Err(session_target_rejected());
        }
        // 既存セッション分岐でも検証済み pm をセッション保存層に書き戻す。
        // 新規セッション分岐と対称化し、リモート UI で start → message とした場合に
        // 選択した permission_mode が ChatSession.permission_mode に反映されるようにする。
        if session.permission_mode != pm {
            session_store.update_permission_mode(data_dir, sid, &pm)?;
            session.permission_mode = pm.clone();
        }
        ensure_session_backend_selected(session_store, registry, data_dir, session)?
    } else {
        let resolved_backend_id = registry.resolve_backend_id(backend_id)?;
        // 新規セッションは検証済み抽象モードを初回保存で確定する。
        // 既定値で save → update_permission_mode の二段階保存を行うと、途中失敗時に
        // 選択値ではない permission_mode で永続化されたセッションが残ってしまうため
        // （Spec issues-947: セッション保存層が permission_mode の正典）、生成 API を一本化する。
        // backend の登録済み初期モデルがあれば selected_model に永続化する（Spec issues-946）。
        crate::usecase::agent_session::session::create_session_with_initial_model(
            session_store,
            registry,
            data_dir,
            &worktree_path,
            resolved_backend_id,
            permission_mode,
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
    let (current_phase, current_state) = {
        let map = handles.lock().await;
        map.get(&sid)
            .map(|p| (p.turn_phase, p.state))
            .unwrap_or((TurnPhase::Idle, BridgeState::Ready))
    };

    // turn_phase に加え、bridge 起動中 (Initializing) も busy として扱う。
    // 起動中は turn_phase がまだ Idle のため、これを見ないと「起動中の送信」を
    // 競合ターンとして即時起動してしまう。
    let is_initializing = current_state == BridgeState::Initializing;
    let active_turn_busy = current_phase == TurnPhase::Streaming
        || current_phase == TurnPhase::WaitingPermission
        || is_initializing;
    let pending_turn_starting = is_pending_turn_starting(&sid).await;
    let turn_busy = active_turn_busy || pending_turn_starting;

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
                images: images.clone(),
                worktree_path: session_worktree_path.clone(),
                mentions: mentions.clone(),
                editor_context: editor_context.clone(),
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
            interrupt_active_agent_turn(handles, registry, &sid).await?;
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

    // 5. Get updated session and list
    let updated_session = session_store
        .get_session(data_dir, &sid)?
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
    backend_id: Option<String>,
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
            backend_id,
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

pub(crate) async fn init_agent_sessions_internal(
    app: &tauri::AppHandle,
    session_store: &Arc<SessionStore>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &Arc<crate::usecase::agent_session::session::OpenTabRegistry>,
    worktree_path: String,
) -> Result<InitSessionsResponse, String> {
    let data_dir = resolve_data_dir(app)?;

    crate::workflow_step_lifecycle_adapters::hydrate_open_workflow_step_tabs(
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
        })
    } else {
        // Start agent processes only for sessions that have already sent a turn or have
        // a resumable SDK session. Empty NewSession entries must remain selectable.
        // Sequential execution is required because the bridge process path makes the
        // Future !Send and incompatible with tokio::spawn.
        for s in &sessions {
            if !should_start_agent_process_for_summary(s) {
                continue;
            }
            if let Err(e) =
                start_existing_session_for_summary(app, handles, session_store, registry, s, None)
                    .await
            {
                log::error!("Failed to start agent session {}: {e}", s.id);
            }
        }

        // spec issues-1023: workflow step として起動された chat session は free chat
        // tab bar 上に同格に並ばないため、初期 active session 候補からも除外する。
        // 候補が無い場合は active_session を None で返し、UI は空状態を描く。
        let active_candidate = pick_initial_active_session_candidate(&sessions);
        let active = if let Some(candidate) = active_candidate {
            get_session_internal(session_store, handles, Some(registry), app, &candidate.id).await?
        } else {
            None
        };

        Ok(InitSessionsResponse {
            sessions,
            active_session: active,
        })
    }
}

async fn start_existing_session_for_summary(
    app: &tauri::AppHandle,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    registry: &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    session: &SessionSummary,
    system_prompt: Option<String>,
) -> Result<(), String> {
    let backend_id = match session.backend_id.as_deref() {
        Some(id) => id.to_string(),
        None => registry.resolve_default_id()?,
    };
    if backend_id == CODEX_BACKEND_ID {
        let backend = registry
            .get(&backend_id)
            .ok_or_else(|| format!("Agent backend not found: {backend_id}"))?;
        backend
            .start_session(SessionConfig {
                chat_session_id: session.id.clone(),
                cwd: session.worktree_path.clone(),
                permission_mode: Some(session.permission_mode.clone()),
                permission_profile_id: session.permission_profile_id.clone(),
                system_prompt,
            })
            .await?;
        return Ok(());
    }

    start_agent_session_internal(
        app,
        handles,
        session_store,
        &session.id,
        &session.worktree_path,
        Some(session.permission_mode.clone()),
        system_prompt,
    )
    .await
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

/// 抽象モード文字列 + backend_id を受け取り、バックエンド固有の init コマンドを構築する。
fn build_init_cmd(
    cwd: &str,
    permission_mode: &str,
    session_id: &Option<String>,
    system_prompt: Option<String>,
    backend_id: &str,
) -> Result<serde_json::Value, String> {
    let pm =
        crate::permission::PermissionMode::parse(permission_mode).map_err(|e| e.to_string())?;
    let mut cmd = serde_json::json!({
        "type": "init",
        "cwd": cwd,
        "sessionId": session_id,
    });
    if let Some(obj) = cmd.as_object_mut() {
        for (k, v) in bridge_permission_fields(pm, backend_id) {
            obj.insert(k, v);
        }
    }
    if let Some(sp) = system_prompt {
        cmd["systemPrompt"] = serde_json::Value::String(sp);
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

/// Maximum image size in bytes (5 MiB).
/// Anthropic Messages API limits base64-encoded images to ~5 MB.
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// Validate and encode an image from raw bytes.
/// Returns base64-encoded data and detected MIME type, or an error for unsupported formats.
fn validate_and_encode_image(bytes: &[u8]) -> Result<ImageAttachment, String> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "Image too large: {} bytes (max {} bytes)",
            bytes.len(),
            MAX_IMAGE_BYTES
        ));
    }

    let media_type =
        detect_image_mime(bytes).ok_or_else(|| "Unsupported image format".to_string())?;

    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);

    Ok(ImageAttachment {
        data,
        media_type: media_type.to_string(),
    })
}

/// Detect MIME type from magic bytes.
fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 4 {
        return None;
    }
    // JPEG: FF D8 FF
    if bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some("image/jpeg");
    }
    // PNG: 89 50 4E 47
    if bytes[0] == 0x89 && bytes[1] == 0x50 && bytes[2] == 0x4E && bytes[3] == 0x47 {
        return Some("image/png");
    }
    // GIF: 47 49 46 38
    if bytes[0] == 0x47 && bytes[1] == 0x49 && bytes[2] == 0x46 && bytes[3] == 0x38 {
        return Some("image/gif");
    }
    // WebP: RIFF....WEBP
    if bytes.len() >= 12
        && bytes[0] == 0x52
        && bytes[1] == 0x49
        && bytes[2] == 0x46
        && bytes[3] == 0x46
        && bytes[8] == 0x57
        && bytes[9] == 0x45
        && bytes[10] == 0x42
        && bytes[11] == 0x50
    {
        return Some("image/webp");
    }
    None
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
        prompt,
        &agent_msg.id,
        &[],
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::agent_session::runtime::{
        AgentBackend, AgentBackendRegistry, AgentMessage, PermissionResponse, SessionConfig,
        SessionHandle,
    };
    use crate::workflow::engine::WorkflowEngine;
    use crate::workflow::state::WorkflowExecutionState;
    use async_trait::async_trait;

    fn approved_fix_policy_output(policy: &str, review_step: &str) -> String {
        format!(
            r#"<workflow_output type="approved-fix-policy">{{"policy":"{policy}","review_step":"{review_step}"}}</workflow_output>"#
        )
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
            permission_mode: "edit".to_string(),
            permission_profile_id: None,
            selected_model: Some("sonnet".to_string()),
            backend_id: Some("mock".to_string()),
            workflow_step_session: true,
        }
    }

    #[test]
    fn persisted_spawn_info_uses_step_agent_session_id_for_resume() {
        let (resume_id, selected_model, backend_id) =
            persisted_spawn_info_from_session(Some(chat_session_for_spawn_info("step")));

        assert_eq!(resume_id.as_deref(), Some("sdk-resume-id"));
        assert_eq!(selected_model.as_deref(), Some("sonnet"));
        assert_eq!(backend_id, "mock");
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
        proc.task_id_map
            .insert("task".to_string(), "tool".to_string());

        proc.reset_streaming_state_for_new_turn();

        assert!(proc.streaming_parts.is_empty());
        assert_eq!(proc.pending_stream_part_count, 0);
        assert_eq!(proc.pending_stream_bytes, 0);
        assert!(proc.last_stream_emit_at.is_none());
        assert!(proc.last_message_id.is_none());
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
        let keep_running = run_streaming_timer_tick(&mut proc, "csid", |mid, parts| {
            emitted.push((mid.to_string(), parts.to_vec()));
            (true, true)
        });

        assert!(keep_running, "still streaming → timer continues");
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
        let keep_running = run_streaming_timer_tick(&mut proc, "csid", |_mid, _parts| {
            emitted = true;
            (true, true)
        });
        assert!(keep_running);
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
        let keep_running = run_streaming_timer_tick(&mut proc, "csid", |_mid, _parts| {
            emitted = true;
            (true, true)
        });
        // pending=0 & still Streaming → continue running but no flush this tick.
        assert!(keep_running);
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

        let keep_running = run_streaming_timer_tick(&mut proc, "csid", |_mid, _parts| (true, true));
        assert!(
            !keep_running,
            "turn ended (state != Streaming) and buffer empty → timer must exit"
        );
    }

    #[tokio::test]
    async fn timer_drains_pending_even_after_turn_ended() {
        // turn 終了直後でも pending が残っていれば drain してから終了する次の
        // tick で keep_running=false を返す。
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
        let keep_running = run_streaming_timer_tick(&mut proc, "csid", |_mid, _parts| {
            emitted += 1;
            (true, true)
        });
        assert!(
            keep_running,
            "pending still > 0 → timer continues (will exit next tick when drained)"
        );
        assert_eq!(emitted, 1, "tail content flushed before exit");
        assert_eq!(proc.pending_stream_part_count, 0);

        let keep_running = run_streaming_timer_tick(&mut proc, "csid", |_mid, _parts| (true, true));
        assert!(!keep_running, "post-drain tick → timer exits");
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
                    | Some(MessagePart::Thinking { content, .. }) => Some(content.clone()),
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

    /// Drive the production `bridge error` lock-block: accumulate an error
    /// part, enqueue it as a pending delta, force-flush via the same
    /// `force_flush_pending_streaming` helper the production reader uses,
    /// then push a `StateChanged(Idle)` event to mirror the post-lock
    /// `emit_session_state_changed`. Mirrors `bridge_common.rs:1982-2038`.
    fn drive_bridge_error_path(
        proc: &mut AgentProcess,
        chat_session_id: &str,
        error_part: MessagePart,
        events: &mut Vec<RecordedEmit>,
    ) {
        let was_streaming = proc.state == BridgeState::Streaming;
        let mid = proc.streaming_message_id.clone();
        if was_streaming {
            proc.streaming_parts.push(error_part.clone());
            enqueue_pending_delta(proc, std::slice::from_ref(&error_part));
            if let Some(ref mid) = mid {
                force_flush_pending_streaming(proc, chat_session_id, mid, |parts| {
                    events.push(RecordedEmit::StreamingFlush {
                        parts_count: parts.len(),
                        tail_text: match parts.last() {
                            Some(MessagePart::Error { content, .. })
                            | Some(MessagePart::Text { content, .. })
                            | Some(MessagePart::Thinking { content, .. }) => Some(content.clone()),
                            _ => None,
                        },
                    });
                    (true, true)
                });
            }
            // Mirror production: state transitions to Crashed → Idle after flush.
            proc.state = BridgeState::Crashed;
            proc.turn_phase = TurnPhase::Idle;
            events.push(RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(1),
            });
        }
    }

    /// Drive the production `EOF crash` lock-block: run
    /// `apply_bridge_eof_crash`, enqueue the synthetic error delta, force-flush
    /// via the same helper the production reader uses, then push a
    /// `StateChanged(Idle)` event to mirror `emit_session_state_changed`.
    /// Mirrors `bridge_common.rs:2268-2333`.
    fn drive_bridge_eof_crash_path(
        proc: &mut AgentProcess,
        chat_session_id: &str,
        events: &mut Vec<RecordedEmit>,
    ) {
        let streaming_message_id = proc.streaming_message_id.clone();
        let backend_id = proc.backend_id.clone();
        let effect = apply_bridge_eof_crash(
            true,
            &mut proc.state,
            &mut proc.turn_phase,
            streaming_message_id.as_deref(),
            &mut proc.streaming_parts,
            &backend_id,
        );
        if let (Some(mid), false) = (streaming_message_id.as_ref(), effect.error_delta.is_empty()) {
            enqueue_pending_delta(proc, &effect.error_delta);
            force_flush_pending_streaming(proc, chat_session_id, mid, |parts| {
                events.push(RecordedEmit::StreamingFlush {
                    parts_count: parts.len(),
                    tail_text: match parts.last() {
                        Some(MessagePart::Error { content, .. })
                        | Some(MessagePart::Text { content, .. })
                        | Some(MessagePart::Thinking { content, .. }) => Some(content.clone()),
                        _ => None,
                    },
                });
                (true, true)
            });
        }
        if effect.was_streaming {
            events.push(RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(-1),
            });
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

        let error_part = MessagePart::Error {
            content: "Error: bridge reported failure".to_string(),
            parent_tool_use_id: None,
        };
        let mut events = Vec::new();
        drive_bridge_error_path(&mut proc, "csid", error_part, &mut events);

        assert_eq!(events.len(), 2, "flush emit then state emit");
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
                // cumulative: pending Text + apply_bridge_eof_crash が積んだ Error。
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

    #[test]
    fn bridge_eof_crash_adds_error_part_for_streaming_message() {
        let mut state = BridgeState::Streaming;
        let mut turn_phase = TurnPhase::Streaming;
        let mut parts = vec![MessagePart::Text {
            content: "partial".to_string(),
            parent_tool_use_id: None,
        }];

        let effect = apply_bridge_eof_crash(
            true,
            &mut state,
            &mut turn_phase,
            Some("message-1"),
            &mut parts,
            "mock",
        );

        assert_eq!(state, BridgeState::Crashed);
        assert_eq!(turn_phase, TurnPhase::Idle);
        assert!(effect.was_streaming);
        assert_eq!(effect.message_id.as_deref(), Some("message-1"));
        assert_eq!(effect.error_delta.len(), 1);
        assert_eq!(effect.persisted_parts.len(), 2);
        assert!(effect
            .sdk_error_message
            .as_deref()
            .unwrap()
            .contains("mock"));
        assert!(matches!(
            &effect.error_delta[0],
            MessagePart::Error { content, .. }
                if content.contains("Bridge process exited unexpectedly")
        ));
    }

    #[test]
    fn bridge_eof_crash_marks_initializing_without_streaming_part() {
        let mut state = BridgeState::Initializing;
        let mut turn_phase = TurnPhase::Idle;
        let mut parts = Vec::new();

        let effect =
            apply_bridge_eof_crash(true, &mut state, &mut turn_phase, None, &mut parts, "mock");

        assert_eq!(state, BridgeState::Crashed);
        assert_eq!(turn_phase, TurnPhase::Idle);
        assert!(effect.was_initializing);
        assert!(effect.error_delta.is_empty());
        assert!(effect.persisted_parts.is_empty());
        assert!(effect.sdk_error_message.is_some());
    }

    #[test]
    fn available_models_for_backend_reads_from_config_via_registry() {
        let mut cfg = crate::config::ReleashConfig::default();
        cfg.agents.claude.models = vec!["mock-model".to_string()];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::config::AppConfig::new(cfg, tmp.path().to_path_buf()));

        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: CLAUDE_BACKEND_ID.to_string(),
            models: vec![],
        }));
        registry.set_config(config);
        let registry = Arc::new(registry);

        let models = available_models_for_backend(CLAUDE_BACKEND_ID, Some(&registry)).unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].value, "mock-model");
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
        let session_store = Arc::new(SessionStore::default());
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
            .save_session(data_dir.path(), &step_session)
            .unwrap();
        let parent_session = create_session_internal(
            &session_store,
            data_dir.path(),
            &worktree_path,
            Some("mock".to_string()),
        )
        .unwrap();

        session_store
            .save_session(data_dir.path(), &parent_session)
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
            .get_session(data_dir.path(), &step_session.id)
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

    fn workflow_state_for_runtime_test(session_id: &str) -> crate::workflow::state::WorkflowState {
        crate::workflow::state::WorkflowState {
            execution_id: "exec-runtime".to_string(),
            workflow_name: "wf".to_string(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            current_step_name: "step".to_string(),
            current_session_id: Some(session_id.to_string()),
            total_steps: 1,
            step_history: Vec::new(),
            step_execution_counts: HashMap::new(),
            workflow_definition: crate::workflow::schema::Workflow {
                variables: Default::default(),
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                nodes: vec![],
            },
            total_token_usage: crate::workflow::state::TokenUsage::default(),
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
        let session_store = Arc::new(SessionStore::default());
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
            .save_session(data_dir.path(), &step_session)
            .unwrap();
        handles
            .lock()
            .await
            .insert(step_session.id.clone(), make_test_agent_process());

        let before = crate::workflow_state_events::build_workflow_state_projection(
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
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(prepared_turn.is_some());
        assert!(handles.lock().await.contains_key(&step_session.id));
        let after = crate::workflow_state_events::build_workflow_state_projection(
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
        let session_store = Arc::new(SessionStore::default());
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
            Some(CLAUDE_BACKEND_ID.to_string()),
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
            .get_session(data_dir.path(), &response.session.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.permission_mode, "ask");
    }

    #[tokio::test]
    async fn prepared_turn_carries_codex_backend_for_runtime_dispatch() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
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
            Some(CODEX_BACKEND_ID.to_string()),
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
    async fn busy_send_uses_active_turn_steer_when_backend_is_ready() {
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
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
        let session_store = Arc::new(SessionStore::default());
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
            .save_session(data_dir.path(), &step_session)
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
    async fn stopped_workflow_step_turn_start_spawns_resume_runtime_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = "stopped-step".to_string();
        let spawn_count = Arc::new(AtomicUsize::new(0));

        start_agent_turn_with_runtime_spawner(
            None::<&tauri::AppHandle>,
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
        let session_store = Arc::new(SessionStore::default());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();

        let mut cfg = crate::config::ReleashConfig::default();
        cfg.agents.claude.models = vec!["mock-model".to_string()];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::config::AppConfig::new(cfg, tmp.path().to_path_buf()));

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
        assert_eq!(response.available_models[0].value, "mock-model");
    }

    #[tokio::test]
    async fn set_session_backend_updates_unstarted_session_and_models() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.selected_model = Some("old-model".to_string());
        session_store.save_session(temp.path(), &session).unwrap();

        let mut cfg = crate::config::ReleashConfig::default();
        cfg.agents.claude.models = vec!["a-model".to_string()];
        cfg.agents.codex.models = vec!["b-model".to_string()];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::config::AppConfig::new(cfg, tmp.path().to_path_buf()));

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
        assert_eq!(response.session.selected_model, Some("b-model".to_string()));
        assert_eq!(response.available_models.len(), 1);
        assert_eq!(response.available_models[0].value, "b-model");
    }

    #[tokio::test]
    async fn set_session_backend_rejects_session_with_messages() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
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
        let session_store = Arc::new(SessionStore::default());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some("mock-a".to_string()),
        )
        .unwrap();
        session.agent_session_id = Some("sdk-session".to_string());
        session_store.save_session(temp.path(), &session).unwrap();
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
        let session_store = Arc::new(SessionStore::default());
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
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
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
                child,
                generation_id: 0,
                #[cfg(unix)]
                pgid: None,
                streaming_message_id: None,
                streaming_parts: Vec::new(),
                last_message_id: None,
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
                .get_session(data_dir.path(), &session.id)
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
            session_store.save_session(data_dir.path(), &saved).unwrap();
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
            .get_session(data_dir.path(), &session.id)
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
        // contract 検証経路の回帰テストは `workflow::contract` の単体テスト群および
        // `workflow::engine` の SubmitOutput 経路テストで別途カバーする。
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
        let session_store = SessionStore::default();
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
            .get_session(temp.path(), &updated.id)
            .unwrap()
            .unwrap();
        assert_eq!(persisted.backend_id, Some("mock-default".to_string()));
    }

    #[test]
    fn should_start_agent_process_for_summary_skips_unstarted_session() {
        let session = SessionSummary {
            id: "empty".to_string(),
            worktree_path: "/repo".to_string(),
            state: crate::usecase::agent_session::session::SessionState::Active,
            created_at: 1.0,
            updated_at: 1.0,
            first_message: String::new(),
            message_count: 0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            permission_profile_id: None,
            backend_id: Some("claude".to_string()),
            workflow_step_session: false,
        };

        assert!(!should_start_agent_process_for_summary(&session));
    }

    #[test]
    fn should_start_agent_process_for_summary_starts_sent_or_resumable_session() {
        let mut session = SessionSummary {
            id: "sent".to_string(),
            worktree_path: "/repo".to_string(),
            state: crate::usecase::agent_session::session::SessionState::Active,
            created_at: 1.0,
            updated_at: 1.0,
            first_message: "hello".to_string(),
            message_count: 1,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            permission_profile_id: None,
            backend_id: Some("claude".to_string()),
            workflow_step_session: false,
        };
        assert!(should_start_agent_process_for_summary(&session));

        session.message_count = 0;
        session.agent_session_id = Some("sdk-session".to_string());
        assert!(should_start_agent_process_for_summary(&session));
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
                permission_mode: "edit".to_string(),
                permission_profile_id: None,
                backend_id: Some("claude".to_string()),
                workflow_step_session: workflow_step,
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

    #[test]
    fn should_start_agent_process_for_summary_skips_workflow_step_session() {
        let session = SessionSummary {
            id: "step".to_string(),
            worktree_path: "/repo".to_string(),
            state: crate::usecase::agent_session::session::SessionState::Idle,
            created_at: 1.0,
            updated_at: 1.0,
            first_message: "history".to_string(),
            message_count: 3,
            agent_session_id: Some("sdk-session".to_string()),
            permission_mode: "edit".to_string(),
            permission_profile_id: None,
            backend_id: Some("claude".to_string()),
            workflow_step_session: true,
        };

        assert!(!should_start_agent_process_for_summary(&session));
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
                images: Vec::new(),
                worktree_path: "/repo".to_string(),
                mentions: Vec::new(),
                editor_context: None,
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
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
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
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
        });
        proc.pending_messages.push_back(PendingMessage {
            id: "queued-2".to_string(),
            content: "second".to_string(),
            created_at: 2.0,
            permission_mode: "ask".to_string(),
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
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
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
        });
        proc.pending_messages.push_back(PendingMessage {
            id: "drop".to_string(),
            content: "second".to_string(),
            created_at: 2.0,
            permission_mode: "ask".to_string(),
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
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
            &Some("sess-abc".to_string()),
            None,
            CLAUDE_BACKEND_ID,
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
        let cmd = build_init_cmd("/repo", "edit", &None, None, CLAUDE_BACKEND_ID).unwrap();
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
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::Error { content, .. } => {
                assert!(content.contains("Something went wrong"));
            }
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
                assert_eq!(notification_type, "compaction");
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
            notification_type: "compaction".to_string(),
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
                assert_eq!(notification_type, "compaction");
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
    fn test_accumulate_hook_started() {
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "hook_started",
            "hook_name": "SessionEnd",
            "hook_event": "StopSession",
            "hook_id": "hook-001"
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::SystemNotification {
                notification_type,
                status,
                label,
                hook_id,
                ..
            } => {
                assert_eq!(notification_type, "hook");
                assert_eq!(status, "in_progress");
                assert_eq!(label, "SessionEnd (StopSession)");
                assert_eq!(hook_id.as_deref(), Some("hook-001"));
            }
            _ => panic!("expected SystemNotification"),
        }
    }

    #[test]
    fn test_accumulate_hook_response_updates_existing() {
        let mut parts = vec![MessagePart::SystemNotification {
            notification_type: "hook".to_string(),
            status: "in_progress".to_string(),
            label: "SessionEnd (StopSession)".to_string(),
            detail: None,
            hook_id: Some("hook-001".to_string()),
        }];
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "hook_response",
            "hook_id": "hook-001",
            "outcome": "success",
            "exit_code": 0
        });
        let (handled, updated) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert!(updated.is_some());
        assert_eq!(parts.len(), 1); // Updated in-place
        match &parts[0] {
            MessagePart::SystemNotification { status, detail, .. } => {
                assert_eq!(status, "completed");
                assert!(detail.as_ref().unwrap().contains("outcome=success"));
                assert!(detail.as_ref().unwrap().contains("exit_code=0"));
            }
            _ => panic!("expected SystemNotification"),
        }
    }

    #[test]
    fn test_accumulate_hook_response_error_status() {
        let mut parts = vec![MessagePart::SystemNotification {
            notification_type: "hook".to_string(),
            status: "in_progress".to_string(),
            label: "PreCompact (Compact)".to_string(),
            detail: None,
            hook_id: Some("hook-err".to_string()),
        }];
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "hook_response",
            "hook_id": "hook-err",
            "outcome": "error",
            "exit_code": 1
        });
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        match &parts[0] {
            MessagePart::SystemNotification { status, .. } => {
                assert_eq!(status, "error");
            }
            _ => panic!("expected SystemNotification"),
        }
    }

    #[test]
    fn test_accumulate_files_persisted() {
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "files_persisted",
            "filePaths": ["CLAUDE.md", "src/main.rs"]
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::SystemNotification {
                notification_type,
                status,
                label,
                detail,
                ..
            } => {
                assert_eq!(notification_type, "files_persisted");
                assert_eq!(status, "completed");
                assert_eq!(label, "Files persisted");
                assert_eq!(detail.as_deref(), Some("CLAUDE.md, src/main.rs"));
            }
            _ => panic!("expected SystemNotification"),
        }
    }

    #[test]
    fn test_accumulate_local_command_output() {
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "local_command_output",
            "content": "npm test output here"
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::SystemNotification {
                notification_type,
                status,
                label,
                detail,
                ..
            } => {
                assert_eq!(notification_type, "local_command_output");
                assert_eq!(status, "completed");
                assert_eq!(label, "Command output");
                assert_eq!(detail.as_deref(), Some("npm test output here"));
            }
            _ => panic!("expected SystemNotification"),
        }
    }

    #[test]
    fn test_accumulate_local_command_output_truncates_long_content() {
        let long_content = "a".repeat(300);
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "local_command_output",
            "content": long_content
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        match &parts[0] {
            MessagePart::SystemNotification { detail, .. } => {
                let d = detail.as_ref().unwrap();
                assert!(d.len() <= 200 + "…".len());
                assert!(d.ends_with('…'));
            }
            _ => panic!("expected SystemNotification"),
        }
    }

    #[test]
    fn test_accumulate_local_command_output_truncates_multibyte() {
        let long_content = "あ".repeat(201);
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "local_command_output",
            "content": long_content
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        match &parts[0] {
            MessagePart::SystemNotification { detail, .. } => {
                let d = detail.as_ref().unwrap();
                assert!(d.ends_with('…'));
                let without_ellipsis = d.trim_end_matches('…');
                assert_eq!(without_ellipsis.chars().count(), 200);
            }
            _ => panic!("expected SystemNotification"),
        }
    }

    #[test]
    fn test_accumulate_local_command_output_no_truncate_short_multibyte() {
        let content = "あ".repeat(100);
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "local_command_output",
            "content": content
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        match &parts[0] {
            MessagePart::SystemNotification { detail, .. } => {
                let d = detail.as_ref().unwrap();
                assert!(!d.ends_with('…'));
                assert_eq!(d.chars().count(), 100);
            }
            _ => panic!("expected SystemNotification"),
        }
    }

    #[test]
    fn test_accumulate_codex_realtime_notification() {
        let msg = serde_json::json!({
            "type": "system",
            "subtype": "codex_realtime",
            "notification_type": "codex_realtime",
            "status": "in_progress",
            "label": "Codex realtime started",
            "detail": "thread=thr_123, version=v2"
        });
        let mut parts = vec![];
        let (handled, _) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
        assert!(handled);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            MessagePart::SystemNotification {
                notification_type,
                status,
                label,
                detail,
                hook_id,
            } => {
                assert_eq!(notification_type, "codex_realtime");
                assert_eq!(status, "in_progress");
                assert_eq!(label, "Codex realtime started");
                assert_eq!(detail.as_deref(), Some("thread=thr_123, version=v2"));
                assert_eq!(*hook_id, None);
            }
            _ => panic!("expected SystemNotification"),
        }
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
        let result = consolidate_parts(parts);
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
        let result = consolidate_parts(parts);
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
        let result = consolidate_parts(parts);
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
        let result = consolidate_parts(parts);
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
        let result = consolidate_parts(parts);
        assert_eq!(result.len(), 1);
        match &result[0] {
            MessagePart::Text { content, .. } => assert_eq!(content, "abc"),
            _ => panic!("expected Text"),
        }
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
        let cmd = build_init_cmd("/repo", "edit", &None, None, CLAUDE_BACKEND_ID).unwrap();
        assert_eq!(cmd["type"], "init");
        assert_eq!(cmd["cwd"], "/repo");
        assert_eq!(cmd["permissionMode"], "acceptEdits");
        assert!(cmd["sessionId"].is_null());
        assert!(cmd.get("systemPrompt").is_none());
    }

    #[test]
    fn build_init_cmd_with_system_prompt_for_claude() {
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            &Some("prev-session".to_string()),
            Some("You are a coder.".to_string()),
            CLAUDE_BACKEND_ID,
        )
        .unwrap();
        assert_eq!(cmd["type"], "init");
        assert_eq!(cmd["cwd"], "/repo");
        assert_eq!(cmd["permissionMode"], "acceptEdits");
        assert_eq!(cmd["sessionId"], "prev-session");
        assert_eq!(cmd["systemPrompt"], "You are a coder.");
    }

    #[test]
    fn build_init_cmd_full_for_claude_emits_bypass_permissions() {
        let cmd = build_init_cmd("/repo", "full", &None, None, CLAUDE_BACKEND_ID).unwrap();
        assert_eq!(cmd["permissionMode"], "bypassPermissions");
        assert!(cmd.get("systemPrompt").is_none());
    }

    #[test]
    fn build_init_cmd_ask_for_claude_emits_default() {
        let cmd = build_init_cmd("/repo", "ask", &None, None, CLAUDE_BACKEND_ID).unwrap();
        assert_eq!(cmd["permissionMode"], "default");
    }

    #[test]
    fn build_init_cmd_for_codex_emits_sandbox_and_approval() {
        let cmd = build_init_cmd("/repo", "edit", &None, None, CODEX_BACKEND_ID).unwrap();
        assert_eq!(cmd["type"], "init");
        assert_eq!(cmd["sandboxMode"], "workspace-write");
        assert_eq!(cmd["approvalPolicy"], "on-request");
        // Codex 用 init には permissionMode は載らない（バックエンド固有フラグのみ）
        assert!(cmd.get("permissionMode").is_none());
    }

    #[test]
    fn build_init_cmd_for_codex_ask_and_full() {
        let ask = build_init_cmd("/repo", "ask", &None, None, CODEX_BACKEND_ID).unwrap();
        assert_eq!(ask["sandboxMode"], "read-only");
        assert_eq!(ask["approvalPolicy"], "on-request");
        let full = build_init_cmd("/repo", "full", &None, None, CODEX_BACKEND_ID).unwrap();
        assert_eq!(full["sandboxMode"], "danger-full-access");
        assert_eq!(full["approvalPolicy"], "never");
    }

    #[test]
    fn build_init_cmd_rejects_invalid_abstract_mode() {
        assert!(
            build_init_cmd("/repo", "acceptEdits", &None, None, CLAUDE_BACKEND_ID).is_err(),
            "legacy claude flag must be rejected at the boundary"
        );
        assert!(build_init_cmd("/repo", "plan", &None, None, CLAUDE_BACKEND_ID).is_err());
        assert!(build_init_cmd("/repo", "", &None, None, CODEX_BACKEND_ID).is_err());
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
            permission_mode: permission.to_string(),
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some("mock".to_string()),
            workflow_step_session: false,
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
            child,
            generation_id: 0,
            #[cfg(unix)]
            pgid: None,
            streaming_message_id: None,
            streaming_parts: Vec::new(),
            last_message_id: None,
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
        };
        (proc, stdout)
    }

    #[tokio::test]
    async fn set_agent_permission_mode_internal_rejects_invalid_without_mutating_state() {
        use tokio::io::AsyncReadExt;

        // Spec issues-947: 外部境界（set_agent_permission_mode 相当）で invalid 値を受けたとき、
        // 保存値・current_permission_mode・bridge stdin のいずれも変化させない。
        // bridge stdin の不変は、stdout を pipe で開いた `cat` を bridge process に見立てて
        // 「invalid を拒否した後で stdin を閉じ、stdout の echo が空である」ことで観測する。
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = chat_session_for_permission_test(&session_id, "edit");
        session_store
            .save_session(data_dir.path(), &session)
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
                .get_session(data_dir.path(), &session_id)
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
        let session_store = Arc::new(SessionStore::default());
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = chat_session_for_permission_test(&session_id, "edit");
        session_store
            .save_session(data_dir.path(), &session)
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
            .get_session(data_dir.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.permission_mode, "ask");
    }

    #[tokio::test]
    async fn prepare_send_persists_selected_permission_mode_for_existing_session() {
        // Spec issues-947: 既存セッションに対する送信時にも、検証済み permission_mode が
        // 異なれば ChatSession.permission_mode に書き戻される（保存層が単一の正典）。
        let data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
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
            .save_session(data_dir.path(), &session)
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
            Some("mock".to_string()),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(response.session.permission_mode, "ask");
        let saved = session_store
            .get_session(data_dir.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.permission_mode, "ask");
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
                child,
                generation_id: 1,
                pgid,
                streaming_message_id: None,
                streaming_parts: Vec::new(),
                last_message_id: None,
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
                proc.available_models = vec![ModelInfo {
                    value: "gpt-5.4".to_string(),
                }];
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
                assert_eq!(proc.available_models[0].value, "gpt-5.4");
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
            let session_store = Arc::new(SessionStore::default());
            let session = create_session_internal(
                &session_store,
                temp.path(),
                "/repo",
                Some(CLAUDE_BACKEND_ID.to_string()),
            )
            .unwrap();

            let mut cfg = crate::config::ReleashConfig::default();
            cfg.agents.codex.models = vec!["b-model".to_string()];
            let tmp = tempfile::NamedTempFile::new().unwrap();
            let config = Arc::new(crate::config::AppConfig::new(cfg, tmp.path().to_path_buf()));

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
            assert_eq!(response.available_models[0].value, "b-model");
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
        let mut cfg = crate::config::ReleashConfig::default();
        cfg.agents.claude.models = claude_models.iter().map(|s| s.to_string()).collect();
        cfg.agents.codex.models = codex_models.iter().map(|s| s.to_string()).collect();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::config::AppConfig::new(cfg, tmp.path().to_path_buf()));
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
        let session_store = Arc::new(SessionStore::default());
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
            .get_session(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some("claude-4".to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_preserves_surrounding_whitespace() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
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
            .get_session(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some(model.to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_unregistered_model() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.selected_model = Some("existing".to_string());
        session_store.save_session(temp.path(), &session).unwrap();

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
            .get_session(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, Some("existing".to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_other_backend_model_as_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
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
        assert!(err.contains("別バックエンド"));

        // selected_model は変更されない
        let after = session_store
            .get_session(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, None);
    }

    #[tokio::test]
    async fn set_agent_model_rejects_empty_model() {
        // モデルは必須。空文字は形式不正として登録判定の前に拒否する。
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.selected_model = Some("claude-4".to_string());
        session_store.save_session(temp.path(), &session).unwrap();

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
            .get_session(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, Some("claude-4".to_string()));
    }

    // --- set_agent_model: 実 backend の固定リスト検証 ---

    fn make_fixed_model_registry() -> Arc<AgentBackendRegistry> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::config::AppConfig::new(
            crate::config::ReleashConfig::default(),
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
        let session_store = Arc::new(SessionStore::default());
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
            .get_session(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some(model));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_model_outside_claude_fixed_list() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
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
            .get_session(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, None);
    }

    #[tokio::test]
    async fn set_agent_model_accepts_codex_fixed_model() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
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
            .get_session(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some(model));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_model_outside_codex_fixed_list() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
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
            .get_session(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, None);
    }

    #[test]
    fn build_agent_models_updated_payload_emits_event_contract_fields() {
        let models = vec![
            ModelInfo {
                value: "a".to_string(),
            },
            ModelInfo {
                value: "b".to_string(),
            },
        ];
        let payload = build_agent_models_updated_payload("sess-1", &models, Some("a"));

        assert_eq!(payload["chat_session_id"], "sess-1");
        let candidates = payload["available_models"]
            .as_array()
            .expect("available_models is array");
        let values: Vec<String> = candidates
            .iter()
            .map(|v| v["value"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(values, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(payload["selected_model"], "a");
    }

    #[test]
    fn build_agent_models_updated_payload_carries_selected_model_non_null() {
        // モデル未選択状態は廃止。set_agent_model は常に非 null の selected_model を emit する。
        let payload = build_agent_models_updated_payload("sess-2", &[], Some("claude-opus-4-8"));
        assert_eq!(payload["selected_model"], "claude-opus-4-8");
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
            permission_mode: "acceptEdits".to_string(),
            permission_profile_id: None,
            selected_model,
            backend_id: Some(backend_id.to_string()),
            workflow_step_session: false,
        }
    }

    #[test]
    fn resolve_spawn_info_without_registry_keeps_none() {
        // registry 未指定（テスト等）では selected_model=None は None のまま。
        let session = make_chat_session_for_spawn(None, None, CODEX_BACKEND_ID);
        let info = resolve_spawn_info(Some(session), None);
        assert_eq!(info.0, None); // resume_sid
        assert_eq!(info.1, None); // selected_model
        assert_eq!(info.2, CODEX_BACKEND_ID.to_string());
    }

    #[test]
    fn resolve_spawn_info_resolves_none_to_default_with_registry() {
        // モデル未選択状態は廃止。selected_model=None は registry の既定モデルへ解決する。
        let registry = make_fixed_model_registry();
        let session = make_chat_session_for_spawn(None, None, CODEX_BACKEND_ID);
        let info = resolve_spawn_info(Some(session), Some(&registry));
        assert_eq!(
            info.1,
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
            info.1,
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
            info.1,
            Some(crate::domain::agent_session::CLAUDE_FIXED_MODELS[0].to_string())
        );
        assert_eq!(info.2, CLAUDE_BACKEND_ID.to_string());
    }

    // --- get_session: active process が居ても config 由来を返す ---

    #[tokio::test]
    async fn get_session_returns_config_derived_available_models_even_with_active_process() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
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
            proc.available_models = vec![ModelInfo {
                value: "stale-from-process".to_string(),
            }];
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
            .map(|m| m.value)
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
    async fn get_session_resolves_none_selected_model_to_default() {
        // spec: モデル未選択状態は廃止。selected_model=None の既存セッションを get_session
        // すると、応答の selected_model は backend の既定モデル（固定リスト先頭）へ解決される。
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        // 旧フォーマット（未選択）を模して None を永続化する。
        session.selected_model = None;
        session_store.save_session(temp.path(), &session).unwrap();

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
            Some(crate::domain::agent_session::CLAUDE_FIXED_MODELS[0].to_string())
        );
    }

    #[tokio::test]
    async fn get_session_errors_when_default_model_unresolvable() {
        // 契約: 応答の selected_model は常に非 null。registry が在りつつ既定モデルへ解決
        // できない場合（fixed_models 無し + config 空）、フィールド脱落を防ぐため Err を返す。
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.selected_model = None;
        session_store.save_session(temp.path(), &session).unwrap();

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
        let session_store = Arc::new(SessionStore::default());
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
            proc.available_models = vec![ModelInfo {
                value: "stale".to_string(),
            }];
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
            .map(|m| m.value.clone())
            .collect();
        assert_eq!(values, vec!["claude-4".to_string(), "haiku".to_string()]);
        // selected_model も反映される
        assert_eq!(proc.selected_model, Some("claude-4".to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_invalid_format_before_registry_check() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::default());
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
}
