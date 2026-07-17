use serde_json::{json, Value};

use crate::domain::agent_session::value_objects::PermissionMode;

/// Claude CLI wire contract used by this backend:
/// - Official headless docs require stream-json output with --verbose and support
///   --include-partial-messages for partial assistant stream events.
/// - The Claude Agent SDK 0.3.x `sdk.d.ts` defines the control protocol
///   frames as `control_request` / `control_response` and the permission request
///   subtype as `can_use_tool`.
/// - Releash only emits the permission modes required by design §5.2:
///   default, acceptEdits, bypassPermissions, and plan.
pub(crate) const TYPE_ASSISTANT: &str = "assistant";
pub(crate) const TYPE_CONTROL_REQUEST: &str = "control_request";
pub(crate) const TYPE_CONTROL_RESPONSE: &str = "control_response";
pub(crate) const TYPE_KEEP_ALIVE: &str = "keep_alive";
pub(crate) const TYPE_RESULT: &str = "result";
pub(crate) const TYPE_STREAM_EVENT: &str = "stream_event";
pub(crate) const TYPE_SYSTEM: &str = "system";
pub(crate) const TYPE_USER: &str = "user";

pub(crate) const SUBTYPE_CAN_USE_TOOL: &str = "can_use_tool";
pub(crate) const SUBTYPE_INITIALIZE: &str = "initialize";
pub(crate) const SUBTYPE_INTERRUPT: &str = "interrupt";
pub(crate) const SUBTYPE_PERMISSION_DENIED: &str = "permission_denied";
pub(crate) const SUBTYPE_SET_MODEL: &str = "set_model";
pub(crate) const SUBTYPE_SET_PERMISSION_MODE: &str = "set_permission_mode";

pub(crate) const SYSTEM_INIT: &str = "init";
pub(crate) const SYSTEM_COMPACT_BOUNDARY: &str = "compact_boundary";
pub(crate) const SYSTEM_STATUS: &str = "status";
pub(crate) const SYSTEM_TASK_NOTIFICATION: &str = "task_notification";
pub(crate) const SYSTEM_TASK_PROGRESS: &str = "task_progress";
pub(crate) const SYSTEM_TASK_STARTED: &str = "task_started";
pub(crate) const SYSTEM_TASK_UPDATED: &str = "task_updated";

pub(crate) const STREAM_CONTENT_BLOCK_DELTA: &str = "content_block_delta";
pub(crate) const DELTA_TEXT: &str = "text_delta";
pub(crate) const DELTA_THINKING: &str = "thinking_delta";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClaudeWireMode {
    Default,
    AcceptEdits,
    BypassPermissions,
    Plan,
}

impl ClaudeWireMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::BypassPermissions => "bypassPermissions",
            Self::Plan => "plan",
        }
    }
}

pub(crate) fn claude_wire_mode(mode: PermissionMode, plan_mode: bool) -> ClaudeWireMode {
    if plan_mode {
        return ClaudeWireMode::Plan;
    }
    match mode {
        PermissionMode::Ask => ClaudeWireMode::Default,
        PermissionMode::Edit => ClaudeWireMode::AcceptEdits,
        PermissionMode::Full => ClaudeWireMode::BypassPermissions,
    }
}

pub(crate) fn permission_mode_from_wire(mode: &str) -> Option<PermissionMode> {
    match mode {
        "default" => Some(PermissionMode::Ask),
        "acceptEdits" => Some(PermissionMode::Edit),
        "bypassPermissions" => Some(PermissionMode::Full),
        "plan" => None,
        _ => None,
    }
}

pub(crate) fn message_type(message: &Value) -> Option<&str> {
    message.get("type").and_then(Value::as_str)
}

pub(crate) fn message_subtype(message: &Value) -> Option<&str> {
    message.get("subtype").and_then(Value::as_str)
}

pub(crate) fn control_request_subtype(message: &Value) -> Option<&str> {
    message
        .get("request")
        .and_then(|request| request.get("subtype"))
        .and_then(Value::as_str)
}

pub(crate) fn initialize_request(request_id: impl Into<String>) -> Value {
    json!({
        "type": TYPE_CONTROL_REQUEST,
        "request_id": request_id.into(),
        "request": {
            "subtype": SUBTYPE_INITIALIZE,
            "hooks": null,
        },
    })
}

pub(crate) fn interrupt_request(request_id: impl Into<String>) -> Value {
    json!({
        "type": TYPE_CONTROL_REQUEST,
        "request_id": request_id.into(),
        "request": { "subtype": SUBTYPE_INTERRUPT },
    })
}

pub(crate) fn set_permission_mode_request(
    request_id: impl Into<String>,
    mode: ClaudeWireMode,
) -> Value {
    json!({
        "type": TYPE_CONTROL_REQUEST,
        "request_id": request_id.into(),
        "request": {
            "subtype": SUBTYPE_SET_PERMISSION_MODE,
            "mode": mode.as_str(),
        },
    })
}

pub(crate) fn set_model_request(request_id: impl Into<String>, model: Option<&str>) -> Value {
    json!({
        "type": TYPE_CONTROL_REQUEST,
        "request_id": request_id.into(),
        "request": {
            "subtype": SUBTYPE_SET_MODEL,
            "model": model,
        },
    })
}

pub(crate) fn user_message(
    prompt: &str,
    images: impl IntoIterator<Item = (String, String)>,
) -> Value {
    let images = images.into_iter().collect::<Vec<_>>();
    let mut content = Vec::new();
    if !prompt.is_empty() || images.is_empty() {
        content.push(json!({
            "type": "text",
            "text": prompt,
        }));
    }
    for (media_type, data) in images {
        content.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": media_type,
                "data": data,
            },
        }));
    }
    json!({
        "type": TYPE_USER,
        "session_id": "",
        "parent_tool_use_id": null,
        "message": {
            "role": "user",
            "content": content,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_wire_mode_plan_優先() {
        assert_eq!(
            claude_wire_mode(PermissionMode::Full, true),
            ClaudeWireMode::Plan
        );
    }

    #[test]
    fn test_claude_user_message画像をstream_json形式にする() {
        let message = user_message("hello", [("image/png".to_string(), "abc".to_string())]);
        let content = message["message"]["content"].as_array().unwrap();

        assert_eq!(message["type"], TYPE_USER);
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "hello");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
    }

    #[test]
    fn test_claude_user_message画像のみなら空text_blockを含めない() {
        let message = user_message("", [("image/png".to_string(), "abc".to_string())]);
        let content = message["message"]["content"].as_array().unwrap();

        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[0]["source"]["media_type"], "image/png");
    }

    #[test]
    fn test_claude_user_message本文のみならtext_blockだけを含める() {
        let message = user_message("hello", []);
        let content = message["message"]["content"].as_array().unwrap();

        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "hello");
    }

    #[test]
    fn test_claude_user_message本文も画像もなければ空text_blockを含める() {
        let message = user_message("", []);
        let content = message["message"]["content"].as_array().unwrap();

        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "");
    }
}
