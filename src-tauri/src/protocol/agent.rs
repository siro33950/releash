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

#[cfg(test)]
// Test-only mirror of the usecase-layer ToolResult merge predicate for protocol
// message DTOs; production code must use the usecase helper.
fn tool_result_refs_can_merge(
    existing: &Option<AgentToolOutputRefMsg>,
    incoming: &Option<AgentToolOutputRefMsg>,
) -> bool {
    match (existing, incoming) {
        (Some(existing), Some(incoming)) => existing.id == incoming.id,
        (Some(_), None) => false,
        _ => true,
    }
}

#[cfg(test)]
pub fn append_agent_stream_delta_parts(
    current_parts: &mut Vec<AgentStreamPartMsg>,
    delta_parts: &[AgentStreamPartMsg],
) {
    for part in delta_parts {
        match part {
            AgentStreamPartMsg::Text {
                content,
                parent_tool_use_id,
            } => match current_parts.last_mut() {
                Some(AgentStreamPartMsg::Text {
                    content: last_content,
                    parent_tool_use_id: last_pid,
                }) if parent_tool_use_id == last_pid => {
                    last_content.push_str(content);
                }
                _ => current_parts.push(part.clone()),
            },
            AgentStreamPartMsg::Thinking {
                content,
                parent_tool_use_id,
            } => match current_parts.last_mut() {
                Some(AgentStreamPartMsg::Thinking {
                    content: last_content,
                    parent_tool_use_id: last_pid,
                }) if parent_tool_use_id == last_pid => {
                    last_content.push_str(content);
                }
                _ => current_parts.push(part.clone()),
            },
            AgentStreamPartMsg::ToolUse { id, .. } => {
                if let Some(existing) = current_parts.iter_mut().rev().find(|existing| {
                    matches!(existing, AgentStreamPartMsg::ToolUse { id: existing_id, .. } if existing_id == id)
                }) {
                    *existing = part.clone();
                } else {
                    current_parts.push(part.clone());
                }
            }
            AgentStreamPartMsg::ToolResult {
                content,
                is_error,
                tool_use_id: Some(tool_use_id),
                parent_tool_use_id,
                content_ref,
                summary,
            } => {
                if let Some(existing) = current_parts.iter_mut().rev().find(|existing| {
                    let AgentStreamPartMsg::ToolResult {
                        tool_use_id: Some(existing_id),
                        content_ref: existing_content_ref,
                        ..
                    } = existing
                    else {
                        return false;
                    };
                    existing_id == tool_use_id
                        && tool_result_refs_can_merge(existing_content_ref, content_ref)
                }) {
                    if let AgentStreamPartMsg::ToolResult {
                        content: existing_content,
                        is_error: existing_error,
                        parent_tool_use_id: existing_parent,
                        content_ref: existing_content_ref,
                        summary: existing_summary,
                        ..
                    } = existing
                    {
                        if existing_parent.is_none() {
                            *existing_parent = parent_tool_use_id.clone();
                        }
                        if content_ref.is_some() {
                            *existing_content = content.clone();
                            *existing_error = *is_error;
                            *existing_content_ref = content_ref.clone();
                            *existing_summary = summary.clone();
                        } else if *existing_error && !*is_error {
                            *existing_content = content.clone();
                            *existing_error = false;
                        } else if content.contains(existing_content.as_str())
                            || existing_content.is_empty()
                        {
                            *existing_content = content.clone();
                        } else {
                            existing_content.push_str(content);
                        }
                        *existing_error = *existing_error || *is_error;
                        if summary.is_some() {
                            *existing_summary = summary.clone();
                        }
                    }
                } else {
                    current_parts.push(part.clone());
                }
            }
            AgentStreamPartMsg::TaskStatus {
                task_tool_use_id, ..
            } => {
                if let Some(existing) = current_parts.iter_mut().rev().find(|existing| {
                    matches!(
                        existing,
                        AgentStreamPartMsg::TaskStatus {
                            task_tool_use_id: existing_id,
                            ..
                        } if existing_id == task_tool_use_id
                    )
                }) {
                    *existing = part.clone();
                } else {
                    current_parts.push(part.clone());
                }
            }
            AgentStreamPartMsg::TodoListSnapshot { .. } => {
                if let Some(existing) = current_parts
                    .iter_mut()
                    .rev()
                    .find(|existing| matches!(existing, AgentStreamPartMsg::TodoListSnapshot { .. }))
                {
                    *existing = part.clone();
                } else {
                    current_parts.push(part.clone());
                }
            }
            AgentStreamPartMsg::SystemNotification {
                notification_type, ..
            } => {
                if let Some(existing) = current_parts.iter_mut().rev().find(|existing| {
                    matches!(
                        existing,
                        AgentStreamPartMsg::SystemNotification {
                            notification_type: existing_type,
                            status,
                            ..
                        } if existing_type == notification_type && status == "in_progress"
                    )
                }) {
                    *existing = part.clone();
                } else {
                    current_parts.push(part.clone());
                }
            }
            AgentStreamPartMsg::Permission { request, .. } => {
                let request_id = request.get("request_id").and_then(|value| value.as_str());
                let tool_use_id = request.get("tool_use_id").and_then(|value| value.as_str());
                if let Some(existing) = current_parts.iter_mut().rev().find(|existing| {
                    let AgentStreamPartMsg::Permission {
                        request: existing_request,
                        ..
                    } = existing
                    else {
                        return false;
                    };
                    let existing_request_id = existing_request
                        .get("request_id")
                        .and_then(|value| value.as_str());
                    let existing_tool_use_id = existing_request
                        .get("tool_use_id")
                        .and_then(|value| value.as_str());
                    request_id.is_some() && request_id == existing_request_id
                        || tool_use_id.is_some() && tool_use_id == existing_tool_use_id
                }) {
                    *existing = part.clone();
                } else {
                    current_parts.push(part.clone());
                }
            }
            _ => current_parts.push(part.clone()),
        }
    }
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

    #[test]
    fn append_agent_stream_delta_parts_updates_existing_non_tail_parts() {
        let mut parts = vec![
            AgentStreamPartMsg::ToolUse {
                tool: "Task".to_string(),
                input: serde_json::json!({"description": "old"}),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
            AgentStreamPartMsg::Text {
                content: "tail".to_string(),
                parent_tool_use_id: None,
            },
            AgentStreamPartMsg::TaskStatus {
                task_tool_use_id: "tool-1".to_string(),
                status: "started".to_string(),
                description: Some("old".to_string()),
                summary: None,
            },
            AgentStreamPartMsg::TodoListSnapshot {
                items: vec![AgentTodoListItemMsg {
                    text: "first".to_string(),
                    completed: false,
                }],
            },
            AgentStreamPartMsg::SystemNotification {
                notification_type: "compaction".to_string(),
                status: "in_progress".to_string(),
                label: "Compacting".to_string(),
                detail: None,
                hook_id: None,
            },
        ];

        append_agent_stream_delta_parts(
            &mut parts,
            &[
                AgentStreamPartMsg::ToolUse {
                    tool: "Task".to_string(),
                    input: serde_json::json!({"description": "new"}),
                    id: "tool-1".to_string(),
                    parent_tool_use_id: None,
                },
                AgentStreamPartMsg::TaskStatus {
                    task_tool_use_id: "tool-1".to_string(),
                    status: "completed".to_string(),
                    description: Some("new".to_string()),
                    summary: Some("done".to_string()),
                },
                AgentStreamPartMsg::TodoListSnapshot {
                    items: vec![AgentTodoListItemMsg {
                        text: "first".to_string(),
                        completed: true,
                    }],
                },
                AgentStreamPartMsg::SystemNotification {
                    notification_type: "compaction".to_string(),
                    status: "completed".to_string(),
                    label: "Compacted".to_string(),
                    detail: Some("ok".to_string()),
                    hook_id: None,
                },
            ],
        );

        assert_eq!(parts.len(), 5);
        assert!(matches!(
            &parts[0],
            AgentStreamPartMsg::ToolUse { input, .. }
                if input.get("description").and_then(|value| value.as_str()) == Some("new")
        ));
        assert!(matches!(
            &parts[2],
            AgentStreamPartMsg::TaskStatus { status, summary, .. }
                if status == "completed" && summary.as_deref() == Some("done")
        ));
        assert!(matches!(
            &parts[3],
            AgentStreamPartMsg::TodoListSnapshot { items }
                if items.first().is_some_and(|item| item.completed)
        ));
        assert!(matches!(
            &parts[4],
            AgentStreamPartMsg::SystemNotification { status, label, .. }
                if status == "completed" && label == "Compacted"
        ));
    }

    #[test]
    fn append_agent_stream_delta_parts_replaces_tool_result_error_with_success() {
        let mut parts = vec![AgentStreamPartMsg::ToolResult {
            content: "failed".to_string(),
            is_error: true,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
            content_ref: None,
            summary: None,
        }];

        append_agent_stream_delta_parts(
            &mut parts,
            &[AgentStreamPartMsg::ToolResult {
                content: "success".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: Some("parent-1".to_string()),
                content_ref: None,
                summary: None,
            }],
        );

        assert_eq!(
            parts,
            vec![AgentStreamPartMsg::ToolResult {
                content: "success".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: Some("parent-1".to_string()),
                content_ref: None,
                summary: None,
            }]
        );
    }

    #[test]
    fn append_agent_stream_delta_parts_replaces_tool_result_when_delta_includes_existing_or_empty()
    {
        let mut parts = vec![
            AgentStreamPartMsg::ToolResult {
                content: "partial".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            },
            AgentStreamPartMsg::ToolResult {
                content: String::new(),
                is_error: false,
                tool_use_id: Some("tool-2".to_string()),
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            },
        ];

        append_agent_stream_delta_parts(
            &mut parts,
            &[
                AgentStreamPartMsg::ToolResult {
                    content: "partial complete".to_string(),
                    is_error: false,
                    tool_use_id: Some("tool-1".to_string()),
                    parent_tool_use_id: None,
                    content_ref: None,
                    summary: None,
                },
                AgentStreamPartMsg::ToolResult {
                    content: "first content".to_string(),
                    is_error: false,
                    tool_use_id: Some("tool-2".to_string()),
                    parent_tool_use_id: None,
                    content_ref: None,
                    summary: None,
                },
            ],
        );

        assert_eq!(
            parts,
            vec![
                AgentStreamPartMsg::ToolResult {
                    content: "partial complete".to_string(),
                    is_error: false,
                    tool_use_id: Some("tool-1".to_string()),
                    parent_tool_use_id: None,
                    content_ref: None,
                    summary: None,
                },
                AgentStreamPartMsg::ToolResult {
                    content: "first content".to_string(),
                    is_error: false,
                    tool_use_id: Some("tool-2".to_string()),
                    parent_tool_use_id: None,
                    content_ref: None,
                    summary: None,
                },
            ]
        );
    }

    #[test]
    fn append_agent_stream_delta_parts_keeps_ref_backed_tool_result_separate() {
        let content_ref = AgentToolOutputRefMsg {
            id: "a".repeat(64),
            byte_size: 4096,
        };
        let summary = AgentToolOutputSummaryMsg {
            line_count: 200,
            byte_size: 4096,
            is_error: false,
            truncated: true,
        };
        let base = AgentStreamPartMsg::ToolResult {
            content: "preview".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
            content_ref: Some(content_ref),
            summary: Some(summary),
        };
        let delta = AgentStreamPartMsg::ToolResult {
            content: " late".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
            content_ref: None,
            summary: None,
        };
        let mut parts = vec![base.clone()];

        append_agent_stream_delta_parts(&mut parts, std::slice::from_ref(&delta));

        assert_eq!(parts, vec![base, delta]);
    }

    #[test]
    fn append_agent_stream_delta_parts_merges_adjacent_thinking() {
        let mut parts = vec![AgentStreamPartMsg::Thinking {
            content: "think".to_string(),
            parent_tool_use_id: None,
        }];

        append_agent_stream_delta_parts(
            &mut parts,
            &[AgentStreamPartMsg::Thinking {
                content: " more".to_string(),
                parent_tool_use_id: None,
            }],
        );

        assert_eq!(
            parts,
            vec![AgentStreamPartMsg::Thinking {
                content: "think more".to_string(),
                parent_tool_use_id: None,
            }]
        );
    }

    #[test]
    fn append_agent_stream_delta_parts_merges_permissions_by_request_or_tool_use_id() {
        let mut parts = vec![
            AgentStreamPartMsg::Permission {
                request: serde_json::json!({
                    "request_id": "req-1",
                    "tool_use_id": "tool-1",
                    "tool_name": "Bash"
                }),
                status: "pending".to_string(),
                answers: None,
                parent_tool_use_id: None,
            },
            AgentStreamPartMsg::Permission {
                request: serde_json::json!({
                    "request_id": "req-2",
                    "tool_use_id": "tool-2",
                    "tool_name": "Read"
                }),
                status: "pending".to_string(),
                answers: None,
                parent_tool_use_id: None,
            },
        ];

        append_agent_stream_delta_parts(
            &mut parts,
            &[
                AgentStreamPartMsg::Permission {
                    request: serde_json::json!({
                        "request_id": "req-1",
                        "tool_use_id": "tool-1",
                        "tool_name": "Bash"
                    }),
                    status: "allowed".to_string(),
                    answers: Some(serde_json::json!({"approve": true})),
                    parent_tool_use_id: None,
                },
                AgentStreamPartMsg::Permission {
                    request: serde_json::json!({
                        "request_id": "req-2-retry",
                        "tool_use_id": "tool-2",
                        "tool_name": "Read"
                    }),
                    status: "denied".to_string(),
                    answers: Some(serde_json::json!({"approve": false})),
                    parent_tool_use_id: None,
                },
            ],
        );

        assert_eq!(parts.len(), 2);
        assert!(matches!(
            &parts[0],
            AgentStreamPartMsg::Permission { status, answers, .. }
                if status == "allowed" && answers.as_ref().is_some_and(|value| value["approve"] == true)
        ));
        assert!(matches!(
            &parts[1],
            AgentStreamPartMsg::Permission { status, request, answers, .. }
                if status == "denied"
                    && request.get("request_id").and_then(|value| value.as_str()) == Some("req-2-retry")
                    && answers.as_ref().is_some_and(|value| value["approve"] == false)
        ));
    }
}
