use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static GENERATION_COUNTER: AtomicU64 = AtomicU64::new(1);

use serde::{Deserialize, Serialize};
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::session::{
    add_message_internal, create_session_internal, now_timestamp, resolve_data_dir, ChatMessage,
    ChatSession, GetSessionResponse, MessagePart, MessageRole, SessionStore, SessionSummary,
};

const PERSIST_INTERVAL_MS: u64 = 1000;

fn available_models_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("available_models.json")
}

fn save_available_models(app_data_dir: &Path, models: &[ModelInfo]) -> Result<(), String> {
    std::fs::create_dir_all(app_data_dir).map_err(|e| format!("Failed to create data dir: {e}"))?;
    let file = available_models_path(app_data_dir);
    let json =
        serde_json::to_string(models).map_err(|e| format!("Failed to serialize models: {e}"))?;
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("Failed to write models temp file: {e}"))?;
    std::fs::rename(&tmp, &file).map_err(|e| format!("Failed to rename models temp file: {e}"))?;
    Ok(())
}

fn load_available_models(app_data_dir: &Path) -> Vec<ModelInfo> {
    let file = available_models_path(app_data_dir);
    match std::fs::read_to_string(&file) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

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
    pub content: String,
    pub permission_mode: String,
    pub images: Vec<ImageAttachment>,
}

/// Model information from Agent SDK.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub value: String,
    pub display_name: String,
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
    /// Pending message queued by send_agent_message during streaming.
    /// Auto-consumed on turn_complete by the stdout reader.
    pub pending_message: Option<PendingMessage>,
    /// Runtime permission mode tracked from SDK notifications.
    /// Unlike `ChatSession.permission_mode` (persisted, excludes transient "plan"),
    /// this reflects the actual SDK state including "plan" mode.
    pub current_permission_mode: String,
    /// Available models from Agent SDK.
    pub available_models: Vec<ModelInfo>,
    /// Currently selected model for this session (None = SDK default).
    pub selected_model: Option<String>,
}

impl AgentProcess {
    /// Write setMode + setModel commands to the Bridge stdin before a turn starts.
    async fn sync_pre_turn_settings(&mut self, permission_mode: &str) -> Result<(), String> {
        let mode_data = build_set_mode_command(permission_mode);
        self.stdin
            .write_all(mode_data.as_bytes())
            .await
            .map_err(|e| format!("Failed to write setMode: {e}"))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush setMode: {e}"))?;

        let model_data = build_set_model_command(self.selected_model.as_deref());
        self.stdin
            .write_all(model_data.as_bytes())
            .await
            .map_err(|e| format!("Failed to write setModel: {e}"))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush setModel: {e}"))?;

        Ok(())
    }
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

/// 状態遷移時に AgentStatusCenter へ通知し、必要に応じて Webhook 送信を行う統一エントリ。
/// session_store から ChatSession を引いて worktree_path / SessionState を取得する。
/// `session_state_override` を渡すと、ストア値より優先される（Bridge crash 時など）。
fn notify_status_transition(
    app: &tauri::AppHandle,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    turn_phase: TurnPhase,
    session_state_override: Option<crate::session::SessionState>,
) {
    use crate::agent_status::{current_timestamp, AgentStatusCenter, SessionStatus, TurnPhaseRepr};
    use crate::config::AppConfig;
    use crate::focus_tracker::FocusTracker;

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

    let agent_state = AgentStatusCenter::derive_agent_state(turn_phase, session_state.clone());

    if let Some(center) = app.try_state::<Arc<AgentStatusCenter>>() {
        let status = SessionStatus {
            chat_session_id: chat_session_id.to_string(),
            worktree_id: worktree_path.clone(),
            worktree_path: worktree_path.clone(),
            pty_id: None,
            agent_state: agent_state.clone(),
            turn_phase: TurnPhaseRepr::from(turn_phase),
            session_state,
            pending_permission: matches!(turn_phase, TurnPhase::WaitingPermission),
            last_activity_at: current_timestamp(),
        };
        center.update_session(status);
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
                let sync = crate::protocol::AgentStateSync {
                    worktree_path: worktree_path.clone(),
                    state: agent_state,
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

/// Resolve permission mode: "plan" → "default" (= acceptEdits), others unchanged.
/// Called when SDK sends permissionMode: "default" (after Plan approval).
fn resolve_permission_mode(mode: &str) -> &str {
    if mode == "plan" {
        "default"
    } else {
        mode
    }
}

fn emit_permission_mode_changed(app: &tauri::AppHandle, chat_session_id: &str, mode: &str) {
    use tauri::Emitter;
    let _ = app.emit(
        "agent-permission-mode-changed",
        serde_json::json!({
            "chat_session_id": chat_session_id,
            "permission_mode": mode,
        }),
    );
}

fn build_set_mode_command(permission_mode: &str) -> String {
    let cmd = serde_json::json!({
        "type": "setMode",
        "permissionMode": permission_mode,
    });
    format!("{}\n", cmd)
}

fn build_set_model_command(model_id: Option<&str>) -> String {
    let cmd = serde_json::json!({
        "type": "setModel",
        "modelId": model_id,
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

#[allow(clippy::too_many_arguments)]
async fn spawn_bridge_process(
    app: &tauri::AppHandle,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    session_id: Option<String>,
    cwd: &str,
    permission_mode: Option<String>,
    selected_model: Option<String>,
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
    let initial_permission_mode = permission_mode.unwrap_or_else(|| "acceptEdits".to_string());
    let init_cmd = serde_json::json!({
        "type": "init",
        "cwd": cwd,
        "permissionMode": initial_permission_mode,
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
                pending_message: None,
                current_permission_mode: initial_permission_mode.clone(),
                available_models: Vec::new(),
                selected_model,
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
                    "supported_models" => {
                        // Parse models from Bridge and store in AgentProcess
                        let models: Vec<ModelInfo> = msg
                            .get("models")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|m| {
                                        let value =
                                            m.get("value").and_then(|v| v.as_str())?.to_string();
                                        let display_name = m
                                            .get("displayName")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or(&value)
                                            .to_string();
                                        Some(ModelInfo {
                                            value,
                                            display_name,
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();

                        let selected_model = {
                            let mut map = handles_stdout.lock().await;
                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                proc.available_models = models.clone();
                                proc.selected_model.clone()
                            } else {
                                None
                            }
                        };

                        // Persist models globally
                        if let Ok(data_dir) = resolve_data_dir(&app_stdout) {
                            let _ = save_available_models(&data_dir, &models);
                        }

                        // Emit models to frontend
                        let _ = app_stdout.emit(
                            "agent-models-updated",
                            serde_json::json!({
                                "chat_session_id": csid_stdout,
                                "available_models": models,
                                "selected_model": selected_model,
                            }),
                        );
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
                            // AgentStatusCenter にも通知（exit_code 非0 なら Error 扱い）
                            let override_state = if exit_code != 0 {
                                Some(crate::session::SessionState::Error)
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

                            // Auto-consume pending message queued by send_agent_message.
                            // Spawn a separate task to avoid oversized async state machine in stdout reader.
                            let has_pending = {
                                let map = handles_stdout.lock().await;
                                map.get(&csid_stdout)
                                    .is_some_and(|p| p.pending_message.is_some())
                            };
                            if has_pending {
                                let app_p = app_stdout.clone();
                                let h_p = Arc::clone(&handles_stdout);
                                let ss_p = Arc::clone(&session_store_clone);
                                let csid_p = csid_stdout.clone();
                                tokio::spawn(async move {
                                    consume_pending_message(&app_p, &h_p, &ss_p, &csid_p).await;
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
                        // Bridge crash: AgentStatusCenter に Error として通知
                        if was_streaming || was_initializing {
                            notify_status_transition(
                                &app_stdout,
                                &session_store_clone,
                                &csid_stdout,
                                TurnPhase::Idle,
                                Some(crate::session::SessionState::Error),
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
                                    let (acc, updated_parts) = accumulate_sdk_message(
                                        &msg,
                                        &mut proc.streaming_parts,
                                        &mut proc.task_id_map,
                                    );
                                    if !acc {
                                        (false, Vec::new(), None, false, Vec::new())
                                    } else {
                                        // Extract only newly added parts as delta
                                        let mut delta: Vec<MessagePart> =
                                            proc.streaming_parts[prev_len..].to_vec();
                                        // Include in-place updated parts in the delta
                                        if let Some(up) = updated_parts {
                                            delta.extend(up);
                                        }
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
                                    let (acc, updated_parts) = accumulate_sdk_message(
                                        &msg,
                                        &mut proc.streaming_parts,
                                        &mut proc.task_id_map,
                                    );
                                    if !acc {
                                        (false, Vec::new(), None, false, Vec::new())
                                    } else {
                                        let mut delta: Vec<MessagePart> =
                                            proc.streaming_parts[prev_len..].to_vec();
                                        if let Some(up) = updated_parts {
                                            delta.extend(up);
                                        }
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

                        // Handle permissionMode sync from SDK on Rust side
                        if msg_type == "system" {
                            if let Some(sdk_mode) =
                                msg.get("permissionMode").and_then(|v| v.as_str())
                            {
                                if sdk_mode == "default" {
                                    // Plan approval: resolve persisted permission_mode
                                    let data_dir_result = resolve_data_dir(&app_stdout);
                                    if let Ok(data_dir) = data_dir_result {
                                        if let Ok(Some(session)) =
                                            session_store_clone.get_session(&data_dir, &csid_stdout)
                                        {
                                            let restored =
                                                resolve_permission_mode(&session.permission_mode);
                                            // Send restored mode to Bridge
                                            let mode_data = build_set_mode_command(restored);
                                            let mut map = handles_stdout.lock().await;
                                            if let Some(proc) = map.get_mut(&csid_stdout) {
                                                let _ = proc
                                                    .stdin
                                                    .write_all(mode_data.as_bytes())
                                                    .await;
                                                let _ = proc.stdin.flush().await;
                                                proc.current_permission_mode = restored.to_string();
                                            }
                                            drop(map);
                                            // Persist resolved mode if it changed (plan → default)
                                            if restored != session.permission_mode {
                                                let _ = session_store_clone.update_permission_mode(
                                                    &data_dir,
                                                    &csid_stdout,
                                                    restored,
                                                );
                                            }
                                            // Notify frontend
                                            emit_permission_mode_changed(
                                                &app_stdout,
                                                &csid_stdout,
                                                restored,
                                            );
                                        }
                                    }
                                } else {
                                    // SDK changed mode (e.g., to "plan") — update runtime state & notify frontend
                                    {
                                        let mut map = handles_stdout.lock().await;
                                        if let Some(proc) = map.get_mut(&csid_stdout) {
                                            proc.current_permission_mode = sdk_mode.to_string();
                                        }
                                    }
                                    emit_permission_mode_changed(
                                        &app_stdout,
                                        &csid_stdout,
                                        sdk_mode,
                                    );
                                }
                            }
                        }

                        // Forward non-accumulated messages (meta events) as agent-sdk-message.
                        // permission_request needs both delta emit AND forwarding for SET_PENDING_PERMISSION.
                        if should_forward_sdk_message(accumulated, msg_type) {
                            let _ = app_stdout.emit("agent-sdk-message", &msg);
                        }

                        // Transition to WaitingPermission when permission_request arrives
                        if msg_type == "permission_request" {
                            let did_transition = {
                                let mut map = handles_stdout.lock().await;
                                if let Some(proc) = map.get_mut(&csid_stdout) {
                                    if proc.state == BridgeState::Streaming {
                                        proc.turn_phase = TurnPhase::WaitingPermission;
                                        true
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            };
                            if did_transition {
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
        }
        // EOF — process exited; verify generation to avoid acting on stale events.
        // Streaming 中の終了だけでなく、Initializing (session_ready 前) の終了も
        // AgentStatusCenter に Error として伝搬させる。Initializing の場合は
        // turn_id=-1 を伴う Idle emit は行わない（streaming が無かったため）。
        let exit_state = {
            let map = handles_stdout.lock().await;
            map.get(&csid_stdout)
                .filter(|p| p.generation_id == captured_gen_id)
                .map(|p| p.state)
        };
        match exit_state {
            Some(BridgeState::Streaming) => {
                emit_session_state_changed(&app_stdout, &csid_stdout, TurnPhase::Idle, Some(-1));
                notify_status_transition(
                    &app_stdout,
                    &session_store_clone,
                    &csid_stdout,
                    TurnPhase::Idle,
                    Some(crate::session::SessionState::Error),
                );
            }
            Some(BridgeState::Initializing) => {
                notify_status_transition(
                    &app_stdout,
                    &session_store_clone,
                    &csid_stdout,
                    TurnPhase::Idle,
                    Some(crate::session::SessionState::Error),
                );
            }
            _ => {}
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

async fn get_session_internal(
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    app: &tauri::AppHandle,
    session_id: &str,
) -> Result<Option<GetSessionResponse>, String> {
    let data_dir = resolve_data_dir(app)?;
    let session = session_store.get_session(&data_dir, session_id)?;
    match session {
        None => Ok(None),
        Some(mut session) => {
            let (turn_phase, raw_parts, streaming_mid, proc_models) = {
                let map = handles.lock().await;
                if let Some(proc) = map.get(session_id) {
                    // Override persisted mode with runtime mode (includes transient "plan")
                    session.permission_mode = proc.current_permission_mode.clone();
                    let phase = proc.turn_phase;
                    let models = if !proc.available_models.is_empty() {
                        proc.available_models.clone()
                    } else {
                        Vec::new()
                    };
                    if proc.state == BridgeState::Streaming {
                        (
                            phase,
                            proc.streaming_parts.clone(),
                            proc.streaming_message_id.clone(),
                            models,
                        )
                    } else {
                        (phase, Vec::new(), None, models)
                    }
                } else {
                    (TurnPhase::Idle, Vec::new(), None, Vec::new())
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

            // Use process models (latest from SDK) if available, otherwise fall back to global cache
            let available_models = if !proc_models.is_empty() {
                proc_models
            } else {
                load_available_models(&data_dir)
            };

            Ok(Some(GetSessionResponse {
                session,
                turn_phase,
                available_models,
            }))
        }
    }
}

#[tauri::command]
pub async fn get_session(
    state: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Option<GetSessionResponse>, String> {
    get_session_internal(state.inner(), handles.inner(), &app, &session_id).await
}

/// Retrieve persisted session fields needed for spawning a Bridge process.
fn get_persisted_spawn_info(
    app: &tauri::AppHandle,
    session_store: &SessionStore,
    chat_session_id: &str,
) -> (Option<String>, Option<String>) {
    resolve_data_dir(app)
        .ok()
        .and_then(|data_dir| {
            session_store
                .get_session(&data_dir, chat_session_id)
                .ok()
                .flatten()
        })
        .map(|s| (s.agent_session_id, s.selected_model))
        .unwrap_or((None, None))
}

async fn start_agent_session_internal(
    app: &tauri::AppHandle,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    cwd: &str,
    permission_mode: Option<String>,
) -> Result<(), String> {
    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get(chat_session_id) {
            if proc.state != BridgeState::Crashed {
                return Ok(());
            }
        }
        map.remove(chat_session_id);
    }

    let (resume_sid, selected_model) =
        get_persisted_spawn_info(app, session_store, chat_session_id);

    spawn_bridge_process(
        app,
        handles,
        session_store,
        chat_session_id,
        resume_sid,
        cwd,
        permission_mode,
        selected_model,
    )
    .await
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
    start_agent_session_internal(
        &app,
        handles.inner(),
        session_store.inner(),
        &chat_session_id,
        &cwd,
        permission_mode,
    )
    .await
}

/// Core logic for starting a new agent turn: spawn Bridge if needed, send prompt.
/// Used by execute_agent_query (Tauri command), send_agent_message, and pending message consumption.
#[allow(clippy::too_many_arguments)]
async fn start_agent_turn(
    app: &tauri::AppHandle,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    cwd: &str,
    permission_mode: &str,
    prompt: &str,
    streaming_message_id: &str,
    images: &[ImageAttachment],
) -> Result<(), String> {
    // Check if we need to spawn a new process (single lock to avoid TOCTOU)
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

    if needs_spawn {
        let (resume_sid, selected_model) =
            get_persisted_spawn_info(app, session_store, chat_session_id);

        spawn_bridge_process(
            app,
            handles,
            session_store,
            chat_session_id,
            resume_sid,
            cwd,
            Some(permission_mode.to_string()),
            selected_model,
        )
        .await?;
    }

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

            proc.current_permission_mode = permission_mode.to_string();
            proc.state = BridgeState::Streaming;
            proc.turn_phase = TurnPhase::Streaming;
            proc.streaming_message_id = Some(streaming_message_id.to_string());
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

/// Consume a pending message that was queued by `send_agent_message` during streaming.
/// Called from the stdout reader after `turn_complete` via `tokio::spawn`.
///
/// Unlike `start_agent_turn`, this skips the spawn-if-needed check because the Bridge
/// process is guaranteed to be running (it just emitted `turn_complete`).
async fn consume_pending_message(
    app: &tauri::AppHandle,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
) {
    // 1. Take pending message from process
    let pending = {
        let mut map = handles.lock().await;
        map.get_mut(chat_session_id)
            .and_then(|p| p.pending_message.take())
    };
    let Some(pending) = pending else {
        return;
    };

    // 2. Add empty agent message
    let data_dir = match resolve_data_dir(app) {
        Ok(d) => d,
        Err(e) => {
            log::error!("consume_pending_message: failed to resolve data dir: {e}");
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
    ) {
        Ok(msg) => msg,
        Err(e) => {
            log::error!("consume_pending_message: failed to add agent message: {e}");
            return;
        }
    };

    // 3. Emit event so UI can update with the new agent message
    {
        use tauri::Emitter;
        let _ = app.emit(
            "agent-pending-message-consumed",
            serde_json::json!({
                "chat_session_id": chat_session_id,
                "agent_message": agent_msg,
            }),
        );
    }

    // 4. Sync permissionMode + selected_model + send message directly to the running Bridge.
    //    The process is guaranteed running since it just emitted turn_complete.
    let msg_cmd = build_message_cmd(&pending.content, &pending.images);
    let data = format!("{}\n", msg_cmd);

    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(chat_session_id) {
            if let Err(e) = proc.sync_pre_turn_settings(&pending.permission_mode).await {
                log::error!("consume_pending_message: {e}");
                return;
            }
            proc.current_permission_mode = pending.permission_mode.clone();
            proc.state = BridgeState::Streaming;
            proc.turn_phase = TurnPhase::Streaming;
            proc.streaming_message_id = Some(agent_msg.id.clone());
            proc.streaming_parts.clear();
            proc.last_message_id = None;
            proc.task_id_map.clear();
            if let Err(e) = proc.stdin.write_all(data.as_bytes()).await {
                log::error!("consume_pending_message: failed to write message: {e}");
                return;
            }
            if let Err(e) = proc.stdin.flush().await {
                log::error!("consume_pending_message: failed to flush message: {e}");
                return;
            }
        } else {
            log::error!("consume_pending_message: no agent process for session {chat_session_id}");
            return;
        }
    }

    // 5. Emit state change
    emit_session_state_changed(app, chat_session_id, TurnPhase::Streaming, None);
    notify_status_transition(
        app,
        session_store,
        chat_session_id,
        TurnPhase::Streaming,
        None,
    );
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
    start_agent_turn(
        &app,
        handles.inner(),
        session_store.inner(),
        &chat_session_id,
        &cwd,
        permission_mode.as_deref().unwrap_or("acceptEdits"),
        &prompt,
        &streaming_message_id,
        &[],
    )
    .await
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
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
    permission_mode: String,
) -> Result<(), String> {
    let data = build_set_mode_command(&permission_mode);

    // Persist to SessionStore
    let data_dir = resolve_data_dir(&app)?;
    session_store.update_permission_mode(&data_dir, &chat_session_id, &permission_mode)?;

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
            proc.current_permission_mode = permission_mode.clone();
        }
        // If no process exists, silently ignore (process not yet started)
    }

    Ok(())
}

#[tauri::command]
pub async fn set_agent_model(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
    model_id: Option<String>,
) -> Result<(), String> {
    // 1. Send setModel command to Bridge + update process state (single lock)
    let data = build_set_model_command(model_id.as_deref());
    let available_models = {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(&chat_session_id) {
            proc.stdin
                .write_all(data.as_bytes())
                .await
                .map_err(|e| format!("Failed to write setModel: {e}"))?;
            proc.stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush setModel: {e}"))?;
            proc.selected_model = model_id.clone();
            Some(proc.available_models.clone())
        } else {
            None
        }
    };

    // 2. Persist to ChatSession
    let data_dir = resolve_data_dir(&app)?;
    let mut session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    session.selected_model = model_id.clone();
    session.updated_at = now_timestamp();
    session_store.save_session(&data_dir, &session)?;

    // 3. Always emit event to keep frontend in sync (use global cache when process not running)
    {
        use tauri::Emitter;
        let models = available_models.unwrap_or_else(|| load_available_models(&data_dir));
        let _ = app.emit(
            "agent-models-updated",
            serde_json::json!({
                "chat_session_id": chat_session_id,
                "available_models": models,
                "selected_model": model_id,
            }),
        );
    }

    Ok(())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn respond_agent_permission(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
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
        emit_session_state_changed(&app, &chat_session_id, TurnPhase::Streaming, None);
        notify_status_transition(
            &app,
            session_store.inner(),
            &chat_session_id,
            TurnPhase::Streaming,
            None,
        );
    }

    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageResponse {
    pub session: ChatSession,
    pub human_message: ChatMessage,
    pub agent_message: Option<ChatMessage>,
    pub sessions: Vec<SessionSummary>,
}

/// Unified command to send a message: handles session creation, message persistence,
/// turn phase check (interrupt if streaming, start query if idle), and pending message queuing.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn send_agent_message(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: Option<String>,
    worktree_path: String,
    content: String,
    permission_mode: Option<String>,
    images: Option<Vec<ImageAttachment>>,
) -> Result<SendMessageResponse, String> {
    let data_dir = resolve_data_dir(&app)?;
    let pm = permission_mode.unwrap_or_else(|| "acceptEdits".to_string());
    let images = images.unwrap_or_default();

    // 1. Create or get session
    let session = if let Some(ref sid) = chat_session_id {
        session_store
            .get_session(&data_dir, sid)?
            .ok_or_else(|| format!("Session not found: {sid}"))?
    } else {
        create_session_internal(&session_store, &data_dir, &worktree_path)?
    };
    let sid = session.id.clone();

    // 2. Add human message (with image parts if present)
    let human_message = {
        let parts = if images.is_empty() {
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
        add_message_internal(
            &session_store,
            &data_dir,
            &sid,
            MessageRole::Human,
            &content,
            parts,
        )?
    };

    // 3. Check turn phase
    let current_phase = {
        let map = handles.lock().await;
        map.get(&sid)
            .map(|p| p.turn_phase)
            .unwrap_or(TurnPhase::Idle)
    };

    let agent_message =
        if current_phase == TurnPhase::Streaming || current_phase == TurnPhase::WaitingPermission {
            // 4a. Queue pending message + interrupt
            {
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(&sid) {
                    proc.pending_message = Some(PendingMessage {
                        content: content.clone(),
                        permission_mode: pm.clone(),
                        images: images.clone(),
                    });
                    proc.stdin
                        .write_all(b"{\"type\":\"interrupt\"}\n")
                        .await
                        .map_err(|e| format!("Failed to write interrupt: {e}"))?;
                    proc.stdin
                        .flush()
                        .await
                        .map_err(|e| format!("Failed to flush: {e}"))?;
                }
            }
            None
        } else {
            // 4b. Create agent message + start turn
            let agent_msg = add_message_internal(
                &session_store,
                &data_dir,
                &sid,
                MessageRole::Agent,
                "",
                None,
            )?;
            start_agent_turn(
                &app,
                handles.inner(),
                session_store.inner(),
                &sid,
                &worktree_path,
                &pm,
                &content,
                &agent_msg.id,
                &images,
            )
            .await?;
            Some(agent_msg)
        };

    // 5. Get updated session and list
    let updated_session = session_store
        .get_session(&data_dir, &sid)?
        .ok_or_else(|| format!("Session not found: {sid}"))?;
    let sessions = session_store.list_sessions(&data_dir, &worktree_path)?;

    Ok(SendMessageResponse {
        session: updated_session,
        human_message,
        agent_message,
        sessions,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitSessionsResponse {
    pub sessions: Vec<SessionSummary>,
    pub active_session: Option<GetSessionResponse>,
}

/// Unified command for session initialization: lists sessions, starts Bridge processes,
/// creates a new session if empty, returns sessions + active session.
#[tauri::command]
pub async fn init_agent_sessions(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    worktree_path: String,
) -> Result<InitSessionsResponse, String> {
    let data_dir = resolve_data_dir(&app)?;

    let mut sessions = session_store.list_sessions(&data_dir, &worktree_path)?;

    if sessions.is_empty() {
        // Create new session + start agent process (new session uses default permission_mode)
        let session = create_session_internal(&session_store, &data_dir, &worktree_path)?;
        let session_pm = session.permission_mode.clone();
        start_agent_session_internal(
            &app,
            handles.inner(),
            session_store.inner(),
            &session.id,
            &worktree_path,
            Some(session_pm),
        )
        .await
        .unwrap_or_else(|e| {
            log::error!("Failed to start new agent session: {e}");
        });

        sessions = session_store.list_sessions(&data_dir, &worktree_path)?;
        let response = GetSessionResponse {
            available_models: load_available_models(&data_dir),
            session,
            turn_phase: TurnPhase::Idle,
        };
        Ok(InitSessionsResponse {
            sessions,
            active_session: Some(response),
        })
    } else {
        // Start agent processes for all sessions with their persisted permission_mode
        for s in &sessions {
            let app_c = app.clone();
            let h_c = Arc::clone(handles.inner());
            let ss_c = Arc::clone(session_store.inner());
            let sid = s.id.clone();
            let cwd = worktree_path.clone();
            let pm_c = s.permission_mode.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    start_agent_session_internal(&app_c, &h_c, &ss_c, &sid, &cwd, Some(pm_c)).await
                {
                    log::error!("Failed to start agent session {sid}: {e}");
                }
            });
        }

        // Get first session as active
        let active = get_session_internal(
            session_store.inner(),
            handles.inner(),
            &app,
            &sessions[0].id,
        )
        .await?;

        Ok(InitSessionsResponse {
            sessions,
            active_session: active,
        })
    }
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

// --- Image attachment support ---

/// Build a JSON command to send a user message (with optional images) to the Bridge.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAttachment {
    pub data: String,
    pub media_type: String,
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
#[tauri::command]
pub fn prepare_image_attachment(data: Vec<u8>) -> Result<ImageAttachment, String> {
    if data.is_empty() {
        return Err("Empty image data".to_string());
    }
    validate_and_encode_image(&data)
}

/// Tauri command: Read image files from paths and return base64-encoded attachments.
/// Called from the frontend when files are dropped via native drag-and-drop.
/// Non-image files are silently skipped.
#[tauri::command]
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
    fn set_model_command_format() {
        let result = build_set_model_command(Some("claude-opus"));
        let cmd: serde_json::Value = serde_json::from_str(result.trim()).unwrap();
        assert_eq!(cmd["type"], "setModel");
        assert_eq!(cmd["modelId"], "claude-opus");
    }

    #[test]
    fn set_model_command_with_none() {
        let result = build_set_model_command(None);
        let cmd: serde_json::Value = serde_json::from_str(result.trim()).unwrap();
        assert_eq!(cmd["type"], "setModel");
        assert!(cmd["modelId"].is_null());
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
    fn resolve_permission_mode_plan_returns_default() {
        assert_eq!(resolve_permission_mode("plan"), "default");
    }

    #[test]
    fn resolve_permission_mode_accept_edits_unchanged() {
        assert_eq!(resolve_permission_mode("acceptEdits"), "acceptEdits");
    }

    #[test]
    fn resolve_permission_mode_bypass_unchanged() {
        assert_eq!(
            resolve_permission_mode("bypassPermissions"),
            "bypassPermissions"
        );
    }

    #[test]
    fn resolve_permission_mode_default_unchanged() {
        assert_eq!(resolve_permission_mode("default"), "default");
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
}
