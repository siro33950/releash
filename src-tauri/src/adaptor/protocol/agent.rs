use serde::{Deserialize, Serialize};

use crate::usecase::agent_session::session::{
    ContextCarryState, MessagePart, ToolOutputRef, ToolOutputSummary,
};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolOutputRefMsg {
    pub id: String,
    pub byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolOutputSummaryMsg {
    pub line_count: u64,
    pub byte_size: u64,
    pub is_error: bool,
    #[serde(default)]
    pub truncated: bool,
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
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "contentRef"
        )]
        content_ref: Option<AgentToolOutputRefMsg>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        summary: Option<AgentToolOutputSummaryMsg>,
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

impl From<ToolOutputRef> for AgentToolOutputRefMsg {
    fn from(content_ref: ToolOutputRef) -> Self {
        Self {
            id: content_ref.id,
            byte_size: content_ref.byte_size,
        }
    }
}

impl From<ToolOutputSummary> for AgentToolOutputSummaryMsg {
    fn from(summary: ToolOutputSummary) -> Self {
        Self {
            line_count: summary.line_count,
            byte_size: summary.byte_size,
            is_error: summary.is_error,
            truncated: summary.truncated,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStreamSync {
    pub session_id: String,
    pub message_id: String,
    #[serde(default)]
    pub seq: u64,
    pub parts: Vec<AgentStreamPartMsg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStreamDeltaMsg {
    pub session_id: String,
    pub message_id: String,
    pub seq: u64,
    pub parts: Vec<AgentStreamPartMsg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResyncStreamReq {
    pub session_id: String,
    pub message_id: String,
    #[serde(default)]
    pub since_seq: u64,
}

impl From<MessagePart> for AgentStreamPartMsg {
    fn from(part: MessagePart) -> Self {
        match part {
            MessagePart::Thinking {
                content,
                parent_tool_use_id,
            } => Self::Thinking {
                content,
                parent_tool_use_id,
            },
            MessagePart::Text {
                content,
                parent_tool_use_id,
            } => Self::Text {
                content,
                parent_tool_use_id,
            },
            MessagePart::ToolUse {
                tool,
                input,
                id,
                parent_tool_use_id,
            } => Self::ToolUse {
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
                content_ref,
                summary,
            } => Self::ToolResult {
                content,
                is_error,
                tool_use_id,
                parent_tool_use_id,
                content_ref: content_ref.map(Into::into),
                summary: summary.map(Into::into),
            },
            MessagePart::Error {
                content,
                parent_tool_use_id,
            } => Self::Error {
                content,
                parent_tool_use_id,
            },
            MessagePart::Permission {
                request,
                status,
                answers,
                parent_tool_use_id,
            } => Self::Permission {
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
            } => Self::TaskStatus {
                task_tool_use_id,
                status,
                description,
                summary,
            },
            MessagePart::TodoListSnapshot { items } => Self::TodoListSnapshot {
                items: items
                    .into_iter()
                    .map(|item| AgentTodoListItemMsg {
                        text: item.text,
                        completed: item.completed,
                    })
                    .collect(),
            },
            MessagePart::SystemNotification {
                notification_type,
                status,
                label,
                detail,
                hook_id,
            } => Self::SystemNotification {
                notification_type: notification_type.as_str().to_string(),
                status,
                label,
                detail,
                hook_id,
            },
            MessagePart::Image { data, media_type } => Self::Image { data, media_type },
            MessagePart::ImageRef { attachment } => Self::ImageRef {
                attachment: AgentStreamAttachmentRefMsg {
                    id: attachment.id,
                    media_type: attachment.media_type,
                    byte_size: attachment.byte_size,
                },
            },
        }
    }
}

impl From<crate::usecase::agent_session::session::StreamResyncSnapshot> for AgentStreamSync {
    fn from(snapshot: crate::usecase::agent_session::session::StreamResyncSnapshot) -> Self {
        Self {
            session_id: snapshot.session_id,
            message_id: snapshot.message_id,
            seq: snapshot.seq,
            parts: snapshot.parts.into_iter().map(Into::into).collect(),
        }
    }
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

    #[test]
    fn agent_stream_delta_serializes_tool_result_ref_without_full_tail() {
        let full_tail = "USER_SECRET_TAIL";
        let output_id = "a".repeat(64);
        let delta = AgentStreamDeltaMsg {
            session_id: "session-1".to_string(),
            message_id: "message-1".to_string(),
            seq: 9,
            parts: vec![AgentStreamPartMsg::ToolResult {
                content: "preview only".to_string(),
                is_error: true,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: Some(AgentToolOutputRefMsg {
                    id: output_id.clone(),
                    byte_size: 4096,
                }),
                summary: Some(AgentToolOutputSummaryMsg {
                    line_count: 1200,
                    byte_size: 4096,
                    is_error: true,
                    truncated: true,
                }),
            }],
        };

        let json = serde_json::to_string(&delta).unwrap();

        assert!(json.contains("\"contentRef\""));
        assert!(json.contains(&output_id));
        assert!(json.contains("preview only"));
        assert!(!json.contains(full_tail));
        let roundtrip: AgentStreamDeltaMsg = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            &roundtrip.parts[0],
            AgentStreamPartMsg::ToolResult {
                content,
                content_ref: Some(content_ref),
                summary: Some(summary),
                ..
            } if content == "preview only"
                && content_ref.id == output_id
                && summary.truncated
                && summary.byte_size == 4096
        ));
    }

    #[test]
    fn agent_stream_sync_serializes_tool_result_ref_without_full_tail() {
        let full_tail = "USER_SECRET_TAIL";
        let output_id = "b".repeat(64);
        let sync = AgentStreamSync {
            session_id: "session-1".to_string(),
            message_id: "message-1".to_string(),
            seq: 10,
            parts: vec![AgentStreamPartMsg::ToolResult {
                content: "sync preview".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: Some(AgentToolOutputRefMsg {
                    id: output_id.clone(),
                    byte_size: 8192,
                }),
                summary: Some(AgentToolOutputSummaryMsg {
                    line_count: 1500,
                    byte_size: 8192,
                    is_error: false,
                    truncated: true,
                }),
            }],
        };

        let json = serde_json::to_string(&sync).unwrap();

        assert!(json.contains("\"contentRef\""));
        assert!(json.contains(&output_id));
        assert!(json.contains("sync preview"));
        assert!(!json.contains(full_tail));
        let roundtrip: AgentStreamSync = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            &roundtrip.parts[0],
            AgentStreamPartMsg::ToolResult {
                content,
                content_ref: Some(content_ref),
                summary: Some(summary),
                ..
            } if content == "sync preview"
                && content_ref.id == output_id
                && summary.truncated
                && summary.byte_size == 8192
        ));
    }

    #[test]
    fn agent_stream_sync_from_snapshot_copies_parts_without_projection() {
        let full_tail = "PROTOCOL_LAYER_COPY_TAIL";
        let content = format!(
            "{}{full_tail}",
            "x".repeat(crate::usecase::agent_session::session::MAX_TOOL_OUTPUT_BYTES + 1)
        );
        let snapshot = crate::usecase::agent_session::session::StreamResyncSnapshot {
            session_id: "session-1".to_string(),
            message_id: "message-1".to_string(),
            seq: 11,
            parts: vec![MessagePart::ToolResult {
                content: content.clone(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            }],
        };

        let sync: AgentStreamSync = snapshot.into();

        assert!(matches!(
            &sync.parts[0],
            AgentStreamPartMsg::ToolResult {
                content: dto_content,
                content_ref: None,
                summary: None,
                ..
            } if dto_content == &content && dto_content.contains(full_tail)
        ));
    }
}
