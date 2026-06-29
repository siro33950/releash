use super::process_registry::{AgentProcessMap, BridgeState, TurnPhase};
use crate::infrastructure::agent_session::resolver_ports::{
    BaseBranchResolverPort, MentionResolverPort,
};
use crate::infrastructure::agent_session::runtime::context_restore::RestoreContextPayload;
use crate::infrastructure::agent_session::runtime::runtime_coordinator::is_pending_turn_starting;
use crate::infrastructure::agent_session::runtime::BackendRuntimeConfig;
use crate::infrastructure::agent_session::runtime::ImageAttachment;
use crate::usecase::agent_session::session::{
    apply_tool_result_update, MessagePart, SessionStore, ToolResultUpdate,
};
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tauri::Manager;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

pub(super) static GENERATION_COUNTER: AtomicU64 = AtomicU64::new(1);

pub const CLAUDE_BACKEND_ID: &str = "claude";
pub const CODEX_BACKEND_ID: &str = "codex";
pub(crate) const DEFER_AGENT_SESSION_ID_PERSIST_ON_READY: &str =
    "defer_agent_session_id_persist_on_ready";

pub(super) fn fallback_prompt_message_id(streaming_message_id: &str) -> String {
    format!("{streaming_message_id}:prompt")
}

/// Normalize accumulated streaming parts by merging consecutive same-type
/// text/thinking chunks sharing the same `parent_tool_use_id`.
/// During streaming, `append_to_parts` keeps each chunk as an individual part;
/// this helper produces the consolidated view used both for streaming emit
/// payloads (via `prepare_streaming_flush`) and for persistence.
pub(super) fn consolidate_parts_from_slice(parts: &[MessagePart]) -> Vec<MessagePart> {
    let mut result: Vec<MessagePart> = Vec::with_capacity(parts.len());
    for part in parts {
        match (part, result.last_mut()) {
            (
                MessagePart::Text {
                    content,
                    parent_tool_use_id,
                },
                Some(MessagePart::Text {
                    content: last_content,
                    parent_tool_use_id: last_pid,
                }),
            ) if parent_tool_use_id == last_pid => {
                last_content.push_str(content);
            }
            (
                MessagePart::Text {
                    content,
                    parent_tool_use_id,
                },
                _,
            ) => result.push(MessagePart::Text {
                content: content.clone(),
                parent_tool_use_id: parent_tool_use_id.clone(),
            }),
            (
                MessagePart::Thinking {
                    content,
                    parent_tool_use_id,
                },
                Some(MessagePart::Thinking {
                    content: last_content,
                    parent_tool_use_id: last_pid,
                }),
            ) if parent_tool_use_id == last_pid => {
                last_content.push_str(content);
            }
            (
                MessagePart::Thinking {
                    content,
                    parent_tool_use_id,
                },
                _,
            ) => result.push(MessagePart::Thinking {
                content: content.clone(),
                parent_tool_use_id: parent_tool_use_id.clone(),
            }),
            _ => {
                result.push(part.clone());
            }
        }
    }
    result
}

pub(super) fn pending_delta_parts(parts: &[MessagePart], pending_count: usize) -> Vec<MessagePart> {
    let pending_count = pending_count.min(parts.len());
    if pending_count == 0 {
        return Vec::new();
    }
    consolidate_parts_from_slice(&parts[parts.len() - pending_count..])
}

/// Normalize a cumulative stream snapshot with the same identity convergence
/// rules used for seq delta application.
pub(super) fn canonical_stream_parts_from_slice(parts: &[MessagePart]) -> Vec<MessagePart> {
    let mut result = Vec::with_capacity(parts.len());
    append_stream_delta_parts(&mut result, parts);
    result
}

pub(super) fn append_display_delta_parts(
    current_parts: &mut Vec<MessagePart>,
    delta_parts: &[MessagePart],
) {
    for part in delta_parts {
        match part {
            MessagePart::Text {
                content,
                parent_tool_use_id,
            } => match current_parts.last_mut() {
                Some(MessagePart::Text {
                    content: last_content,
                    parent_tool_use_id: last_pid,
                }) if parent_tool_use_id == last_pid => {
                    last_content.push_str(content);
                }
                _ => current_parts.push(part.clone()),
            },
            MessagePart::Thinking {
                content,
                parent_tool_use_id,
            } => match current_parts.last_mut() {
                Some(MessagePart::Thinking {
                    content: last_content,
                    parent_tool_use_id: last_pid,
                }) if parent_tool_use_id == last_pid => {
                    last_content.push_str(content);
                }
                _ => current_parts.push(part.clone()),
            },
            _ => current_parts.push(part.clone()),
        }
    }
}

pub(super) fn append_stream_delta_parts(
    current_parts: &mut Vec<MessagePart>,
    delta_parts: &[MessagePart],
) {
    for part in delta_parts {
        match part {
            MessagePart::Text {
                content,
                parent_tool_use_id,
            } => match current_parts.last_mut() {
                Some(MessagePart::Text {
                    content: last_content,
                    parent_tool_use_id: last_pid,
                }) if parent_tool_use_id == last_pid => {
                    last_content.push_str(content);
                }
                _ => current_parts.push(part.clone()),
            },
            MessagePart::Thinking {
                content,
                parent_tool_use_id,
            } => match current_parts.last_mut() {
                Some(MessagePart::Thinking {
                    content: last_content,
                    parent_tool_use_id: last_pid,
                }) if parent_tool_use_id == last_pid => {
                    last_content.push_str(content);
                }
                _ => current_parts.push(part.clone()),
            },
            MessagePart::ToolUse { id, .. } => {
                if let Some(existing) = current_parts.iter_mut().rev().find(|existing| {
                    matches!(existing, MessagePart::ToolUse { id: existing_id, .. } if existing_id == id)
                }) {
                    *existing = part.clone();
                } else {
                    current_parts.push(part.clone());
                }
            }
            MessagePart::ToolResult {
                content,
                is_error,
                tool_use_id: Some(tool_use_id),
                parent_tool_use_id,
                content_ref,
                summary,
            } => {
                let _ = apply_tool_result_update(
                    current_parts,
                    ToolResultUpdate {
                        content: content.clone(),
                        is_error: *is_error,
                        tool_use_id: Some(tool_use_id.clone()),
                        parent_tool_use_id: parent_tool_use_id.clone(),
                        content_ref: content_ref.clone(),
                        summary: summary.clone(),
                    },
                );
            }
            MessagePart::TaskStatus {
                task_tool_use_id, ..
            } => {
                if let Some(existing) = current_parts.iter_mut().rev().find(|existing| {
                    matches!(
                        existing,
                        MessagePart::TaskStatus {
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
            MessagePart::TodoListSnapshot { .. } => {
                if let Some(existing) = current_parts
                    .iter_mut()
                    .rev()
                    .find(|existing| matches!(existing, MessagePart::TodoListSnapshot { .. }))
                {
                    *existing = part.clone();
                } else {
                    current_parts.push(part.clone());
                }
            }
            MessagePart::SystemNotification {
                notification_type, ..
            } => {
                if let Some(existing) = current_parts.iter_mut().rev().find(|existing| {
                    matches!(
                        existing,
                        MessagePart::SystemNotification {
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
            MessagePart::Permission { request, .. } => {
                let request_id = request.get("request_id").and_then(|value| value.as_str());
                let tool_use_id = request.get("tool_use_id").and_then(|value| value.as_str());
                if let Some(existing) = current_parts.iter_mut().rev().find(|existing| {
                    let MessagePart::Permission {
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

pub(super) fn parts_have_tool_output_ref(parts: &[MessagePart]) -> bool {
    parts.iter().any(|part| {
        matches!(
            part,
            MessagePart::ToolResult {
                content_ref: Some(_),
                ..
            }
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StreamDeltaApplyResult {
    Applied,
    Duplicate,
    Gap { expected: u64, actual: u64 },
}

pub(super) fn apply_stream_delta_to_parts(
    current_parts: &mut Vec<MessagePart>,
    last_seq: &mut u64,
    seq: u64,
    delta_parts: &[MessagePart],
) -> StreamDeltaApplyResult {
    if seq <= *last_seq {
        return StreamDeltaApplyResult::Duplicate;
    }
    let expected = last_seq.saturating_add(1);
    if seq != expected {
        return StreamDeltaApplyResult::Gap {
            expected,
            actual: seq,
        };
    }
    append_stream_delta_parts(current_parts, delta_parts);
    *last_seq = seq;
    StreamDeltaApplyResult::Applied
}

#[cfg(test)]
pub(super) fn apply_stream_snapshot_to_parts(
    current_parts: &mut Vec<MessagePart>,
    last_seq: &mut u64,
    seq: u64,
    snapshot_parts: &[MessagePart],
) {
    current_parts.clear();
    current_parts.extend_from_slice(snapshot_parts);
    *last_seq = seq;
}

#[cfg(test)]
mod stream_delta_tests {
    use super::*;

    fn text(content: &str) -> MessagePart {
        MessagePart::Text {
            content: content.to_string(),
            parent_tool_use_id: None,
        }
    }

    fn tool_use(id: &str, description: &str) -> MessagePart {
        MessagePart::ToolUse {
            tool: "Task".to_string(),
            input: serde_json::json!({ "description": description }),
            id: id.to_string(),
            parent_tool_use_id: None,
        }
    }

    fn tool_result(tool_use_id: &str, content: &str, is_error: bool) -> MessagePart {
        MessagePart::ToolResult {
            content: content.to_string(),
            is_error,
            tool_use_id: Some(tool_use_id.to_string()),
            parent_tool_use_id: None,
            content_ref: None,
            summary: None,
        }
    }

    fn ref_tool_result(tool_use_id: &str, content: &str, id: &str) -> MessagePart {
        MessagePart::ToolResult {
            content: content.to_string(),
            is_error: false,
            tool_use_id: Some(tool_use_id.to_string()),
            parent_tool_use_id: None,
            content_ref: Some(crate::usecase::agent_session::session::ToolOutputRef {
                id: id.to_string(),
                byte_size: 4096,
            }),
            summary: Some(crate::usecase::agent_session::session::ToolOutputSummary {
                line_count: 200,
                byte_size: 4096,
                is_error: false,
                truncated: true,
            }),
        }
    }

    fn ref_error_tool_result(tool_use_id: &str, content: &str, id: &str) -> MessagePart {
        MessagePart::ToolResult {
            content: content.to_string(),
            is_error: true,
            tool_use_id: Some(tool_use_id.to_string()),
            parent_tool_use_id: None,
            content_ref: Some(crate::usecase::agent_session::session::ToolOutputRef {
                id: id.to_string(),
                byte_size: 4096,
            }),
            summary: Some(crate::usecase::agent_session::session::ToolOutputSummary {
                line_count: 200,
                byte_size: 4096,
                is_error: true,
                truncated: true,
            }),
        }
    }

    fn task_status(task_tool_use_id: &str, status: &str, summary: Option<&str>) -> MessagePart {
        MessagePart::TaskStatus {
            task_tool_use_id: task_tool_use_id.to_string(),
            status: status.to_string(),
            description: Some(status.to_string()),
            summary: summary.map(str::to_string),
        }
    }

    fn todo_snapshot(completed: bool) -> MessagePart {
        MessagePart::TodoListSnapshot {
            items: vec![crate::usecase::agent_session::session::TodoListItem {
                text: "ship".to_string(),
                completed,
            }],
        }
    }

    fn system_notification(status: &str, label: &str) -> MessagePart {
        MessagePart::SystemNotification {
            notification_type:
                crate::usecase::agent_session::session::SystemNotificationType::Compaction,
            status: status.to_string(),
            label: label.to_string(),
            detail: None,
            hook_id: None,
        }
    }

    fn permission(request_id: &str, tool_use_id: &str, status: &str) -> MessagePart {
        MessagePart::Permission {
            request: serde_json::json!({
                "request_id": request_id,
                "tool_use_id": tool_use_id,
                "tool_name": "Bash",
                "input": {},
            }),
            status: status.to_string(),
            answers: None,
            parent_tool_use_id: None,
        }
    }

    #[test]
    fn pending_delta_parts_uses_only_pending_suffix() {
        let parts = vec![text("old "), text("new")];

        let delta = pending_delta_parts(&parts, 1);

        assert_eq!(delta, vec![text("new")]);
    }

    #[test]
    fn apply_stream_delta_is_idempotent_for_duplicate_seq() {
        let mut parts = vec![text("Hel")];
        let mut last_seq = 1;

        assert_eq!(
            apply_stream_delta_to_parts(&mut parts, &mut last_seq, 2, &[text("lo")]),
            StreamDeltaApplyResult::Applied
        );
        assert_eq!(
            apply_stream_delta_to_parts(&mut parts, &mut last_seq, 2, &[text("lo")]),
            StreamDeltaApplyResult::Duplicate
        );

        assert_eq!(parts, vec![text("Hello")]);
        assert_eq!(last_seq, 2);
    }

    #[test]
    fn apply_stream_delta_detects_seq_gap_without_mutating_parts() {
        let mut parts = vec![text("Hello")];
        let mut last_seq = 1;

        assert_eq!(
            apply_stream_delta_to_parts(&mut parts, &mut last_seq, 3, &[text(" skipped")]),
            StreamDeltaApplyResult::Gap {
                expected: 2,
                actual: 3,
            }
        );

        assert_eq!(parts, vec![text("Hello")]);
        assert_eq!(last_seq, 1);
    }

    #[test]
    fn snapshot_application_converges_to_cumulative_state() {
        let all_parts = vec![text("Hel"), text("lo"), text(" world")];
        let snapshot = consolidate_parts_from_slice(&all_parts);
        let mut applied = Vec::new();
        let mut last_seq = 0;

        assert_eq!(
            apply_stream_delta_to_parts(&mut applied, &mut last_seq, 1, &[text("Hel")]),
            StreamDeltaApplyResult::Applied
        );
        apply_stream_snapshot_to_parts(&mut applied, &mut last_seq, 3, &snapshot);

        assert_eq!(applied, snapshot);
        assert_eq!(last_seq, 3);
    }

    #[test]
    fn seq_delta_application_converges_identity_updates_to_snapshot_state() {
        let mut applied = vec![
            tool_use("tool-1", "old"),
            text("tail"),
            tool_result("tool-1", "failed", true),
            tool_result("tool-2", "partial", false),
            tool_result("tool-3", "hello", false),
            task_status("tool-1", "started", None),
            todo_snapshot(false),
            system_notification("in_progress", "Compacting"),
            permission("req-1", "tool-1", "pending"),
        ];
        let mut last_seq = 0;

        let first_delta = vec![
            tool_use("tool-1", "new"),
            MessagePart::ToolResult {
                content: "success".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: Some("parent-1".to_string()),
                content_ref: None,
                summary: None,
            },
            tool_result("tool-2", "partial complete", false),
            tool_result("tool-3", " world", false),
        ];
        let second_delta = vec![
            task_status("tool-1", "completed", Some("done")),
            todo_snapshot(true),
            system_notification("completed", "Compacted"),
            permission("req-1", "tool-1", "allowed"),
        ];

        assert_eq!(
            apply_stream_delta_to_parts(&mut applied, &mut last_seq, 1, &first_delta),
            StreamDeltaApplyResult::Applied
        );
        assert_eq!(
            apply_stream_delta_to_parts(&mut applied, &mut last_seq, 2, &second_delta),
            StreamDeltaApplyResult::Applied
        );

        assert_eq!(
            applied,
            vec![
                tool_use("tool-1", "new"),
                text("tail"),
                MessagePart::ToolResult {
                    content: "success".to_string(),
                    is_error: false,
                    tool_use_id: Some("tool-1".to_string()),
                    parent_tool_use_id: Some("parent-1".to_string()),
                    content_ref: None,
                    summary: None,
                },
                tool_result("tool-2", "partial complete", false),
                tool_result("tool-3", "hello world", false),
                task_status("tool-1", "completed", Some("done")),
                todo_snapshot(true),
                system_notification("completed", "Compacted"),
                permission("req-1", "tool-1", "allowed"),
            ]
        );
        assert_eq!(last_seq, 2);
    }

    #[test]
    fn seq_delta_application_matches_canonical_snapshot_with_identity_updates() {
        let deltas = [
            vec![
                tool_use("tool-1", "old"),
                tool_result("tool-1", "failed", true),
                tool_result("tool-2", "partial", false),
                tool_result("tool-3", "hello", false),
                task_status("tool-1", "started", None),
                todo_snapshot(false),
                system_notification("in_progress", "Compacting"),
                permission("req-1", "tool-1", "pending"),
            ],
            vec![
                tool_use("tool-1", "new"),
                MessagePart::ToolResult {
                    content: "success".to_string(),
                    is_error: false,
                    tool_use_id: Some("tool-1".to_string()),
                    parent_tool_use_id: Some("parent-1".to_string()),
                    content_ref: None,
                    summary: None,
                },
                tool_result("tool-2", "partial complete", false),
                tool_result("tool-3", " world", false),
            ],
            vec![
                task_status("tool-1", "completed", Some("done")),
                todo_snapshot(true),
                system_notification("completed", "Compacted"),
                permission("req-1", "tool-1", "allowed"),
            ],
        ];
        let mut applied = Vec::new();
        let mut raw_snapshot_parts = Vec::new();
        let mut last_seq = 0;

        for (index, delta) in deltas.iter().enumerate() {
            let seq = (index + 1) as u64;
            raw_snapshot_parts.extend_from_slice(delta);
            assert_eq!(
                apply_stream_delta_to_parts(&mut applied, &mut last_seq, seq, delta),
                StreamDeltaApplyResult::Applied
            );
        }

        assert_eq!(
            applied,
            canonical_stream_parts_from_slice(&raw_snapshot_parts)
        );
        assert_eq!(
            applied,
            vec![
                tool_use("tool-1", "new"),
                MessagePart::ToolResult {
                    content: "success".to_string(),
                    is_error: false,
                    tool_use_id: Some("tool-1".to_string()),
                    parent_tool_use_id: Some("parent-1".to_string()),
                    content_ref: None,
                    summary: None,
                },
                tool_result("tool-2", "partial complete", false),
                tool_result("tool-3", "hello world", false),
                task_status("tool-1", "completed", Some("done")),
                todo_snapshot(true),
                system_notification("completed", "Compacted"),
                permission("req-1", "tool-1", "allowed"),
            ]
        );
        let before_duplicate = applied.clone();
        assert_eq!(
            apply_stream_delta_to_parts(&mut applied, &mut last_seq, 3, &deltas[2]),
            StreamDeltaApplyResult::Duplicate
        );
        assert_eq!(applied, before_duplicate);
    }

    #[test]
    fn ref_backed_tool_result_keeps_later_delta_as_separate_part() {
        let base = ref_tool_result("tool-1", "preview", &"a".repeat(64));
        let delta = tool_result("tool-1", " late", false);
        let mut applied = vec![base.clone()];
        let mut last_seq = 0;

        assert_eq!(
            apply_stream_delta_to_parts(
                &mut applied,
                &mut last_seq,
                1,
                std::slice::from_ref(&delta)
            ),
            StreamDeltaApplyResult::Applied
        );

        assert_eq!(applied, vec![base.clone(), delta.clone()]);
        assert_eq!(
            canonical_stream_parts_from_slice(&[base.clone(), delta.clone()]),
            vec![base, delta]
        );
    }

    #[test]
    fn ref_backed_tool_result_ignores_empty_inline_delta_without_separate_part() {
        let base = ref_tool_result("tool-1", "preview", &"a".repeat(64));
        let delta = tool_result("tool-1", "", false);
        let mut applied = vec![base.clone()];
        let mut last_seq = 0;

        assert_eq!(
            apply_stream_delta_to_parts(&mut applied, &mut last_seq, 1, &[delta]),
            StreamDeltaApplyResult::Applied
        );

        assert_eq!(applied, vec![base.clone()]);
        assert_eq!(
            canonical_stream_parts_from_slice(&[base.clone(), tool_result("tool-1", "", false)]),
            vec![base]
        );
    }

    #[test]
    fn error_recovery_clears_ref_and_summary_consistently_across_merge_paths() {
        use crate::usecase::agent_session::event_log::{project, AgentSessionEvent, PromptInput};

        for content in ["", "success"] {
            let base = ref_error_tool_result("tool-1", "error preview", &"b".repeat(64));
            let delta = tool_result("tool-1", content, false);
            let live_parts = canonical_stream_parts_from_slice(&[base.clone(), delta.clone()]);

            let projected_parts = project(&[
                AgentSessionEvent::TurnStarted {
                    turn_id: 1,
                    message_id: "human-1".to_string(),
                    assistant_message_id: Some("agent-1".to_string()),
                    prompt: PromptInput::default(),
                    at: 0.0,
                },
                AgentSessionEvent::ToolResultRecorded {
                    turn_id: 1,
                    message_id: "agent-1".to_string(),
                    content: "error preview".to_string(),
                    is_error: true,
                    content_ref: match &base {
                        MessagePart::ToolResult { content_ref, .. } => content_ref.clone(),
                        _ => None,
                    },
                    summary: match &base {
                        MessagePart::ToolResult { summary, .. } => summary.clone(),
                        _ => None,
                    },
                    tool_use_id: Some("tool-1".to_string()),
                    parent_tool_use_id: None,
                },
                AgentSessionEvent::ToolResultRecorded {
                    turn_id: 1,
                    message_id: "agent-1".to_string(),
                    content: content.to_string(),
                    is_error: false,
                    content_ref: None,
                    summary: None,
                    tool_use_id: Some("tool-1".to_string()),
                    parent_tool_use_id: None,
                },
            ])
            .agent_parts_for_message("agent-1");

            let mut sdk_parts = vec![base];
            let _ = super::super::sdk_message::push_or_update_tool_result(
                &mut sdk_parts,
                content.to_string(),
                false,
                Some("tool-1".to_string()),
                None,
            );

            let expected = vec![MessagePart::ToolResult {
                content: content.to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            }];
            assert_eq!(live_parts, expected);
            assert_eq!(projected_parts, expected);
            assert_eq!(sdk_parts, expected);
        }
    }

    #[test]
    fn append_stream_delta_parts_updates_existing_non_tail_parts() {
        let mut parts = vec![
            MessagePart::ToolUse {
                tool: "Task".to_string(),
                input: serde_json::json!({"description": "old"}),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            },
            text("tail"),
            MessagePart::TaskStatus {
                task_tool_use_id: "tool-1".to_string(),
                status: "started".to_string(),
                description: Some("old".to_string()),
                summary: None,
            },
            MessagePart::TodoListSnapshot {
                items: vec![crate::usecase::agent_session::session::TodoListItem {
                    text: "first".to_string(),
                    completed: false,
                }],
            },
            MessagePart::SystemNotification {
                notification_type:
                    crate::usecase::agent_session::session::SystemNotificationType::Compaction,
                status: "in_progress".to_string(),
                label: "Compacting".to_string(),
                detail: None,
                hook_id: None,
            },
        ];

        append_stream_delta_parts(
            &mut parts,
            &[
                MessagePart::ToolUse {
                    tool: "Task".to_string(),
                    input: serde_json::json!({"description": "new"}),
                    id: "tool-1".to_string(),
                    parent_tool_use_id: None,
                },
                MessagePart::TaskStatus {
                    task_tool_use_id: "tool-1".to_string(),
                    status: "completed".to_string(),
                    description: Some("new".to_string()),
                    summary: Some("done".to_string()),
                },
                MessagePart::TodoListSnapshot {
                    items: vec![crate::usecase::agent_session::session::TodoListItem {
                        text: "first".to_string(),
                        completed: true,
                    }],
                },
                MessagePart::SystemNotification {
                    notification_type:
                        crate::usecase::agent_session::session::SystemNotificationType::Compaction,
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
            MessagePart::ToolUse { input, .. }
                if input.get("description").and_then(|value| value.as_str()) == Some("new")
        ));
        assert!(matches!(
            &parts[2],
            MessagePart::TaskStatus { status, summary, .. }
                if status == "completed" && summary.as_deref() == Some("done")
        ));
        assert!(matches!(
            &parts[3],
            MessagePart::TodoListSnapshot { items }
                if items.first().is_some_and(|item| item.completed)
        ));
        assert!(matches!(
            &parts[4],
            MessagePart::SystemNotification { status, label, .. }
                if status == "completed" && label == "Compacted"
        ));
    }
}

/// 状態遷移時に AgentStatusCenter へ通知する統一エントリ。
/// session_store から metadata だけを引いて worktree_path / SessionState を取得する。
/// `session_state_override` を渡すと、ストア値より優先される（Bridge crash 時など）。
pub(crate) fn notify_status_transition<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    turn_phase: TurnPhase,
    session_state_override: Option<crate::usecase::agent_session::session::SessionState>,
) {
    notify_status_transition_with_pending_permission_request(
        app,
        session_store,
        chat_session_id,
        turn_phase,
        session_state_override,
        None,
    );
}

pub(crate) fn notify_status_transition_with_pending_permission_request<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    chat_session_id: &str,
    turn_phase: TurnPhase,
    session_state_override: Option<crate::usecase::agent_session::session::SessionState>,
    pending_permission_request: Option<serde_json::Value>,
) {
    use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
    use crate::usecase::agent_session::status::{
        current_timestamp, AgentStatusCenter, AgentStatusNotifier, SessionStatus, TurnPhaseRepr,
    };

    let data_dir = match resolve_data_dir(app) {
        Ok(d) => d,
        Err(_) => return,
    };
    let meta = match session_store.get_session_meta(&data_dir, chat_session_id) {
        Ok(Some(meta)) => meta,
        _ => return,
    };
    let worktree_path = meta.worktree_path.clone();
    let session_state = match (&meta.state, session_state_override) {
        (
            crate::usecase::agent_session::session::SessionState::Done
            | crate::usecase::agent_session::session::SessionState::Archived,
            _,
        ) => meta.state.clone(),
        (_, Some(projected)) => projected,
        (_, None) => meta.state.clone(),
    };

    let status_turn_phase = match turn_phase {
        TurnPhase::Idle => crate::usecase::agent_session::status::TurnPhase::Idle,
        TurnPhase::Streaming => crate::usecase::agent_session::status::TurnPhase::Streaming,
        TurnPhase::WaitingPermission => {
            crate::usecase::agent_session::status::TurnPhase::WaitingPermission
        }
    };
    let agent_state =
        AgentStatusCenter::derive_agent_state(status_turn_phase, session_state.clone());

    if let Some(center) = app.try_state::<Arc<AgentStatusCenter>>() {
        let (wf_step, wf_state, wf_execution_id, wf_run_index, wf_step_progress) = center
            .get_session(chat_session_id)
            .map(|s| {
                (
                    s.workflow_step,
                    s.workflow_execution_state,
                    s.workflow_execution_id,
                    s.workflow_run_index,
                    s.workflow_step_progress,
                )
            })
            .unwrap_or((None, None, None, None, None));
        let status = SessionStatus {
            chat_session_id: chat_session_id.to_string(),
            worktree_id: worktree_path.clone(),
            worktree_path: worktree_path.clone(),
            pty_id: None,
            agent_state: agent_state.clone(),
            turn_phase: TurnPhaseRepr::from(status_turn_phase),
            session_state,
            pending_permission: matches!(turn_phase, TurnPhase::WaitingPermission),
            pending_permission_request: matches!(turn_phase, TurnPhase::WaitingPermission)
                .then_some(pending_permission_request)
                .flatten(),
            last_activity_at: current_timestamp(),
            workflow_step: wf_step,
            workflow_execution_state: wf_state,
            workflow_execution_id: wf_execution_id,
            workflow_run_index: wf_run_index,
            workflow_step_progress: wf_step_progress,
        };
        let changes = center.update_session(status);
        if let Some(notifier) = app.try_state::<Arc<dyn AgentStatusNotifier>>() {
            notifier.status_changed(changes);
        }
    }
}

/// `(PermissionMode, backend_id)` から JS bridge の init / setMode コマンドに載せる
/// permission 関連フィールドのみを生成する。init と setMode の双方からこのヘルパー経由で
/// 同じ変換ロジックを参照し、Claude/Codex 判定とフラグ変換の重複実装を防ぐ
/// （Spec issues-947: バックエンド変換層の DRY 化）。
pub(super) fn bridge_permission_fields(
    pm: crate::domain::agent_session::PermissionMode,
    backend_id: &str,
    plan_mode: bool,
) -> Vec<(String, serde_json::Value)> {
    use crate::infrastructure::agent_session::runtime::permission_flags::{
        claude_flag_from_mode, codex_approval_policy_from_mode, codex_sandbox_mode_from_mode,
    };
    if backend_id == CODEX_BACKEND_ID {
        if plan_mode {
            return vec![
                (
                    "approvalPolicy".to_string(),
                    serde_json::Value::String("on-request".to_string()),
                ),
                (
                    "sandboxMode".to_string(),
                    serde_json::Value::String("read-only".to_string()),
                ),
                (
                    "collaborationMode".to_string(),
                    serde_json::Value::String("plan".to_string()),
                ),
            ];
        }
        vec![
            (
                "approvalPolicy".to_string(),
                serde_json::Value::String(codex_approval_policy_from_mode(pm).to_string()),
            ),
            (
                "sandboxMode".to_string(),
                serde_json::Value::String(codex_sandbox_mode_from_mode(pm).to_string()),
            ),
        ]
    } else {
        vec![(
            "permissionMode".to_string(),
            serde_json::Value::String(
                if plan_mode {
                    "plan"
                } else {
                    claude_flag_from_mode(pm)
                }
                .to_string(),
            ),
        )]
    }
}

pub(crate) async fn is_agent_step_runtime_busy(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
) -> bool {
    if is_pending_turn_starting(chat_session_id).await {
        return true;
    }
    let map = handles.lock().await;
    map.get(chat_session_id).is_some_and(|proc| {
        proc.state == BridgeState::Initializing
            || proc.state == BridgeState::Streaming
            || proc.turn_phase == TurnPhase::WaitingPermission
            || !proc.pending_messages.is_empty()
    })
}

#[cfg(test)]
pub(super) async fn agent_session_has_pending_message(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
) -> bool {
    let map = handles.lock().await;
    map.get(chat_session_id)
        .is_some_and(|proc| !proc.pending_messages.is_empty())
}

const CLAUDE_BRIDGE_RESOURCE: &str = "generated/bridges/claude-sdk-bridge.bundled.mjs";

pub(super) fn resolve_bridge_script<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    backend_id: &str,
) -> Result<PathBuf, String> {
    if backend_id != CLAUDE_BACKEND_ID {
        return Err(format!("Unsupported Node bridge backend: {backend_id}"));
    }

    app.path()
        .resource_dir()
        .map(|d| d.join(CLAUDE_BRIDGE_RESOURCE))
        .map_err(|e| format!("Failed to resolve resource dir: {e}"))
}

pub(crate) async fn write_bridge_command(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    payload: serde_json::Value,
) -> Result<(), String> {
    let data = format!("{payload}\n");
    let stdin = {
        let map = handles.lock().await;
        map.get(chat_session_id)
            .map(|proc| Arc::clone(&proc.stdin))
            .ok_or_else(|| format!("No active agent process for session {chat_session_id}"))?
    };
    let mut stdin = stdin.lock().await;
    stdin
        .write_all(data.as_bytes())
        .await
        .map_err(|e| format!("Failed to write bridge command: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush bridge command: {e}"))?;
    Ok(())
}

pub(super) async fn write_bridge_command_for_captured_turn(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    captured_gen_id: u64,
    captured_turn_seq: u64,
    payload: serde_json::Value,
) -> Result<bool, String> {
    let data = format!("{payload}\n");
    let stdin = {
        let map = handles.lock().await;
        let Some(proc) = map.get(chat_session_id) else {
            return Ok(false);
        };
        if proc.generation_id != captured_gen_id || proc.turn_seq != captured_turn_seq {
            return Ok(false);
        }
        Arc::clone(&proc.stdin)
    };
    let mut stdin = stdin.lock().await;
    stdin
        .write_all(data.as_bytes())
        .await
        .map_err(|e| format!("Failed to write bridge command: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush bridge command: {e}"))?;
    Ok(true)
}

pub(super) fn backend_runtime_config<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    backend_id: &str,
) -> BackendRuntimeConfig {
    app.try_state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>()
        .and_then(
            |registry| match registry.runtime_config_for(backend_id, app) {
                Ok(config) => Some(config),
                Err(e) => {
                    log::warn!("backend '{backend_id}' runtime config could not be resolved: {e}");
                    None
                }
            },
        )
        .unwrap_or_default()
}

/// 指定 backend の登録モデル一覧を config.toml から取得する。
///
/// - registry 未指定（テスト等）: `Ok(Vec::new())`
/// - registry の lookup が失敗（config 未紐付け／schema 未対応／lock 失敗）: `Err`
///
/// 「登録済みモデルが 0 件」と infrastructure 故障を呼び出し側で区別できるよう
/// 必ず Err を伝播する。表示専用経路は warn + 既存値維持で空上書きを防ぐこと、
/// 永続化に絡む経路は Err をそのまま呼び出し元に伝えること。
pub(crate) fn session_specific_env_overrides(
    chat_session_id: &str,
    base_branch: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut env = vec![("RELEASH_SESSION_ID", chat_session_id.to_string())];
    if let Some(b) = base_branch {
        env.push(("RELEASH_BASE_BRANCH", b.to_string()));
    }
    env
}

pub(super) fn resolve_effective_base_branch_from_port<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    cwd: &str,
) -> Option<String> {
    let Some(resolver) = app
        .try_state::<Arc<dyn BaseBranchResolverPort>>()
        .map(|state| state.inner().clone())
    else {
        log::warn!("base branch resolver port is not registered; continuing without base branch");
        return None;
    };
    resolver.resolve_effective_base_branch_name(cwd)
}

pub(super) fn resolve_mentions_or_fallback_from_port<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    worktree_path: &str,
    content: &str,
    mentions: &[crate::domain::code::MentionReference],
) -> String {
    let Some(resolver) = app
        .try_state::<Arc<dyn MentionResolverPort>>()
        .map(|state| state.inner().clone())
    else {
        log::warn!("mention resolver port is not registered; continuing without resolved mentions");
        return content.to_string();
    };
    resolver.resolve_mentions_or_fallback(worktree_path, content, mentions)
}

/// ユーザー指定の system_prompt に Releash CLI の long help を append する。
///
/// spec issues-1022 "Agent process environment contract": Agent process の
/// system_prompt には Releash CLI の long help が常に含まれ、Agent は help を
/// 別経路で取得する必要を持たない。clap derive 由来 (`cli::render_long_help`) を
/// 単一ソースとし、Agent 向けに別個の文字列を手書きしない。
///
/// - `None` または空文字 → `Some(<help>)`
/// - `Some(user)` → `Some("{user}\n\n{help}")`
pub(super) fn compose_system_prompt(user: Option<String>) -> Option<String> {
    let help = crate::cli::render_long_help();
    let composed = match user {
        Some(ref s) if !s.is_empty() => format!("{s}\n\n{help}"),
        _ => help.to_string(),
    };
    Some(composed)
}

pub(super) fn runtime_system_prompt_fingerprint(system_prompt: Option<&str>) -> Option<String> {
    system_prompt
        .filter(|prompt| !prompt.trim().is_empty())
        .map(crate::usecase::agent_session::context::stable_content_fingerprint)
}

#[allow(clippy::too_many_arguments)]
#[derive(Default)]
pub(super) struct BridgeInitOptions<'a> {
    pub(crate) system_prompt: Option<String>,
    pub(crate) selected_model: Option<&'a str>,
    pub(crate) restore_context: Option<&'a RestoreContextPayload>,
}

/// 抽象モード文字列 + backend_id を受け取り、バックエンド固有の init コマンドを構築する。
pub(super) fn build_init_cmd(
    cwd: &str,
    permission_mode: &str,
    plan_mode: bool,
    session_id: &Option<String>,
    backend_id: &str,
    options: BridgeInitOptions<'_>,
) -> Result<serde_json::Value, String> {
    let pm = crate::domain::agent_session::PermissionMode::parse(permission_mode)
        .map_err(|e| e.to_string())?;
    let mut cmd = serde_json::json!({
        "type": "init",
        "cwd": cwd,
        "sessionId": session_id,
    });
    if let Some(obj) = cmd.as_object_mut() {
        for (k, v) in bridge_permission_fields(pm, backend_id, plan_mode) {
            obj.insert(k, v);
        }
    }
    if let Some(sp) = options.system_prompt {
        cmd["systemPrompt"] = serde_json::Value::String(sp);
    }
    if let Some(model) = options.selected_model {
        cmd["model"] = serde_json::Value::String(model.to_string());
    }
    if let Some(restore_context) = options.restore_context {
        if !restore_context.prompt_prefix.trim().is_empty() {
            cmd["restoreContext"] = serde_json::to_value(restore_context)
                .map_err(|e| format!("Failed to serialize restore context: {e}"))?;
        }
    }
    Ok(cmd)
}

pub(super) fn build_message_cmd(
    prompt: &str,
    images: &[ImageAttachment],
    turn_token: Option<&str>,
) -> serde_json::Value {
    let mut cmd = if images.is_empty() {
        serde_json::json!({
            "type": "message",
            "prompt": prompt,
        })
    } else {
        let img_blocks: Vec<serde_json::Value> = images
            .iter()
            .map(|img| {
                serde_json::json!({
                    "data": img.data,
                    "mediaType": img.media_type,
                })
            })
            .collect();
        serde_json::json!({
            "type": "message",
            "prompt": prompt,
            "images": img_blocks,
        })
    };
    if let Some(turn_token) = turn_token.filter(|value| !value.is_empty()) {
        cmd["turn_token"] = serde_json::Value::String(turn_token.to_string());
    }
    cmd
}
#[cfg(test)]
pub(in crate::infrastructure::agent_session::runtime::bridge_common) mod test_support {

    use super::super::permission::*;
    use super::super::process_registry::*;
    use super::super::recovery::*;

    use super::super::session_lifecycle::*;

    use super::super::turn_event_log::*;
    use super::{CLAUDE_BACKEND_ID, CODEX_BACKEND_ID};
    use crate::domain::app_config::{AgentConfigRepository, AppConfigError};
    use crate::infrastructure::agent_session::runtime::{
        AgentBackend, AgentBackendRegistry, AgentMessage, ImageAttachment, PermissionResponse,
        SessionConfig, SessionHandle,
    };
    use crate::usecase::agent_session::event_log::{PromptInput, TurnEventLog};
    use crate::usecase::agent_session::session::{ChatSession, MessagePart};
    use async_trait::async_trait;
    use std::collections::{HashMap, VecDeque};
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::sync::Mutex;

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn approved_fix_policy_output(
        policy: &str,
        review_step: &str,
    ) -> String {
        format!(
            r#"<workflow_output type="approved-fix-policy">{{"policy":"{policy}","review_step":"{review_step}"}}</workflow_output>"#
        )
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn pending_message_for_test(
        id: &str,
        content: &str,
        created_at: f64,
    ) -> PendingMessage {
        PendingMessage {
            id: id.to_string(),
            content: content.to_string(),
            created_at,
            client_sent_at_ms: None,
            request_received_at_ms: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        }
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn test_pending_message(
        id: &str,
        content: &str,
    ) -> PendingMessage {
        pending_message_for_test(id, content, 1.0)
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn test_prompt_input(
        content: &str,
    ) -> PromptInput {
        PromptInput::from_content_images(content, std::iter::empty::<(String, String)>())
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn begin_test_turn_event_log(
        proc: &mut AgentProcess,
    ) {
        let assistant_message_id = proc
            .streaming_message_id
            .as_deref()
            .unwrap_or("m1")
            .to_string();
        begin_turn_event_log(
            proc,
            "human-1",
            test_prompt_input("prompt"),
            &assistant_message_id,
            1.0,
        );
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn assert_started_turn_prompt_matches_fallback(
        started_turn_prompt: StartedTurnPrompt,
        streaming_message_id: &str,
        fallback_prompt: &str,
        fallback_images: &[ImageAttachment],
    ) {
        let expected = started_turn_prompt_from_fallback(
            streaming_message_id,
            fallback_prompt,
            fallback_images,
        );
        assert_eq!(started_turn_prompt.message_id, expected.message_id);
        assert_eq!(started_turn_prompt.prompt, expected.prompt);
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) struct MockModelBackend {
        pub(in crate::infrastructure::agent_session::runtime::bridge_common) backend_id: String,
    }

    #[async_trait]
    impl AgentBackend for MockModelBackend {
        fn id(&self) -> &str {
            &self.backend_id
        }

        fn name(&self) -> &str {
            "Mock"
        }

        async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String> {
            Ok(SessionHandle {
                chat_session_id: config.chat_session_id,
                backend_id: self.backend_id.clone(),
            })
        }

        async fn send_message(
            &self,
            _session: &SessionHandle,
            _message: AgentMessage,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn interrupt(&self, _session: &SessionHandle) -> Result<(), String> {
            Ok(())
        }

        async fn respond_permission(
            &self,
            _session: &SessionHandle,
            _response: PermissionResponse,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestAgentConfig {
        models_by_backend: HashMap<String, Vec<String>>,
    }

    impl TestAgentConfig {
        fn with_models(claude_models: &[&str], codex_models: &[&str]) -> Self {
            let mut models_by_backend = HashMap::new();
            models_by_backend.insert(
                CLAUDE_BACKEND_ID.to_string(),
                claude_models
                    .iter()
                    .map(|model| model.to_string())
                    .collect(),
            );
            models_by_backend.insert(
                CODEX_BACKEND_ID.to_string(),
                codex_models.iter().map(|model| model.to_string()).collect(),
            );
            Self { models_by_backend }
        }
    }

    impl AgentConfigRepository for TestAgentConfig {
        fn default_agent_backend(&self) -> Result<Option<String>, AppConfigError> {
            Ok(None)
        }

        fn models_for_backend(&self, backend_id: &str) -> Result<Vec<String>, AppConfigError> {
            Ok(self
                .models_by_backend
                .get(backend_id)
                .cloned()
                .unwrap_or_default())
        }

        fn codex_cli_path(&self) -> Result<Option<String>, AppConfigError> {
            Ok(None)
        }
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) struct MockSteeringBackend {
        pub(in crate::infrastructure::agent_session::runtime::bridge_common) backend_id: String,
    }

    #[async_trait]
    impl AgentBackend for MockSteeringBackend {
        fn id(&self) -> &str {
            &self.backend_id
        }

        fn name(&self) -> &str {
            "MockSteering"
        }

        async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String> {
            Ok(SessionHandle {
                chat_session_id: config.chat_session_id,
                backend_id: self.backend_id.clone(),
            })
        }

        async fn send_message(
            &self,
            _session: &SessionHandle,
            _message: AgentMessage,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn steer_message(
            &self,
            _session: &SessionHandle,
            _message: AgentMessage,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn active_turn_steering_ready(&self, _session: &SessionHandle) -> bool {
            true
        }

        async fn interrupt(&self, _session: &SessionHandle) -> Result<(), String> {
            panic!("steering-ready backend should not be interrupted")
        }

        async fn respond_permission(
            &self,
            _session: &SessionHandle,
            _response: PermissionResponse,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Default)]
    pub(in crate::infrastructure::agent_session::runtime::bridge_common) struct RecordingWorkflowTurnCompleteGateway
    {
        pub(in crate::infrastructure::agent_session::runtime::bridge_common) session_running: bool,
        pub(in crate::infrastructure::agent_session::runtime::bridge_common) calls:
            std::sync::Mutex<Vec<&'static str>>,
    }

    #[async_trait]
    impl crate::usecase::workflow::ports::WorkflowTurnCompleteGateway
        for RecordingWorkflowTurnCompleteGateway
    {
        async fn is_session_running(&self, _chat_session_id: &str) -> bool {
            self.calls.lock().unwrap().push("is_running");
            self.session_running
        }

        async fn pickup_pending_submit_outputs(&self) {
            self.calls.lock().unwrap().push("pickup_pending");
        }

        async fn complete_turn(
            &self,
            _command: crate::usecase::workflow::ports::WorkflowTurnCompleteCommand,
        ) -> Result<(), crate::domain::workflow::WorkflowError> {
            self.calls.lock().unwrap().push("complete_turn");
            Ok(())
        }
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn expect_prepared_turn(
        input: PreparedAgentRuntimeInput,
    ) -> PreparedAgentTurn {
        match input {
            PreparedAgentRuntimeInput::Turn(turn) => turn,
            PreparedAgentRuntimeInput::Steer(_) => {
                panic!("expected a prepared turn, got active-turn steer")
            }
        }
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn expect_prepared_steer(
        input: PreparedAgentRuntimeInput,
    ) -> PreparedAgentSteer {
        match input {
            PreparedAgentRuntimeInput::Steer(steer) => steer,
            PreparedAgentRuntimeInput::Turn(_) => {
                panic!("expected active-turn steer, got a prepared turn")
            }
        }
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn chat_session_for_spawn_info(
        session_id: &str,
    ) -> ChatSession {
        ChatSession {
            id: session_id.to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::usecase::agent_session::session::SessionState::Closed,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: Some("sdk-resume-id".to_string()),
            context_carry: Some(crate::usecase::agent_session::session::ContextCarryState::Resumed),
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: Some("sonnet".to_string()),
            backend_id: Some("mock".to_string()),
            workflow_step_session: true,
            workflow_step_context: None,
            context_epoch: None,
        }
    }

    /// 呼び出し元経路で発火する 2 種類の emit を順序付きで記録するテスト用イベント。
    /// 実コードの `emit_streaming_delta` と `emit_session_state_changed` は
    /// `tauri::AppHandle` 直叩きでユニットテストから直接観測できないため、
    /// 呼び出し元ロジックをミラーした下記ヘルパで両 emit を同じ Vec に
    /// 記録し、ストリーム emit が state emit より先に来ることを確認する。
    #[derive(Debug, PartialEq)]
    pub(in crate::infrastructure::agent_session::runtime::bridge_common) enum RecordedEmit {
        StreamingFlush {
            parts_count: usize,
            tail_text: Option<String>,
        },
        StateChanged {
            phase: TurnPhase,
            exit_code: Option<i64>,
        },
    }

    /// Build a recording emit closure that pushes a `StreamingFlush` event
    /// for each delta payload it observes. Shared by the
    /// `permission_request` / `turn_complete` order tests so they exercise
    /// the same `flush_streaming_before_transition` helper the production
    /// stdout reader uses, instead of mirroring the prepare/apply sequence.
    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn recording_emit<'a>(
        events: &'a mut Vec<RecordedEmit>,
    ) -> impl FnMut(&str, u64, bool, &[MessagePart], &dyn Fn() -> Vec<MessagePart>) -> bool + 'a
    {
        |_mid, _seq, _snapshot, parts, _snapshot_parts| {
            events.push(RecordedEmit::StreamingFlush {
                parts_count: parts.len(),
                tail_text: match parts.last() {
                    Some(MessagePart::Text { content, .. })
                    | Some(MessagePart::Thinking { content, .. })
                    | Some(MessagePart::Error { content, .. }) => Some(content.clone()),
                    _ => None,
                },
            });
            true
        }
    }

    /// Drive the production `permission_request` lock-block via
    /// `run_permission_request_transition_locked` — the same helper the
    /// production stdout reader calls. This guarantees that any drift in the
    /// flush → state-mutation order would be caught here. The post-lock state
    /// emit (production: `emit_session_state_changed` outside the lock) is
    /// simulated by pushing a `StateChanged` event after the helper returns.
    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn drive_permission_request_path(
        proc: &mut AgentProcess,
        chat_session_id: &str,
        events: &mut Vec<RecordedEmit>,
    ) -> bool {
        let effect = run_permission_request_transition_locked(
            proc,
            chat_session_id,
            None,
            Instant::now(),
            recording_emit(events),
        );
        if effect.did_transition {
            events.push(RecordedEmit::StateChanged {
                phase: TurnPhase::WaitingPermission,
                exit_code: None,
            });
        }
        effect.did_transition
    }

    /// Drive the production `turn_complete` lock-block via
    /// `run_turn_complete_transition_locked` — the same helper the production
    /// stdout reader calls. State emit outside the lock is mirrored as a
    /// pushed `StateChanged` event so the ordering invariant is asserted on
    /// the event sequence.
    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn drive_turn_complete_path(
        proc: &mut AgentProcess,
        chat_session_id: &str,
        exit_code: i64,
        events: &mut Vec<RecordedEmit>,
    ) {
        let effect = run_turn_complete_transition_locked(
            proc,
            chat_session_id,
            exit_code,
            recording_emit(events),
        );
        if effect.turn_completed {
            events.push(RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(exit_code),
            });
        }
    }

    /// Drive the production `respond_agent_permission` lock-block via
    /// `apply_respond_permission_locked`. State emit outside the lock is
    /// mirrored as a pushed `StateChanged(Streaming)` event after the helper
    /// returns so the order assertion mirrors production.
    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn drive_respond_permission_path(
        proc: &mut AgentProcess,
        chat_session_id: &str,
        request_id: &str,
        behavior: &str,
        answers_value: Option<&serde_json::Value>,
        events: &mut Vec<RecordedEmit>,
    ) -> bool {
        let effect = apply_respond_permission_locked(
            proc,
            chat_session_id,
            request_id,
            behavior,
            answers_value,
            recording_emit(events),
        );
        if effect.did_transition {
            events.push(RecordedEmit::StateChanged {
                phase: TurnPhase::Streaming,
                exit_code: None,
            });
        }
        effect.did_transition
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn post_turn_tool_result_message(
        tool_use_id: &str,
        content: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "type": "user",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": content,
                        "is_error": false
                    }
                ]
            }
        })
    }

    /// Build a streaming-test process with one pending `Permission` part in
    /// the streaming buffer matching `request_id`. Used by the
    /// respond_permission tests to mimic the production state at the moment
    /// `respond_agent_permission` runs.
    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn make_process_waiting_for_permission(
        request_id: &str,
    ) -> AgentProcess {
        let mut proc = make_streaming_test_process();
        proc.turn_phase = TurnPhase::WaitingPermission;
        proc.streaming_parts.push(MessagePart::Permission {
            request: serde_json::json!({ "request_id": request_id }),
            status: "pending".to_string(),
            answers: None,
            parent_tool_use_id: None,
        });
        proc
    }

    /// Drive the production `bridge error` lock-block via
    /// `run_bridge_error_transition_locked`, then mirror the post-lock
    /// `emit_session_state_changed`.
    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn drive_bridge_error_path(
        proc: &mut AgentProcess,
        chat_session_id: &str,
        error_message: &str,
        events: &mut Vec<RecordedEmit>,
    ) -> BridgeErrorTransition {
        let msg = serde_json::json!({
            "type": "error",
            "message": error_message,
        });
        let transition =
            run_bridge_error_transition_locked(proc, chat_session_id, &msg, recording_emit(events));
        if transition.turn_complete.turn_completed {
            events.push(RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(1),
            });
        }
        transition
    }

    /// Drive the production `EOF crash` lock-block via
    /// `run_bridge_eof_crash_transition_locked`, then push a
    /// `StateChanged(Idle)` event to mirror `emit_session_state_changed`.
    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn drive_bridge_eof_crash_path(
        proc: &mut AgentProcess,
        chat_session_id: &str,
        events: &mut Vec<RecordedEmit>,
    ) {
        let transition = run_bridge_eof_crash_transition_locked(
            true,
            proc,
            chat_session_id,
            recording_emit(events),
        );
        if transition.turn_complete.turn_completed {
            events.push(RecordedEmit::StateChanged {
                phase: TurnPhase::Idle,
                exit_code: Some(-1),
            });
        }
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn make_streaming_test_process(
    ) -> AgentProcess {
        // Standalone, non-running AgentProcess used purely to exercise the
        // coalescing helpers. Stdin/child are tied to a `cat` subprocess so
        // the struct is well-formed. Must run inside a Tokio runtime.
        let mut proc = make_test_agent_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Streaming;
        proc.streaming_message_id = Some("m1".to_string());
        proc
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn workflow_state_for_runtime_test(
        session_id: &str,
    ) -> crate::domain::workflow::WorkflowStateSnapshot {
        crate::domain::workflow::WorkflowStateSnapshot {
            execution_id: "exec-runtime".to_string(),
            workflow_name: "wf".to_string(),
            state: crate::domain::workflow::WorkflowExecutionState::Running,
            current_step_index: 0,
            current_step_name: "step".to_string(),
            current_session_id: Some(session_id.to_string()),
            total_steps: 1,
            step_history: Vec::new(),
            step_execution_counts: HashMap::new(),
            workflow_definition: crate::domain::workflow::WorkflowDefinition {
                variables: Default::default(),
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                nodes: vec![],
            },
            total_token_usage: crate::domain::workflow::TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![],
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 1.0,
            updated_at: 1.0,
        }
    }

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn chat_session_for_permission_test(
        session_id: &str,
        permission: &str,
    ) -> ChatSession {
        ChatSession {
            id: session_id.to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::usecase::agent_session::session::SessionState::Active,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: permission.to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some("mock".to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        }
    }

    /// Spec issues-947: bridge stdin への書き込みを観測するために、stdout 側を pipe で
    /// 開いた `cat` を spawn し、process が複製した stdout を返す。`cat` は stdin を
    /// stdout にエコーするので、stdout が空 == bridge への書き込みなし、を観測できる。
    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn make_test_agent_process_with_stdout(
    ) -> (AgentProcess, tokio::process::ChildStdout) {
        use std::process::Stdio;
        let mut command = test_echo_command();
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn test echo process");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let proc = AgentProcess {
            stdin: Arc::new(Mutex::new(stdin)),
            backend_id: "mock".to_string(),
            state: BridgeState::Ready,
            turn_phase: TurnPhase::Idle,
            sdk_session_id: None,
            system_prompt_fingerprint: None,
            context_carry_on_ready: None,
            child,
            generation_id: 0,
            #[cfg(unix)]
            pgid: None,
            streaming_message_id: None,
            active_turn_token: None,
            turn_latency: None,
            post_turn_message_token: None,
            streaming_parts: Vec::new(),
            confirmed_stream_part_len: 0,
            turn_event_log: TurnEventLog::default(),
            last_message_id: None,
            post_turn_base_untrusted_message_id: None,
            task_id_map: HashMap::new(),
            pending_messages: VecDeque::new(),
            current_permission_mode: "edit".to_string(),
            available_models: Vec::new(),
            selected_model: None,
            stale_timeout: std::time::Duration::from_secs(180),
            last_result_token_usage: None,
            current_turn_stop_reason: None,
            latest_token_usage: None,
            pending_stream_parts: Vec::new(),
            pending_stream_part_rollbacks: Vec::new(),
            retry_stream_delta: None,
            pending_stream_bytes: 0,
            streaming_delta_seq: 0,
            streaming_delta_seq_by_message: HashMap::new(),
            pending_persisted_tool_output_resyncs: HashMap::new(),
            last_stream_emit_at: None,
            streaming_timer_active: false,
            last_progress_at: None,
            turn_phase_since: Instant::now(),
            turn_seq: 0,
            turn_watchdog_active: false,
        };
        (proc, stdout)
    }

    // --- set_agent_model_internal: 仕様の中核 Rule の回帰防止テスト ---

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn make_test_registry_with_models(
        claude_models: &[&str],
        codex_models: &[&str],
    ) -> Arc<AgentBackendRegistry> {
        let config = Arc::new(TestAgentConfig::with_models(claude_models, codex_models));
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: CLAUDE_BACKEND_ID.to_string(),
        }));
        registry.register(Arc::new(MockModelBackend {
            backend_id: CODEX_BACKEND_ID.to_string(),
        }));
        registry.set_config(config);
        Arc::new(registry)
    }

    // --- set_agent_model: 実 backend の固定リスト検証 ---

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn make_fixed_model_registry(
    ) -> Arc<AgentBackendRegistry> {
        let config = Arc::new(TestAgentConfig::default());
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(
            crate::infrastructure::agent_session::runtime::claude::ClaudeBackend::new(),
        ));
        registry.register(Arc::new(
            crate::infrastructure::agent_session::runtime::codex::CodexBackend::new(),
        ));
        registry.set_config(config);
        Arc::new(registry)
    }

    // --- get_persisted_spawn_info: 新規未起動セッションと選択解除後の区別 ---

    pub(in crate::infrastructure::agent_session::runtime::bridge_common) fn make_chat_session_for_spawn(
        agent_session_id: Option<String>,
        selected_model: Option<String>,
        backend_id: &str,
    ) -> ChatSession {
        ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::usecase::agent_session::session::SessionState::Active,
            created_at: 0.0,
            updated_at: 0.0,
            agent_session_id,
            context_carry: None,
            permission_mode: "acceptEdits".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model,
            backend_id: Some(backend_id.to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        }
    }
}
#[cfg(test)]
mod moved_tests {

    use super::super::process_registry::*;
    use super::super::session_lifecycle::take_pending_message;
    use super::super::shared::test_support::*;
    use super::super::shared::*;
    use crate::infrastructure::agent_session::runtime::runtime_coordinator::clear_pending_turn_starting;
    use crate::infrastructure::agent_session::runtime::ImageAttachment;

    use std::sync::Arc;
    use tokio::io::AsyncBufReadExt;
    use tokio::sync::Mutex;

    #[test]
    fn init_command_format() {
        // 抽象モード "edit" を Claude バックエンド向けに変換すると acceptEdits になる。
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &Some("sess-abc".to_string()),
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(cmd["type"], "init");
        assert_eq!(cmd["cwd"], "/repo");
        assert_eq!(cmd["permissionMode"], "acceptEdits");
        assert_eq!(cmd["sessionId"], "sess-abc");
    }

    #[test]
    fn init_command_without_session_id() {
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert!(cmd["sessionId"].is_null());
    }

    #[test]
    fn message_command_format() {
        let prompt = "Hello, agent!";
        let cmd = serde_json::json!({
            "type": "message",
            "prompt": prompt,
        });
        assert_eq!(cmd["type"], "message");
        assert_eq!(cmd["prompt"], "Hello, agent!");
    }

    #[test]
    fn build_message_cmd_text_only() {
        let cmd = build_message_cmd("hello", &[], None);
        assert_eq!(cmd["type"], "message");
        assert_eq!(cmd["prompt"], "hello");
        assert!(cmd.get("images").is_none());
        assert!(cmd.get("turn_token").is_none());
    }

    #[test]
    fn build_message_cmd_with_images() {
        let images = vec![ImageAttachment {
            data: "base64data".to_string(),
            media_type: "image/png".to_string(),
        }];
        let cmd = build_message_cmd("check this", &images, Some("agent-message-1"));
        assert_eq!(cmd["type"], "message");
        assert_eq!(cmd["prompt"], "check this");
        assert_eq!(cmd["turn_token"], "agent-message-1");
        let imgs = cmd["images"].as_array().unwrap();
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0]["data"], "base64data");
        assert_eq!(imgs[0]["mediaType"], "image/png");
    }

    #[tokio::test]
    async fn agent_step_runtime_busy_tracks_streaming_permission_and_pending_message() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_streaming_test_process();
        proc.state = BridgeState::Streaming;
        proc.turn_phase = TurnPhase::Idle;
        handles.lock().await.insert("step".to_string(), proc);
        assert!(is_agent_step_runtime_busy(&handles, "step").await);

        {
            let mut map = handles.lock().await;
            let proc = map.get_mut("step").unwrap();
            proc.state = BridgeState::Ready;
            proc.turn_phase = TurnPhase::WaitingPermission;
        }
        assert!(is_agent_step_runtime_busy(&handles, "step").await);

        {
            let mut map = handles.lock().await;
            let proc = map.get_mut("step").unwrap();
            proc.turn_phase = TurnPhase::Idle;
            proc.pending_messages.push_back(PendingMessage {
                id: "queued-1".to_string(),
                content: "next".to_string(),
                created_at: 1.0,
                client_sent_at_ms: None,
                request_received_at_ms: None,
                permission_mode: "edit".to_string(),
                plan_mode: false,
                images: Vec::new(),
                worktree_path: "/repo".to_string(),
                mentions: Vec::new(),
                editor_context: None,
                existing_human_message_id: None,
                existing_agent_message_id: None,
            });
        }
        assert!(is_agent_step_runtime_busy(&handles, "step").await);
        assert!(agent_session_has_pending_message(&handles, "step").await);

        {
            let mut map = handles.lock().await;
            let proc = map.get_mut("step").unwrap();
            proc.pending_messages.clear();
        }
        assert!(!is_agent_step_runtime_busy(&handles, "step").await);
        assert!(!agent_session_has_pending_message(&handles, "step").await);
    }

    #[tokio::test]
    async fn pending_turn_starting_keeps_step_runtime_busy_after_pending_is_taken() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = make_test_agent_process();
        proc.pending_messages.push_back(PendingMessage {
            id: "queued-1".to_string(),
            content: "next".to_string(),
            created_at: 1.0,
            client_sent_at_ms: None,
            request_received_at_ms: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
            editor_context: None,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        });
        handles
            .lock()
            .await
            .insert("step-pending".to_string(), proc);

        let pending = take_pending_message(&handles, "step-pending").await;

        assert!(pending.is_some());
        assert!(!agent_session_has_pending_message(&handles, "step-pending").await);
        assert!(is_agent_step_runtime_busy(&handles, "step-pending").await);

        clear_pending_turn_starting("step-pending").await;
        assert!(!is_agent_step_runtime_busy(&handles, "step-pending").await);
    }

    #[tokio::test]
    async fn node_subprocess_stdout_is_readable_as_ndjson() {
        let mock_script = r#"
                process.stdout.write(JSON.stringify({type:"system",session_id:"test-sid"}) + "\n");
                process.stdout.write(JSON.stringify({type:"stream_event",event:{type:"content_block_delta",delta:{type:"text_delta",text:"hello"}}}) + "\n");
                process.stdout.write(JSON.stringify({type:"result",subtype:"success",session_id:"test-sid"}) + "\n");
            "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(mock_script)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();

        let mut messages: Vec<serde_json::Value> = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty() {
                continue;
            }
            let msg: serde_json::Value =
                serde_json::from_str(&line).unwrap_or_else(|_| panic!("Failed to parse: {line}"));
            messages.push(msg);
        }

        let status = child.wait().await.unwrap();
        assert!(status.success(), "node process should exit 0");
        assert_eq!(messages.len(), 3, "Should have 3 messages");

        assert_eq!(
            messages[0].get("session_id").and_then(|v| v.as_str()),
            Some("test-sid")
        );

        let event = &messages[1]["event"];
        assert_eq!(event["type"].as_str(), Some("content_block_delta"));
        assert_eq!(event["delta"]["type"].as_str(), Some("text_delta"));
        assert_eq!(event["delta"]["text"].as_str(), Some("hello"));

        assert_eq!(messages[2]["type"].as_str(), Some("result"));
        assert_eq!(messages[2]["subtype"].as_str(), Some("success"));
    }

    #[tokio::test]
    async fn bridge_stdin_command_protocol_roundtrip() {
        use tokio::io::AsyncWriteExt;

        // Simulate the bridge's stdin protocol: init → message handling
        // Uses an inline script that mirrors the bridge's command parsing.
        let test_script = r#"
                let stdinBuffer = "";
                const commands = [];
                process.stdin.setEncoding("utf8");
                process.stdin.on("data", (chunk) => {
                    stdinBuffer += chunk;
                    const lines = stdinBuffer.split("\n");
                    stdinBuffer = lines.pop();
                    for (const line of lines) {
                        if (!line.trim()) continue;
                        try {
                            commands.push(JSON.parse(line));
                        } catch {}
                    }
                });
                process.stdin.on("end", () => {
                    process.stdout.write(JSON.stringify({ received: commands }) + "\n");
                });
            "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(test_script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        // Send init and message commands
        let init_cmd =
            serde_json::json!({"type": "init", "cwd": "/tmp", "permissionMode": "acceptEdits"});
        let msg_cmd = serde_json::json!({"type": "message", "prompt": "hello"});
        let close_cmd = serde_json::json!({"type": "close"});

        stdin
            .write_all(format!("{}\n{}\n{}\n", init_cmd, msg_cmd, close_cmd).as_bytes())
            .await
            .unwrap();
        drop(stdin); // Close stdin to trigger "end" event

        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();
        let line = tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
            .await
            .expect("Timeout")
            .unwrap()
            .unwrap();

        let result: serde_json::Value = serde_json::from_str(&line).unwrap();
        let received = result["received"].as_array().unwrap();
        assert_eq!(received.len(), 3);
        assert_eq!(received[0]["type"], "init");
        assert_eq!(received[1]["type"], "message");
        assert_eq!(received[1]["prompt"], "hello");
        assert_eq!(received[2]["type"], "close");

        let status = child.wait().await.unwrap();
        assert!(status.success());
    }

    #[tokio::test]
    async fn bridge_sets_can_use_tool_for_interactive_tools_in_accept_edits_mode() {
        let test_script = r#"
                const permissionMode = "acceptEdits";
                const INTERACTIVE_TOOLS = ["AskUserQuestion", "EnterPlanMode", "ExitPlanMode"];
                let canUseToolSet = false;

                if (permissionMode !== "bypassPermissions") {
                    canUseToolSet = true;
                }

                const result = {
                    permissionMode,
                    canUseToolSet,
                    interactiveToolsHandled: canUseToolSet,
                };
                process.stdout.write(JSON.stringify(result) + "\n");
            "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(test_script)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();
        let line = lines.next_line().await.unwrap().unwrap();
        let result: serde_json::Value = serde_json::from_str(&line).unwrap();

        let status = child.wait().await.unwrap();
        assert!(status.success());

        assert!(
            result["interactiveToolsHandled"].as_bool().unwrap(),
            "acceptEdits mode should set canUseTool for interactive tools. Result: {}",
            result
        );
    }

    #[tokio::test]
    async fn bridge_sets_can_use_tool_for_interactive_tools_in_plan_mode() {
        let test_script = r#"
                const permissionMode = "plan";
                const INTERACTIVE_TOOLS = ["AskUserQuestion", "EnterPlanMode", "ExitPlanMode"];
                let canUseToolSet = false;

                if (permissionMode !== "bypassPermissions") {
                    canUseToolSet = true;
                }

                const result = {
                    permissionMode,
                    canUseToolSet,
                    interactiveToolsHandled: canUseToolSet,
                };
                process.stdout.write(JSON.stringify(result) + "\n");
            "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(test_script)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();
        let line = lines.next_line().await.unwrap().unwrap();
        let result: serde_json::Value = serde_json::from_str(&line).unwrap();

        let status = child.wait().await.unwrap();
        assert!(status.success());

        assert!(
            result["interactiveToolsHandled"].as_bool().unwrap(),
            "plan mode should set canUseTool for interactive tools. Result: {}",
            result
        );
    }

    #[tokio::test]
    async fn bridge_exit_plan_mode_permission_response_roundtrip() {
        use tokio::io::AsyncWriteExt;

        let test_script = r#"
                const pendingPermissions = new Map();

                process.stdin.setEncoding('utf8');
                let buffer = '';
                process.stdin.on('data', (chunk) => {
                    buffer += chunk;
                    const lines = buffer.split('\n');
                    buffer = lines.pop();
                    for (const line of lines) {
                        if (!line.trim()) continue;
                        try {
                            const cmd = JSON.parse(line);
                            if (cmd.type === 'permission_response') {
                                const pending = pendingPermissions.get(cmd.request_id);
                                if (pending) {
                                    pendingPermissions.delete(cmd.request_id);
                                    const result = cmd.result;
                                    if (result.behavior === 'allow' && !result.updatedInput) {
                                        result.updatedInput = pending.input;
                                    }
                                    pending.resolve(result);
                                }
                            }
                        } catch {}
                    }
                });

                const requestId = 'req-exit-001';
                const toolInput = {
                    allowedPrompts: [{ tool: 'Bash', prompt: 'run tests' }],
                    pushToRemote: false,
                };

                const resultPromise = new Promise((resolve) => {
                    pendingPermissions.set(requestId, { resolve, input: toolInput });
                });

                process.stdout.write(JSON.stringify({
                    type: 'permission_request',
                    request_id: requestId,
                    tool_name: 'ExitPlanMode',
                    input: toolInput,
                    tool_use_id: 'toolu_exit_001',
                }) + '\n');

                resultPromise.then((result) => {
                    process.stdout.write(JSON.stringify({
                        type: 'canUseTool_resolved',
                        tool_name: 'ExitPlanMode',
                        result: result,
                        result_keys: Object.keys(result).sort(),
                        result_json: JSON.stringify(result),
                    }) + '\n');
                    process.exit(0);
                });
            "#;

        let mut child = tokio::process::Command::new("node")
            .arg("-e")
            .arg(test_script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn node");

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();

        let request_line = lines.next_line().await.unwrap().unwrap();
        let request: serde_json::Value = serde_json::from_str(&request_line).unwrap();
        assert_eq!(request["type"], "permission_request");
        assert_eq!(request["tool_name"], "ExitPlanMode");

        let behavior = "allow";
        let message: Option<String> = None;
        let updated_input: Option<String> = None;
        let mut result = serde_json::json!({ "behavior": behavior });
        if let Some(msg) = &message {
            result["message"] = serde_json::Value::String(msg.clone());
        }
        if let Some(input_json) = &updated_input {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(input_json) {
                result["updatedInput"] = parsed;
            }
        }
        let response = serde_json::json!({
            "type": "permission_response",
            "request_id": request["request_id"].as_str().unwrap(),
            "result": result,
        });
        let data = format!("{}\n", response);
        stdin.write_all(data.as_bytes()).await.unwrap();
        stdin.flush().await.unwrap();

        let resolved_line =
            tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
                .await
                .expect("Timeout waiting for resolved line")
                .unwrap()
                .unwrap();
        let resolved: serde_json::Value = serde_json::from_str(&resolved_line).unwrap();

        assert_eq!(resolved["type"], "canUseTool_resolved");
        assert_eq!(resolved["tool_name"], "ExitPlanMode");

        let can_use_tool_result = &resolved["result"];
        assert_eq!(
            can_use_tool_result["behavior"], "allow",
            "behavior should be 'allow'"
        );
        assert!(
            can_use_tool_result.get("updatedInput").is_some(),
            "updatedInput must be present in allow response (required by CLI Zod schema)"
        );
        assert_eq!(
            can_use_tool_result["updatedInput"]["allowedPrompts"][0]["tool"], "Bash",
            "updatedInput should contain the original tool input"
        );

        let status = child.wait().await.unwrap();
        assert!(status.success());
    }

    /// spec issues-1022 "Agent process environment contract": user system_prompt が
    /// 未指定でも、Releash CLI の long help が必ず注入されること。
    #[test]
    fn compose_system_prompt_none_returns_only_cli_help() {
        let composed = super::compose_system_prompt(None).expect("must return Some");
        let help = crate::cli::render_long_help();
        assert_eq!(composed, help);
    }

    /// user system_prompt 指定時は、user prompt の後ろに CLI help を append する。
    #[test]
    fn compose_system_prompt_some_appends_cli_help() {
        let composed = super::compose_system_prompt(Some("user prompt".to_string()))
            .expect("must return Some");
        let help = crate::cli::render_long_help();
        assert!(
            composed.starts_with("user prompt\n\n"),
            "composed must start with user prompt: {composed}"
        );
        assert!(
            composed.ends_with(help),
            "composed must end with CLI help: {composed}"
        );
    }

    /// 空文字の user system_prompt は None と同じ扱いとし、CLI help のみを返す。
    #[test]
    fn compose_system_prompt_empty_string_treated_as_none() {
        let composed = super::compose_system_prompt(Some(String::new())).expect("must return Some");
        let help = crate::cli::render_long_help();
        assert_eq!(composed, help);
    }

    #[test]
    fn build_init_cmd_without_system_prompt_for_claude() {
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(cmd["type"], "init");
        assert_eq!(cmd["cwd"], "/repo");
        assert_eq!(cmd["permissionMode"], "acceptEdits");
        assert!(cmd["sessionId"].is_null());
        assert!(cmd.get("systemPrompt").is_none());
    }

    #[test]
    fn build_init_cmd_includes_model_when_selected() {
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions {
                selected_model: Some("claude-sonnet-4-5"),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(cmd["model"], "claude-sonnet-4-5");
    }

    #[test]
    fn build_init_cmd_omits_model_when_unset() {
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();

        assert!(cmd.get("model").is_none());
    }

    #[test]
    fn build_init_cmd_with_system_prompt_for_claude() {
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &Some("prev-session".to_string()),
            CLAUDE_BACKEND_ID,
            BridgeInitOptions {
                system_prompt: Some("You are a coder.".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(cmd["type"], "init");
        assert_eq!(cmd["cwd"], "/repo");
        assert_eq!(cmd["permissionMode"], "acceptEdits");
        assert_eq!(cmd["sessionId"], "prev-session");
        assert_eq!(cmd["systemPrompt"], "You are a coder.");
    }

    #[test]
    fn build_init_cmd_includes_restore_context_for_reinjection() {
        let payload = RestoreContextPayload {
            prompt_prefix: "restored prefix".to_string(),
        };
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions {
                restore_context: Some(&payload),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(cmd["sessionId"].is_null());
        assert_eq!(cmd["restoreContext"]["promptPrefix"], "restored prefix");
        assert!(cmd["restoreContext"].get("messages").is_none());
    }

    #[test]
    fn build_init_cmd_omits_empty_restore_context_prefix() {
        let payload = RestoreContextPayload {
            prompt_prefix: "  ".to_string(),
        };
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions {
                restore_context: Some(&payload),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(cmd.get("restoreContext").is_none());
    }

    #[test]
    fn build_init_cmd_full_for_claude_emits_bypass_permissions() {
        let cmd = build_init_cmd(
            "/repo",
            "full",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(cmd["permissionMode"], "bypassPermissions");
        assert!(cmd.get("systemPrompt").is_none());
    }

    #[test]
    fn build_init_cmd_ask_for_claude_emits_default() {
        let cmd = build_init_cmd(
            "/repo",
            "ask",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(cmd["permissionMode"], "default");
    }

    #[test]
    fn build_init_cmd_for_codex_emits_sandbox_and_approval() {
        let cmd = build_init_cmd(
            "/repo",
            "edit",
            false,
            &None,
            CODEX_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(cmd["type"], "init");
        assert_eq!(cmd["sandboxMode"], "workspace-write");
        assert_eq!(cmd["approvalPolicy"], "on-request");
        // Codex 用 init には permissionMode は載らない（バックエンド固有フラグのみ）
        assert!(cmd.get("permissionMode").is_none());
    }

    #[test]
    fn build_init_cmd_for_codex_ask_and_full() {
        let ask = build_init_cmd(
            "/repo",
            "ask",
            false,
            &None,
            CODEX_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(ask["sandboxMode"], "read-only");
        assert_eq!(ask["approvalPolicy"], "on-request");
        let full = build_init_cmd(
            "/repo",
            "full",
            false,
            &None,
            CODEX_BACKEND_ID,
            BridgeInitOptions::default(),
        )
        .unwrap();
        assert_eq!(full["sandboxMode"], "danger-full-access");
        assert_eq!(full["approvalPolicy"], "never");
    }

    #[test]
    fn build_init_cmd_rejects_invalid_abstract_mode() {
        assert!(
            build_init_cmd(
                "/repo",
                "acceptEdits",
                false,
                &None,
                CLAUDE_BACKEND_ID,
                BridgeInitOptions::default()
            )
            .is_err(),
            "legacy claude flag must be rejected at the boundary"
        );
        assert!(build_init_cmd(
            "/repo",
            "plan",
            false,
            &None,
            CLAUDE_BACKEND_ID,
            BridgeInitOptions::default()
        )
        .is_err());
        assert!(build_init_cmd(
            "/repo",
            "",
            false,
            &None,
            CODEX_BACKEND_ID,
            BridgeInitOptions::default()
        )
        .is_err());
    }

    /// spec issues-1022 "Agent process environment contract":
    /// agent process には自分の chat_session_id が `RELEASH_SESSION_ID` env として渡る。
    /// pure helper 単位で固定し、spawn_bridge_process の経路改修で env 注入が漏れた場合に
    /// 即座に検知できるようにする。
    #[test]
    fn session_specific_env_overrides_includes_releash_session_id() {
        let env = session_specific_env_overrides("my-session-id", None);
        let session_id = env
            .iter()
            .find_map(|(k, v)| (*k == "RELEASH_SESSION_ID").then_some(v.as_str()));
        assert_eq!(
            session_id,
            Some("my-session-id"),
            "agent process must receive its chat_session_id as RELEASH_SESSION_ID env"
        );
    }

    /// helper は受け取った文字列を env 値としてそのまま返す。入力検証 (空文字等) は
    /// spawn_bridge_process の呼び出し側 (Tauri command / WS handler) で行われる責務であり、
    /// helper はその境界を越えて値を加工しないことを固定する。
    #[test]
    fn session_specific_env_overrides_passes_through_value_verbatim() {
        let env = session_specific_env_overrides("", None);
        let session_id = env
            .iter()
            .find_map(|(k, v)| (*k == "RELEASH_SESSION_ID").then_some(v.as_str()));
        assert_eq!(session_id, Some(""));
    }

    /// spec issues-1022 "Agent process environment contract":
    /// base_branch が解決できた場合 (= Some) は `RELEASH_BASE_BRANCH` env が渡る。
    /// reviewer agent が `git diff "$RELEASH_BASE_BRANCH"...HEAD` で今回差分のみを
    /// 対象化できるようにする境界の固定。
    #[test]
    fn session_specific_env_overrides_includes_releash_base_branch_when_resolved() {
        let env = session_specific_env_overrides("sid", Some("main"));
        let base = env
            .iter()
            .find_map(|(k, v)| (*k == "RELEASH_BASE_BRANCH").then_some(v.as_str()));
        assert_eq!(
            base,
            Some("main"),
            "agent process must receive base branch as RELEASH_BASE_BRANCH env when resolved"
        );
    }

    /// base_branch が解決できない場合 (None) は `RELEASH_BASE_BRANCH` env を立てない。
    /// 空文字を立ててしまうと `git diff "$RELEASH_BASE_BRANCH"...HEAD` が
    /// `git diff ...HEAD` になり仕様外の挙動を起こすため、env 自体を立てないことを固定する。
    #[test]
    fn session_specific_env_overrides_omits_releash_base_branch_when_none() {
        let env = session_specific_env_overrides("sid", None);
        assert!(
            !env.iter().any(|(k, _)| *k == "RELEASH_BASE_BRANCH"),
            "RELEASH_BASE_BRANCH must not be set when base branch cannot be resolved"
        );
    }
}
