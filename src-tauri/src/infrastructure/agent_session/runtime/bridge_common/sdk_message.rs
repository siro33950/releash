use super::external_agent::{
    spawn_workflow_turn_complete_notification, ExternalBridgeMessageState,
};
use super::permission::{
    handle_sdk_permission_mode_notification, run_permission_request_transition_locked,
    PermissionRequestTransition,
};
use super::process_registry::{
    AgentProcess, AgentProcessMap, BridgeState, StreamPartRollback, TurnPhase,
};
use super::recovery::run_bridge_error_transition_locked;
use super::session_lifecycle::{
    crash_agent_process_for_context_reinject, run_turn_complete_transition_locked_with_interrupt,
    take_pending_message, token_usage_from_result_message, TurnCompleteTransition,
};
use super::session_persistence::{
    load_persisted_agent_session_id_for_resume, load_post_turn_base_parts_from_store,
    persist_agent_session_id, persist_context_carry_failed_after_init_error,
    persist_context_carry_state, persist_resume_mismatch_for_reinject, persist_streaming_parts,
    session_ready_resume_mismatch, take_defer_agent_session_id_persist_on_ready,
    PERSIST_INTERVAL_MS,
};
use super::shared::{
    consolidate_parts_from_slice, fallback_prompt_message_id, notify_status_transition,
    CLAUDE_BACKEND_ID, CODEX_BACKEND_ID,
};
use super::skills::supported_commands_from_bridge_message;
use super::stream_emit::{
    emit_one_shot_streaming_delta, emit_session_state_changed, emit_streaming_delta,
    enqueue_pending_delta_with_rollbacks, force_flush_pending_streaming, has_pending_stream_flush,
    release_completed_turn_streaming_buffer, should_flush_per_delta, spawn_streaming_timer,
};
use super::turn_event_log::{
    append_parts_to_event_log_in_order, clear_post_turn_store_base_untrusted_for_message,
    mark_post_turn_store_base_untrusted, record_durable_parts_for_current_turn,
};
use crate::app_data_dir::resolve_data_dir;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::acquire_session_runtime_lock;
use crate::infrastructure::agent_session::runtime::turn_latency;
use crate::usecase::agent_session::event_log::InterruptReason;
use crate::usecase::agent_session::event_log::PromptInput;
use crate::usecase::agent_session::event_log::TurnEventLog;
use crate::usecase::agent_session::session::now_timestamp;
use crate::usecase::agent_session::session::MessagePart;
use crate::usecase::agent_session::session::SessionStore;
use crate::usecase::agent_session::session::SystemNotificationType;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

pub(super) fn bridge_message_turn_token(msg: &serde_json::Value) -> Option<&str> {
    msg.get("turn_token")
        .and_then(|v| v.as_str())
        .filter(|token| !token.is_empty())
}

/// Accept token-bearing events for the active turn, or for the retained
/// normally completed turn while post-turn background task updates are allowed.
pub(super) fn bridge_message_is_stale_for_active_turn(
    proc: &AgentProcess,
    msg: &serde_json::Value,
) -> bool {
    let Some(turn_token) = bridge_message_turn_token(msg) else {
        return false;
    };
    if proc.active_turn_token.as_deref() == Some(turn_token) {
        return false;
    }
    if proc.active_turn_token.is_none()
        && proc.last_message_id.is_some()
        && proc.post_turn_message_token.as_deref() == Some(turn_token)
    {
        return false;
    }
    true
}

pub(super) fn append_to_parts(
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

/// Extract tool_result content from SDK content blocks.
pub(super) fn extract_tool_result_content(content: &serde_json::Value) -> String {
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

pub(super) fn extract_todo_items(
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

pub(super) fn todo_update_log(
    items: &[crate::usecase::agent_session::session::TodoListItem],
) -> String {
    let completed = items.iter().filter(|item| item.completed).count();
    format!("TODO を更新しました（{completed}/{} 完了）", items.len())
}

pub(super) fn push_todo_snapshot(
    parts: &mut Vec<MessagePart>,
    items: Vec<crate::usecase::agent_session::session::TodoListItem>,
) -> Option<MessagePart> {
    parts.push(MessagePart::Text {
        content: todo_update_log(&items),
        parent_tool_use_id: None,
    });
    let snapshot = MessagePart::TodoListSnapshot { items };
    if let Some(existing) = parts
        .iter_mut()
        .rev()
        .find(|part| matches!(part, MessagePart::TodoListSnapshot { .. }))
    {
        *existing = snapshot.clone();
        Some(snapshot)
    } else {
        parts.push(snapshot);
        None
    }
}

pub(super) fn push_or_update_tool_result(
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
                if *existing_error && !is_error {
                    delta_content = content.clone();
                    *existing = content;
                    *existing_error = false;
                } else if content.contains(existing.as_str()) || existing.is_empty() {
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

pub(super) fn push_or_update_tool_use(
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
pub(super) fn should_forward_sdk_message(accumulated: bool, msg_type: &str) -> bool {
    !accumulated || msg_type == "permission_request"
}

pub(super) struct SdkMessageAccumulation {
    pub(crate) handled: bool,
    pub(crate) updated_parts: Option<Vec<MessagePart>>,
    pub(crate) liveness: bool,
}

pub(super) fn is_explicit_liveness_progress_message(msg: &serde_json::Value) -> bool {
    msg.get("type").and_then(|v| v.as_str()) == Some("system")
        && matches!(
            msg.get("subtype").and_then(|v| v.as_str()),
            Some("task_started" | "task_notification" | "task_progress" | "task_updated")
        )
}

pub(super) fn accumulate_sdk_message_with_liveness(
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

fn sdk_message_can_update_existing_parts(msg: &serde_json::Value) -> bool {
    match msg.get("type").and_then(|v| v.as_str()).unwrap_or("") {
        "assistant" | "user" | "todo_list_snapshot" => true,
        "system" => matches!(
            msg.get("subtype").and_then(|v| v.as_str()).unwrap_or(""),
            "task_updated" | "compact_boundary"
        ),
        _ => false,
    }
}

fn stream_part_rollbacks(before: &[MessagePart], after: &[MessagePart]) -> Vec<StreamPartRollback> {
    before
        .iter()
        .zip(after.iter())
        .enumerate()
        .filter(|(_, (previous, current))| previous != current)
        .map(|(index, (previous, _))| StreamPartRollback {
            index,
            previous: previous.clone(),
        })
        .collect()
}

/// Extract background task ID from tool_result content.
/// Handles both Task tool ("agentId: a72ca50") and Bash tool ("with ID: b8625ae") formats.
pub(super) fn extract_agent_id(content: &str) -> Option<&str> {
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

pub(super) fn task_id_map_from_parts(parts: &[MessagePart]) -> HashMap<String, String> {
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
pub(super) fn sdk_error_part_from_message(msg: &serde_json::Value) -> MessagePart {
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
pub(super) fn accumulate_sdk_message(
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
                                    if let Some(updated) = push_todo_snapshot(parts, items) {
                                        updated_parts.push(updated);
                                    }
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
                let updated = push_todo_snapshot(parts, items);
                return (true, updated.map(|part| vec![part]));
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

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PostTurnBaseRequirement {
    RequiresBase,
    AccumulatedWithoutParts,
    NotAccumulated,
}

/// Keep this classifier in lockstep with `accumulate_sdk_message`; the
/// `post_turn_base_requirement_matches_accumulate_sdk_message` test covers the
/// msg_type/subtype table and should fail when either side drifts.
pub(super) fn post_turn_base_requirement_for_empty_buffer(
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
pub(super) struct AccumulateStreamMessageEffect {
    pub(crate) accumulated: bool,
    pub(crate) emit_msg_id: Option<String>,
    pub(crate) should_persist: bool,
    pub(crate) persist_parts: Vec<MessagePart>,
    pub(crate) post_turn_reseed_message_id: Option<String>,
    pub(crate) start_streaming_timer: bool,
    pub(crate) released_streaming_parts: Vec<MessagePart>,
}

pub(super) fn accumulate_loaded_post_turn_base_without_streaming_state<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    msg: &serde_json::Value,
    base_mid: String,
    base_parts: Vec<MessagePart>,
    emit_stream: &mut F,
) -> AccumulateStreamMessageEffect
where
    F: FnMut(&str, u64, &[MessagePart], &dyn Fn() -> Vec<MessagePart>) -> (bool, bool),
{
    let old_turn_id = 1;
    let mut old_message_log = TurnEventLog::default();
    old_message_log.begin_turn(
        old_turn_id,
        fallback_prompt_message_id(&base_mid),
        base_mid.clone(),
        PromptInput::default(),
        now_timestamp(),
    );
    append_parts_to_event_log_in_order(&mut old_message_log, old_turn_id, &base_mid, &base_parts);

    let mut parts = base_parts.clone();
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

    append_parts_to_event_log_in_order(&mut old_message_log, old_turn_id, &base_mid, &delta);
    let persist_parts = old_message_log.project().agent_parts_for_message(&base_mid);
    let base_seq = proc
        .streaming_delta_seq_by_message
        .get(&base_mid)
        .copied()
        .unwrap_or_else(|| {
            let targets_current = proc.streaming_message_id.as_deref() == Some(base_mid.as_str());
            let targets_last = proc.last_message_id.as_deref() == Some(base_mid.as_str());
            if targets_current || targets_last {
                proc.streaming_delta_seq
            } else {
                0
            }
        });
    let next_seq = base_seq.saturating_add(1);
    let emitted = emit_one_shot_streaming_delta(
        proc,
        chat_session_id,
        &base_mid,
        next_seq,
        delta.clone(),
        persist_parts.clone(),
        |seq, parts, snapshot_parts| emit_stream(&base_mid, seq, parts, snapshot_parts),
    );

    log::warn!(
        "Persisting stale post-turn streaming reseed into loaded base: \
         session {chat_session_id}, loaded message {base_mid}, current message {:?}, state {:?}",
        proc.last_message_id,
        proc.state
    );

    AccumulateStreamMessageEffect {
        accumulated: true,
        emit_msg_id: Some(base_mid),
        should_persist: emitted,
        persist_parts: if emitted { persist_parts } else { Vec::new() },
        ..AccumulateStreamMessageEffect::default()
    }
}

pub(super) fn accumulate_stream_or_post_turn_message_locked<F>(
    proc: &mut AgentProcess,
    chat_session_id: &str,
    msg: &serde_json::Value,
    elapsed_persist_ms: u64,
    mut emit_stream: F,
    post_turn_base: Option<(String, Vec<MessagePart>)>,
) -> AccumulateStreamMessageEffect
where
    F: FnMut(&str, u64, &[MessagePart], &dyn Fn() -> Vec<MessagePart>) -> (bool, bool),
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
                proc.confirmed_stream_part_len = base_parts.len();
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

    let rollback_source =
        sdk_message_can_update_existing_parts(msg).then(|| proc.streaming_parts.clone());
    let prev_len = proc.streaming_parts.len();
    let accumulation =
        accumulate_sdk_message_with_liveness(msg, &mut proc.streaming_parts, &mut proc.task_id_map);
    let acc = accumulation.handled;
    let updated_parts = accumulation.updated_parts;
    if !acc {
        proc.streaming_parts.truncate(prev_len);
        if post_turn {
            let start_streaming_timer = has_pending_stream_flush(proc);
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
    let rollbacks = rollback_source
        .as_deref()
        .map(|before| stream_part_rollbacks(before, &proc.streaming_parts))
        .unwrap_or_default();

    if delta.is_empty() {
        if post_turn {
            let start_streaming_timer = has_pending_stream_flush(proc);
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

    let durable_parts_appended = if let Some(ref mid) = mid {
        record_durable_parts_for_current_turn(proc, mid, &delta) > 0
    } else {
        false
    };
    enqueue_pending_delta_with_rollbacks(proc, &delta, rollbacks);

    if should_flush_per_delta(proc, &delta, post_turn) {
        if let Some(ref mid) = mid {
            let _ = force_flush_pending_streaming(
                proc,
                chat_session_id,
                mid,
                |seq, parts, snapshot_parts| emit_stream(mid, seq, parts, snapshot_parts),
            );
        }
    }

    let mut should_persist = post_turn || elapsed_persist_ms >= PERSIST_INTERVAL_MS;
    if in_streaming && !durable_parts_appended {
        should_persist = false;
    }
    let persist_parts = if should_persist {
        if in_streaming {
            mid.as_ref()
                .map(|message_id| {
                    proc.turn_event_log
                        .project()
                        .agent_parts_for_message(message_id)
                })
                .unwrap_or_default()
        } else {
            consolidate_parts_from_slice(&proc.streaming_parts)
        }
    } else {
        Vec::new()
    };
    if in_streaming && persist_parts.is_empty() {
        should_persist = false;
    }
    if post_turn && should_persist {
        if let Some(ref mid) = mid {
            mark_post_turn_store_base_untrusted(proc, mid);
        }
    }

    let start_streaming_timer = post_turn && has_pending_stream_flush(proc);
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

pub(super) async fn accumulate_stream_or_post_turn_message<R: tauri::Runtime>(
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
                    |mid, seq, parts, snapshot_parts| {
                        emit_streaming_delta(
                            app,
                            chat_session_id,
                            mid,
                            seq,
                            parts.to_vec(),
                            snapshot_parts,
                        )
                    },
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

async fn streaming_final_seq_for_message(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    message_id: &str,
) -> u64 {
    let map = handles.lock().await;
    map.get(chat_session_id)
        .and_then(|proc| proc.streaming_delta_seq_by_message.get(message_id).copied())
        .unwrap_or(0)
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
        "supported_commands" => {
            let commands = supported_commands_from_bridge_message(&msg);
            let payload = crate::protocol::AgentSupportedCommandsUpdated {
                chat_session_id: chat_session_id.to_string(),
                commands: commands
                    .into_iter()
                    .map(|command| crate::protocol::AgentSupportedCommandMsg {
                        name: command.name,
                        description: command.description,
                        argument_hint: command.argument_hint,
                    })
                    .collect(),
            };
            let _ = app.emit("agent-supported-commands-updated", &payload);
        }
        "telemetry" => {
            let mut map = handles.lock().await;
            if let Some(proc) = map.get_mut(chat_session_id) {
                turn_latency::record_bridge_telemetry_message(
                    &mut proc.turn_latency,
                    proc.active_turn_token.as_deref(),
                    bridge_message_turn_token(&msg),
                    &msg,
                );
            }
        }
        "session_ready" => {
            let (context_carry_on_ready, resume_mismatch, stale_event) = {
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(chat_session_id) {
                    if bridge_message_is_stale_for_active_turn(proc, &msg) {
                        (None, false, true)
                    } else {
                        turn_latency::record_sdk_message(&mut proc.turn_latency, &msg);
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
                            let defer_streaming_claude_session_id = proc.backend_id
                                == CLAUDE_BACKEND_ID
                                && proc.turn_phase != TurnPhase::Idle;
                            if !resume_mismatch
                                && !defer_agent_session_id_persist_on_ready
                                && !defer_streaming_claude_session_id
                            {
                                persist_agent_session_id(app, session_store, chat_session_id, sid);
                            }
                        }
                        (context_carry_on_ready, resume_mismatch, false)
                    }
                } else {
                    (None, false, false)
                }
            };
            if stale_event {
                return;
            }
            if resume_mismatch {
                persist_resume_mismatch_for_reinject(app, session_store, chat_session_id);
                crash_agent_process_for_context_reinject(app, handles, chat_session_id).await;
                return;
            } else if let Some(context_carry) = context_carry_on_ready {
                persist_context_carry_state(app, session_store, chat_session_id, context_carry);
            }
            let _ = app.emit("agent-sdk-message", &msg);
        }
        "session_cleared" => {
            {
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(chat_session_id) {
                    proc.sdk_session_id = None;
                }
            }
            let result = resolve_data_dir(app).and_then(|data_dir| {
                session_store
                    .update_agent_session_id_if_changed(&data_dir, chat_session_id, None)
                    .map(|_| ())
            });
            if let Err(e) = result {
                log::warn!("Failed to clear agent session id for {chat_session_id}: {e}");
            }
            let _ = app.emit("agent-sdk-message", &msg);
        }
        "result" => {
            let stale_event = {
                let map = handles.lock().await;
                map.get(chat_session_id)
                    .is_some_and(|proc| bridge_message_is_stale_for_active_turn(proc, &msg))
            };
            if stale_event {
                return;
            }
            {
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(chat_session_id) {
                    turn_latency::record_sdk_message(&mut proc.turn_latency, &msg);
                }
            }
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
            let interrupted = msg
                .get("interrupted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let completed_session_id = msg
                .get("session_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let rollback_sdk_session_id = if interrupted {
                load_persisted_agent_session_id_for_resume(app, session_store, chat_session_id)
            } else {
                None
            };
            let (effect, context_restore_failed_on_init, stale_event) = {
                let _runtime_guard = acquire_session_runtime_lock(chat_session_id).await;
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(chat_session_id) {
                    if bridge_message_is_stale_for_active_turn(proc, &msg) {
                        (None, false, true)
                    } else {
                        let effect = run_turn_complete_transition_locked_with_interrupt(
                            proc,
                            chat_session_id,
                            exit_code,
                            interrupted.then_some(InterruptReason::Abort),
                            interrupted.then(|| "Turn interrupted by abort".to_string()),
                            |mid, seq, parts, snapshot_parts| {
                                emit_streaming_delta(
                                    app,
                                    chat_session_id,
                                    mid,
                                    seq,
                                    parts.to_vec(),
                                    snapshot_parts,
                                )
                            },
                        );
                        if interrupted {
                            proc.sdk_session_id = rollback_sdk_session_id.clone();
                            proc.post_turn_message_token = None;
                        }
                        let context_restore_failed_on_init = !effect.turn_completed
                            && exit_code != 0
                            && proc.context_carry_on_ready.take().is_some();
                        (Some(effect), context_restore_failed_on_init, false)
                    }
                } else {
                    (None, false, false)
                }
            };
            if stale_event {
                return;
            }

            let Some(effect) = effect else {
                return;
            };
            let TurnCompleteTransition {
                turn_completed,
                final_msg_id,
                final_parts,
                final_streaming_seq,
                workflow_turn_complete,
                projected_session_state,
                released_streaming_parts,
            } = effect;
            if turn_completed {
                if exit_code == 0 && !interrupted {
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
                            final_streaming_seq,
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
                let Some(projected_turn_complete) = workflow_turn_complete.as_ref() else {
                    return;
                };
                emit_session_state_changed(
                    app,
                    chat_session_id,
                    TurnPhase::Idle,
                    Some(projected_turn_complete.exit_code),
                    projected_turn_complete.interrupted,
                );
                notify_status_transition(
                    app,
                    session_store,
                    chat_session_id,
                    TurnPhase::Idle,
                    projected_session_state,
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
                    workflow_turn_complete,
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
            let (transition, stale_event) = {
                let _runtime_guard = acquire_session_runtime_lock(chat_session_id).await;
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(chat_session_id) {
                    if bridge_message_is_stale_for_active_turn(proc, &msg) {
                        (None, true)
                    } else {
                        turn_latency::record_sdk_message(&mut proc.turn_latency, &msg);
                        (
                            Some(run_bridge_error_transition_locked(
                                proc,
                                chat_session_id,
                                &msg,
                                |mid, seq, parts, snapshot_parts| {
                                    emit_streaming_delta(
                                        app,
                                        chat_session_id,
                                        mid,
                                        seq,
                                        parts.to_vec(),
                                        snapshot_parts,
                                    )
                                },
                            )),
                            false,
                        )
                    }
                } else {
                    (None, false)
                }
            };
            if stale_event {
                return;
            }
            let _ = app.emit("agent-sdk-message", &msg);

            let transition = transition.unwrap_or_default();
            let effect = transition.turn_complete;
            if effect.turn_completed {
                if let Some(ref mid) = effect.final_msg_id {
                    if !effect.final_parts.is_empty() {
                        let persisted = persist_streaming_parts(
                            session_store,
                            app,
                            chat_session_id,
                            mid,
                            &effect.final_parts,
                            effect.final_streaming_seq,
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
                let Some(projected_turn_complete) = effect.workflow_turn_complete.as_ref() else {
                    return;
                };
                emit_session_state_changed(
                    app,
                    chat_session_id,
                    TurnPhase::Idle,
                    Some(projected_turn_complete.exit_code),
                    projected_turn_complete.interrupted,
                );
                notify_status_transition(
                    app,
                    session_store,
                    chat_session_id,
                    TurnPhase::Idle,
                    effect.projected_session_state.clone(),
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
                    effect.workflow_turn_complete,
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
            let (stale_event, permission_request_received_at) = {
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(chat_session_id) {
                    if bridge_message_is_stale_for_active_turn(proc, &msg) {
                        (true, None)
                    } else {
                        let permission_request_received_at =
                            (msg_type == "permission_request").then(Instant::now);
                        turn_latency::record_sdk_message(&mut proc.turn_latency, &msg);
                        (false, permission_request_received_at)
                    }
                } else {
                    (false, (msg_type == "permission_request").then(Instant::now))
                }
            };
            if stale_event {
                return;
            }
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
                        streaming_final_seq_for_message(handles, chat_session_id, mid).await,
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

            // Claude SDK は "default"/"acceptEdits"/"bypassPermissions"/"plan" を送る。
            // 永続化は検証済み Tauri/WS リクエストや明示 UI 操作経由に限定するため、
            // ここでは SessionStore の値は書き換えない（Spec issues-947）。
            if msg_type == "system" {
                if let Some(sdk_mode) = msg.get("permissionMode").and_then(|v| v.as_str()) {
                    handle_sdk_permission_mode_notification(
                        sdk_mode,
                        app,
                        session_store,
                        handles,
                        chat_session_id,
                    )
                    .await;
                }
            }

            let permission_transition = if msg_type == "permission_request" {
                let request_id = msg.get("request_id").and_then(|v| v.as_str());
                let mut map = handles.lock().await;
                if let Some(proc) = map.get_mut(chat_session_id) {
                    run_permission_request_transition_locked(
                        proc,
                        chat_session_id,
                        request_id,
                        permission_request_received_at
                            .expect("permission_request receive time captured after stale check"),
                        |mid, seq, parts, snapshot_parts| {
                            emit_streaming_delta(
                                app,
                                chat_session_id,
                                mid,
                                seq,
                                parts.to_vec(),
                                snapshot_parts,
                            )
                        },
                    )
                } else {
                    PermissionRequestTransition::default()
                }
            } else {
                PermissionRequestTransition::default()
            };

            if should_forward_sdk_message(effect.accumulated, msg_type) {
                let _ = app.emit("agent-sdk-message", &msg);
            }
            if permission_transition.did_transition {
                emit_session_state_changed(
                    app,
                    chat_session_id,
                    TurnPhase::WaitingPermission,
                    None,
                    false,
                );
                notify_status_transition(
                    app,
                    session_store,
                    chat_session_id,
                    TurnPhase::WaitingPermission,
                    permission_transition.projected_session_state,
                );
            }
        }
    }
}
#[cfg(test)]
mod moved_tests {

    use super::super::process_registry::make_test_agent_process;
    use super::super::sdk_message::*;

    use crate::usecase::agent_session::session::{MessagePart, SystemNotificationType};

    use std::collections::HashMap;

    #[test]
    fn session_ready_message_parsing() {
        let msg_str = r#"{"type":"session_ready","session_id":"sess-456"}"#;
        let msg: serde_json::Value = serde_json::from_str(msg_str).unwrap();
        assert_eq!(msg["type"], "session_ready");
        assert_eq!(msg["session_id"], "sess-456");
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

    #[tokio::test]
    async fn post_turn_reseed_emit_failure_does_not_persist_or_advance_seq() {
        let mut proc = make_test_agent_process();
        let base_mid = "agent-message-1".to_string();
        proc.streaming_delta_seq_by_message
            .insert(base_mid.clone(), 4);
        let base_parts = vec![MessagePart::Text {
            content: "base".to_string(),
            parent_tool_use_id: None,
        }];
        let msg = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "text_delta", "text": " delta"}
            }
        });
        let mut emitted = Vec::new();

        let effect = accumulate_loaded_post_turn_base_without_streaming_state(
            &mut proc,
            "csid",
            &msg,
            base_mid.clone(),
            base_parts,
            &mut |mid, seq, parts, snapshot_parts| {
                emitted.push((mid.to_string(), seq, parts.to_vec(), snapshot_parts()));
                (false, true)
            },
        );

        assert!(effect.accumulated);
        assert_eq!(effect.emit_msg_id.as_deref(), Some(base_mid.as_str()));
        assert!(!effect.should_persist);
        assert!(effect.persist_parts.is_empty());
        assert_eq!(proc.streaming_delta_seq_by_message.get(&base_mid), Some(&4));
        assert_eq!(proc.streaming_delta_seq, 0);
        assert!(proc.retry_stream_delta.is_none());
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].1, 5);
        let _ = proc.child.kill().await;
    }

    #[tokio::test]
    async fn post_turn_reseed_emit_success_persists_parts_and_advances_seq() {
        let mut proc = make_test_agent_process();
        let base_mid = "agent-message-1".to_string();
        proc.streaming_delta_seq_by_message
            .insert(base_mid.clone(), 4);
        let base_parts = vec![MessagePart::Text {
            content: "base".to_string(),
            parent_tool_use_id: None,
        }];
        let msg = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "text_delta", "text": " delta"}
            }
        });

        let effect = accumulate_loaded_post_turn_base_without_streaming_state(
            &mut proc,
            "csid",
            &msg,
            base_mid.clone(),
            base_parts,
            &mut |_mid, _seq, _parts, _snapshot_parts| (true, true),
        );

        assert!(effect.accumulated);
        assert!(effect.should_persist);
        assert_eq!(proc.streaming_delta_seq_by_message.get(&base_mid), Some(&5));
        assert_eq!(
            effect.persist_parts,
            vec![MessagePart::Text {
                content: "base delta".to_string(),
                parent_tool_use_id: None,
            }]
        );
        let _ = proc.child.kill().await;
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
        let updated_parts = updated.expect("existing snapshot update must be returned as delta");
        assert_eq!(updated_parts.len(), 1);
        assert!(matches!(
            &updated_parts[0],
            MessagePart::TodoListSnapshot { items } if items.is_empty()
        ));
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
    fn test_accumulate_todo_snapshot_initial_addition_uses_new_parts_delta() {
        let msg = serde_json::json!({
            "type": "todo_list_snapshot",
            "items": [{ "text": "new todo", "completed": false }]
        });
        let mut parts = vec![];

        let (handled, updated) = accumulate_sdk_message(&msg, &mut parts, &mut HashMap::new());

        assert!(handled);
        assert!(updated.is_none());
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], MessagePart::Text { .. }));
        assert!(matches!(
            &parts[1],
            MessagePart::TodoListSnapshot { items }
                if items.len() == 1 && items[0].text == "new todo"
        ));
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
}
