use super::events::{AgentSessionEvent, PermissionDecision, TurnId};
use crate::usecase::agent_session::session::MessagePart;
use crate::usecase::agent_session::session::{PermissionPartStatus, PermissionRequestMsg};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartEventMode {
    DurableOnly,
    FinalLiveBlocks,
}

pub fn append_part_events(
    events: &mut Vec<AgentSessionEvent>,
    turn_id: TurnId,
    message_id: &str,
    parts: &[MessagePart],
    mode: PartEventMode,
) -> usize {
    let before_len = events.len();
    for part in parts {
        match part {
            MessagePart::Text {
                content,
                parent_tool_use_id,
            } if mode == PartEventMode::FinalLiveBlocks => {
                events.push(AgentSessionEvent::TextRecorded {
                    turn_id,
                    message_id: message_id.to_string(),
                    content: content.clone(),
                    parent_tool_use_id: parent_tool_use_id.clone(),
                });
            }
            MessagePart::Thinking {
                content,
                parent_tool_use_id,
            } if mode == PartEventMode::FinalLiveBlocks => {
                events.push(AgentSessionEvent::ReasoningRecorded {
                    turn_id,
                    message_id: message_id.to_string(),
                    content: content.clone(),
                    parent_tool_use_id: parent_tool_use_id.clone(),
                });
            }
            MessagePart::Error {
                content,
                parent_tool_use_id,
            } if mode == PartEventMode::FinalLiveBlocks => {
                events.push(AgentSessionEvent::ErrorRecorded {
                    turn_id,
                    message_id: message_id.to_string(),
                    content: content.clone(),
                    parent_tool_use_id: parent_tool_use_id.clone(),
                });
            }
            MessagePart::ToolUse {
                tool,
                input,
                id,
                parent_tool_use_id,
            } if mode == PartEventMode::DurableOnly => {
                if let Some(attempt) = next_tool_retry_attempt(events, turn_id, id) {
                    events.push(AgentSessionEvent::ToolCallRetried {
                        turn_id,
                        tool_use_id: id.clone(),
                        attempt,
                    });
                }
                events.push(AgentSessionEvent::ToolCallStarted {
                    turn_id,
                    tool_use_id: id.clone(),
                    tool: tool.clone(),
                    input: input.clone(),
                    parent_tool_use_id: parent_tool_use_id.clone(),
                });
            }
            MessagePart::ToolResult {
                content,
                is_error,
                tool_use_id: Some(tool_use_id),
                content_ref,
                summary,
                ..
            } if mode == PartEventMode::DurableOnly => {
                events.push(if *is_error {
                    AgentSessionEvent::ToolCallFailed {
                        turn_id,
                        tool_use_id: tool_use_id.clone(),
                        content: content.clone(),
                        content_ref: content_ref.clone(),
                        summary: summary.clone(),
                    }
                } else {
                    AgentSessionEvent::ToolCallSucceeded {
                        turn_id,
                        tool_use_id: tool_use_id.clone(),
                        content: content.clone(),
                        content_ref: content_ref.clone(),
                        summary: summary.clone(),
                    }
                });
            }
            MessagePart::ToolResult {
                content,
                is_error,
                tool_use_id: None,
                parent_tool_use_id,
                content_ref,
                summary,
                ..
            } if mode == PartEventMode::DurableOnly => {
                events.push(AgentSessionEvent::ToolResultRecorded {
                    turn_id,
                    message_id: message_id.to_string(),
                    content: content.clone(),
                    is_error: *is_error,
                    content_ref: content_ref.clone(),
                    summary: summary.clone(),
                    tool_use_id: None,
                    parent_tool_use_id: parent_tool_use_id.clone(),
                });
            }
            MessagePart::Permission {
                request,
                status,
                answers,
                parent_tool_use_id,
            } if mode == PartEventMode::DurableOnly => {
                let tool_use_id = request
                    .tool_use_id
                    .clone()
                    .filter(|value| !value.is_empty())
                    .or_else(|| parent_tool_use_id.clone());
                if *status == PermissionPartStatus::Pending {
                    events.push(AgentSessionEvent::PermissionRequested {
                        turn_id,
                        tool_use_id,
                        request: request.clone(),
                    });
                } else if let Some(decision) = PermissionDecision::from_status(status.as_str()) {
                    events.push(AgentSessionEvent::PermissionRequested {
                        turn_id,
                        tool_use_id: tool_use_id.clone(),
                        request: request.clone(),
                    });
                    events.push(AgentSessionEvent::PermissionResolved {
                        turn_id,
                        tool_use_id,
                        request_id: permission_request_id(request),
                        decision,
                        answers: answers.clone(),
                    });
                }
            }
            MessagePart::TaskStatus {
                task_tool_use_id,
                status,
                description,
                summary,
            } if mode == PartEventMode::DurableOnly => {
                events.push(AgentSessionEvent::TaskStatusChanged {
                    turn_id,
                    message_id: message_id.to_string(),
                    task_tool_use_id: task_tool_use_id.clone(),
                    status: status.clone(),
                    description: description.clone(),
                    summary: summary.clone(),
                });
            }
            MessagePart::TodoListSnapshot { items } if mode == PartEventMode::DurableOnly => {
                events.push(AgentSessionEvent::TodoListSnapshotRecorded {
                    turn_id,
                    message_id: message_id.to_string(),
                    items: items.clone(),
                });
            }
            MessagePart::SystemNotification {
                notification_type,
                status,
                label,
                detail,
                hook_id,
            } if mode == PartEventMode::DurableOnly => {
                events.push(AgentSessionEvent::SystemNotificationRecorded {
                    turn_id,
                    message_id: message_id.to_string(),
                    notification_type: notification_type.clone(),
                    status: status.clone(),
                    label: label.clone(),
                    detail: detail.clone(),
                    hook_id: hook_id.clone(),
                });
            }
            MessagePart::Image { data, media_type } if mode == PartEventMode::DurableOnly => {
                events.push(AgentSessionEvent::ImageRecorded {
                    turn_id,
                    message_id: message_id.to_string(),
                    data: data.clone(),
                    media_type: media_type.clone(),
                });
            }
            MessagePart::ImageRef { attachment } if mode == PartEventMode::DurableOnly => {
                events.push(AgentSessionEvent::ImageRefRecorded {
                    turn_id,
                    message_id: message_id.to_string(),
                    attachment: attachment.clone(),
                });
            }
            _ => {}
        }
    }
    events.len().saturating_sub(before_len)
}

fn next_tool_retry_attempt(
    events: &[AgentSessionEvent],
    turn_id: TurnId,
    tool_use_id: &str,
) -> Option<u32> {
    let prior_starts = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                AgentSessionEvent::ToolCallStarted {
                    turn_id: id,
                    tool_use_id: existing_id,
                    ..
                } if *id == turn_id && existing_id == tool_use_id
            )
        })
        .count();
    (prior_starts > 0).then_some(prior_starts.saturating_add(1) as u32)
}

pub(super) fn permission_request_id(request: &PermissionRequestMsg) -> Option<String> {
    (!request.id.is_empty()).then(|| request.id.clone())
}

pub(super) fn permission_tool_use_id(request: &PermissionRequestMsg) -> Option<String> {
    request
        .tool_use_id
        .clone()
        .filter(|value| !value.is_empty())
}
