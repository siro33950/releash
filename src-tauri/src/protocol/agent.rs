use serde::{Deserialize, Serialize};

use crate::usecase::agent_session::session::ContextCarryState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Running,
    Done,
    Error,
    Waiting,
}

impl From<crate::usecase::agent_session::status::AgentState> for AgentState {
    fn from(state: crate::usecase::agent_session::status::AgentState) -> Self {
        match state {
            crate::usecase::agent_session::status::AgentState::Running => Self::Running,
            crate::usecase::agent_session::status::AgentState::Done => Self::Done,
            crate::usecase::agent_session::status::AgentState::Error => Self::Error,
            crate::usecase::agent_session::status::AgentState::Waiting => Self::Waiting,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStateSync {
    pub worktree_path: String,
    pub state: AgentState,
    pub exit_code: Option<i32>,
    pub timestamp: f64,
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pty_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSupportedCommandMsg {
    pub name: String,
    pub description: String,
    #[serde(
        rename = "argumentHint",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub argument_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSupportedCommandsUpdated {
    pub chat_session_id: String,
    pub commands: Vec<AgentSupportedCommandMsg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionContextCarryUpdated {
    pub chat_session_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_carry: Option<ContextCarryState>,
    pub updated_at: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTodoListItemMsg {
    pub text: String,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStreamAttachmentRefMsg {
    pub id: String,
    pub media_type: String,
    pub byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentStreamPartMsg {
    Thinking {
        content: String,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    Text {
        content: String,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    ToolUse {
        tool: String,
        input: serde_json::Value,
        id: String,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    ToolResult {
        content: String,
        #[serde(rename = "isError")]
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none", default, rename = "toolUseId")]
        tool_use_id: Option<String>,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    Error {
        content: String,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    Permission {
        request: serde_json::Value,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        answers: Option<serde_json::Value>,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    TaskStatus {
        #[serde(rename = "taskToolUseId")]
        task_tool_use_id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        summary: Option<String>,
    },
    TodoListSnapshot {
        items: Vec<AgentTodoListItemMsg>,
    },
    SystemNotification {
        #[serde(rename = "notificationType")]
        notification_type: String,
        status: String,
        label: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default, rename = "hookId")]
        hook_id: Option<String>,
    },
    Image {
        data: String,
        #[serde(rename = "mediaType")]
        media_type: String,
    },
    ImageRef {
        attachment: AgentStreamAttachmentRefMsg,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStreamSync {
    pub session_id: String,
    pub message_id: String,
    pub parts: Vec<AgentStreamPartMsg>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_agent_state_sync() {
        let sync = AgentStateSync {
            worktree_path: "/repo".to_string(),
            state: AgentState::Running,
            exit_code: None,
            timestamp: 1234567890.0,
            session_id: None,
            pty_id: None,
        };
        let json = serde_json::to_string(&sync).unwrap();
        let back: AgentStateSync = serde_json::from_str(&json).unwrap();
        assert_eq!(back.state, AgentState::Running);
        assert_eq!(back.worktree_path, "/repo");
    }

    #[test]
    fn agent_state_serializes_snake_case() {
        let json = serde_json::to_string(&AgentState::Running).unwrap();
        assert_eq!(json, "\"running\"");
        let json = serde_json::to_string(&AgentState::Done).unwrap();
        assert_eq!(json, "\"done\"");
        let json = serde_json::to_string(&AgentState::Error).unwrap();
        assert_eq!(json, "\"error\"");
        let json = serde_json::to_string(&AgentState::Waiting).unwrap();
        assert_eq!(json, "\"waiting\"");
    }

    #[test]
    fn pty_id_none_is_skipped_in_serialization() {
        let sync = AgentStateSync {
            worktree_path: "/repo".to_string(),
            state: AgentState::Running,
            exit_code: None,
            timestamp: 1000.0,
            session_id: None,
            pty_id: None,
        };
        let json = serde_json::to_string(&sync).unwrap();
        assert!(!json.contains("pty_id"));
    }

    #[test]
    fn pty_id_some_is_serialized() {
        let sync = AgentStateSync {
            worktree_path: "/repo".to_string(),
            state: AgentState::Running,
            exit_code: None,
            timestamp: 1000.0,
            session_id: None,
            pty_id: Some("7".to_string()),
        };
        let json = serde_json::to_string(&sync).unwrap();
        assert!(json.contains("\"pty_id\":\"7\""));
    }

    #[test]
    fn image_ref_uses_protocol_attachment_ref_shape() {
        let part = AgentStreamPartMsg::ImageRef {
            attachment: AgentStreamAttachmentRefMsg {
                id: "att-1".to_string(),
                media_type: "image/png".to_string(),
                byte_size: 42,
            },
        };

        let json = serde_json::to_string(&part).unwrap();

        assert_eq!(
            json,
            r#"{"type":"image_ref","attachment":{"id":"att-1","mediaType":"image/png","byteSize":42}}"#
        );
        let roundtrip: AgentStreamPartMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, part);
    }
}
