use std::time::{Duration, Instant};

use crate::usecase::agent_session::session::MessagePart;

pub(crate) const STREAMING_EMIT_INTERVAL: Duration = Duration::from_millis(33);
pub(crate) const STREAMING_PERSIST_INTERVAL: Duration = Duration::from_secs(1);
pub(crate) const STREAMING_PENDING_PART_LIMIT: usize = 1000;
pub(crate) const STREAMING_PENDING_BYTE_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamingFlushDecision {
    Now,
    Later(Duration),
    NotNeeded,
}

pub(crate) fn streaming_part_byte_size(part: &MessagePart) -> usize {
    match part {
        MessagePart::Text { content, .. }
        | MessagePart::Thinking { content, .. }
        | MessagePart::Error { content, .. }
        | MessagePart::ToolResult { content, .. } => content.len(),
        MessagePart::ToolUse {
            tool, input, id, ..
        } => tool.len() + id.len() + serde_json::to_string(input).map_or(0, |body| body.len()),
        MessagePart::Permission {
            request,
            status,
            answers,
            ..
        } => {
            serde_json::to_string(request).map_or(0, |body| body.len())
                + serde_json::to_string(status).map_or(0, |body| body.len())
                + answers
                    .as_ref()
                    .and_then(|answers| serde_json::to_string(answers).ok())
                    .map_or(0, |body| body.len())
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

pub(crate) fn streaming_parts_byte_size(parts: &[MessagePart]) -> usize {
    parts.iter().map(streaming_part_byte_size).sum()
}

pub(crate) fn part_can_stream_as_append_delta(part: &MessagePart) -> bool {
    matches!(
        part,
        MessagePart::Text { .. } | MessagePart::Thinking { .. }
    )
}

pub(crate) fn parts_can_stream_as_append_delta(parts: &[MessagePart]) -> bool {
    parts.iter().all(part_can_stream_as_append_delta)
}

pub(crate) fn merge_streaming_append_delta_parts(
    streaming_parts: &mut Vec<MessagePart>,
    delta_parts: &[MessagePart],
) {
    for part in delta_parts {
        match part {
            MessagePart::Text {
                content,
                parent_tool_use_id,
            } => {
                if let Some(MessagePart::Text {
                    content: existing,
                    parent_tool_use_id: existing_parent,
                }) = streaming_parts.last_mut()
                {
                    if existing_parent == parent_tool_use_id {
                        existing.push_str(content);
                        continue;
                    }
                }
                streaming_parts.push(part.clone());
            }
            MessagePart::Thinking {
                content,
                parent_tool_use_id,
            } => {
                if let Some(MessagePart::Thinking {
                    content: existing,
                    parent_tool_use_id: existing_parent,
                }) = streaming_parts.last_mut()
                {
                    if existing_parent == parent_tool_use_id {
                        existing.push_str(content);
                        continue;
                    }
                }
                streaming_parts.push(part.clone());
            }
            _ => streaming_parts.push(part.clone()),
        }
    }
}

pub(crate) fn pending_exceeds_streaming_threshold(
    pending_part_count: usize,
    pending_byte_size: usize,
) -> bool {
    pending_part_count >= STREAMING_PENDING_PART_LIMIT
        || pending_byte_size >= STREAMING_PENDING_BYTE_LIMIT
}

pub(crate) fn streaming_flush_decision(
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
    if has_retry || pending_exceeds_streaming_threshold(pending_part_count, pending_byte_size) {
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

pub(crate) fn should_persist_streaming_snapshot(
    last_persist_at: Option<Instant>,
    now: Instant,
    force: bool,
) -> bool {
    force
        || last_persist_at.is_none_or(|last_persist_at| {
            now.saturating_duration_since(last_persist_at) >= STREAMING_PERSIST_INTERVAL
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_streaming_flush_decision_初回pendingは即時flushする() {
        let now = Instant::now();

        assert_eq!(
            streaming_flush_decision(true, false, 1, 1, None, now),
            StreamingFlushDecision::Now
        );
    }

    #[test]
    fn test_streaming_flush_decision_間隔未満なら残り時間を返す() {
        let now = Instant::now();
        let last = now - Duration::from_millis(10);

        assert_eq!(
            streaming_flush_decision(true, false, 1, 1, Some(last), now),
            StreamingFlushDecision::Later(Duration::from_millis(23))
        );
    }

    #[test]
    fn test_streaming_flush_decision_閾値超過とretryは即時flushする() {
        let now = Instant::now();

        assert_eq!(
            streaming_flush_decision(true, false, STREAMING_PENDING_PART_LIMIT, 1, Some(now), now,),
            StreamingFlushDecision::Now
        );
        assert_eq!(
            streaming_flush_decision(false, true, 0, 0, Some(now), now),
            StreamingFlushDecision::Now
        );
    }

    #[test]
    fn test_parts_can_stream_as_append_delta_文字系のみtrue() {
        assert!(parts_can_stream_as_append_delta(&[
            MessagePart::Text {
                content: "a".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Thinking {
                content: "b".to_string(),
                parent_tool_use_id: None,
            },
        ]));
        assert!(!parts_can_stream_as_append_delta(&[MessagePart::Error {
            content: "boom".to_string(),
            parent_tool_use_id: None,
        }]));
    }

    #[test]
    fn test_merge_streaming_append_delta_parts_merges_adjacent_text_and_thinking() {
        let mut parts = vec![
            MessagePart::Text {
                content: "hel".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Thinking {
                content: "th".to_string(),
                parent_tool_use_id: Some("tool-1".to_string()),
            },
        ];

        merge_streaming_append_delta_parts(
            &mut parts,
            &[
                MessagePart::Thinking {
                    content: "ink".to_string(),
                    parent_tool_use_id: Some("tool-1".to_string()),
                },
                MessagePart::Text {
                    content: "lo".to_string(),
                    parent_tool_use_id: None,
                },
            ],
        );

        assert_eq!(
            parts,
            vec![
                MessagePart::Text {
                    content: "hel".to_string(),
                    parent_tool_use_id: None,
                },
                MessagePart::Thinking {
                    content: "think".to_string(),
                    parent_tool_use_id: Some("tool-1".to_string()),
                },
                MessagePart::Text {
                    content: "lo".to_string(),
                    parent_tool_use_id: None,
                },
            ]
        );
    }

    #[test]
    fn test_should_persist_streaming_snapshot_一秒間隔またはforceでtrue() {
        let now = Instant::now();

        assert!(should_persist_streaming_snapshot(None, now, false));
        assert!(!should_persist_streaming_snapshot(
            Some(now - Duration::from_millis(999)),
            now,
            false,
        ));
        assert!(should_persist_streaming_snapshot(
            Some(now - Duration::from_millis(999)),
            now,
            true,
        ));
        assert!(should_persist_streaming_snapshot(
            Some(now - Duration::from_secs(1)),
            now,
            false,
        ));
    }
}
