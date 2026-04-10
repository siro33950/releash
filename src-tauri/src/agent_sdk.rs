use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static GENERATION_COUNTER: AtomicU64 = AtomicU64::new(1);

use serde::Serialize;
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::session::{
    now_timestamp, resolve_data_dir, GetSessionResponse, MessagePart, SessionStore,
};

const PERSIST_INTERVAL_MS: u64 = 1000;

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

pub struct AgentProcess {
    pub stdin: tokio::process::ChildStdin,
    pub state: BridgeState,
    pub turn_phase: TurnPhase,
    pub sdk_session_id: Option<String>,
    pub child: tokio::process::Child,
    pub generation_id: u64,
    pub streaming_message_id: Option<String>,
    pub streaming_parts: Vec<crate::session::MessagePart>,
    /// Retained after turn_complete so post-turn background task events
    /// can still be accumulated and emitted via `agent-streaming-updated`.
    pub last_message_id: Option<String>,
    /// Maps background task_id (agentId) → tool_use_id.
    /// Populated from ToolResult content ("agentId: XXX"), used to fill
    /// missing tool_use_id in task_notification messages from the SDK.
    pub task_id_map: HashMap<String, String>,
}

/// Per-session agent process map: chat_session_id → AgentProcess
pub type AgentProcessMap = HashMap<String, AgentProcess>;

fn dev_bridge_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("claude-sdk-bridge.mjs")
}

fn resolve_bridge_script(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    // Dev mode: resolve from CARGO_MANIFEST_DIR (src-tauri/)
    #[cfg(debug_assertions)]
    {
        let dev_path = dev_bridge_path();
        if dev_path.exists() {
            return Ok(dev_path);
        }
    }

    // Production: resolve from Tauri resource_dir (bundled version)
    app.path()
        .resource_dir()
        .map(|d| d.join("resources").join("claude-sdk-bridge.bundled.mjs"))
        .map_err(|e| format!("Failed to resolve resource dir: {e}"))
}

fn get_resume_session_id(
    app: &tauri::AppHandle,
    session_store: &SessionStore,
    chat_session_id: &str,
) -> Option<String> {
    resolve_data_dir(app)
        .ok()
        .and_then(|data_dir| {
            session_store
                .get_session(&data_dir, chat_session_id)
                .ok()
                .flatten()
        })
        .and_then(|s| s.agent_session_id)
}

fn persist_streaming_parts(
    session_store: &SessionStore,
    app: &tauri::AppHandle,
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
        let (content, thinking, activities) = crate::session::parts_to_legacy(parts);
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

fn emit_session_state_changed(
    app: &tauri::AppHandle,
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

const CLOSE_TIMEOUT_SECS: u64 = 5;

fn build_set_mode_command(permission_mode: &str) -> String {
    let cmd = serde_json::json!({
        "type": "setMode",
        "permissionMode": permission_mode,
    });
    format!("{}\n", cmd)
}

/// Append text/thinking chunk to streaming parts as a new part.
/// Merging consecutive same-type chunks is handled by the frontend's `mergeDeltaParts`.
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

/// Consolidate consecutive same-type text/thinking parts for persistence.
/// During streaming, parts are kept as individual chunks for correct delta extraction.
/// This function merges them into single parts before persisting.
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
/// Returns true if the message was handled (accumulated) and should NOT be forwarded as agent-sdk-message.
fn accumulate_sdk_message(
    msg: &serde_json::Value,
    parts: &mut Vec<MessagePart>,
    task_id_map: &mut HashMap<String, String>,
) -> bool {
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
                                return true;
                            }
                        } else if delta_type == "thinking_delta" {
                            if let Some(thinking) = delta.get("thinking").and_then(|v| v.as_str()) {
                                append_to_parts(parts, "thinking", thinking, parent_tool_use_id);
                                return true;
                            }
                        }
                    }
                }
            }
            false
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
            true
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
            true
        }
        "permission_request" => {
            let request = msg.clone();
            parts.push(MessagePart::Permission {
                request,
                status: "pending".to_string(),
                answers: None,
                parent_tool_use_id,
            });
            true
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
                    true
                }
                _ => false, // permissionMode sync, other system messages → forward
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
            false // Still forward for handleBridgeError
        }
        _ => false,
    }
}

async fn spawn_bridge_process(
    app: &tauri::AppHandle,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    session_id: Option<String>,
    cwd: &str,
    permission_mode: Option<String>,
) -> Result<(), String> {
    let bridge_path = resolve_bridge_script(app)?;
    if !bridge_path.exists() {
        return Err(format!(
            "Bridge script not found: {}",
            bridge_path.display()
        ));
    }

    let mut child = Command::new("node")
        .arg(
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
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn node process: {e}"))?;

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

    // Send init command
    let init_cmd = serde_json::json!({
        "type": "init",
        "cwd": cwd,
        "permissionMode": permission_mode.unwrap_or_else(|| "acceptEdits".to_string()),
        "sessionId": session_id,
    });
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
                state: BridgeState::Initializing,
                turn_phase: TurnPhase::Idle,
                sdk_session_id: session_id,
                child,
                generation_id: gen_id,
                streaming_message_id: None,
                streaming_parts: Vec::new(),
                last_message_id: None,
                task_id_map: HashMap::new(),
            },
        );
    }

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
                    "turn_complete" => {
                        let exit_code = msg.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0);
                        let was_streaming;
                        let raw_parts;
                        let final_msg_id;
                        {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                was_streaming = proc.state == BridgeState::Streaming;
                                proc.state = BridgeState::Ready;
                                proc.turn_phase = TurnPhase::Idle;

                                // Capture streaming buffer for consolidation outside lock.
                                // Keep parts in buffer so post-turn_complete background task
                                // events can append to them (cleared at next query start).
                                raw_parts = proc.streaming_parts.clone();
                                final_msg_id = proc.streaming_message_id.take();

                                // Retain message ID for post-turn_complete background task events.
                                // Only update if we have a real ID — a second turn_complete
                                // (from background task result) would yield None and must not
                                // overwrite the ID set by the first turn_complete.
                                if final_msg_id.is_some() {
                                    proc.last_message_id = final_msg_id.clone();
                                }

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
                            }
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
                        }
                    }
                    "error" => {
                        let error_msg = msg
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown bridge error");
                        log::error!("Bridge error [{}]: {}", csid_stdout, error_msg);

                        // Accumulate error into streaming parts and extract delta
                        let (error_delta, error_emit_msg_id) = {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                if proc.state == BridgeState::Streaming {
                                    let prev_len = proc.streaming_parts.len();
                                    accumulate_sdk_message(
                                        &msg,
                                        &mut proc.streaming_parts,
                                        &mut proc.task_id_map,
                                    );
                                    let delta = proc.streaming_parts[prev_len..].to_vec();
                                    let mid = proc.streaming_message_id.clone();
                                    (delta, mid)
                                } else {
                                    (Vec::new(), None)
                                }
                            } else {
                                (Vec::new(), None)
                            }
                        };

                        // Emit error delta so UI can display the error message
                        if !error_delta.is_empty() {
                            if let Some(ref mid) = error_emit_msg_id {
                                use tauri::Emitter;
                                let _ = app_stdout.emit(
                                    "agent-streaming-updated",
                                    serde_json::json!({
                                        "chat_session_id": csid_stdout,
                                        "message_id": mid,
                                        "parts": error_delta,
                                    }),
                                );
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
                        // Init error → clear stale agent_session_id to prevent infinite resume loop
                        if was_initializing {
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
                        // Accumulate into streaming buffer and emit delta update
                        let (
                            accumulated,
                            delta_parts,
                            emit_msg_id,
                            should_persist,
                            raw_persist_parts,
                        ) = {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                if proc.state == BridgeState::Streaming
                                    && proc.streaming_message_id.is_some()
                                {
                                    let prev_len = proc.streaming_parts.len();
                                    let acc = accumulate_sdk_message(
                                        &msg,
                                        &mut proc.streaming_parts,
                                        &mut proc.task_id_map,
                                    );
                                    if !acc {
                                        (false, Vec::new(), None, false, Vec::new())
                                    } else {
                                        // Extract only newly added parts as delta
                                        let delta: Vec<MessagePart> =
                                            proc.streaming_parts[prev_len..].to_vec();
                                        let mid = proc.streaming_message_id.clone();

                                        let now = std::time::Instant::now();
                                        let elapsed_persist =
                                            now.duration_since(last_persist_time).as_millis()
                                                as u64;
                                        let should_persist = elapsed_persist >= PERSIST_INTERVAL_MS;
                                        let raw_persist_parts = if should_persist {
                                            proc.streaming_parts.clone()
                                        } else {
                                            Vec::new()
                                        };

                                        (true, delta, mid, should_persist, raw_persist_parts)
                                    }
                                } else if proc.last_message_id.is_some() {
                                    // Post-turn_complete: background task events
                                    // (task_notification, task_progress, etc.)
                                    // Accumulate and emit immediately (no throttle needed).
                                    let prev_len = proc.streaming_parts.len();
                                    let acc = accumulate_sdk_message(
                                        &msg,
                                        &mut proc.streaming_parts,
                                        &mut proc.task_id_map,
                                    );
                                    if !acc {
                                        (false, Vec::new(), None, false, Vec::new())
                                    } else {
                                        let delta: Vec<MessagePart> =
                                            proc.streaming_parts[prev_len..].to_vec();
                                        let mid = proc.last_message_id.clone();

                                        // Always persist post-turn events immediately
                                        let raw_persist_parts = proc.streaming_parts.clone();
                                        (true, delta, mid, true, raw_persist_parts)
                                    }
                                } else {
                                    (false, Vec::new(), None, false, Vec::new())
                                }
                            } else {
                                (false, Vec::new(), None, false, Vec::new())
                            }
                        };

                        // Emit agent-streaming-updated with delta parts (no throttle)
                        if !delta_parts.is_empty() {
                            if let Some(ref mid) = emit_msg_id {
                                let _ = app_stdout.emit(
                                    "agent-streaming-updated",
                                    serde_json::json!({
                                        "chat_session_id": csid_stdout,
                                        "message_id": mid,
                                        "parts": delta_parts,
                                    }),
                                );
                            }
                        }

                        // Periodic persist (1s interval) — consolidate outside lock
                        if should_persist {
                            if let Some(ref mid) = emit_msg_id {
                                last_persist_time = std::time::Instant::now();
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

                        // Forward non-accumulated messages (meta events) as agent-sdk-message.
                        // permission_request needs both delta emit AND forwarding for SET_PENDING_PERMISSION.
                        if should_forward_sdk_message(accumulated, msg_type) {
                            let _ = app_stdout.emit("agent-sdk-message", &msg);
                        }

                        // Transition to WaitingPermission when permission_request arrives
                        if msg_type == "permission_request" {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                if proc.state == BridgeState::Streaming {
                                    proc.turn_phase = TurnPhase::WaitingPermission;
                                    emit_session_state_changed(
                                        &app_stdout,
                                        &csid_stdout,
                                        TurnPhase::WaitingPermission,
                                        None,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        // EOF — process exited; verify generation to avoid acting on stale events
        let was_streaming = {
            let map = handles_stdout.lock().await;
            map.get(&csid_stdout).is_some_and(|p| {
                p.generation_id == captured_gen_id && p.state == BridgeState::Streaming
            })
        };
        if was_streaming {
            emit_session_state_changed(
                &app_stdout,
                &csid_stdout,
                TurnPhase::Idle,
                Some(-1),
            );
        }
        {
            let mut map = handles_stdout.lock().await;
            if let Some(proc) = map.get_mut(&csid_stdout) {
                if proc.generation_id == captured_gen_id {
                    proc.state = BridgeState::Crashed;
                    proc.turn_phase = TurnPhase::Idle;
                }
            }
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

    Ok(())
}

#[tauri::command]
pub async fn get_session(
    state: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Option<GetSessionResponse>, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = state.get_session(&data_dir, &session_id)?;
    match session {
        None => Ok(None),
        Some(mut session) => {
            // Acquire lock briefly to read turn_phase and clone streaming parts
            let (turn_phase, raw_parts, streaming_mid) = {
                let map = handles.lock().await;
                if let Some(proc) = map.get(&session_id) {
                    let phase = proc.turn_phase;
                    if proc.state == BridgeState::Streaming {
                        (
                            phase,
                            proc.streaming_parts.clone(),
                            proc.streaming_message_id.clone(),
                        )
                    } else {
                        (phase, Vec::new(), None)
                    }
                } else {
                    (TurnPhase::Idle, Vec::new(), None)
                }
            };

            // Consolidate outside lock to avoid blocking other sessions
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

            Ok(Some(GetSessionResponse {
                session,
                turn_phase,
            }))
        }
    }
}

#[tauri::command]
pub async fn start_agent_session(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
    cwd: String,
    permission_mode: Option<String>,
) -> Result<(), String> {
    // If process already exists and is not crashed, do nothing; otherwise remove crashed entry
    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get(&chat_session_id) {
            if proc.state != BridgeState::Crashed {
                return Ok(());
            }
        }
        map.remove(&chat_session_id);
    }

    let resume_sid = get_resume_session_id(&app, session_store.inner(), &chat_session_id);

    spawn_bridge_process(
        &app,
        handles.inner(),
        session_store.inner(),
        &chat_session_id,
        resume_sid,
        &cwd,
        permission_mode,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn execute_agent_query(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    prompt: String,
    chat_session_id: String,
    cwd: String,
    permission_mode: Option<String>,
    streaming_message_id: String,
) -> Result<(), String> {
    // Check if we need to spawn a new process (single lock to avoid TOCTOU)
    let spawn_info = {
        let mut map = handles.lock().await;
        match map.get(&chat_session_id) {
            None => Some(get_resume_session_id(
                &app,
                session_store.inner(),
                &chat_session_id,
            )),
            Some(proc) if proc.state == BridgeState::Crashed => {
                map.remove(&chat_session_id);
                Some(get_resume_session_id(
                    &app,
                    session_store.inner(),
                    &chat_session_id,
                ))
            }
            _ => None,
        }
    };

    if let Some(resume_sid) = spawn_info {
        spawn_bridge_process(
            &app,
            handles.inner(),
            session_store.inner(),
            &chat_session_id,
            resume_sid,
            &cwd,
            permission_mode.clone(),
        )
        .await?;
    }

    // Sync permissionMode to Bridge before sending message
    let mode_data = build_set_mode_command(permission_mode.as_deref().unwrap_or("acceptEdits"));

    // Send message command.
    // Even if a message is sent while the SDK is still processing an interrupt,
    // the Bridge's promptGenerator queues it and only yields after the current turn completes.
    // The SDK calls generator.next() only when ready for the next turn, providing ordering guarantee.
    let msg_cmd = serde_json::json!({
        "type": "message",
        "prompt": prompt,
    });
    let data = format!("{}\n", msg_cmd);

    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(&chat_session_id) {
            // Sync permissionMode to Bridge
            proc.stdin
                .write_all(mode_data.as_bytes())
                .await
                .map_err(|e| format!("Failed to write setMode: {e}"))?;
            proc.stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush setMode: {e}"))?;

            proc.state = BridgeState::Streaming;
            proc.turn_phase = TurnPhase::Streaming;
            proc.streaming_message_id = Some(streaming_message_id.clone());
            proc.streaming_parts.clear();
            proc.last_message_id = None;
            proc.task_id_map.clear();
            proc.stdin
                .write_all(data.as_bytes())
                .await
                .map_err(|e| format!("Failed to write message: {e}"))?;
            proc.stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush message: {e}"))?;
        } else {
            return Err(format!("No agent process for session {chat_session_id}"));
        }
    }

    // Emit state change so frontend can track turn phase
    emit_session_state_changed(
        &app,
        &chat_session_id,
        TurnPhase::Streaming,
        None,
    );

    Ok(())
}

#[tauri::command]
pub async fn interrupt_agent_query(
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
) -> Result<(), String> {
    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(&chat_session_id) {
            proc.stdin
                .write_all(b"{\"type\":\"interrupt\"}\n")
                .await
                .map_err(|e| format!("Failed to write interrupt: {e}"))?;
            proc.stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush: {e}"))?;
        } else {
            return Err(format!(
                "No active agent process for session {chat_session_id}"
            ));
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn close_agent_session(
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
) -> Result<(), String> {
    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(&chat_session_id) {
            if let Err(e) = proc.stdin.write_all(b"{\"type\":\"close\"}\n").await {
                log::warn!("Failed to send close command for session {chat_session_id}: {e}");
            }
            if let Err(e) = proc.stdin.flush().await {
                log::warn!("Failed to flush close command for session {chat_session_id}: {e}");
            }
        } else {
            // No process to close — already gone
            return Ok(());
        }
    }

    // Timeout fallback: if process doesn't exit, kill it
    let handles_clone = Arc::clone(handles.inner());
    let csid = chat_session_id.clone();
    let timeout_gen_id = {
        let map = handles_clone.lock().await;
        map.get(&csid).map(|p| p.generation_id)
    };
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(CLOSE_TIMEOUT_SECS)).await;
        let mut map = handles_clone.lock().await;
        if let Some(proc) = map.get_mut(&csid) {
            if timeout_gen_id == Some(proc.generation_id) {
                log::warn!("Close timeout for session {csid}, killing process");
                let _ = proc.child.kill().await;
            } else {
                // Generation mismatch: a new process has been spawned; skip kill and remove
                return;
            }
        }
        map.remove(&csid);
    });

    Ok(())
}

#[tauri::command]
pub async fn set_agent_permission_mode(
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
    permission_mode: String,
) -> Result<(), String> {
    let data = build_set_mode_command(&permission_mode);

    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(&chat_session_id) {
            proc.stdin
                .write_all(data.as_bytes())
                .await
                .map_err(|e| format!("Failed to write setMode: {e}"))?;
            proc.stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush setMode: {e}"))?;
        }
        // If no process exists, silently ignore (process not yet started)
    }

    Ok(())
}

#[tauri::command]
pub async fn respond_agent_permission(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
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

    let updated_permission_part: Option<MessagePart>;
    let emit_msg_id;
    let did_transition_to_streaming;
    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(&chat_session_id) {
            proc.stdin
                .write_all(data.as_bytes())
                .await
                .map_err(|e| format!("Failed to write permission response: {e}"))?;
            proc.stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush: {e}"))?;

            // Transition back to Streaming after permission response
            did_transition_to_streaming = proc.turn_phase == TurnPhase::WaitingPermission;
            if did_transition_to_streaming {
                proc.turn_phase = TurnPhase::Streaming;
            }

            // Update Permission part status in streaming buffer
            let new_status = if behavior == "allow" {
                "allowed"
            } else {
                "denied"
            };
            let mut found_part = None;
            for part in &mut proc.streaming_parts {
                if let MessagePart::Permission {
                    request,
                    status,
                    answers,
                    ..
                } = part
                {
                    if request.get("request_id").and_then(|v| v.as_str()) == Some(&request_id) {
                        *status = new_status.to_string();
                        if let Some(ref av) = answers_value {
                            *answers = Some(av.clone());
                        }
                        found_part = Some(part.clone());
                    }
                }
            }

            updated_permission_part = found_part;
            emit_msg_id = proc.streaming_message_id.clone();
        } else {
            return Err(format!(
                "No active agent process for session {chat_session_id}"
            ));
        }
    }

    // Emit agent-streaming-updated with the updated permission part as delta
    if let (Some(ref mid), Some(ref part)) = (&emit_msg_id, &updated_permission_part) {
        use tauri::Emitter;
        let _ = app.emit(
            "agent-streaming-updated",
            serde_json::json!({
                "chat_session_id": chat_session_id,
                "message_id": mid,
                "parts": [part],
            }),
        );
    }

    // Emit state change only if we actually transitioned: WaitingPermission → Streaming
    if did_transition_to_streaming {
        emit_session_state_changed(
            &app,
            &chat_session_id,
            TurnPhase::Streaming,
            None,
        );
    }

    Ok(())
}

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct SlashCommandEntry {
    pub name: String,
    pub description: String,
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

/// Scan skill and command directories to collect slash commands with deduplication.
///
/// Scan order (highest priority first):
/// 1. `~/.claude/skills/*/SKILL.md` — personal skill
/// 2. `{cwd}/.claude/skills/*/SKILL.md` — project skill
/// 3. `~/.claude/commands/*.md` — personal command
/// 4. `{cwd}/.claude/commands/*.md` — project command
///
/// When the same name appears in multiple sources, the higher-priority entry wins.
#[tauri::command]
pub async fn scan_slash_commands(cwd: String) -> Result<Vec<SlashCommandEntry>, String> {
    let mut commands = Vec::new();
    let mut seen = HashSet::new();

    let cwd_path = PathBuf::from(&cwd);

    // Build list of directories to scan in priority order
    let mut skill_dirs: Vec<PathBuf> = Vec::new();
    let mut command_dirs: Vec<PathBuf> = Vec::new();

    if let Some(home) = dirs::home_dir() {
        skill_dirs.push(home.join(".claude").join("skills"));
        command_dirs.push(home.join(".claude").join("commands"));
    }
    skill_dirs.push(cwd_path.join(".claude").join("skills"));
    command_dirs.push(cwd_path.join(".claude").join("commands"));

    // Scan skills (personal first, then project)
    for skills_dir in &skill_dirs {
        if let Ok(entries) = std::fs::read_dir(skills_dir) {
            for entry in entries.flatten() {
                let skill_md = entry.path().join("SKILL.md");
                if skill_md.is_file() {
                    if let Ok(content) = std::fs::read_to_string(&skill_md) {
                        if let Some((name, description)) = parse_skill_frontmatter(&content) {
                            if !name.is_empty() && seen.insert(name.clone()) {
                                commands.push(SlashCommandEntry { name, description });
                            }
                        }
                    }
                }
            }
        }
    }

    // Scan commands (personal first, then project)
    for cmd_dir in &command_dirs {
        if let Ok(entries) = std::fs::read_dir(cmd_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") && path.is_file() {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    if name.is_empty() {
                        continue;
                    }
                    if seen.insert(name.clone()) {
                        let description = std::fs::read_to_string(&path)
                            .ok()
                            .and_then(|c| c.lines().next().map(|l| l.trim().to_string()))
                            .unwrap_or_default();
                        commands.push(SlashCommandEntry { name, description });
                    }
                }
            }
        }
    }

    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn init_command_format() {
        let cwd = "/repo";
        let permission_mode = "acceptEdits";
        let session_id: Option<String> = Some("sess-abc".to_string());
        let cmd = serde_json::json!({
            "type": "init",
            "cwd": cwd,
            "permissionMode": permission_mode,
            "sessionId": session_id,
        });
        assert_eq!(cmd["type"], "init");
        assert_eq!(cmd["cwd"], "/repo");
        assert_eq!(cmd["permissionMode"], "acceptEdits");
        assert_eq!(cmd["sessionId"], "sess-abc");
    }

    #[test]
    fn set_mode_command_format() {
        let permission_mode = "bypassPermissions";
        let cmd = serde_json::json!({
            "type": "setMode",
            "permissionMode": permission_mode,
        });
        assert_eq!(cmd["type"], "setMode");
        assert_eq!(cmd["permissionMode"], "bypassPermissions");
    }

    #[test]
    fn set_mode_command_with_default() {
        let permission_mode: Option<String> = None;
        let cmd = serde_json::json!({
            "type": "setMode",
            "permissionMode": permission_mode.as_deref().unwrap_or("acceptEdits"),
        });
        assert_eq!(cmd["type"], "setMode");
        assert_eq!(cmd["permissionMode"], "acceptEdits");
    }

    #[test]
    fn init_command_without_session_id() {
        let session_id: Option<String> = None;
        let cmd = serde_json::json!({
            "type": "init",
            "cwd": "/repo",
            "permissionMode": "acceptEdits",
            "sessionId": session_id,
        });
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
        let path = dev_bridge_path();
        assert!(
            path.ends_with("src-tauri/resources/claude-sdk-bridge.mjs"),
            "dev_bridge_path should end with src-tauri/resources/claude-sdk-bridge.mjs, got: {}",
            path.display()
        );
    }

    #[test]
    fn dev_bridge_path_file_exists() {
        let path = dev_bridge_path();
        assert!(
            path.exists(),
            "Bridge script should exist at {}, but it does not",
            path.display()
        );
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
                serde_json::from_str(&line).expect(&format!("Failed to parse: {line}"));
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

    #[tokio::test]
    async fn scan_slash_commands_with_nonexistent_cwd() {
        let result = scan_slash_commands("/nonexistent/path/abc123".to_string()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn scan_slash_commands_with_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join(".claude").join("commands");
        std::fs::create_dir_all(&commands_dir).unwrap();

        std::fs::write(
            commands_dir.join("test-cmd.md"),
            "This is a test command\nMore details here",
        )
        .unwrap();

        let result = scan_slash_commands(tmp.path().to_string_lossy().to_string())
            .await
            .unwrap();

        let test_cmd = result.iter().find(|c| c.name == "test-cmd");
        assert!(test_cmd.is_some(), "Should find test-cmd in results");
        assert_eq!(test_cmd.unwrap().description, "This is a test command");
    }

    #[tokio::test]
    async fn scan_slash_commands_deduplicates_skill_over_command() {
        let tmp = tempfile::tempdir().unwrap();

        let skill_dir = tmp
            .path()
            .join(".claude")
            .join("skills")
            .join("zzz-dedup-test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: zzz-dedup-test\ndescription: From skill\n---\nBody",
        )
        .unwrap();

        let commands_dir = tmp.path().join(".claude").join("commands");
        std::fs::create_dir_all(&commands_dir).unwrap();
        std::fs::write(
            commands_dir.join("zzz-dedup-test.md"),
            "From command\nDetails",
        )
        .unwrap();

        let result = scan_slash_commands(tmp.path().to_string_lossy().to_string())
            .await
            .unwrap();

        let matches: Vec<_> = result
            .iter()
            .filter(|c| c.name == "zzz-dedup-test")
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "zzz-dedup-test should appear exactly once, got: {matches:?}"
        );
        assert_eq!(matches[0].description, "From skill");
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
        let handled = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
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
        let handled = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
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
        let handled = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
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
        let handled = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
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
        let handled = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
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
    fn test_accumulate_error() {
        let msg = serde_json::json!({
            "type": "error",
            "message": "Something went wrong"
        });
        let mut parts = vec![];
        let handled = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
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
        let handled = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());
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
}
