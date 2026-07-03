use std::collections::HashMap;

use serde_json::{json, Value};

use crate::domain::agent_session::entities::{
    MessagePart, PermissionDecision, PermissionRequest, PermissionRequestStatus, TokenUsage,
    TurnResult,
};
use crate::domain::agent_session::gateway::{AgentRuntimeEvent, ResumeOutcome};
use crate::domain::agent_session::value_objects::{
    JsonPayload, SlashCommand, SystemNotificationType, TodoListItem,
};

use super::permission::{permission_action_from_can_use_tool, ClaudePermissionAction};
use super::wire::{
    control_request_subtype, message_subtype, message_type, permission_mode_from_wire,
    ClaudeWireMode, DELTA_TEXT, DELTA_THINKING, STREAM_CONTENT_BLOCK_DELTA, SUBTYPE_CAN_USE_TOOL,
    SUBTYPE_PERMISSION_DENIED, SYSTEM_COMPACT_BOUNDARY, SYSTEM_INIT, SYSTEM_STATUS,
    SYSTEM_TASK_NOTIFICATION, SYSTEM_TASK_PROGRESS, SYSTEM_TASK_STARTED, SYSTEM_TASK_UPDATED,
    TYPE_ASSISTANT, TYPE_CONTROL_REQUEST, TYPE_CONTROL_RESPONSE, TYPE_KEEP_ALIVE, TYPE_RESULT,
    TYPE_STREAM_EVENT, TYPE_SYSTEM, TYPE_USER,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaudeConversion {
    pub events: Vec<AgentRuntimeEvent>,
    pub auto_responses: Vec<Value>,
}

impl ClaudeConversion {
    fn events(events: Vec<AgentRuntimeEvent>) -> Self {
        Self {
            events,
            auto_responses: Vec::new(),
        }
    }

    fn none() -> Self {
        Self::events(Vec::new())
    }

    fn auto_response(response: Value) -> Self {
        Self {
            events: Vec::new(),
            auto_responses: vec![response],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaudeConvertState {
    pub requested_resume_id: Option<String>,
    pub backend_session_id: Option<String>,
    pub wire_mode: ClaudeWireMode,
    pub latest_usage: Option<TokenUsage>,
    task_tool_use_ids: HashMap<String, String>,
}

impl ClaudeConvertState {
    pub(crate) fn new(requested_resume_id: Option<String>, wire_mode: ClaudeWireMode) -> Self {
        Self {
            requested_resume_id,
            backend_session_id: None,
            wire_mode,
            latest_usage: None,
            task_tool_use_ids: HashMap::new(),
        }
    }
}

pub(crate) fn convert_claude_message(
    message: &Value,
    state: &mut ClaudeConvertState,
) -> ClaudeConversion {
    match message_type(message) {
        Some(TYPE_SYSTEM) => convert_system_message(message, state),
        Some(TYPE_STREAM_EVENT) => ClaudeConversion::events(parts(stream_event_parts(message))),
        Some(TYPE_ASSISTANT) => ClaudeConversion::events(parts(assistant_parts(message))),
        Some(TYPE_USER) => ClaudeConversion::events(parts(user_parts(message, state))),
        Some(TYPE_CONTROL_REQUEST) => convert_control_request(message, state),
        Some(TYPE_CONTROL_RESPONSE) => convert_control_response(message),
        Some(TYPE_RESULT) => convert_result(message, state),
        Some(TYPE_KEEP_ALIVE) => ClaudeConversion::events(vec![AgentRuntimeEvent::KeepAlive]),
        None => ClaudeConversion::none(),
        _ => ClaudeConversion::none(),
    }
}

fn convert_system_message(message: &Value, state: &mut ClaudeConvertState) -> ClaudeConversion {
    match message_subtype(message) {
        Some(SYSTEM_INIT) => {
            let session_id = string_field(message, "session_id").unwrap_or_default();
            state.backend_session_id = Some(session_id.clone());
            let resume = match state.requested_resume_id.as_deref() {
                None => ResumeOutcome::NotRequested,
                Some(requested) if requested == session_id => ResumeOutcome::Resumed,
                Some(_) => ResumeOutcome::Mismatch {
                    actual: session_id.clone(),
                },
            };
            let mut events = vec![AgentRuntimeEvent::SessionEstablished {
                backend_session_id: session_id,
                resume,
            }];
            let commands = slash_commands_from_system_init(message);
            if !commands.is_empty() {
                events.push(AgentRuntimeEvent::SlashCommandsUpdated(commands));
            }
            ClaudeConversion::events(events)
        }
        Some(SYSTEM_STATUS) => {
            let mut events = Vec::new();
            if let Some(mode) = message
                .get("permissionMode")
                .and_then(Value::as_str)
                .and_then(permission_mode_from_wire)
            {
                events.push(AgentRuntimeEvent::PermissionModeChanged(mode));
            }
            let compacting = message.get("status").and_then(Value::as_str) == Some("compacting");
            if compacting {
                events.extend(parts(vec![compaction_notification(
                    "in_progress",
                    "Compacting conversation",
                    message_or_content(message),
                )]));
            }
            if !compacting {
                events.extend(parts(system_text_parts(message)));
            }
            ClaudeConversion::events(events)
        }
        Some(SYSTEM_COMPACT_BOUNDARY) => {
            ClaudeConversion::events(parts(vec![compaction_notification(
                "completed",
                "Conversation compacted",
                message_or_content(message),
            )]))
        }
        Some(SUBTYPE_PERMISSION_DENIED) => {
            let request = PermissionRequest {
                id: string_field(message, "tool_use_id")
                    .unwrap_or_else(|| "permission-denied".to_string()),
                tool_use_id: string_field(message, "tool_use_id"),
                parent_tool_use_id: string_field(message, "agent_id"),
                tool_name: string_field(message, "tool_name").unwrap_or_else(|| "Tool".to_string()),
                body: crate::domain::agent_session::entities::PermissionRequestBody::ToolApproval {
                    input: JsonPayload::new_unchecked(json!({}).to_string()),
                },
                title: None,
                display_name: None,
                description: string_field(message, "message"),
                decision_reason: string_field(message, "decision_reason"),
                status: PermissionRequestStatus::Resolved {
                    decision: PermissionDecision::Denied,
                    answers: None,
                },
            };
            ClaudeConversion::events(vec![AgentRuntimeEvent::PermissionRequested(request)])
        }
        Some(SYSTEM_TASK_STARTED)
        | Some(SYSTEM_TASK_PROGRESS)
        | Some(SYSTEM_TASK_UPDATED)
        | Some(SYSTEM_TASK_NOTIFICATION) => {
            ClaudeConversion::events(parts(task_status_parts(message, state)))
        }
        _ => ClaudeConversion::events(parts(system_text_parts(message))),
    }
}

fn convert_control_response(message: &Value) -> ClaudeConversion {
    if message.get("request_id").and_then(Value::as_str) != Some("releash-initialize") {
        return ClaudeConversion::none();
    }
    let commands = message
        .get("response")
        .map(slash_commands_from_system_init)
        .unwrap_or_default();
    if commands.is_empty() {
        ClaudeConversion::none()
    } else {
        ClaudeConversion::events(vec![AgentRuntimeEvent::SlashCommandsUpdated(commands)])
    }
}

fn convert_control_request(message: &Value, state: &mut ClaudeConvertState) -> ClaudeConversion {
    if control_request_subtype(message) != Some(SUBTYPE_CAN_USE_TOOL) {
        return ClaudeConversion::none();
    }
    let Some(request_id) = message.get("request_id").and_then(Value::as_str) else {
        return ClaudeConversion::none();
    };
    let request = message.get("request").unwrap_or(&Value::Null);
    match permission_action_from_can_use_tool(request_id, request, state.wire_mode) {
        Some(ClaudePermissionAction::Respond { response }) => {
            ClaudeConversion::auto_response(response)
        }
        Some(ClaudePermissionAction::Prompt { request }) => {
            ClaudeConversion::events(vec![AgentRuntimeEvent::PermissionRequested(*request)])
        }
        None => ClaudeConversion::none(),
    }
}

fn convert_result(message: &Value, state: &mut ClaudeConvertState) -> ClaudeConversion {
    let usage = token_usage(message);
    state.latest_usage = usage;
    let mut events = Vec::new();
    if let Some(usage) = usage {
        events.push(AgentRuntimeEvent::TokenUsageUpdated(usage));
    }

    let is_error = message
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if is_error {
        let error = result_error_text(message);
        events.extend(parts(vec![MessagePart::Error {
            content: error.clone(),
            parent_tool_use_id: None,
        }]));
        events.push(AgentRuntimeEvent::TurnCompleted(TurnResult::Failed {
            error,
            token_usage: usage,
        }));
    } else {
        events.push(AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
            stop_reason: None,
            token_usage: usage,
        }));
    }
    ClaudeConversion::events(events)
}

fn stream_event_parts(message: &Value) -> Vec<MessagePart> {
    let event = message.get("event").unwrap_or(message);
    if event.get("type").and_then(Value::as_str) != Some(STREAM_CONTENT_BLOCK_DELTA) {
        return Vec::new();
    }
    let delta = event.get("delta").unwrap_or(&Value::Null);
    match delta.get("type").and_then(Value::as_str) {
        Some(DELTA_TEXT) => string_field(delta, "text")
            .map(|content| {
                vec![MessagePart::Text {
                    content,
                    parent_tool_use_id: parent_tool_use_id(message),
                }]
            })
            .unwrap_or_default(),
        Some(DELTA_THINKING) => string_field(delta, "thinking")
            .or_else(|| string_field(delta, "text"))
            .map(|content| {
                vec![MessagePart::Thinking {
                    content,
                    parent_tool_use_id: parent_tool_use_id(message),
                }]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn assistant_parts(message: &Value) -> Vec<MessagePart> {
    let parent_tool_use_id = parent_tool_use_id(message);
    let Some(content) = message
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    for block in content {
        if block.get("type").and_then(Value::as_str) == Some("tool_use") {
            let id = string_field(block, "id").unwrap_or_default();
            let name = string_field(block, "name").unwrap_or_else(|| "Tool".to_string());
            let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
            if name == "TodoWrite" {
                parts.push(MessagePart::Text {
                    content: "Updated todo list".to_string(),
                    parent_tool_use_id: parent_tool_use_id.clone(),
                });
                parts.push(MessagePart::TodoListSnapshot {
                    items: todo_items_from_input(&input),
                });
            } else {
                parts.push(MessagePart::ToolUse {
                    id,
                    tool: name,
                    input: JsonPayload::new_unchecked(input.to_string()),
                    parent_tool_use_id: parent_tool_use_id.clone(),
                });
            }
        }
    }
    parts
}

fn user_parts(message: &Value, state: &mut ClaudeConvertState) -> Vec<MessagePart> {
    let parent_tool_use_id = parent_tool_use_id(message);
    let Some(content) = message
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    content
        .iter()
        .filter_map(|block| {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                return None;
            }
            let content = tool_result_content(block);
            let tool_use_id = string_field(block, "tool_use_id");
            if let (Some(task_id), Some(tool_use_id)) =
                (extract_agent_id(&content), tool_use_id.clone())
            {
                state.task_tool_use_ids.insert(task_id, tool_use_id);
            }
            Some(MessagePart::ToolResult {
                content,
                is_error: block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                tool_use_id,
                parent_tool_use_id: parent_tool_use_id.clone(),
                content_ref: None,
                summary: None,
            })
        })
        .collect()
}

fn task_status_parts(message: &Value, state: &ClaudeConvertState) -> Vec<MessagePart> {
    let task_id = string_field(message, "task_id");
    let id = string_field(message, "tool_use_id")
        .or_else(|| {
            task_id
                .as_ref()
                .and_then(|task_id| state.task_tool_use_ids.get(task_id).cloned())
        })
        .or(task_id)
        .unwrap_or_else(|| "task".to_string());
    let status = string_field(message, "status")
        .or_else(|| {
            message
                .get("patch")
                .and_then(|patch| patch.get("status"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "running".to_string());
    let description = string_field(message, "description").or_else(|| {
        message
            .get("patch")
            .and_then(|patch| patch.get("description"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    vec![MessagePart::TaskStatus {
        task_tool_use_id: id,
        status,
        description,
        summary: string_field(message, "summary"),
    }]
}

fn compaction_notification(
    status: impl Into<String>,
    label: impl Into<String>,
    detail: Option<String>,
) -> MessagePart {
    MessagePart::SystemNotification {
        notification_type: SystemNotificationType::Compaction,
        status: status.into(),
        label: label.into(),
        detail,
        hook_id: None,
    }
}

fn system_text_parts(message: &Value) -> Vec<MessagePart> {
    message_or_content(message)
        .map(|content| {
            vec![MessagePart::Error {
                content,
                parent_tool_use_id: parent_tool_use_id(message),
            }]
        })
        .unwrap_or_default()
}

fn message_or_content(message: &Value) -> Option<String> {
    string_field(message, "message").or_else(|| string_field(message, "content"))
}

fn slash_commands_from_system_init(message: &Value) -> Vec<SlashCommand> {
    if let Some(commands) = message.get("commands").and_then(Value::as_array) {
        return commands
            .iter()
            .filter_map(|command| {
                let name = command
                    .get("name")
                    .and_then(Value::as_str)
                    .or_else(|| command.get("command").and_then(Value::as_str))?;
                Some(SlashCommand {
                    name: name.to_string(),
                    description: command
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    argument_hint: command
                        .get("argument_hint")
                        .or_else(|| command.get("argumentHint"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            })
            .collect();
    }
    message
        .get("slash_commands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|command| {
            command.as_str().map(|name| SlashCommand {
                name: name.to_string(),
                description: String::new(),
                argument_hint: None,
            })
        })
        .collect()
}

fn todo_items_from_input(input: &Value) -> Vec<TodoListItem> {
    input
        .get("todos")
        .or_else(|| input.get("items"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| TodoListItem {
            text: item
                .get("content")
                .or_else(|| item.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            completed: item
                .get("status")
                .and_then(Value::as_str)
                .map(|status| status == "completed")
                .or_else(|| item.get("completed").and_then(Value::as_bool))
                .unwrap_or(false),
        })
        .collect()
}

fn tool_result_content(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(content)) => content.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str())
            })
            .collect::<Vec<_>>()
            .join(""),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn extract_agent_id(content: &str) -> Option<String> {
    ["agentId:", "with ID:"].iter().find_map(|marker| {
        let start = content.find(marker)? + marker.len();
        let candidate = content[start..].trim_start();
        let id = candidate
            .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ')' | ']' | '}'))
            .next()
            .unwrap_or_default()
            .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '.' | ';' | ':'));
        (!id.is_empty()).then(|| id.to_string())
    })
}

fn token_usage(message: &Value) -> Option<TokenUsage> {
    if let Some(model_usage) = message.get("modelUsage").and_then(Value::as_object) {
        let mut input_tokens = 0;
        let mut output_tokens = 0;
        let mut context_window_tokens = None;
        for usage in model_usage.values() {
            input_tokens += u64_field(usage, "inputTokens")
                + u64_field(usage, "cacheReadInputTokens")
                + u64_field(usage, "cacheCreationInputTokens");
            output_tokens += u64_field(usage, "outputTokens");
            context_window_tokens = context_window_tokens.or_else(|| {
                usage
                    .get("contextWindow")
                    .and_then(Value::as_u64)
                    .filter(|value| *value > 0)
            });
        }
        return Some(TokenUsage {
            input_tokens,
            output_tokens,
            total_tokens: Some(input_tokens + output_tokens),
            context_window_tokens,
        });
    }

    let usage = message.get("usage")?;
    let input_tokens = u64_field(usage, "input_tokens")
        + u64_field(usage, "cache_read_input_tokens")
        + u64_field(usage, "cache_creation_input_tokens");
    let output_tokens = u64_field(usage, "output_tokens");
    Some(TokenUsage {
        input_tokens,
        output_tokens,
        total_tokens: Some(input_tokens + output_tokens),
        context_window_tokens: None,
    })
}

fn result_error_text(message: &Value) -> String {
    if let Some(errors) = message.get("errors").and_then(Value::as_array) {
        let text = errors
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("; ");
        if !text.is_empty() {
            return text;
        }
    }
    string_field(message, "result").unwrap_or_else(|| "Claude turn failed".to_string())
}

fn parts(parts: Vec<MessagePart>) -> Vec<AgentRuntimeEvent> {
    if parts.is_empty() {
        Vec::new()
    } else {
        vec![AgentRuntimeEvent::PartsMerged(parts)]
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parent_tool_use_id(message: &Value) -> Option<String> {
    string_field(message, "parent_tool_use_id")
}

fn u64_field(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_initは_resume_mismatchを_session_establishedへ変換する() {
        let mut state =
            ClaudeConvertState::new(Some("expected".to_string()), ClaudeWireMode::Default);
        let conversion = convert_claude_message(
            &json!({
                "type": "system",
                "subtype": "init",
                "session_id": "actual",
                "slash_commands": ["/clear"]
            }),
            &mut state,
        );

        assert_eq!(
            conversion.events[0],
            AgentRuntimeEvent::SessionEstablished {
                backend_session_id: "actual".to_string(),
                resume: ResumeOutcome::Mismatch {
                    actual: "actual".to_string()
                },
            }
        );
        assert!(matches!(
            conversion.events[1],
            AgentRuntimeEvent::SlashCommandsUpdated(_)
        ));
    }

    #[test]
    fn test_keep_aliveは_keep_aliveイベントへ変換する() {
        // Given: the CLI emits a keep_alive liveness line.
        let mut state = ClaudeConvertState::new(None, ClaudeWireMode::Default);

        // When: converting the message.
        let conversion = convert_claude_message(&json!({ "type": "keep_alive" }), &mut state);

        // Then: a KeepAlive runtime event is emitted so the executor can refresh
        // its stale-progress clock.
        assert_eq!(conversion.events, vec![AgentRuntimeEvent::KeepAlive]);
        assert!(conversion.auto_responses.is_empty());
    }

    #[test]
    fn test_stream_text_deltaは_text_partへ変換する() {
        let mut state = ClaudeConvertState::new(None, ClaudeWireMode::Default);
        let conversion = convert_claude_message(
            &json!({
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "delta": { "type": "text_delta", "text": "hi" }
                }
            }),
            &mut state,
        );

        assert_eq!(
            conversion.events,
            vec![AgentRuntimeEvent::PartsMerged(vec![MessagePart::Text {
                content: "hi".to_string(),
                parent_tool_use_id: None,
            }])]
        );
    }

    #[test]
    fn test_assistant_tool_useは_entityへ変換する() {
        let mut state = ClaudeConvertState::new(None, ClaudeWireMode::Default);
        let conversion = convert_claude_message(
            &json!({
                "type": "assistant",
                "parent_tool_use_id": null,
                "message": {
                    "content": [{
                        "type": "tool_use",
                        "id": "tool-1",
                        "name": "Bash",
                        "input": { "command": "cargo test" }
                    }]
                }
            }),
            &mut state,
        );

        let AgentRuntimeEvent::PartsMerged(parts) = &conversion.events[0] else {
            panic!("expected parts");
        };
        assert!(matches!(
            &parts[0],
            MessagePart::ToolUse { tool, .. } if tool == "Bash"
        ));
    }

    #[test]
    fn test_can_use_tool_planは非対話modeで_auto_allowする() {
        let mut state = ClaudeConvertState::new(None, ClaudeWireMode::Plan);
        let conversion = convert_claude_message(
            &json!({
                "type": "control_request",
                "request_id": "req-1",
                "request": {
                    "subtype": "can_use_tool",
                    "tool_name": "Bash",
                    "tool_use_id": "tool-1",
                    "input": { "command": "cargo test" }
                }
            }),
            &mut state,
        );

        assert!(conversion.events.is_empty());
        assert_eq!(
            conversion.auto_responses[0]["response"]["response"]["behavior"],
            "allow"
        );
    }

    #[test]
    fn test_result_errorは_failed_completion前に_errorをemitする() {
        let mut state = ClaudeConvertState::new(None, ClaudeWireMode::Default);
        let conversion = convert_claude_message(
            &json!({
                "type": "result",
                "subtype": "error_during_execution",
                "is_error": true,
                "errors": ["boom"],
                "modelUsage": {
                    "claude-sonnet-4-5": {
                        "inputTokens": 3,
                        "outputTokens": 4,
                        "cacheReadInputTokens": 0,
                        "cacheCreationInputTokens": 0,
                        "webSearchRequests": 0,
                        "costUSD": 0,
                        "contextWindow": 200000,
                        "maxOutputTokens": 32000
                    }
                }
            }),
            &mut state,
        );

        assert!(matches!(
            conversion.events[0],
            AgentRuntimeEvent::TokenUsageUpdated(_)
        ));
        assert!(matches!(
            conversion.events[1],
            AgentRuntimeEvent::PartsMerged(_)
        ));
        assert!(matches!(
            conversion.events[2],
            AgentRuntimeEvent::TurnCompleted(TurnResult::Failed { .. })
        ));
    }

    #[test]
    fn test_system_compact_boundaryは_compaction_notificationへ変換する() {
        let mut state = ClaudeConvertState::new(None, ClaudeWireMode::Default);
        let conversion = convert_claude_message(
            &json!({
                "type": "system",
                "subtype": "compact_boundary",
                "message": "Compacted"
            }),
            &mut state,
        );

        let AgentRuntimeEvent::PartsMerged(parts) = &conversion.events[0] else {
            panic!("expected parts");
        };
        assert_eq!(
            parts,
            &vec![MessagePart::SystemNotification {
                notification_type: SystemNotificationType::Compaction,
                status: "completed".to_string(),
                label: "Conversation compacted".to_string(),
                detail: Some("Compacted".to_string()),
                hook_id: None,
            }]
        );
    }

    #[test]
    fn test_system_status_compactingは_compaction_notificationへ変換する() {
        let mut state = ClaudeConvertState::new(None, ClaudeWireMode::Default);
        let conversion = convert_claude_message(
            &json!({
                "type": "system",
                "subtype": "status",
                "status": "compacting",
                "content": "Reducing context"
            }),
            &mut state,
        );

        let AgentRuntimeEvent::PartsMerged(parts) = &conversion.events[0] else {
            panic!("expected parts");
        };
        assert!(matches!(
            &parts[0],
            MessagePart::SystemNotification {
                notification_type: SystemNotificationType::Compaction,
                status,
                detail: Some(detail),
                ..
            } if status == "in_progress" && detail == "Reducing context"
        ));
    }

    #[test]
    fn test_system_text_subtypeは旧listener同様に表示partへ変換する() {
        let mut state = ClaudeConvertState::new(None, ClaudeWireMode::Default);
        let conversion = convert_claude_message(
            &json!({
                "type": "system",
                "subtype": "hook_failed",
                "message": "Hook failed"
            }),
            &mut state,
        );

        assert_eq!(
            conversion.events,
            vec![AgentRuntimeEvent::PartsMerged(vec![MessagePart::Error {
                content: "Hook failed".to_string(),
                parent_tool_use_id: None,
            }])]
        );
    }

    #[test]
    fn test_system_task_subtypesは_task_statusへ変換し本文textを重複表示しない() {
        for subtype in [
            "task_started",
            "task_notification",
            "task_progress",
            "task_updated",
        ] {
            let mut state = ClaudeConvertState::new(None, ClaudeWireMode::Default);
            let conversion = convert_claude_message(
                &json!({
                    "type": "system",
                    "subtype": subtype,
                    "tool_use_id": "task-1",
                    "status": "running",
                    "message": "Task text"
                }),
                &mut state,
            );

            let AgentRuntimeEvent::PartsMerged(parts) = &conversion.events[0] else {
                panic!("expected parts");
            };
            assert!(matches!(
                &parts[0],
                MessagePart::TaskStatus {
                    task_tool_use_id,
                    status,
                    ..
                } if task_tool_use_id == "task-1" && status == "running"
            ));
            assert!(!parts.iter().any(|part| matches!(
                part,
                MessagePart::Error { content, .. } if content == "Task text"
            )));
        }
    }

    #[test]
    fn test_user_tool_resultの_agent_idを_task_statusの_tool_use_idへ対応付ける() {
        let mut state = ClaudeConvertState::new(None, ClaudeWireMode::Default);
        let result = convert_claude_message(
            &json!({
                "type": "user",
                "message": {
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "tool-parent",
                        "content": "Started background task with ID: agent-123"
                    }]
                }
            }),
            &mut state,
        );
        assert!(matches!(
            result.events[0],
            AgentRuntimeEvent::PartsMerged(_)
        ));

        let conversion = convert_claude_message(
            &json!({
                "type": "system",
                "subtype": "task_progress",
                "task_id": "agent-123",
                "status": "running"
            }),
            &mut state,
        );

        let AgentRuntimeEvent::PartsMerged(parts) = &conversion.events[0] else {
            panic!("expected parts");
        };
        assert!(matches!(
            &parts[0],
            MessagePart::TaskStatus {
                task_tool_use_id,
                status,
                ..
            } if task_tool_use_id == "tool-parent" && status == "running"
        ));
    }

    #[test]
    fn test_initialize_control_responseは説明付き_slash_commandsへ変換する() {
        let mut state = ClaudeConvertState::new(None, ClaudeWireMode::Default);
        let conversion = convert_claude_message(
            &json!({
                "type": "control_response",
                "request_id": "releash-initialize",
                "response": {
                    "commands": [{
                        "name": "/review",
                        "description": "Review changes",
                        "argumentHint": "<path>"
                    }]
                }
            }),
            &mut state,
        );

        assert_eq!(
            conversion.events,
            vec![AgentRuntimeEvent::SlashCommandsUpdated(vec![
                SlashCommand {
                    name: "/review".to_string(),
                    description: "Review changes".to_string(),
                    argument_hint: Some("<path>".to_string()),
                }
            ])]
        );
    }
}
