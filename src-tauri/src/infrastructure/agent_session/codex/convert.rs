use std::collections::HashMap;

use serde_json::{json, Value};

use crate::domain::agent_session::entities::{
    MessagePart, PermissionQuestion, PermissionQuestionOption, PermissionRequest,
    PermissionRequestBody, PermissionRequestStatus, TokenUsage, TurnResult,
};
use crate::domain::agent_session::gateway::{AgentRuntimeEvent, ResumeOutcome};
use crate::domain::agent_session::value_objects::{
    JsonPayload, SlashCommand, SystemNotificationType,
};

use super::wire::{
    message_kind, AppServerMessageKind, METHOD_INITIALIZE, METHOD_THREAD_RESUME,
    METHOD_THREAD_START, METHOD_TURN_START, NOTIFY_AGENT_MESSAGE_DELTA,
    NOTIFY_COMMAND_OUTPUT_DELTA, NOTIFY_ERROR, NOTIFY_FILE_CHANGE_OUTPUT_DELTA,
    NOTIFY_FILE_CHANGE_PATCH_UPDATED, NOTIFY_ITEM_COMPLETED, NOTIFY_ITEM_STARTED,
    NOTIFY_THREAD_COMPACTED, NOTIFY_THREAD_STARTED, NOTIFY_THREAD_TOKEN_USAGE_UPDATED,
    NOTIFY_TURN_COMPLETED, NOTIFY_TURN_STARTED, REQUEST_COMMAND_APPROVAL,
    REQUEST_FILE_CHANGE_APPROVAL, REQUEST_PERMISSIONS_APPROVAL,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CodexConvertState {
    pub turn_id: Option<String>,
    pub latest_usage: Option<TokenUsage>,
    pub requested_resume_id: Option<String>,
    pub client_response_methods: HashMap<u64, String>,
    pub compaction_in_progress: bool,
}

pub(crate) fn convert_jsonrpc_message(
    message: &Value,
    state: &mut CodexConvertState,
) -> Vec<AgentRuntimeEvent> {
    match message_kind(message) {
        Some(AppServerMessageKind::Response { id }) => convert_response(id, message, state),
        Some(AppServerMessageKind::Request { id, method }) => {
            convert_server_request(id, &method, message.get("params").unwrap_or(&Value::Null))
        }
        Some(AppServerMessageKind::Notification { method }) => convert_notification(
            &method,
            message.get("params").unwrap_or(&Value::Null),
            state,
        ),
        None => Vec::new(),
    }
}

fn convert_response(
    id: u64,
    message: &Value,
    state: &mut CodexConvertState,
) -> Vec<AgentRuntimeEvent> {
    let source_method = state.client_response_methods.remove(&id);
    if let Some(error) = message.get("error") {
        if matches!(
            source_method.as_deref(),
            Some(METHOD_INITIALIZE | METHOD_THREAD_START | METHOD_THREAD_RESUME)
        ) {
            let mut events = Vec::new();
            if state.requested_resume_id.is_some() {
                events.push(AgentRuntimeEvent::BackendSessionCleared);
            }
            events.push(AgentRuntimeEvent::Fatal {
                message: error_message(error),
            });
            return events;
        }
        let message = error_message(error);
        if source_method.as_deref() == Some(METHOD_TURN_START) {
            state.turn_id = None;
            return vec![
                AgentRuntimeEvent::PartsMerged(vec![MessagePart::Error {
                    content: message.clone(),
                    parent_tool_use_id: None,
                }]),
                AgentRuntimeEvent::TurnCompleted(TurnResult::Failed {
                    error: message,
                    token_usage: state.latest_usage,
                }),
            ];
        }
        log::warn!(
            "unhandled Codex JSON-RPC error response id={id} method={:?}: {message}",
            source_method
        );
        return Vec::new();
    }
    let result = message.get("result").unwrap_or(&Value::Null);
    if let Some(thread_id) = get_string(result, &["thread", "id"]) {
        return vec![AgentRuntimeEvent::SessionEstablished {
            backend_session_id: thread_id.to_string(),
            resume: resume_outcome(state.requested_resume_id.as_deref(), thread_id),
        }];
    }
    let commands = slash_commands_from_result(result);
    if !commands.is_empty() {
        return vec![AgentRuntimeEvent::SlashCommandsUpdated(commands)];
    }
    Vec::new()
}

fn convert_notification(
    method: &str,
    params: &Value,
    state: &mut CodexConvertState,
) -> Vec<AgentRuntimeEvent> {
    match method {
        NOTIFY_THREAD_STARTED => get_string(params, &["thread", "id"])
            .map(|thread_id| {
                vec![AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: thread_id.to_string(),
                    resume: resume_outcome(state.requested_resume_id.as_deref(), thread_id),
                }]
            })
            .unwrap_or_default(),
        NOTIFY_TURN_STARTED => {
            if let Some(turn_id) = get_string(params, &["turn", "id"]) {
                state.turn_id = Some(turn_id.to_string());
            }
            Vec::new()
        }
        NOTIFY_AGENT_MESSAGE_DELTA => get_string(params, &["delta"])
            .map(|delta| {
                parts(vec![MessagePart::Text {
                    content: delta.to_string(),
                    parent_tool_use_id: None,
                }])
            })
            .unwrap_or_default(),
        NOTIFY_ITEM_STARTED => {
            let item = params.get("item").unwrap_or(&Value::Null);
            if is_context_compaction_item(item) {
                state.compaction_in_progress = true;
                return parts(vec![compaction_notification(
                    "in_progress",
                    "Compacting conversation",
                )]);
            }
            parts(item_started_parts(item))
        }
        NOTIFY_ITEM_COMPLETED => {
            let item = params.get("item").unwrap_or(&Value::Null);
            if is_context_compaction_item(item) {
                state.compaction_in_progress = false;
                return parts(vec![compaction_notification(
                    "completed",
                    "Conversation compacted",
                )]);
            }
            parts(item_completed_parts(item))
        }
        NOTIFY_COMMAND_OUTPUT_DELTA | NOTIFY_FILE_CHANGE_OUTPUT_DELTA => {
            command_output_delta_part(params)
                .map(parts_one)
                .unwrap_or_default()
        }
        NOTIFY_FILE_CHANGE_PATCH_UPDATED => parts(file_change_patch_parts(params)),
        NOTIFY_THREAD_COMPACTED => {
            state.compaction_in_progress = false;
            parts(vec![compaction_notification(
                "completed",
                "Conversation compacted",
            )])
        }
        NOTIFY_THREAD_TOKEN_USAGE_UPDATED => {
            let usage = token_usage_from_value(params);
            state.latest_usage = Some(usage);
            vec![AgentRuntimeEvent::TokenUsageUpdated(usage)]
        }
        NOTIFY_ERROR => parts(vec![MessagePart::Error {
            content: get_string(params, &["error", "message"])
                .unwrap_or("Codex app-server error")
                .to_string(),
            parent_tool_use_id: None,
        }]),
        NOTIFY_TURN_COMPLETED => {
            state.turn_id = None;
            let compaction_was_in_progress = state.compaction_in_progress;
            state.compaction_in_progress = false;
            let status = get_string(params, &["turn", "status"]).unwrap_or("completed");
            let result = match status {
                "failed" | "errored" => {
                    let error = get_string(params, &["turn", "error", "message"])
                        .unwrap_or("Codex turn failed")
                        .to_string();
                    let mut failure_parts = Vec::new();
                    if compaction_was_in_progress {
                        failure_parts.push(compaction_notification("failed", "Compaction failed"));
                    }
                    failure_parts.push(MessagePart::Error {
                        content: error.clone(),
                        parent_tool_use_id: None,
                    });
                    return vec![
                        AgentRuntimeEvent::PartsMerged(failure_parts),
                        AgentRuntimeEvent::TurnCompleted(TurnResult::Failed {
                            error,
                            token_usage: state.latest_usage,
                        }),
                    ];
                }
                "interrupted" => TurnResult::Interrupted {
                    reason: crate::domain::agent_session::entities::InterruptReason::Abort,
                    error: None,
                },
                _ => TurnResult::Completed {
                    stop_reason: None,
                    token_usage: state.latest_usage,
                },
            };
            vec![AgentRuntimeEvent::TurnCompleted(result)]
        }
        _ => Vec::new(),
    }
}

fn resume_outcome(requested: Option<&str>, actual: &str) -> ResumeOutcome {
    match requested {
        Some(requested) if requested == actual => ResumeOutcome::Resumed,
        Some(_) => ResumeOutcome::Mismatch {
            actual: actual.to_string(),
        },
        None => ResumeOutcome::NotRequested,
    }
}

fn convert_server_request(id: u64, method: &str, params: &Value) -> Vec<AgentRuntimeEvent> {
    let Some(request) = permission_request_from_server_request(id, method, params) else {
        return Vec::new();
    };
    vec![AgentRuntimeEvent::PermissionRequested(request)]
}

fn get_string<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str()
}

fn payload(value: Value) -> JsonPayload {
    JsonPayload::new_unchecked(value.to_string())
}

fn parts(parts: Vec<MessagePart>) -> Vec<AgentRuntimeEvent> {
    if parts.is_empty() {
        Vec::new()
    } else {
        vec![AgentRuntimeEvent::PartsMerged(parts)]
    }
}

fn parts_one(part: MessagePart) -> Vec<AgentRuntimeEvent> {
    parts(vec![part])
}

fn tool_use_part(tool: &str, input: Value, id: &str) -> MessagePart {
    MessagePart::ToolUse {
        id: id.to_string(),
        tool: tool.to_string(),
        input: payload(input),
        parent_tool_use_id: None,
    }
}

fn tool_result_part(id: &str, content: String, is_error: bool) -> MessagePart {
    MessagePart::ToolResult {
        content,
        is_error,
        tool_use_id: Some(id.to_string()),
        parent_tool_use_id: None,
        content_ref: None,
        summary: None,
    }
}

fn is_context_compaction_item(item: &Value) -> bool {
    get_string(item, &["type"]) == Some("contextCompaction")
}

fn compaction_notification(status: impl Into<String>, label: impl Into<String>) -> MessagePart {
    MessagePart::SystemNotification {
        notification_type: SystemNotificationType::Compaction,
        status: status.into(),
        label: label.into(),
        detail: None,
        hook_id: None,
    }
}

fn item_type_name(item_type: &str) -> &str {
    match item_type {
        "command_execution" => "commandExecution",
        "file_change" => "fileChange",
        "mcp_tool_call" => "mcpToolCall",
        "web_search" => "webSearch",
        other => other,
    }
}

fn item_started_parts(item: &Value) -> Vec<MessagePart> {
    let item_id = get_string(item, &["id"]).unwrap_or("");
    let item_type = get_string(item, &["type"])
        .map(item_type_name)
        .unwrap_or("");
    if item_type == "fileChange" {
        return file_change_tool_parts(
            item_id,
            &item.get("changes").cloned().unwrap_or_else(|| json!([])),
            false,
        );
    }
    let Some(tool) = item_tool_name(item_type, item) else {
        return Vec::new();
    };
    let input = match item_type {
        "commandExecution" => json!({
            "command": item.get("command").cloned().unwrap_or(Value::Null),
            "cwd": item.get("cwd").cloned().unwrap_or(Value::Null),
            "status": item.get("status").cloned().unwrap_or(Value::Null),
        }),
        "mcpToolCall" => json!({
            "server": item.get("server").cloned().unwrap_or(Value::Null),
            "tool": item.get("tool").cloned().unwrap_or(Value::Null),
            "arguments": item.get("arguments").cloned().unwrap_or(Value::Null),
        }),
        "webSearch" => json!({ "query": item.get("query").cloned().unwrap_or(Value::Null) }),
        "dynamicToolCall" => json!({
            "tool": item.get("tool").cloned().unwrap_or(Value::Null),
            "arguments": item.get("arguments").cloned().unwrap_or(Value::Null),
        }),
        _ => return Vec::new(),
    };
    vec![tool_use_part(&tool, input, item_id)]
}

fn item_completed_parts(item: &Value) -> Vec<MessagePart> {
    let item_id = get_string(item, &["id"]).unwrap_or("");
    let item_type = get_string(item, &["type"])
        .map(item_type_name)
        .unwrap_or("");
    match item_type {
        "reasoning" => get_string(item, &["text"])
            .or_else(|| get_string(item, &["summary", "text"]))
            .map(|content| MessagePart::Thinking {
                content: content.to_string(),
                parent_tool_use_id: None,
            })
            .into_iter()
            .collect(),
        "webSearch" => vec![tool_result_part(
            item_id,
            "Web search completed.".to_string(),
            false,
        )],
        "commandExecution" => vec![tool_result_part(
            item_id,
            command_result_content(item),
            command_is_error(item),
        )],
        "fileChange" => {
            let is_error = !matches!(get_string(item, &["status"]), Some("completed"));
            file_change_tool_parts(
                item_id,
                &item.get("changes").cloned().unwrap_or_else(|| json!([])),
                is_error,
            )
        }
        "mcpToolCall" => vec![tool_result_part(
            item_id,
            mcp_result_content(item)
                .unwrap_or_else(|| "Codex MCP tool call completed.".to_string()),
            item.get("error").is_some()
                || get_string(item, &["status"])
                    .is_some_and(|status| matches!(status, "failed" | "errored" | "declined")),
        )],
        "dynamicToolCall" => vec![tool_result_part(
            item_id,
            serde_json::to_string(item)
                .unwrap_or_else(|_| "Codex tool call completed.".to_string()),
            get_string(item, &["status"])
                .is_some_and(|status| matches!(status, "failed" | "errored" | "declined")),
        )],
        _ => Vec::new(),
    }
}

fn item_tool_name(item_type: &str, item: &Value) -> Option<String> {
    match item_type {
        "commandExecution" => Some("Bash".to_string()),
        "fileChange" => Some("Edit".to_string()),
        "webSearch" => Some("WebSearch".to_string()),
        "mcpToolCall" => Some(format!(
            "mcp__{}__{}",
            get_string(item, &["server"]).unwrap_or("server"),
            get_string(item, &["tool"]).unwrap_or("tool")
        )),
        "dynamicToolCall" => get_string(item, &["tool"])
            .map(ToString::to_string)
            .or_else(|| Some("CodexTool".to_string())),
        _ => None,
    }
}

fn file_change_tool_parts(item_id: &str, changes: &Value, is_error: bool) -> Vec<MessagePart> {
    let Some(changes) = changes.as_array() else {
        return Vec::new();
    };
    changes
        .iter()
        .enumerate()
        .flat_map(|(index, change)| {
            let path = get_string(change, &["path"]).unwrap_or("");
            let diff = get_string(change, &["diff"]).unwrap_or("");
            let tool_use_id = format!("{item_id}:{}", sanitized_suffix(path, index));
            let input = json!({
                "file_path": path,
                "kind": file_change_kind(change),
                "diff": diff,
                "changes": [file_change_metadata_without_diff(change)],
            });
            let content = if diff.is_empty() {
                serde_json::to_string(change)
                    .unwrap_or_else(|_| "Codex file change completed.".to_string())
            } else {
                diff.to_string()
            };
            [
                tool_use_part("Edit", input, &tool_use_id),
                tool_result_part(&tool_use_id, content, is_error),
            ]
        })
        .collect()
}

fn sanitized_suffix(path: &str, index: usize) -> String {
    if path.is_empty() {
        return format!("file_{}", index + 1);
    }
    path.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn file_change_kind(change: &Value) -> String {
    change
        .get("kind")
        .and_then(|kind| {
            kind.as_str()
                .or_else(|| kind.get("type").and_then(Value::as_str))
        })
        .unwrap_or("update")
        .to_string()
}

fn file_change_metadata_without_diff(change: &Value) -> Value {
    let Some(object) = change.as_object() else {
        return change.clone();
    };
    Value::Object(
        object
            .iter()
            .filter(|(key, _)| key.as_str() != "diff")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

fn command_output_delta_part(params: &Value) -> Option<MessagePart> {
    let item_id = get_string(params, &["itemId"])?;
    let delta = get_string(params, &["delta"])?;
    Some(tool_result_part(item_id, delta.to_string(), false))
}

fn file_change_patch_parts(params: &Value) -> Vec<MessagePart> {
    let Some(item_id) = get_string(params, &["itemId"]) else {
        return Vec::new();
    };
    file_change_tool_parts(
        item_id,
        &params.get("changes").cloned().unwrap_or_else(|| json!([])),
        false,
    )
}

fn command_result_content(item: &Value) -> String {
    item.get("aggregatedOutput")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            let command = get_string(item, &["command"]).unwrap_or("command");
            let status = get_string(item, &["status"]).unwrap_or("completed");
            format!("Codex command `{command}` finished with status {status}.")
        })
}

fn command_is_error(item: &Value) -> bool {
    matches!(
        get_string(item, &["status"]),
        Some("failed") | Some("declined")
    ) || item
        .get("exitCode")
        .and_then(Value::as_i64)
        .is_some_and(|code| code != 0)
}

fn mcp_result_content(item: &Value) -> Option<String> {
    if let Some(message) = get_string(item, &["error", "message"]) {
        return Some(message.to_string());
    }
    let content = item.get("result")?.get("content")?.as_array()?;
    let parts = content
        .iter()
        .filter_map(|block| {
            block
                .get("text")
                .and_then(Value::as_str)
                .or_else(|| block.get("content").and_then(Value::as_str))
                .or_else(|| block.as_str())
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn permission_request_from_server_request(
    id: u64,
    method: &str,
    params: &Value,
) -> Option<PermissionRequest> {
    let request_id = id.to_string();
    let item_id = get_string(params, &["itemId"])
        .or_else(|| get_string(params, &["item", "id"]))
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("app-server-request-{id}"));
    let (tool_name, body, title, description) = match method {
        REQUEST_COMMAND_APPROVAL => (
            "CodexCommand",
            PermissionRequestBody::ToolApproval {
                input: payload(params.clone()),
            },
            "Codex approval requested",
            Some("Command approval".to_string()),
        ),
        REQUEST_FILE_CHANGE_APPROVAL => (
            "CodexFileChange",
            PermissionRequestBody::ToolApproval {
                input: payload(params.clone()),
            },
            "Codex approval requested",
            Some("File change approval".to_string()),
        ),
        REQUEST_PERMISSIONS_APPROVAL => (
            "CodexPermissions",
            PermissionRequestBody::PermissionGrant {
                requested: payload(params.clone()),
            },
            "Codex permissions requested",
            Some("Additional permissions".to_string()),
        ),
        method if super::wire::is_user_input_request_method(method) => {
            return Some(user_input_permission(request_id, params.clone()));
        }
        _ => return None,
    };
    Some(PermissionRequest {
        id: request_id,
        tool_use_id: Some(item_id),
        parent_tool_use_id: None,
        tool_name: tool_name.to_string(),
        body,
        title: Some(title.to_string()),
        display_name: Some(tool_name.to_string()),
        description,
        decision_reason: None,
        status: PermissionRequestStatus::Pending,
    })
}

fn user_input_permission(id: String, arguments: Value) -> PermissionRequest {
    PermissionRequest {
        id: id.clone(),
        tool_use_id: Some(id),
        parent_tool_use_id: None,
        tool_name: "AskUserQuestion".to_string(),
        body: PermissionRequestBody::Question {
            questions: codex_questions(arguments),
        },
        title: Some("Question".to_string()),
        display_name: Some("Question".to_string()),
        description: Some("Codex requests user input".to_string()),
        decision_reason: None,
        status: PermissionRequestStatus::Pending,
    }
}

fn codex_questions(arguments: Value) -> Vec<PermissionQuestion> {
    if let Some(questions) = arguments.get("questions").and_then(Value::as_array) {
        return questions.iter().map(question_from_value).collect();
    }
    vec![question_from_value(&arguments)]
}

fn question_from_value(value: &Value) -> PermissionQuestion {
    let options = value
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|option| {
            if let Some(label) = option.as_str() {
                return Some(PermissionQuestionOption {
                    label: label.to_string(),
                    description: None,
                });
            }
            let label = option.get("label").and_then(Value::as_str)?;
            Some(PermissionQuestionOption {
                label: label.to_string(),
                description: option
                    .get("description")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            })
        })
        .collect();
    PermissionQuestion {
        question: value
            .get("question")
            .or_else(|| value.get("prompt"))
            .and_then(Value::as_str)
            .unwrap_or("Please choose an option.")
            .to_string(),
        header: value
            .get("header")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        options,
        multi_select: value
            .get("multiSelect")
            .or_else(|| value.get("multi_select"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

fn token_usage_from_value(params: &Value) -> TokenUsage {
    let usage = params.get("usage").unwrap_or(params);
    let input_tokens = usage
        .get("inputTokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("outputTokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .get("totalTokens")
        .or_else(|| usage.get("total_tokens"))
        .and_then(Value::as_u64)
        .or_else(|| Some(input_tokens + output_tokens));
    let context_window_tokens = usage
        .get("contextWindowTokens")
        .or_else(|| usage.get("context_window_tokens"))
        .and_then(Value::as_u64);
    TokenUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        context_window_tokens,
    }
}

fn slash_commands_from_result(result: &Value) -> Vec<SlashCommand> {
    result
        .get("commands")
        .or_else(|| result.get("slashCommands"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|command| {
            let name = command
                .get("name")
                .or_else(|| command.get("command"))
                .and_then(Value::as_str)
                .or_else(|| command.as_str())?;
            Some(SlashCommand {
                name: name.to_string(),
                description: command
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                argument_hint: command
                    .get("argumentHint")
                    .or_else(|| command.get("argument_hint"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            })
        })
        .collect()
}

fn error_message(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Codex app-server request failed")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(value: Value) -> Vec<AgentRuntimeEvent> {
        convert_jsonrpc_message(&value, &mut CodexConvertState::default())
    }

    #[test]
    fn test_convert_thread_startedは_session_establishedへ変換する() {
        let events = convert(json!({
            "method": "thread/started",
            "params": { "thread": { "id": "thread-1" } }
        }));

        assert_eq!(
            events,
            vec![AgentRuntimeEvent::SessionEstablished {
                backend_session_id: "thread-1".to_string(),
                resume: ResumeOutcome::NotRequested,
            }]
        );
    }

    #[test]
    fn test_convert_agent_message_deltaは_text_partへ変換する() {
        let events = convert(json!({
            "method": "item/agentMessage/delta",
            "params": { "delta": "hello" }
        }));

        assert!(matches!(
            &events[0],
            AgentRuntimeEvent::PartsMerged(parts)
                if matches!(&parts[0], MessagePart::Text { content, .. } if content == "hello")
        ));
    }

    #[test]
    fn test_convert_command_executionは_bash_tool_partsへ変換する() {
        let events = convert(json!({
            "method": "item/started",
            "params": {
                "item": {
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "cargo test",
                    "cwd": "/repo",
                    "status": "inProgress"
                }
            }
        }));

        assert!(matches!(
            &events[0],
            AgentRuntimeEvent::PartsMerged(parts)
                if matches!(&parts[0], MessagePart::ToolUse { tool, input, .. }
                    if tool == "Bash" && input.as_str().contains("cargo test"))
        ));
    }

    #[test]
    fn test_convert_file_changeは_legacy_markerなしのdiff_shapeを使う() {
        let events = convert(json!({
            "method": "item/completed",
            "params": {
                "item": {
                    "type": "fileChange",
                    "id": "edit-1",
                    "status": "completed",
                    "changes": [{
                        "path": "src/lib.rs",
                        "kind": "update",
                        "diff": "@@ -1 +1 @@"
                    }]
                }
            }
        }));

        let AgentRuntimeEvent::PartsMerged(parts) = &events[0] else {
            panic!("expected parts");
        };
        assert!(matches!(&parts[0], MessagePart::ToolUse { tool, input, .. }
            if tool == "Edit"
                && serde_json::from_str::<serde_json::Value>(input.as_str())
                    .ok()
                    .and_then(|value| value.get("diff").cloned())
                    .is_some()));
    }

    #[test]
    fn test_convert_user_input_requestは_question_permissionへ変換する() {
        let events = convert(json!({
            "id": 42,
            "method": "item/tool/requestUserInput",
            "params": {
                "question": "Deploy?",
                "header": "Confirm",
                "options": [{ "label": "Yes", "description": "Deploy now" }]
            }
        }));

        assert!(matches!(
            &events[0],
            AgentRuntimeEvent::PermissionRequested(PermissionRequest {
                tool_name,
                body: PermissionRequestBody::Question { questions },
                ..
            }) if tool_name == "AskUserQuestion"
                && questions[0].question == "Deploy?"
                && questions[0].options[0].label == "Yes"
        ));
    }

    #[test]
    fn test_convert_permissions_requestは_permission_grantへ変換する() {
        let events = convert(json!({
            "id": 7,
            "method": "item/permissions/requestApproval",
            "params": { "permissions": { "network": { "enabled": true } } }
        }));

        assert!(matches!(
            &events[0],
            AgentRuntimeEvent::PermissionRequested(PermissionRequest {
                tool_name,
                body: PermissionRequestBody::PermissionGrant { .. },
                ..
            }) if tool_name == "CodexPermissions"
        ));
    }

    #[test]
    fn test_convert_context_compaction_startedは_in_progress通知へ変換する() {
        let mut state = CodexConvertState::default();
        let events = convert_jsonrpc_message(
            &json!({
                "method": "item/started",
                "params": { "item": { "type": "contextCompaction", "id": "compact-1" } }
            }),
            &mut state,
        );

        assert_eq!(
            events,
            vec![AgentRuntimeEvent::PartsMerged(vec![
                MessagePart::SystemNotification {
                    notification_type: SystemNotificationType::Compaction,
                    status: "in_progress".to_string(),
                    label: "Compacting conversation".to_string(),
                    detail: None,
                    hook_id: None,
                }
            ])]
        );
        assert!(state.compaction_in_progress);
    }

    #[test]
    fn test_convert_context_compaction_completedは_completed通知で閉じる() {
        let mut state = CodexConvertState::default();
        convert_jsonrpc_message(
            &json!({
                "method": "item/started",
                "params": { "item": { "type": "contextCompaction", "id": "compact-1" } }
            }),
            &mut state,
        );

        let events = convert_jsonrpc_message(
            &json!({
                "method": "item/completed",
                "params": { "item": { "type": "contextCompaction", "id": "compact-1" } }
            }),
            &mut state,
        );

        assert_eq!(
            events,
            vec![AgentRuntimeEvent::PartsMerged(vec![
                MessagePart::SystemNotification {
                    notification_type: SystemNotificationType::Compaction,
                    status: "completed".to_string(),
                    label: "Conversation compacted".to_string(),
                    detail: None,
                    hook_id: None,
                }
            ])]
        );
        assert!(!state.compaction_in_progress);
    }

    #[test]
    fn test_convert_turn_failedは_error_partと_failed完了へ変換する() {
        let mut state = CodexConvertState {
            turn_id: Some("turn-1".to_string()),
            ..Default::default()
        };

        let events = convert_jsonrpc_message(
            &json!({
                "method": "turn/completed",
                "params": {
                    "turn": {
                        "id": "turn-1",
                        "status": "failed",
                        "error": { "message": "boom" }
                    }
                }
            }),
            &mut state,
        );

        assert_eq!(
            events,
            vec![
                AgentRuntimeEvent::PartsMerged(vec![MessagePart::Error {
                    content: "boom".to_string(),
                    parent_tool_use_id: None,
                }]),
                AgentRuntimeEvent::TurnCompleted(TurnResult::Failed {
                    error: "boom".to_string(),
                    token_usage: None,
                }),
            ]
        );
        assert_eq!(state.turn_id, None);
    }

    #[test]
    fn test_convert_compact失敗turnは_error_partで構造化しcompaction通知をfailedで閉じる() {
        let compact_error = "Error running remote compact task: stream disconnected before completion: error sending request for url (https://chatgpt.com/backend-api/codex/responses/compact)";
        let mut state = CodexConvertState::default();
        let mut events = Vec::new();
        events.extend(convert_jsonrpc_message(
            &json!({
                "method": "item/started",
                "params": { "item": { "type": "contextCompaction", "id": "compact-1" } }
            }),
            &mut state,
        ));
        events.extend(convert_jsonrpc_message(
            &json!({
                "method": "error",
                "params": { "error": { "message": compact_error }, "willRetry": false }
            }),
            &mut state,
        ));
        events.extend(convert_jsonrpc_message(
            &json!({
                "method": "turn/completed",
                "params": {
                    "turn": { "status": "failed", "error": { "message": compact_error } }
                }
            }),
            &mut state,
        ));

        assert!(matches!(
            events.last(),
            Some(AgentRuntimeEvent::TurnCompleted(TurnResult::Failed { error, .. }))
                if error == compact_error
        ));

        let mut merged = Vec::new();
        for event in &events {
            if let AgentRuntimeEvent::PartsMerged(parts) = event {
                for part in parts {
                    crate::domain::agent_session::entities::merge_part(&mut merged, part.clone());
                }
            }
        }
        assert!(!merged
            .iter()
            .any(|part| matches!(part, MessagePart::Text { .. })));
        assert_eq!(
            merged
                .iter()
                .filter(|part| matches!(
                    part,
                    MessagePart::Error { content, .. } if content == compact_error
                ))
                .count(),
            1
        );
        assert_eq!(
            merged
                .iter()
                .filter_map(|part| match part {
                    MessagePart::SystemNotification {
                        notification_type: SystemNotificationType::Compaction,
                        status,
                        ..
                    } => Some(status.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec!["failed"]
        );
        assert!(!state.compaction_in_progress);
    }

    #[test]
    fn test_convert_token_usageと_turn_completedを反映する() {
        let mut state = CodexConvertState {
            turn_id: Some("turn-1".to_string()),
            ..Default::default()
        };
        let usage_events = convert_jsonrpc_message(
            &json!({
                "method": "thread/tokenUsage/updated",
                "params": { "inputTokens": 3, "outputTokens": 5, "totalTokens": 8 }
            }),
            &mut state,
        );
        let done_events = convert_jsonrpc_message(
            &json!({
                "method": "turn/completed",
                "params": { "turn": { "status": "completed" } }
            }),
            &mut state,
        );

        assert!(matches!(
            usage_events[0],
            AgentRuntimeEvent::TokenUsageUpdated(TokenUsage {
                input_tokens: 3,
                output_tokens: 5,
                total_tokens: Some(8),
                ..
            })
        ));
        assert!(matches!(
            done_events[0],
            AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                token_usage: Some(TokenUsage {
                    total_tokens: Some(8),
                    ..
                }),
                ..
            })
        ));
        assert_eq!(state.turn_id, None);
    }

    #[test]
    fn test_startup_response_errorは新規threadでも_fatalにする() {
        let mut state = CodexConvertState {
            ..CodexConvertState::default()
        };
        state
            .client_response_methods
            .insert(2, METHOD_THREAD_START.to_string());
        let events = convert_jsonrpc_message(
            &json!({
                "id": 2,
                "error": { "message": "bad api key" }
            }),
            &mut state,
        );

        assert_eq!(
            events,
            vec![AgentRuntimeEvent::Fatal {
                message: "bad api key".to_string(),
            }]
        );
    }

    #[test]
    fn test_initialize_response_errorは_fatalにする() {
        let mut state = CodexConvertState {
            requested_resume_id: Some("thread-old".to_string()),
            ..CodexConvertState::default()
        };
        state
            .client_response_methods
            .insert(1, METHOD_INITIALIZE.to_string());

        let events = convert_jsonrpc_message(
            &json!({
                "id": 1,
                "error": { "message": "initialize failed" }
            }),
            &mut state,
        );

        assert_eq!(
            events,
            vec![
                AgentRuntimeEvent::BackendSessionCleared,
                AgentRuntimeEvent::Fatal {
                    message: "initialize failed".to_string(),
                },
            ]
        );
    }

    #[test]
    fn test_startup_response_errorは_resume時だけ_backend_session_clearedを先行する() {
        let mut state = CodexConvertState {
            requested_resume_id: Some("thread-old".to_string()),
            ..CodexConvertState::default()
        };
        state
            .client_response_methods
            .insert(2, METHOD_THREAD_RESUME.to_string());
        let events = convert_jsonrpc_message(
            &json!({
                "id": 2,
                "error": { "message": "not found" }
            }),
            &mut state,
        );

        assert!(matches!(
            events[0],
            AgentRuntimeEvent::BackendSessionCleared
        ));
        assert!(matches!(events[1], AgentRuntimeEvent::Fatal { .. }));
    }

    #[test]
    fn test_untracked_response_errorは_transcript_errorへ変換しない() {
        let events = convert(json!({
            "id": 99,
            "error": { "message": "rename failed" }
        }));

        assert!(events.is_empty());
    }

    #[test]
    fn test_turn_start_response_errorは_error_partと_failed完了へ変換する() {
        let mut state = CodexConvertState::default();
        state
            .client_response_methods
            .insert(101, METHOD_TURN_START.to_string());

        let events = convert_jsonrpc_message(
            &json!({
                "id": 101,
                "error": { "message": "invalid model" }
            }),
            &mut state,
        );

        assert_eq!(
            events,
            vec![
                AgentRuntimeEvent::PartsMerged(vec![MessagePart::Error {
                    content: "invalid model".to_string(),
                    parent_tool_use_id: None,
                }]),
                AgentRuntimeEvent::TurnCompleted(TurnResult::Failed {
                    error: "invalid model".to_string(),
                    token_usage: None,
                }),
            ]
        );
    }

    #[test]
    fn test_initialize_response_commandsを_slash_commands_updatedへ変換する() {
        let events = convert(json!({
            "id": 1,
            "result": {
                "commands": [
                    {
                        "name": "/review",
                        "description": "Review changes",
                        "argumentHint": "[path]"
                    }
                ]
            }
        }));

        assert_eq!(
            events,
            vec![AgentRuntimeEvent::SlashCommandsUpdated(vec![
                SlashCommand {
                    name: "/review".to_string(),
                    description: "Review changes".to_string(),
                    argument_hint: Some("[path]".to_string()),
                }
            ])]
        );
    }
}
