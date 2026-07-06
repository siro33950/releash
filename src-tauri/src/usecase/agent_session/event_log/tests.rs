use super::finalization::{finalize_turn, latest_unresolved_permission_request};
use super::projector::{project, ProjectedStatus};
use super::*;
use crate::usecase::agent_session::event_log::AgentTurnFailureSignal;
use crate::usecase::agent_session::session::{
    AttachmentRef, MessageMention, MessagePart, PermissionPartStatus, PermissionRequestKindMsg,
    PermissionRequestMsg, SessionState, SystemNotificationType, TodoListItem, ToolOutputRef,
    ToolOutputSummary,
};
use crate::usecase::agent_session::status::TurnPhase;

fn start_event() -> AgentSessionEvent {
    turn_started_event(1)
}

fn turn_started_event(turn_id: u64) -> AgentSessionEvent {
    AgentSessionEvent::TurnStarted {
        turn_id,
        message_id: format!("human-{turn_id}"),
        assistant_message_id: Some(format!("agent-{turn_id}")),
        prompt: PromptInput {
            content: "please read".to_string(),
            mentions: Vec::new(),
            attachment_refs: Vec::new(),
            parts: Vec::new(),
        },
        at: 10.0,
    }
}

fn permission_request_fixture() -> PermissionRequestMsg {
    PermissionRequestMsg {
        id: "req-1".to_string(),
        tool_use_id: Some("tool-1".to_string()),
        tool_name: "Edit".to_string(),
        kind: PermissionRequestKindMsg::ToolApproval,
        input: Some(serde_json::json!({})),
        plan: None,
        allowed_prompts: Vec::new(),
        questions: Vec::new(),
        title: None,
        display_name: None,
        description: None,
        decision_reason: None,
    }
}

#[test]
fn workflow_input_preserves_model_refusal_signal_from_completed_stop_reason() {
    let events = vec![
        start_event(),
        AgentSessionEvent::TextRecorded {
            turn_id: 1,
            message_id: "agent-1".to_string(),
            content: "I cannot comply.".to_string(),
            parent_tool_use_id: None,
        },
        AgentSessionEvent::TurnCompleted {
            turn_id: 1,
            exit_code: 0,
            stop_reason: Some(TurnStopReason::Refusal),
            token_usage: None,
        },
    ];

    let read_model = project(&events);

    assert_eq!(
        read_model
            .workflow_turn_complete
            .as_ref()
            .and_then(|input| input.failure_signal),
        Some(AgentTurnFailureSignal::ModelRefusal)
    );
}

#[test]
fn workflow_input_does_not_scan_policy_text_for_model_refusal() {
    let events = vec![
        start_event(),
        AgentSessionEvent::TextRecorded {
            turn_id: 1,
            message_id: "agent-1".to_string(),
            content: "Codex text can mention model_refusal, provider policy, and content policy."
                .to_string(),
            parent_tool_use_id: None,
        },
        AgentSessionEvent::TurnCompleted {
            turn_id: 1,
            exit_code: 0,
            stop_reason: None,
            token_usage: None,
        },
    ];

    let read_model = project(&events);

    assert_eq!(
        read_model
            .workflow_turn_complete
            .as_ref()
            .and_then(|input| input.failure_signal),
        None
    );
}

#[test]
fn append_events_project_message_page_and_workflow_input() {
    let events = vec![
        start_event(),
        AgentSessionEvent::ToolCallStarted {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            tool: "Read".to_string(),
            input: serde_json::json!({"file_path": "src/lib.rs"}),
            parent_tool_use_id: None,
        },
        AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: "contents".to_string(),
            content_ref: None,
            summary: None,
        },
        AgentSessionEvent::TextRecorded {
            turn_id: 1,
            message_id: "agent-1".to_string(),
            content: "done".to_string(),
            parent_tool_use_id: None,
        },
        AgentSessionEvent::TurnCompleted {
            turn_id: 1,
            exit_code: 0,
            stop_reason: None,
            token_usage: Some(TurnTokenUsage {
                input_tokens: 3,
                output_tokens: 5,
            }),
        },
    ];

    let read_model = project(&events);

    assert_eq!(read_model.messages.len(), 2);
    assert_eq!(read_model.messages[0].content, "please read");
    let agent_parts = read_model.agent_parts_for_message("agent-1");
    assert!(agent_parts.iter().any(|part| matches!(
        part,
        MessagePart::ToolResult {
            content,
            tool_use_id: Some(id),
            is_error: false,
            ..
        } if id == "tool-1" && content == "contents"
    )));
    assert_eq!(
        read_model.status,
        ProjectedStatus {
            session_state: SessionState::Done,
            turn_phase: TurnPhase::Idle,
        }
    );
    assert_eq!(
        read_model.workflow_turn_complete,
        Some(WorkflowTurnCompleteInput {
            turn_id: 1,
            exit_code: 0,
            final_text_parts: vec!["done".to_string()],
            failure_signal: None,
            token_usage: Some(TurnTokenUsage {
                input_tokens: 3,
                output_tokens: 5,
            }),
            interrupted: false,
        })
    );
}

#[test]
fn projection_is_deterministic_and_reconnect_does_not_duplicate() {
    let events = vec![
        start_event(),
        AgentSessionEvent::ToolCallStarted {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            tool: "Read".to_string(),
            input: serde_json::json!({}),
            parent_tool_use_id: None,
        },
        AgentSessionEvent::ToolCallStarted {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            tool: "Read".to_string(),
            input: serde_json::json!({"path": "updated"}),
            parent_tool_use_id: None,
        },
    ];

    let first = project(&events).agent_parts_for_message("agent-1");
    let second = project(&events).agent_parts_for_message("agent-1");

    assert_eq!(first, second);
    assert_eq!(
        first
            .iter()
            .filter(|part| matches!(part, MessagePart::ToolUse { id, .. } if id == "tool-1"))
            .count(),
        1
    );
}

#[test]
fn repeated_tool_use_appends_retry_event_and_projector_recovers_history() {
    let mut log = TurnEventLog::default();
    log.begin_turn(
        1,
        "human-1".to_string(),
        "agent-1".to_string(),
        PromptInput::default(),
        10.0,
    );
    log.append_part_events(
        1,
        "agent-1",
        &[MessagePart::ToolUse {
            tool: "Read".to_string(),
            input: serde_json::json!({"file_path": "src/lib.rs"}),
            id: "tool-1".to_string(),
            parent_tool_use_id: None,
        }],
        PartEventMode::DurableOnly,
    );
    log.append_part_events(
        1,
        "agent-1",
        &[MessagePart::ToolUse {
            tool: "Read".to_string(),
            input: serde_json::json!({"file_path": "src/main.rs"}),
            id: "tool-1".to_string(),
            parent_tool_use_id: None,
        }],
        PartEventMode::DurableOnly,
    );

    let read_model = log.project();

    assert_eq!(read_model.tool_retries.len(), 1);
    assert_eq!(read_model.tool_retries[0].turn_id, 1);
    assert_eq!(read_model.tool_retries[0].tool_use_id, "tool-1");
    assert_eq!(read_model.tool_retries[0].attempt, 2);
    assert!(read_model
        .agent_parts_for_message("agent-1")
        .iter()
        .any(|part| matches!(
            part,
            MessagePart::ToolUse { id, input, .. }
                if id == "tool-1" && input == &serde_json::json!({"file_path": "src/main.rs"})
        )));
}

#[test]
fn background_events_project_to_explicit_message_target_not_last_turn() {
    let mut second_turn = start_event();
    if let AgentSessionEvent::TurnStarted {
        turn_id,
        message_id,
        assistant_message_id,
        prompt,
        at,
    } = &mut second_turn
    {
        *turn_id = 2;
        *message_id = "human-2".to_string();
        *assistant_message_id = Some("agent-2".to_string());
        prompt.content = "second".to_string();
        *at = 20.0;
    }
    let events = vec![
        start_event(),
        second_turn,
        AgentSessionEvent::TaskStatusChanged {
            turn_id: 1,
            message_id: "agent-1".to_string(),
            task_tool_use_id: "task-1".to_string(),
            status: "completed".to_string(),
            description: None,
            summary: Some("done".to_string()),
        },
        AgentSessionEvent::TodoListSnapshotRecorded {
            turn_id: 1,
            message_id: "agent-1".to_string(),
            items: vec![TodoListItem {
                text: "ship".to_string(),
                completed: true,
            }],
        },
        AgentSessionEvent::SystemNotificationRecorded {
            turn_id: 1,
            message_id: "agent-1".to_string(),
            notification_type: SystemNotificationType::Compaction,
            status: "completed".to_string(),
            label: "Compacted".to_string(),
            detail: Some("done".to_string()),
            hook_id: None,
        },
    ];

    let read_model = project(&events);
    let first_parts = read_model.agent_parts_for_message("agent-1");
    let second_parts = read_model.agent_parts_for_message("agent-2");

    assert!(first_parts.iter().any(|part| matches!(
        part,
        MessagePart::TaskStatus {
            task_tool_use_id,
            status,
            summary,
            ..
        } if task_tool_use_id == "task-1"
            && status == "completed"
            && summary.as_deref() == Some("done")
    )));
    assert!(first_parts
        .iter()
        .any(|part| matches!(part, MessagePart::TodoListSnapshot { .. })));
    assert!(first_parts.iter().any(|part| matches!(
        part,
        MessagePart::SystemNotification {
            notification_type: SystemNotificationType::Compaction,
            status,
            ..
        } if status == "completed"
    )));
    assert!(second_parts.is_empty());
}

#[test]
fn later_tool_success_replaces_interrupted_failure_content() {
    let events = vec![
        start_event(),
        AgentSessionEvent::ToolCallStarted {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            tool: "Read".to_string(),
            input: serde_json::json!({}),
            parent_tool_use_id: None,
        },
        AgentSessionEvent::ToolCallFailed {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: "crash により中断".to_string(),
            content_ref: None,
            summary: None,
        },
        AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: "late result".to_string(),
            content_ref: None,
            summary: None,
        },
    ];

    let parts = project(&events).agent_parts_for_message("agent-1");

    assert!(parts.iter().any(|part| matches!(
        part,
        MessagePart::ToolResult {
            content,
            is_error: false,
            tool_use_id: Some(id),
            ..
        } if id == "tool-1" && content == "late result"
    )));
}

#[test]
fn later_tool_success_replaces_content_when_new_content_contains_existing() {
    let events = vec![
        start_event(),
        AgentSessionEvent::ToolCallStarted {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            tool: "Read".to_string(),
            input: serde_json::json!({}),
            parent_tool_use_id: None,
        },
        AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: "partial".to_string(),
            content_ref: None,
            summary: None,
        },
        AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: "partial result".to_string(),
            content_ref: None,
            summary: None,
        },
    ];

    let parts = project(&events).agent_parts_for_message("agent-1");

    assert!(parts.iter().any(|part| matches!(
        part,
        MessagePart::ToolResult {
            content,
            is_error: false,
            tool_use_id: Some(id),
            ..
        } if id == "tool-1" && content == "partial result"
    )));
}

#[test]
fn later_tool_success_appends_content_when_new_content_does_not_contain_existing() {
    let events = vec![
        start_event(),
        AgentSessionEvent::ToolCallStarted {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            tool: "Read".to_string(),
            input: serde_json::json!({}),
            parent_tool_use_id: None,
        },
        AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: "first".to_string(),
            content_ref: None,
            summary: None,
        },
        AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: " second".to_string(),
            content_ref: None,
            summary: None,
        },
    ];

    let parts = project(&events).agent_parts_for_message("agent-1");

    assert!(parts.iter().any(|part| matches!(
        part,
        MessagePart::ToolResult {
            content,
            is_error: false,
            tool_use_id: Some(id),
            ..
        } if id == "tool-1" && content == "first second"
    )));
}

#[test]
fn tool_result_restores_parent_from_tool_call_started() {
    let events = vec![
        start_event(),
        AgentSessionEvent::ToolCallStarted {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            tool: "Read".to_string(),
            input: serde_json::json!({}),
            parent_tool_use_id: Some("parent-1".to_string()),
        },
        AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: "done".to_string(),
            content_ref: None,
            summary: None,
        },
    ];

    let parts = project(&events).agent_parts_for_message("agent-1");

    assert!(parts.iter().any(|part| matches!(
        part,
        MessagePart::ToolResult {
            content,
            is_error: false,
            tool_use_id: Some(id),
            parent_tool_use_id: Some(parent),
            ..
        } if id == "tool-1" && parent == "parent-1" && content == "done"
    )));
}

#[test]
fn tool_result_without_tool_use_id_round_trips_through_part_events() {
    let mut log = TurnEventLog::default();
    log.begin_turn(
        1,
        "human-1".to_string(),
        "agent-1".to_string(),
        PromptInput::default(),
        10.0,
    );
    let part = MessagePart::ToolResult {
        content: "standalone".to_string(),
        is_error: true,
        tool_use_id: None,
        parent_tool_use_id: Some("parent-1".to_string()),
        content_ref: None,
        summary: None,
    };

    let appended = log.append_part_events(
        1,
        "agent-1",
        std::slice::from_ref(&part),
        PartEventMode::DurableOnly,
    );

    assert_eq!(appended, 1);
    assert_eq!(log.project().agent_parts_for_message("agent-1"), vec![part]);
}

#[test]
fn externalized_tool_result_round_trips_through_part_events() {
    let mut log = TurnEventLog::default();
    log.begin_turn(
        1,
        "human-1".to_string(),
        "agent-1".to_string(),
        PromptInput::default(),
        10.0,
    );
    let content_ref = ToolOutputRef {
        id: "f".repeat(64),
        byte_size: 4096,
    };
    let summary = ToolOutputSummary {
        line_count: 200,
        byte_size: 4096,
        is_error: false,
        truncated: true,
    };
    let tool_use = MessagePart::ToolUse {
        tool: "Bash".to_string(),
        input: serde_json::json!({"command": "pnpm test"}),
        id: "tool-1".to_string(),
        parent_tool_use_id: Some("parent-1".to_string()),
    };
    let part = MessagePart::ToolResult {
        content: "preview".to_string(),
        is_error: false,
        tool_use_id: Some("tool-1".to_string()),
        parent_tool_use_id: Some("parent-1".to_string()),
        content_ref: Some(content_ref),
        summary: Some(summary),
    };

    let appended = log.append_part_events(
        1,
        "agent-1",
        &[tool_use.clone(), part.clone()],
        PartEventMode::DurableOnly,
    );

    assert_eq!(appended, 2);
    assert_eq!(
        log.project().agent_parts_for_message("agent-1"),
        vec![tool_use, part]
    );
}

#[test]
fn externalized_tool_result_keeps_ref_when_later_inline_delta_arrives() {
    let content_ref = ToolOutputRef {
        id: "a".repeat(64),
        byte_size: 8192,
    };
    let summary = ToolOutputSummary {
        line_count: 300,
        byte_size: 8192,
        is_error: false,
        truncated: true,
    };
    let tool_use = MessagePart::ToolUse {
        tool: "Bash".to_string(),
        input: serde_json::json!({"command": "cargo test"}),
        id: "tool-1".to_string(),
        parent_tool_use_id: Some("parent-1".to_string()),
    };
    let events = vec![
        start_event(),
        AgentSessionEvent::ToolCallStarted {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            tool: "Bash".to_string(),
            input: serde_json::json!({"command": "cargo test"}),
            parent_tool_use_id: Some("parent-1".to_string()),
        },
        AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: "preview".to_string(),
            content_ref: Some(content_ref.clone()),
            summary: Some(summary.clone()),
        },
        AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: "\nlate chunk".to_string(),
            content_ref: None,
            summary: None,
        },
    ];

    let parts = project(&events).agent_parts_for_message("agent-1");

    assert_eq!(
        parts,
        vec![
            tool_use,
            MessagePart::ToolResult {
                content: "preview".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: Some("parent-1".to_string()),
                content_ref: Some(content_ref),
                summary: Some(summary),
            },
            MessagePart::ToolResult {
                content: "\nlate chunk".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: Some("parent-1".to_string()),
                content_ref: None,
                summary: None,
            },
        ]
    );
}

#[test]
fn externalized_tool_result_ignores_empty_later_inline_delta() {
    let content_ref = ToolOutputRef {
        id: "a".repeat(64),
        byte_size: 8192,
    };
    let summary = ToolOutputSummary {
        line_count: 300,
        byte_size: 8192,
        is_error: false,
        truncated: true,
    };
    let events = vec![
        start_event(),
        AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: "preview".to_string(),
            content_ref: Some(content_ref.clone()),
            summary: Some(summary.clone()),
        },
        AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: String::new(),
            content_ref: None,
            summary: None,
        },
    ];

    let parts = project(&events).agent_parts_for_message("agent-1");

    assert_eq!(
        parts,
        vec![MessagePart::ToolResult {
            content: "preview".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
            content_ref: Some(content_ref),
            summary: Some(summary),
        }]
    );
}

#[test]
fn inline_tool_result_is_replaced_by_later_externalized_preview_ref() {
    let sentinel = "INLINE_TO_REF_SECRET_TAIL";
    let full_output = format!("full output\n{sentinel}");
    let content_ref = ToolOutputRef {
        id: "b".repeat(64),
        byte_size: full_output.len() as u64,
    };
    let summary = ToolOutputSummary {
        line_count: 2,
        byte_size: full_output.len() as u64,
        is_error: false,
        truncated: true,
    };
    let events = vec![
        start_event(),
        AgentSessionEvent::ToolCallStarted {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            tool: "Bash".to_string(),
            input: serde_json::json!({"command": "cargo test"}),
            parent_tool_use_id: Some("parent-1".to_string()),
        },
        AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: full_output.clone(),
            content_ref: None,
            summary: None,
        },
        AgentSessionEvent::ToolCallSucceeded {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            content: "preview only".to_string(),
            content_ref: Some(content_ref.clone()),
            summary: Some(summary.clone()),
        },
    ];

    let parts = project(&events).agent_parts_for_message("agent-1");

    assert_eq!(
        parts,
        vec![
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({"command": "cargo test"}),
                id: "tool-1".to_string(),
                parent_tool_use_id: Some("parent-1".to_string()),
            },
            MessagePart::ToolResult {
                content: "preview only".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: Some("parent-1".to_string()),
                content_ref: Some(content_ref),
                summary: Some(summary),
            },
        ]
    );
    assert!(!serde_json::to_string(&parts).unwrap().contains(sentinel));
}

#[test]
fn orphan_turn_events_are_skipped_without_synthetic_messages() {
    let read_model = project(&[AgentSessionEvent::ToolCallSucceeded {
        turn_id: 99,
        tool_use_id: "tool-1".to_string(),
        content: "orphan".to_string(),
        content_ref: None,
        summary: None,
    }]);

    assert!(read_model.messages.is_empty());
    assert_eq!(
        read_model.status,
        ProjectedStatus {
            session_state: SessionState::Idle,
            turn_phase: TurnPhase::Idle,
        }
    );
    assert!(read_model.workflow_turn_complete.is_none());
}

#[test]
fn live_only_delta_is_not_part_of_durable_event_log() {
    let events = vec![
        start_event(),
        AgentSessionEvent::TextRecorded {
            turn_id: 1,
            message_id: "agent-1".to_string(),
            content: "final".to_string(),
            parent_tool_use_id: None,
        },
    ];

    let parts = project(&events).agent_parts_for_message("agent-1");

    assert_eq!(
        parts,
        vec![MessagePart::Text {
            content: "final".to_string(),
            parent_tool_use_id: None,
        }]
    );
}

#[test]
fn turn_started_projects_prompt_mentions_and_parts() {
    let attachment = AttachmentRef {
        id: "att-1".to_string(),
        media_type: "image/png".to_string(),
        byte_size: 42,
    };
    let prompt_parts = vec![
        MessagePart::Text {
            content: "inspect this".to_string(),
            parent_tool_use_id: None,
        },
        MessagePart::ImageRef {
            attachment: attachment.clone(),
        },
    ];
    let events = vec![AgentSessionEvent::TurnStarted {
        turn_id: 1,
        message_id: "human-1".to_string(),
        assistant_message_id: Some("agent-1".to_string()),
        prompt: PromptInput {
            content: "inspect this".to_string(),
            mentions: vec![MessageMention {
                file_path: "src/main.rs".to_string(),
                start_line: Some(3),
                end_line: Some(5),
            }],
            attachment_refs: vec![attachment],
            parts: prompt_parts.clone(),
        },
        at: 10.0,
    }];

    let read_model = project(&events);
    let human = read_model
        .messages
        .iter()
        .find(|message| message.id == "human-1")
        .expect("human message");

    assert_eq!(human.content, "inspect this");
    assert_eq!(
        human.mentions,
        Some(vec![MessageMention {
            file_path: "src/main.rs".to_string(),
            start_line: Some(3),
            end_line: Some(5),
        }])
    );
    assert_eq!(human.parts, Some(prompt_parts));
}

#[test]
fn turn_started_projects_attachment_refs_as_prompt_parts() {
    let attachment = AttachmentRef {
        id: "att-1".to_string(),
        media_type: "image/png".to_string(),
        byte_size: 42,
    };
    let read_model = project(&[AgentSessionEvent::TurnStarted {
        turn_id: 1,
        message_id: "human-1".to_string(),
        assistant_message_id: Some("agent-1".to_string()),
        prompt: PromptInput {
            content: "inspect this".to_string(),
            mentions: Vec::new(),
            attachment_refs: vec![attachment.clone()],
            parts: Vec::new(),
        },
        at: 10.0,
    }]);

    let human = read_model
        .messages
        .iter()
        .find(|message| message.id == "human-1")
        .expect("human message");

    assert_eq!(
        human.parts,
        Some(vec![
            MessagePart::Text {
                content: "inspect this".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ImageRef { attachment },
        ])
    );
}

#[test]
fn status_projection_covers_runtime_transition_states() {
    let streaming = project(&[start_event()]);
    assert_eq!(streaming.status.session_state, SessionState::Active);
    assert_eq!(streaming.status.turn_phase, TurnPhase::Streaming);

    let waiting = project(&[
        start_event(),
        AgentSessionEvent::PermissionRequested {
            turn_id: 1,
            tool_use_id: Some("tool-1".to_string()),
            request: permission_request_fixture(),
        },
    ]);
    assert_eq!(waiting.status.session_state, SessionState::Active);
    assert_eq!(waiting.status.turn_phase, TurnPhase::WaitingPermission);

    let closed = project(&[start_event(), AgentSessionEvent::SessionClosed { at: 11.0 }]);
    assert_eq!(closed.status.session_state, SessionState::Closed);
    assert_eq!(closed.status.turn_phase, TurnPhase::Idle);
}

#[test]
fn terminal_status_projection_marks_nonzero_completed_as_error() {
    let read_model = project(&[
        start_event(),
        AgentSessionEvent::TurnCompleted {
            turn_id: 1,
            exit_code: 2,
            stop_reason: None,
            token_usage: None,
        },
    ]);

    assert_eq!(read_model.status.session_state, SessionState::Error);
    assert_eq!(read_model.status.turn_phase, TurnPhase::Idle);
}

#[test]
fn terminal_status_projection_marks_timeout_and_crash_as_error() {
    for reason in [InterruptReason::Timeout, InterruptReason::Crash] {
        let read_model = project(&[
            start_event(),
            AgentSessionEvent::TurnInterrupted {
                turn_id: 1,
                reason,
                exit_code: 1,
                error: None,
            },
        ]);

        assert_eq!(read_model.status.session_state, SessionState::Error);
        assert_eq!(read_model.status.turn_phase, TurnPhase::Idle);
    }
}

#[test]
fn terminal_status_projection_marks_abort_as_idle() {
    let read_model = project(&[
        start_event(),
        AgentSessionEvent::TurnInterrupted {
            turn_id: 1,
            reason: InterruptReason::Abort,
            exit_code: 1,
            error: None,
        },
    ]);

    assert_eq!(read_model.status.session_state, SessionState::Idle);
    assert_eq!(read_model.status.turn_phase, TurnPhase::Idle);
}

#[test]
fn finalization_closes_tools_permissions_and_turn() {
    for reason in [
        InterruptReason::Abort,
        InterruptReason::Timeout,
        InterruptReason::Crash,
    ] {
        let mut events = vec![
            start_event(),
            AgentSessionEvent::ToolCallStarted {
                turn_id: 1,
                tool_use_id: "tool-1".to_string(),
                tool: "Edit".to_string(),
                input: serde_json::json!({}),
                parent_tool_use_id: None,
            },
            AgentSessionEvent::PermissionRequested {
                turn_id: 1,
                tool_use_id: Some("tool-1".to_string()),
                request: permission_request_fixture(),
            },
        ];

        finalize_turn(
            &mut events,
            1,
            reason,
            Some("bridge failed".to_string()),
            -1,
        );
        let read_model = project(&events);
        let agent_parts = read_model.agent_parts_for_message("agent-1");

        assert!(agent_parts.iter().any(|part| matches!(
            part,
            MessagePart::ToolResult {
                tool_use_id: Some(id),
                is_error: true,
                content,
                ..
            } if id == "tool-1" && content == "bridge failed により中断"
        )));
        assert!(agent_parts.iter().any(|part| matches!(
            part,
            MessagePart::Permission { status, .. } if *status == PermissionPartStatus::Cancelled
        )));
        assert_eq!(read_model.status.turn_phase, TurnPhase::Idle);
        assert_eq!(
            read_model.workflow_turn_complete,
            Some(WorkflowTurnCompleteInput {
                turn_id: 1,
                exit_code: -1,
                final_text_parts: Vec::new(),
                failure_signal: None,
                token_usage: None,
                interrupted: true,
            })
        );
    }
}

#[test]
fn latest_unresolved_permission_request_uses_finalization_key_matching() {
    let mut second = permission_request_fixture();
    second.id = "req-2".to_string();
    second.tool_use_id = Some("tool-2".to_string());
    let events = vec![
        start_event(),
        AgentSessionEvent::PermissionRequested {
            turn_id: 1,
            tool_use_id: Some("tool-1".to_string()),
            request: permission_request_fixture(),
        },
        AgentSessionEvent::PermissionResolved {
            turn_id: 1,
            tool_use_id: None,
            request_id: Some("req-1".to_string()),
            decision: PermissionDecision::Allowed,
            answers: None,
        },
        AgentSessionEvent::PermissionRequested {
            turn_id: 1,
            tool_use_id: Some("tool-2".to_string()),
            request: second,
        },
    ];

    let pending = latest_unresolved_permission_request(&events).expect("pending permission");

    assert_eq!(pending.turn_id, 1);
    assert_eq!(pending.request.id, "req-2");
}

#[test]
fn latest_unresolved_permission_request_ignores_terminal_turns() {
    let events = vec![
        start_event(),
        AgentSessionEvent::PermissionRequested {
            turn_id: 1,
            tool_use_id: Some("tool-1".to_string()),
            request: permission_request_fixture(),
        },
        AgentSessionEvent::TurnInterrupted {
            turn_id: 1,
            reason: InterruptReason::Abort,
            exit_code: 1,
            error: None,
        },
    ];

    assert!(latest_unresolved_permission_request(&events).is_none());
}

#[test]
fn latest_unresolved_permission_request_ignores_previous_turn_after_new_turn_started() {
    let events = vec![
        start_event(),
        AgentSessionEvent::PermissionRequested {
            turn_id: 1,
            tool_use_id: Some("tool-1".to_string()),
            request: permission_request_fixture(),
        },
        turn_started_event(2),
    ];

    assert!(latest_unresolved_permission_request(&events).is_none());
}

#[test]
fn latest_unresolved_permission_request_returns_latest_turn_request_only() {
    let mut second = permission_request_fixture();
    second.id = "req-2".to_string();
    second.tool_use_id = Some("tool-2".to_string());
    let events = vec![
        start_event(),
        AgentSessionEvent::PermissionRequested {
            turn_id: 1,
            tool_use_id: Some("tool-1".to_string()),
            request: permission_request_fixture(),
        },
        turn_started_event(2),
        AgentSessionEvent::PermissionRequested {
            turn_id: 2,
            tool_use_id: Some("tool-2".to_string()),
            request: second,
        },
    ];

    let pending = latest_unresolved_permission_request(&events).expect("pending permission");

    assert_eq!(pending.turn_id, 2);
    assert_eq!(pending.request.id, "req-2");
}

#[test]
fn finalization_uses_reason_label_when_error_is_none() {
    for (reason, expected) in [
        (InterruptReason::Abort, "abort により中断"),
        (InterruptReason::Timeout, "timeout により中断"),
        (InterruptReason::Crash, "crash により中断"),
    ] {
        let mut events = vec![
            start_event(),
            AgentSessionEvent::ToolCallStarted {
                turn_id: 1,
                tool_use_id: "tool-1".to_string(),
                tool: "Edit".to_string(),
                input: serde_json::json!({}),
                parent_tool_use_id: None,
            },
        ];

        finalize_turn(&mut events, 1, reason, None, -1);
        let agent_parts = project(&events).agent_parts_for_message("agent-1");

        assert!(agent_parts.iter().any(|part| matches!(
            part,
            MessagePart::ToolResult {
                tool_use_id: Some(id),
                is_error: true,
                content,
                ..
            } if id == "tool-1" && content == expected
        )));
    }
}

#[test]
fn finalization_keeps_existing_interrupted_detail_without_double_suffix() {
    for detail in ["already interrupted", "すでに中断しました"] {
        let mut events = vec![
            start_event(),
            AgentSessionEvent::ToolCallStarted {
                turn_id: 1,
                tool_use_id: "tool-1".to_string(),
                tool: "Edit".to_string(),
                input: serde_json::json!({}),
                parent_tool_use_id: None,
            },
        ];

        finalize_turn(
            &mut events,
            1,
            InterruptReason::Crash,
            Some(detail.to_string()),
            -1,
        );
        let agent_parts = project(&events).agent_parts_for_message("agent-1");

        assert!(agent_parts.iter().any(|part| matches!(
            part,
            MessagePart::ToolResult {
                tool_use_id: Some(id),
                is_error: true,
                content,
                ..
            } if id == "tool-1" && content == detail
        )));
    }
}

#[test]
fn finalization_is_idempotent() {
    let mut events = vec![
        start_event(),
        AgentSessionEvent::ToolCallStarted {
            turn_id: 1,
            tool_use_id: "tool-1".to_string(),
            tool: "Edit".to_string(),
            input: serde_json::json!({}),
            parent_tool_use_id: None,
        },
    ];
    finalize_turn(&mut events, 1, InterruptReason::Abort, None, 1);
    let once = events.clone();
    finalize_turn(&mut events, 1, InterruptReason::Abort, None, 1);

    assert_eq!(events, once);
}
