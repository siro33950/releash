use std::time::{Duration, Instant};

use crate::domain::agent_session::entities::{
    MessagePart, PermissionDecision, PermissionRequest, PermissionRequestBody,
    PermissionRequestStatus, PermissionResponse, PermissionResponseDecision,
};

pub const STREAMING_EMIT_INTERVAL: Duration = Duration::from_millis(33);
const STREAMING_PERSIST_INTERVAL: Duration = Duration::from_secs(1);
const STREAMING_PENDING_PART_LIMIT: usize = 1000;
const STREAMING_PENDING_BYTE_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingFlushDecision {
    Now,
    Later(Duration),
    NotNeeded,
}

pub fn streaming_part_byte_size(part: &MessagePart) -> usize {
    match part {
        MessagePart::Text { content, .. }
        | MessagePart::Thinking { content, .. }
        | MessagePart::Error { content, .. }
        | MessagePart::ToolResult { content, .. } => content.len(),
        MessagePart::ToolUse {
            tool, input, id, ..
        } => tool.len() + id.len() + input.as_str().len(),
        MessagePart::Permission {
            request,
            status,
            answers,
            parent_tool_use_id,
        } => {
            permission_request_byte_size(request)
                + status.as_str().len()
                + answers.as_ref().map_or(0, |value| value.as_str().len())
                + parent_tool_use_id.as_ref().map_or(0, String::len)
        }
        MessagePart::TaskStatus {
            task_tool_use_id,
            status,
            description,
            summary,
        } => {
            task_tool_use_id.len()
                + status.len()
                + description.as_ref().map_or(0, String::len)
                + summary.as_ref().map_or(0, String::len)
        }
        MessagePart::TodoListSnapshot { items } => {
            items.iter().map(|item| item.text.len() + 1).sum()
        }
        MessagePart::SystemNotification {
            notification_type,
            status,
            label,
            detail,
            hook_id,
        } => {
            notification_type.as_str().len()
                + status.len()
                + label.len()
                + detail.as_ref().map_or(0, String::len)
                + hook_id.as_ref().map_or(0, String::len)
        }
        MessagePart::Image { data, media_type } => data.len() + media_type.len(),
        MessagePart::ImageRef { attachment } => {
            attachment.id.len() + attachment.media_type.len() + std::mem::size_of::<u64>()
        }
    }
}

fn permission_request_byte_size(request: &PermissionRequest) -> usize {
    let body = match &request.body {
        PermissionRequestBody::ToolApproval { input } => input.as_str().len(),
        PermissionRequestBody::PlanApproval {
            plan,
            allowed_prompts,
        } => {
            plan.len()
                + allowed_prompts
                    .iter()
                    .map(|prompt| prompt.tool.len() + prompt.prompt.len())
                    .sum::<usize>()
        }
        PermissionRequestBody::Question { questions } => questions
            .iter()
            .map(|question| {
                question.question.len()
                    + question.header.as_ref().map_or(0, String::len)
                    + question
                        .options
                        .iter()
                        .map(|option| {
                            option.label.len() + option.description.as_ref().map_or(0, String::len)
                        })
                        .sum::<usize>()
                    + usize::from(question.multi_select)
            })
            .sum(),
        PermissionRequestBody::PermissionGrant { requested } => requested.as_str().len(),
    };
    let request_status = match &request.status {
        PermissionRequestStatus::Pending => "pending".len(),
        PermissionRequestStatus::Resolved { decision, answers } => {
            let decision = match decision {
                PermissionDecision::Allowed => "allowed",
                PermissionDecision::Denied => "denied",
                PermissionDecision::Cancelled => "cancelled",
            };
            decision.len() + answers.as_ref().map_or(0, |value| value.as_str().len())
        }
    };
    request.id.len()
        + request.tool_use_id.as_ref().map_or(0, String::len)
        + request.parent_tool_use_id.as_ref().map_or(0, String::len)
        + request.tool_name.len()
        + body
        + request.title.as_ref().map_or(0, String::len)
        + request.display_name.as_ref().map_or(0, String::len)
        + request.description.as_ref().map_or(0, String::len)
        + request.decision_reason.as_ref().map_or(0, String::len)
        + request_status
}

pub fn streaming_parts_byte_size(parts: &[MessagePart]) -> usize {
    parts.iter().map(streaming_part_byte_size).sum()
}

pub fn add_streaming_byte_size(current: usize, added: usize) -> usize {
    current.saturating_add(added)
}

pub fn next_stream_sequence(current: u64) -> u64 {
    current.saturating_add(1)
}

pub fn parts_can_stream_as_append_delta(parts: &[MessagePart]) -> bool {
    parts.iter().all(|part| {
        matches!(
            part,
            MessagePart::Text { .. } | MessagePart::Thinking { .. }
        )
    })
}

pub fn part_records_durable_event(part: &MessagePart) -> bool {
    matches!(
        part,
        MessagePart::ToolUse { .. }
            | MessagePart::ToolResult { .. }
            | MessagePart::Permission { .. }
            | MessagePart::TaskStatus { .. }
            | MessagePart::TodoListSnapshot { .. }
            | MessagePart::SystemNotification { .. }
            | MessagePart::Image { .. }
            | MessagePart::ImageRef { .. }
    )
}

pub fn part_needs_event_history(part: &MessagePart) -> bool {
    matches!(part, MessagePart::ToolUse { tool, .. } if tool != "Edit")
}

pub fn streaming_flush_decision(
    has_pending: bool,
    has_retry: bool,
    pending_part_count: usize,
    pending_byte_size: usize,
    last_emit_at: Option<Instant>,
    now: Instant,
) -> StreamingFlushDecision {
    if !has_pending && !has_retry {
        return StreamingFlushDecision::NotNeeded;
    }
    if has_retry
        || pending_part_count >= STREAMING_PENDING_PART_LIMIT
        || pending_byte_size >= STREAMING_PENDING_BYTE_LIMIT
    {
        return StreamingFlushDecision::Now;
    }
    match last_emit_at {
        None => StreamingFlushDecision::Now,
        Some(last_emit_at) => {
            let elapsed = now.saturating_duration_since(last_emit_at);
            if elapsed >= STREAMING_EMIT_INTERVAL {
                StreamingFlushDecision::Now
            } else {
                StreamingFlushDecision::Later(STREAMING_EMIT_INTERVAL - elapsed)
            }
        }
    }
}

pub fn streaming_flush_decision_for_apply(
    immediate: bool,
    has_pending: bool,
    has_retry: bool,
    pending_part_count: usize,
    pending_byte_size: usize,
    last_emit_at: Option<Instant>,
    now: Instant,
) -> StreamingFlushDecision {
    if immediate {
        StreamingFlushDecision::Now
    } else {
        streaming_flush_decision(
            has_pending,
            has_retry,
            pending_part_count,
            pending_byte_size,
            last_emit_at,
            now,
        )
    }
}

pub fn stream_target_is_current(
    active_turn_id: Option<u64>,
    current_message_id: Option<&str>,
    expected_turn_id: u64,
    expected_message_id: &str,
) -> bool {
    active_turn_id == Some(expected_turn_id) && current_message_id == Some(expected_message_id)
}

pub fn should_persist_streaming_snapshot(
    last_persist_at: Option<Instant>,
    now: Instant,
    force: bool,
) -> bool {
    force
        || last_persist_at.is_none_or(|last_persist_at| {
            now.saturating_duration_since(last_persist_at) >= STREAMING_PERSIST_INTERVAL
        })
}

pub fn patch_permission_response(parts: &mut [MessagePart], response: &PermissionResponse) -> bool {
    let (decision, answers) = match &response.decision {
        PermissionResponseDecision::Allow { answers, .. } => {
            (PermissionDecision::Allowed, answers.clone())
        }
        PermissionResponseDecision::Deny { .. } => (PermissionDecision::Denied, None),
    };
    let mut patched = false;
    for part in parts {
        let MessagePart::Permission {
            request,
            status,
            answers: part_answers,
            parent_tool_use_id,
        } = part
        else {
            continue;
        };
        if request.id != response.request_id {
            continue;
        }
        request.status = PermissionRequestStatus::Resolved {
            decision,
            answers: answers.clone(),
        };
        *status = match decision {
            PermissionDecision::Allowed => {
                crate::domain::agent_session::entities::PermissionPartStatus::Allowed
            }
            PermissionDecision::Denied => {
                crate::domain::agent_session::entities::PermissionPartStatus::Denied
            }
            PermissionDecision::Cancelled => {
                crate::domain::agent_session::entities::PermissionPartStatus::Cancelled
            }
        };
        *part_answers = answers.clone();
        request.parent_tool_use_id = parent_tool_use_id.clone();
        patched = true;
    }
    patched
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::entities::PermissionPartStatus;
    use crate::domain::agent_session::value_objects::JsonPayload;

    #[test]
    fn flush_timing_and_thresholds_are_domain_decisions() {
        let now = Instant::now();
        assert_eq!(
            streaming_flush_decision(true, false, 1, 1, None, now),
            StreamingFlushDecision::Now
        );
        assert_eq!(
            streaming_flush_decision(
                true,
                false,
                1,
                1,
                Some(now - Duration::from_millis(10)),
                now
            ),
            StreamingFlushDecision::Later(Duration::from_millis(23))
        );
        assert_eq!(
            streaming_flush_decision(true, false, STREAMING_PENDING_PART_LIMIT, 1, Some(now), now),
            StreamingFlushDecision::Now
        );
        assert!(should_persist_streaming_snapshot(None, now, false));
        assert!(!should_persist_streaming_snapshot(
            Some(now - Duration::from_millis(999)),
            now,
            false
        ));
    }

    #[test]
    fn append_and_durable_classification_are_domain_owned() {
        assert!(parts_can_stream_as_append_delta(&[MessagePart::Text {
            content: "a".into(),
            parent_tool_use_id: None,
        }]));
        assert!(!parts_can_stream_as_append_delta(&[MessagePart::Error {
            content: "boom".into(),
            parent_tool_use_id: None,
        }]));
        assert!(part_records_durable_event(&MessagePart::ToolUse {
            id: "tool".into(),
            tool: "Edit".into(),
            input: JsonPayload::new_unchecked("{}".into()),
            parent_tool_use_id: None,
        }));
        assert!(!part_needs_event_history(&MessagePart::ToolUse {
            id: "tool".into(),
            tool: "Edit".into(),
            input: JsonPayload::new_unchecked("{}".into()),
            parent_tool_use_id: None,
        }));
    }

    #[test]
    fn permission_response_transition_updates_only_its_target() {
        let mut parts = vec![MessagePart::Permission {
            request: PermissionRequest {
                id: "permission".into(),
                tool_use_id: None,
                parent_tool_use_id: None,
                tool_name: "Plan".into(),
                body: PermissionRequestBody::PlanApproval {
                    plan: "plan".into(),
                    allowed_prompts: Vec::new(),
                },
                title: None,
                display_name: None,
                description: None,
                decision_reason: None,
                status: PermissionRequestStatus::Pending,
            },
            status: PermissionPartStatus::Pending,
            answers: None,
            parent_tool_use_id: None,
        }];
        assert!(!patch_permission_response(
            &mut parts,
            &PermissionResponse {
                request_id: "other".into(),
                decision: PermissionResponseDecision::Deny { message: None },
            }
        ));
        assert!(patch_permission_response(
            &mut parts,
            &PermissionResponse {
                request_id: "permission".into(),
                decision: PermissionResponseDecision::Deny { message: None },
            }
        ));
        assert!(matches!(
            parts[0],
            MessagePart::Permission {
                status: PermissionPartStatus::Denied,
                ..
            }
        ));
    }
}
