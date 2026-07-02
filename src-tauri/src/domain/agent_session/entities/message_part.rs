use super::attachment::Attachment;
use super::permission_request::PermissionRequest;
use crate::domain::agent_session::value_objects::{
    JsonPayload, SystemNotificationType, TodoListItem, ToolOutputRef, ToolOutputSummary,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagePart {
    Thinking {
        content: String,
        parent_tool_use_id: Option<String>,
    },
    Text {
        content: String,
        parent_tool_use_id: Option<String>,
    },
    ToolUse {
        id: String,
        tool: String,
        input: JsonPayload,
        parent_tool_use_id: Option<String>,
    },
    ToolResult {
        content: String,
        is_error: bool,
        tool_use_id: Option<String>,
        parent_tool_use_id: Option<String>,
        content_ref: Option<ToolOutputRef>,
        summary: Option<ToolOutputSummary>,
    },
    Error {
        content: String,
        parent_tool_use_id: Option<String>,
    },
    Permission {
        request: PermissionRequest,
    },
    TaskStatus {
        task_tool_use_id: String,
        status: String,
        description: Option<String>,
        summary: Option<String>,
    },
    TodoListSnapshot {
        items: Vec<TodoListItem>,
    },
    SystemNotification {
        notification_type: SystemNotificationType,
        status: String,
        label: String,
        detail: Option<String>,
        hook_id: Option<String>,
    },
    #[allow(dead_code)]
    // issues-1301 G-1: image entities are accepted by the domain contract; current production image persistence still uses session DTOs.
    Image {
        data: String,
        media_type: String,
    },
    #[allow(dead_code)]
    // issues-1301 G-1: image-ref entities are accepted by the domain contract; current production image persistence still uses session DTOs.
    ImageRef {
        attachment: Attachment,
    },
}

pub fn merge_part(parts: &mut Vec<MessagePart>, incoming: MessagePart) {
    match incoming {
        MessagePart::Text {
            content,
            parent_tool_use_id,
        } => merge_adjacent_text(parts, content, parent_tool_use_id, TextKind::Text),
        MessagePart::Thinking {
            content,
            parent_tool_use_id,
        } => merge_adjacent_text(parts, content, parent_tool_use_id, TextKind::Thinking),
        MessagePart::ToolUse {
            id,
            tool,
            input,
            parent_tool_use_id,
        } => {
            if let Some(existing) = parts.iter_mut().find(|part| {
                matches!(
                    part,
                    MessagePart::ToolUse {
                        id: existing_id,
                        ..
                    } if existing_id == &id
                )
            }) {
                *existing = MessagePart::ToolUse {
                    id,
                    tool,
                    input,
                    parent_tool_use_id,
                };
            } else {
                parts.push(MessagePart::ToolUse {
                    id,
                    tool,
                    input,
                    parent_tool_use_id,
                });
            }
        }
        MessagePart::ToolResult {
            content,
            is_error,
            tool_use_id,
            parent_tool_use_id,
            content_ref,
            summary,
        } => merge_tool_result(
            parts,
            ToolResultUpdate {
                content,
                is_error,
                tool_use_id,
                parent_tool_use_id,
                content_ref,
                summary,
            },
        ),
        MessagePart::Permission { request } => {
            if let Some(existing) = parts.iter_mut().find(|part| {
                matches!(
                    part,
                    MessagePart::Permission {
                        request: existing_request
                    } if existing_request.id == request.id
                )
            }) {
                *existing = MessagePart::Permission { request };
            } else {
                parts.push(MessagePart::Permission { request });
            }
        }
        MessagePart::TaskStatus {
            task_tool_use_id,
            status,
            description,
            summary,
        } => {
            if let Some(existing) = parts.iter_mut().find(|part| {
                matches!(
                    part,
                    MessagePart::TaskStatus {
                        task_tool_use_id: existing_id,
                        ..
                    } if existing_id == &task_tool_use_id
                )
            }) {
                *existing = MessagePart::TaskStatus {
                    task_tool_use_id,
                    status,
                    description,
                    summary,
                };
            } else {
                parts.push(MessagePart::TaskStatus {
                    task_tool_use_id,
                    status,
                    description,
                    summary,
                });
            }
        }
        MessagePart::TodoListSnapshot { items } => {
            if let Some(existing) = parts
                .iter_mut()
                .find(|part| matches!(part, MessagePart::TodoListSnapshot { .. }))
            {
                *existing = MessagePart::TodoListSnapshot { items };
            } else {
                parts.push(MessagePart::TodoListSnapshot { items });
            }
        }
        MessagePart::SystemNotification {
            notification_type,
            status,
            label,
            detail,
            hook_id,
        } => {
            if let Some(existing) = parts.iter_mut().find(|part| {
                matches!(
                    part,
                    MessagePart::SystemNotification {
                        notification_type: existing_type,
                        status: existing_status,
                        ..
                    } if existing_type == &notification_type && existing_status == "in_progress"
                )
            }) {
                *existing = MessagePart::SystemNotification {
                    notification_type,
                    status,
                    label,
                    detail,
                    hook_id,
                };
            } else {
                parts.push(MessagePart::SystemNotification {
                    notification_type,
                    status,
                    label,
                    detail,
                    hook_id,
                });
            }
        }
        MessagePart::Error {
            content,
            parent_tool_use_id,
        } => {
            let duplicate = parts.iter().any(|part| {
                matches!(
                    part,
                    MessagePart::Error {
                        content: existing_content,
                        parent_tool_use_id: existing_parent,
                    } if existing_content == &content && existing_parent == &parent_tool_use_id
                )
            });
            if !duplicate {
                parts.push(MessagePart::Error {
                    content,
                    parent_tool_use_id,
                });
            }
        }
        MessagePart::Image { data, media_type } => {
            parts.push(MessagePart::Image { data, media_type });
        }
        MessagePart::ImageRef { attachment } => {
            parts.push(MessagePart::ImageRef { attachment });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextKind {
    Text,
    Thinking,
}

fn merge_adjacent_text(
    parts: &mut Vec<MessagePart>,
    content: String,
    parent_tool_use_id: Option<String>,
    kind: TextKind,
) {
    let same_parent = |existing_parent: &Option<String>| existing_parent == &parent_tool_use_id;
    match (parts.last_mut(), kind) {
        (
            Some(MessagePart::Text {
                content: existing,
                parent_tool_use_id: existing_parent,
            }),
            TextKind::Text,
        ) if same_parent(existing_parent) => existing.push_str(&content),
        (
            Some(MessagePart::Thinking {
                content: existing,
                parent_tool_use_id: existing_parent,
            }),
            TextKind::Thinking,
        ) if same_parent(existing_parent) => existing.push_str(&content),
        (_, TextKind::Text) => parts.push(MessagePart::Text {
            content,
            parent_tool_use_id,
        }),
        (_, TextKind::Thinking) => parts.push(MessagePart::Thinking {
            content,
            parent_tool_use_id,
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResultUpdate {
    pub content: String,
    pub is_error: bool,
    pub tool_use_id: Option<String>,
    pub parent_tool_use_id: Option<String>,
    pub content_ref: Option<ToolOutputRef>,
    pub summary: Option<ToolOutputSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolResultMergeDecision {
    Merge,
    Replace,
    AppendSeparate,
    Skip,
}

pub fn decide_tool_result_merge(
    existing_content_ref: &Option<ToolOutputRef>,
    existing_is_error: bool,
    incoming_content_ref: &Option<ToolOutputRef>,
    incoming_is_error: bool,
    incoming_content: &str,
) -> ToolResultMergeDecision {
    if existing_is_error && !incoming_is_error && incoming_content_ref.is_none() {
        ToolResultMergeDecision::Merge
    } else if existing_content_ref.is_some() && incoming_content_ref.is_none() {
        if incoming_content.is_empty() {
            ToolResultMergeDecision::Skip
        } else {
            ToolResultMergeDecision::AppendSeparate
        }
    } else if incoming_content_ref.is_some() {
        ToolResultMergeDecision::Replace
    } else {
        ToolResultMergeDecision::Merge
    }
}

fn merge_tool_result(parts: &mut Vec<MessagePart>, update: ToolResultUpdate) {
    let Some(tool_use_id) = update.tool_use_id.as_deref() else {
        parts.push(update.into_part());
        return;
    };
    let Some(existing_index) = parts.iter().rposition(|part| {
        matches!(
            part,
            MessagePart::ToolResult {
                tool_use_id: Some(id),
                ..
            } if id == tool_use_id
        )
    }) else {
        parts.push(update.into_part());
        return;
    };

    let MessagePart::ToolResult {
        content: existing_content,
        is_error: existing_error,
        parent_tool_use_id: existing_parent_tool_use_id,
        content_ref: existing_content_ref,
        summary: existing_summary,
        ..
    } = &mut parts[existing_index]
    else {
        return;
    };

    let decision = decide_tool_result_merge(
        existing_content_ref,
        *existing_error,
        &update.content_ref,
        update.is_error,
        &update.content,
    );
    match decision {
        ToolResultMergeDecision::Skip => {}
        ToolResultMergeDecision::AppendSeparate => parts.push(update.into_part()),
        ToolResultMergeDecision::Replace => {
            if existing_parent_tool_use_id.is_none() {
                *existing_parent_tool_use_id = update.parent_tool_use_id.clone();
            }
            *existing_content = update.content;
            *existing_error = update.is_error;
            *existing_content_ref = update.content_ref;
            *existing_summary = update.summary;
        }
        ToolResultMergeDecision::Merge => {
            if existing_parent_tool_use_id.is_none() {
                *existing_parent_tool_use_id = update.parent_tool_use_id.clone();
            }
            if *existing_error && !update.is_error && update.content_ref.is_none() {
                *existing_content = update.content;
                *existing_error = false;
                *existing_content_ref = None;
                *existing_summary = None;
            } else {
                if update.content.contains(existing_content.as_str()) || existing_content.is_empty()
                {
                    *existing_content = update.content;
                    *existing_summary = update.summary;
                } else {
                    existing_content.push_str(&update.content);
                    *existing_summary = None;
                }
                *existing_content_ref = update.content_ref;
                *existing_error = *existing_error || update.is_error;
            }
        }
    }
}

impl ToolResultUpdate {
    pub fn into_part(self) -> MessagePart {
        MessagePart::ToolResult {
            content: self.content,
            is_error: self.is_error,
            tool_use_id: self.tool_use_id,
            parent_tool_use_id: self.parent_tool_use_id,
            content_ref: self.content_ref,
            summary: self.summary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::entities::{
        PermissionRequest, PermissionRequestBody, PermissionRequestStatus,
    };

    fn json(raw: &str) -> JsonPayload {
        JsonPayload::new_unchecked(raw.to_string())
    }

    fn pending_permission(id: &str) -> PermissionRequest {
        PermissionRequest {
            id: id.to_string(),
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
            tool_name: "Edit".to_string(),
            body: PermissionRequestBody::ToolApproval {
                input: json(r#"{"file_path":"a.rs"}"#),
            },
            title: None,
            display_name: None,
            description: None,
            decision_reason: None,
            status: PermissionRequestStatus::Pending,
        }
    }

    #[test]
    fn test_merge_part_textとthinkingは隣接する同parentへ追記する() {
        let mut parts = vec![MessagePart::Text {
            content: "hel".to_string(),
            parent_tool_use_id: Some("task-1".to_string()),
        }];

        merge_part(
            &mut parts,
            MessagePart::Text {
                content: "lo".to_string(),
                parent_tool_use_id: Some("task-1".to_string()),
            },
        );
        merge_part(
            &mut parts,
            MessagePart::Thinking {
                content: "one".to_string(),
                parent_tool_use_id: None,
            },
        );
        merge_part(
            &mut parts,
            MessagePart::Thinking {
                content: " two".to_string(),
                parent_tool_use_id: None,
            },
        );

        assert_eq!(
            parts,
            vec![
                MessagePart::Text {
                    content: "hello".to_string(),
                    parent_tool_use_id: Some("task-1".to_string()),
                },
                MessagePart::Thinking {
                    content: "one two".to_string(),
                    parent_tool_use_id: None,
                },
            ]
        );
    }

    #[test]
    fn test_merge_part_tool_useとpermissionはidで更新する() {
        let mut parts = vec![
            MessagePart::ToolUse {
                id: "tool-1".to_string(),
                tool: "Bash".to_string(),
                input: json(r#"{"command":"cargo check"}"#),
                parent_tool_use_id: None,
            },
            MessagePart::Permission {
                request: pending_permission("permission-1"),
            },
        ];

        merge_part(
            &mut parts,
            MessagePart::ToolUse {
                id: "tool-1".to_string(),
                tool: "Bash".to_string(),
                input: json(r#"{"command":"cargo test"}"#),
                parent_tool_use_id: Some("task-1".to_string()),
            },
        );
        let mut resolved = pending_permission("permission-1");
        resolved.status = PermissionRequestStatus::Resolved {
            decision: crate::domain::agent_session::entities::PermissionDecision::Allowed,
            answers: None,
        };
        merge_part(&mut parts, MessagePart::Permission { request: resolved });

        assert!(matches!(
            &parts[0],
            MessagePart::ToolUse {
                input,
                parent_tool_use_id: Some(parent),
                ..
            } if input.as_str().contains("cargo test") && parent == "task-1"
        ));
        assert!(matches!(
            &parts[1],
            MessagePart::Permission {
                request: PermissionRequest {
                    status: PermissionRequestStatus::Resolved { .. },
                    ..
                }
            }
        ));
    }

    #[test]
    fn test_merge_part_tool_resultはref単位で蓄積または置換する() {
        let mut parts = Vec::new();

        merge_part(
            &mut parts,
            MessagePart::ToolResult {
                content: "line 1\n".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            },
        );
        merge_part(
            &mut parts,
            MessagePart::ToolResult {
                content: "line 2\n".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: Some("task-1".to_string()),
                content_ref: None,
                summary: None,
            },
        );
        merge_part(
            &mut parts,
            MessagePart::ToolResult {
                content: "preview".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: Some(ToolOutputRef {
                    id: "blob-1".to_string(),
                    byte_size: 10,
                }),
                summary: Some(ToolOutputSummary {
                    line_count: 2,
                    byte_size: 10,
                    is_error: false,
                    truncated: true,
                }),
            },
        );

        assert_eq!(parts.len(), 1);
        assert!(matches!(
            &parts[0],
            MessagePart::ToolResult {
                content,
                parent_tool_use_id: Some(parent),
                content_ref: Some(ToolOutputRef { id, .. }),
                ..
            } if content == "preview" && parent == "task-1" && id == "blob-1"
        ));
    }

    #[test]
    fn test_merge_part_single_slotと重複規則を適用する() {
        let mut parts = Vec::new();

        merge_part(
            &mut parts,
            MessagePart::TodoListSnapshot {
                items: vec![TodoListItem {
                    text: "old".to_string(),
                    completed: false,
                }],
            },
        );
        merge_part(
            &mut parts,
            MessagePart::TodoListSnapshot {
                items: vec![TodoListItem {
                    text: "new".to_string(),
                    completed: true,
                }],
            },
        );
        merge_part(
            &mut parts,
            MessagePart::SystemNotification {
                notification_type: SystemNotificationType::Compaction,
                status: "in_progress".to_string(),
                label: "Compacting".to_string(),
                detail: None,
                hook_id: None,
            },
        );
        merge_part(
            &mut parts,
            MessagePart::SystemNotification {
                notification_type: SystemNotificationType::Compaction,
                status: "done".to_string(),
                label: "Compacted".to_string(),
                detail: None,
                hook_id: None,
            },
        );
        for _ in 0..2 {
            merge_part(
                &mut parts,
                MessagePart::Error {
                    content: "failed".to_string(),
                    parent_tool_use_id: None,
                },
            );
        }

        assert_eq!(parts.len(), 3);
        assert!(matches!(
            &parts[0],
            MessagePart::TodoListSnapshot { items } if items[0].text == "new"
        ));
        assert!(matches!(
            &parts[1],
            MessagePart::SystemNotification { status, label, .. }
                if status == "done" && label == "Compacted"
        ));
    }
}
