use std::collections::HashMap;

use super::events::{
    AgentSessionEvent, BackendSessionRecoveryReason, InterruptReason, PromptInput, TurnId,
    TurnStopReason, TurnTokenUsage,
};
use super::finalization::has_unresolved_permissions;
use super::part_events::{permission_request_id, permission_tool_use_id};
use crate::domain::agent_session::entities::ToolResultUpdate;
use crate::usecase::agent_session::session::{
    apply_tool_result_update, parts_to_legacy, ChatMessage, MessagePart, MessageRole,
    PermissionPartStatus, PermissionRequestMsg, SessionState, SystemNotificationType, TodoListItem,
    ToolOutputRef, ToolOutputSummary,
};
use crate::usecase::agent_session::status::TurnPhase;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct ProjectedStatus {
    pub session_state: SessionState,
    pub turn_phase: TurnPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct WorkflowTurnCompleteInput {
    pub turn_id: TurnId,
    pub exit_code: i64,
    pub final_text_parts: Vec<String>,
    pub failure_signal: Option<AgentTurnFailureSignal>,
    pub token_usage: Option<TurnTokenUsage>,
    pub interrupted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub enum AgentTurnFailureSignal {
    ModelRefusal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct ToolRetryProjection {
    pub turn_id: TurnId,
    pub tool_use_id: String,
    pub attempt: u32,
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct SessionReadModel {
    pub messages: Vec<ChatMessage>,
    pub status: ProjectedStatus,
    pub workflow_turn_complete: Option<WorkflowTurnCompleteInput>,
    #[allow(dead_code)]
    // issues-1301 B-5/E-1: retry projection is retained for tool retry surface while runtime events are fully migrated.
    pub tool_retries: Vec<ToolRetryProjection>,
    #[cfg_attr(test, serde(skip_serializing_if = "Option::is_none"))]
    pub backend_recovery: Option<BackendSessionRecoveryProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub enum BackendSessionRecoveryProjection {
    Recovering {
        recovery_id: String,
        old_provider_session_generation: u64,
        reason: BackendSessionRecoveryReason,
    },
    ReconciliationRequired {
        recovery_id: String,
        error: String,
    },
}

impl SessionReadModel {
    pub fn agent_parts_for_message(&self, message_id: &str) -> Vec<MessagePart> {
        self.messages
            .iter()
            .find(|message| message.id == message_id && message.role == MessageRole::Agent)
            .and_then(|message| message.parts.clone())
            .unwrap_or_default()
    }
}

pub fn project(events: &[AgentSessionEvent]) -> SessionReadModel {
    let mut turns: Vec<TurnProjection> = Vec::new();
    let mut turn_index: HashMap<TurnId, usize> = HashMap::new();
    let mut terminal_by_turn: HashMap<TurnId, TerminalEvent> = HashMap::new();
    let mut tool_retries = Vec::new();
    let mut session_closed = false;
    let mut backend_recovery = None;

    for event in events {
        match event {
            AgentSessionEvent::BackendSessionRecoveryStarted {
                recovery_id,
                old_provider_session_generation,
                reason,
                ..
            } => {
                backend_recovery = Some(BackendSessionRecoveryProjection::Recovering {
                    recovery_id: recovery_id.clone(),
                    old_provider_session_generation: *old_provider_session_generation,
                    reason: *reason,
                });
            }
            AgentSessionEvent::SessionConfigurationReactivated { .. }
            | AgentSessionEvent::SessionGoalReactivated { .. } => {}
            AgentSessionEvent::BackendSessionRecoveryCompleted { recovery_id, .. } => {
                if matches!(
                    &backend_recovery,
                    Some(BackendSessionRecoveryProjection::Recovering {
                        recovery_id: active,
                        ..
                    }) if active == recovery_id
                ) {
                    backend_recovery = None;
                }
            }
            AgentSessionEvent::BackendSessionRecoveryFailed {
                recovery_id, error, ..
            } => {
                backend_recovery = Some(BackendSessionRecoveryProjection::ReconciliationRequired {
                    recovery_id: recovery_id.clone(),
                    error: error.clone(),
                });
            }
            AgentSessionEvent::TurnStarted {
                turn_id,
                message_id,
                assistant_message_id,
                prompt,
                at,
            } => {
                let index = *turn_index.entry(*turn_id).or_insert_with(|| {
                    turns.push(TurnProjection::new(
                        *turn_id,
                        message_id.clone(),
                        assistant_message_id
                            .clone()
                            .unwrap_or_else(|| format!("{message_id}:agent")),
                        prompt.clone(),
                        *at,
                    ));
                    turns.len() - 1
                });
                turns[index].prompt = prompt.clone();
                turns[index].prompt_message_id = message_id.clone();
                if let Some(assistant_message_id) = assistant_message_id {
                    turns[index].assistant_message_id = assistant_message_id.clone();
                }
                turns[index].started_at = *at;
            }
            AgentSessionEvent::TextRecorded {
                turn_id,
                message_id,
                content,
                parent_tool_use_id,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    turn.assistant_message_id = message_id.clone();
                    push_consolidated_text(
                        &mut turn.assistant_parts,
                        content,
                        parent_tool_use_id,
                        false,
                    );
                }
            }
            AgentSessionEvent::ReasoningRecorded {
                turn_id,
                message_id,
                content,
                parent_tool_use_id,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    turn.assistant_message_id = message_id.clone();
                    push_consolidated_text(
                        &mut turn.assistant_parts,
                        content,
                        parent_tool_use_id,
                        true,
                    );
                }
            }
            AgentSessionEvent::ErrorRecorded {
                turn_id,
                message_id,
                content,
                parent_tool_use_id,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    turn.assistant_message_id = message_id.clone();
                    push_unique_error(&mut turn.assistant_parts, content, parent_tool_use_id);
                }
            }
            AgentSessionEvent::ToolCallStarted {
                turn_id,
                tool_use_id,
                tool,
                input,
                parent_tool_use_id,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    push_or_update_tool_use(
                        &mut turn.assistant_parts,
                        tool_use_id,
                        tool,
                        input,
                        parent_tool_use_id,
                    );
                }
            }
            AgentSessionEvent::ToolCallSucceeded {
                turn_id,
                tool_use_id,
                content,
                content_ref,
                summary,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    let parent_tool_use_id =
                        parent_tool_use_id_for_tool(&turn.assistant_parts, tool_use_id);
                    push_or_update_tool_result(
                        &mut turn.assistant_parts,
                        tool_use_id,
                        content,
                        false,
                        parent_tool_use_id,
                        content_ref.clone(),
                        summary.clone(),
                    );
                }
            }
            AgentSessionEvent::ToolCallFailed {
                turn_id,
                tool_use_id,
                content,
                content_ref,
                summary,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    let parent_tool_use_id =
                        parent_tool_use_id_for_tool(&turn.assistant_parts, tool_use_id);
                    push_or_update_tool_result(
                        &mut turn.assistant_parts,
                        tool_use_id,
                        content,
                        true,
                        parent_tool_use_id,
                        content_ref.clone(),
                        summary.clone(),
                    );
                }
            }
            AgentSessionEvent::ToolResultRecorded {
                turn_id,
                message_id,
                content,
                is_error,
                content_ref,
                summary,
                tool_use_id,
                parent_tool_use_id,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    turn.assistant_message_id = message_id.clone();
                    match tool_use_id {
                        Some(tool_use_id) => {
                            let parent_tool_use_id = parent_tool_use_id.clone().or_else(|| {
                                parent_tool_use_id_for_tool(&turn.assistant_parts, tool_use_id)
                            });
                            push_or_update_tool_result(
                                &mut turn.assistant_parts,
                                tool_use_id,
                                content,
                                *is_error,
                                parent_tool_use_id,
                                content_ref.clone(),
                                summary.clone(),
                            );
                        }
                        None => {
                            turn.assistant_parts.push(MessagePart::ToolResult {
                                content: content.clone(),
                                is_error: *is_error,
                                tool_use_id: None,
                                parent_tool_use_id: parent_tool_use_id.clone(),
                                content_ref: content_ref.clone(),
                                summary: summary.clone(),
                            });
                        }
                    }
                }
            }
            AgentSessionEvent::ToolCallRetried {
                turn_id,
                tool_use_id,
                attempt,
            } => {
                if turn_index.contains_key(turn_id) {
                    tool_retries.push(ToolRetryProjection {
                        turn_id: *turn_id,
                        tool_use_id: tool_use_id.clone(),
                        attempt: *attempt,
                    });
                } else {
                    log::warn!("agent session event projector saw orphan event for turn {turn_id}");
                }
            }
            AgentSessionEvent::PermissionRequested {
                turn_id,
                tool_use_id,
                request,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    push_or_update_permission(
                        &mut turn.assistant_parts,
                        request.clone(),
                        PermissionPartStatus::Pending,
                        None,
                        tool_use_id.clone(),
                    );
                }
            }
            AgentSessionEvent::PermissionResolved {
                turn_id,
                tool_use_id,
                request_id,
                decision,
                answers,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    resolve_permission(
                        &mut turn.assistant_parts,
                        tool_use_id.as_deref(),
                        request_id.as_deref(),
                        *decision,
                        answers.clone(),
                    );
                }
            }
            AgentSessionEvent::TaskStatusChanged {
                turn_id,
                message_id,
                task_tool_use_id,
                status,
                description,
                summary,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    turn.assistant_message_id = message_id.clone();
                    push_or_update_task_status(
                        &mut turn.assistant_parts,
                        task_tool_use_id,
                        status,
                        description,
                        summary,
                    );
                }
            }
            AgentSessionEvent::TodoListSnapshotRecorded {
                turn_id,
                message_id,
                items,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    turn.assistant_message_id = message_id.clone();
                    push_todo_snapshot(&mut turn.assistant_parts, items.clone());
                }
            }
            AgentSessionEvent::SystemNotificationRecorded {
                turn_id,
                message_id,
                notification_type,
                status,
                label,
                detail,
                hook_id,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    turn.assistant_message_id = message_id.clone();
                    push_or_update_system_notification(
                        &mut turn.assistant_parts,
                        notification_type,
                        status,
                        label,
                        detail,
                        hook_id,
                    );
                }
            }
            AgentSessionEvent::ImageRecorded {
                turn_id,
                message_id,
                data,
                media_type,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    turn.assistant_message_id = message_id.clone();
                    turn.assistant_parts.push(MessagePart::Image {
                        data: data.clone(),
                        media_type: media_type.clone(),
                    });
                }
            }
            AgentSessionEvent::ImageRefRecorded {
                turn_id,
                message_id,
                attachment,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    turn.assistant_message_id = message_id.clone();
                    turn.assistant_parts.push(MessagePart::ImageRef {
                        attachment: attachment.clone(),
                    });
                }
            }
            AgentSessionEvent::FinalPartsRecorded {
                turn_id,
                message_id,
                parts,
            } => {
                if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                    turn.assistant_message_id = message_id.clone();
                    turn.assistant_parts = parts.clone();
                }
            }
            AgentSessionEvent::TurnCompleted {
                turn_id,
                exit_code,
                stop_reason,
                token_usage,
            } => {
                terminal_by_turn.insert(
                    *turn_id,
                    TerminalEvent::Completed {
                        exit_code: *exit_code,
                        stop_reason: *stop_reason,
                        token_usage: *token_usage,
                    },
                );
            }
            AgentSessionEvent::TurnInterrupted {
                turn_id,
                reason,
                exit_code,
                error,
            } => {
                if let Some(error) = error {
                    if let Some(turn) = turn_mut(&mut turns, &turn_index, *turn_id) {
                        push_unique_error(&mut turn.assistant_parts, error, &None);
                    }
                }
                terminal_by_turn.insert(
                    *turn_id,
                    TerminalEvent::Interrupted {
                        reason: *reason,
                        exit_code: *exit_code,
                    },
                );
            }
            AgentSessionEvent::SessionClosed { .. } => {
                session_closed = true;
            }
        }
    }

    let messages = turns
        .iter()
        .flat_map(|turn| turn.to_messages())
        .collect::<Vec<_>>();
    let status = project_status(events, session_closed, &terminal_by_turn);
    let workflow_turn_complete = project_workflow_turn_complete(&turns, &terminal_by_turn);

    SessionReadModel {
        messages,
        status,
        workflow_turn_complete,
        tool_retries,
        backend_recovery,
    }
}

#[derive(Debug, Clone)]
struct TurnProjection {
    turn_id: TurnId,
    prompt_message_id: String,
    assistant_message_id: String,
    prompt: PromptInput,
    started_at: f64,
    assistant_parts: Vec<MessagePart>,
}

impl TurnProjection {
    fn new(
        turn_id: TurnId,
        prompt_message_id: String,
        assistant_message_id: String,
        prompt: PromptInput,
        started_at: f64,
    ) -> Self {
        Self {
            turn_id,
            prompt_message_id,
            assistant_message_id,
            prompt,
            started_at,
            assistant_parts: Vec::new(),
        }
    }

    fn to_messages(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::with_capacity(2);
        let prompt_parts = prompt_parts_for_message(&self.prompt);
        messages.push(ChatMessage {
            id: self.prompt_message_id.clone(),
            role: MessageRole::Human,
            content: self.prompt.content.clone(),
            thinking: None,
            activities: None,
            parts: (!prompt_parts.is_empty()).then_some(prompt_parts),
            streaming_final_seq: 0,
            timestamp: self.started_at,
            mentions: (!self.prompt.mentions.is_empty()).then_some(self.prompt.mentions.clone()),
        });

        let (content, thinking, activities) = parts_to_legacy(&self.assistant_parts);
        messages.push(ChatMessage {
            id: self.assistant_message_id.clone(),
            role: MessageRole::Agent,
            content,
            thinking,
            activities,
            parts: (!self.assistant_parts.is_empty()).then_some(self.assistant_parts.clone()),
            streaming_final_seq: 0,
            timestamp: self.started_at,
            mentions: None,
        });

        messages
    }
}

fn prompt_parts_for_message(prompt: &PromptInput) -> Vec<MessagePart> {
    if !prompt.parts.is_empty() {
        return prompt.parts.clone();
    }

    if prompt.attachment_refs.is_empty() {
        return Vec::new();
    }

    let mut parts = Vec::with_capacity(prompt.attachment_refs.len() + 1);
    if !prompt.content.is_empty() {
        parts.push(MessagePart::Text {
            content: prompt.content.clone(),
            parent_tool_use_id: None,
        });
    }
    parts.extend(
        prompt
            .attachment_refs
            .iter()
            .cloned()
            .map(|attachment| MessagePart::ImageRef { attachment }),
    );
    parts
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TerminalEvent {
    Completed {
        exit_code: i64,
        stop_reason: Option<TurnStopReason>,
        token_usage: Option<TurnTokenUsage>,
    },
    Interrupted {
        reason: InterruptReason,
        exit_code: i64,
    },
}

fn turn_mut<'a>(
    turns: &'a mut [TurnProjection],
    turn_index: &HashMap<TurnId, usize>,
    turn_id: TurnId,
) -> Option<&'a mut TurnProjection> {
    let Some(index) = turn_index.get(&turn_id) else {
        log::warn!("agent session event projector saw orphan event for turn {turn_id}");
        return None;
    };
    turns.get_mut(*index)
}

fn push_consolidated_text(
    parts: &mut Vec<MessagePart>,
    content: &str,
    parent_tool_use_id: &Option<String>,
    thinking: bool,
) {
    match (thinking, parts.last_mut()) {
        (
            false,
            Some(MessagePart::Text {
                content: existing,
                parent_tool_use_id: existing_parent,
            }),
        ) if existing_parent == parent_tool_use_id => existing.push_str(content),
        (
            true,
            Some(MessagePart::Thinking {
                content: existing,
                parent_tool_use_id: existing_parent,
            }),
        ) if existing_parent == parent_tool_use_id => existing.push_str(content),
        (false, _) => parts.push(MessagePart::Text {
            content: content.to_string(),
            parent_tool_use_id: parent_tool_use_id.clone(),
        }),
        (true, _) => parts.push(MessagePart::Thinking {
            content: content.to_string(),
            parent_tool_use_id: parent_tool_use_id.clone(),
        }),
    }
}

fn push_unique_error(
    parts: &mut Vec<MessagePart>,
    content: &str,
    parent_tool_use_id: &Option<String>,
) {
    if parts.iter().any(|part| {
        matches!(
            part,
            MessagePart::Error {
                content: existing,
                parent_tool_use_id: existing_parent,
            } if existing == content && existing_parent == parent_tool_use_id
        )
    }) {
        return;
    }
    parts.push(MessagePart::Error {
        content: content.to_string(),
        parent_tool_use_id: parent_tool_use_id.clone(),
    });
}

fn push_or_update_tool_use(
    parts: &mut Vec<MessagePart>,
    tool_use_id: &str,
    tool: &str,
    input: &serde_json::Value,
    parent_tool_use_id: &Option<String>,
) {
    if let Some(existing) = parts.iter_mut().rev().find(|part| {
        matches!(
            part,
            MessagePart::ToolUse { id, .. } if id == tool_use_id
        )
    }) {
        *existing = MessagePart::ToolUse {
            tool: tool.to_string(),
            input: input.clone(),
            id: tool_use_id.to_string(),
            parent_tool_use_id: parent_tool_use_id.clone(),
        };
        return;
    }
    parts.push(MessagePart::ToolUse {
        tool: tool.to_string(),
        input: input.clone(),
        id: tool_use_id.to_string(),
        parent_tool_use_id: parent_tool_use_id.clone(),
    });
}

fn push_or_update_tool_result(
    parts: &mut Vec<MessagePart>,
    tool_use_id: &str,
    content: &str,
    is_error: bool,
    parent_tool_use_id: Option<String>,
    content_ref: Option<ToolOutputRef>,
    summary: Option<ToolOutputSummary>,
) {
    let _ = apply_tool_result_update(
        parts,
        ToolResultUpdate {
            content: content.to_string(),
            is_error,
            tool_use_id: Some(tool_use_id.to_string()),
            parent_tool_use_id,
            content_ref: content_ref.map(Into::into),
            summary: summary.map(Into::into),
        },
    );
}

fn parent_tool_use_id_for_tool(parts: &[MessagePart], tool_use_id: &str) -> Option<String> {
    parts.iter().rev().find_map(|part| match part {
        MessagePart::ToolUse {
            id,
            parent_tool_use_id,
            ..
        } if id == tool_use_id => parent_tool_use_id.clone(),
        _ => None,
    })
}

fn push_or_update_permission(
    parts: &mut Vec<MessagePart>,
    request: PermissionRequestMsg,
    status: PermissionPartStatus,
    answers: Option<serde_json::Value>,
    tool_use_id: Option<String>,
) {
    let request_id = permission_request_id(&request);
    if let Some(existing) = parts.iter_mut().rev().find(|part| match part {
        MessagePart::Permission {
            request: existing_request,
            ..
        } => {
            permission_request_id(existing_request) == request_id
                || permission_tool_use_id(existing_request) == tool_use_id
        }
        _ => false,
    }) {
        *existing = MessagePart::Permission {
            request,
            status,
            answers,
            parent_tool_use_id: tool_use_id,
        };
        return;
    }
    parts.push(MessagePart::Permission {
        request,
        status,
        answers,
        parent_tool_use_id: tool_use_id,
    });
}

fn resolve_permission(
    parts: &mut [MessagePart],
    tool_use_id: Option<&str>,
    request_id: Option<&str>,
    decision: super::events::PermissionDecision,
    answers: Option<serde_json::Value>,
) {
    if let Some(MessagePart::Permission {
        status,
        answers: existing_answers,
        ..
    }) = parts.iter_mut().rev().find(|part| match part {
        MessagePart::Permission { request, .. } => {
            request_id.is_some_and(|id| permission_request_id(request).as_deref() == Some(id))
                || tool_use_id
                    .is_some_and(|id| permission_tool_use_id(request).as_deref() == Some(id))
                || (request_id.is_none() && tool_use_id.is_none())
        }
        _ => false,
    }) {
        *status = PermissionPartStatus::from_wire(decision.status())
            .unwrap_or(PermissionPartStatus::Denied);
        *existing_answers = answers;
    }
}

fn push_or_update_task_status(
    parts: &mut Vec<MessagePart>,
    task_tool_use_id: &str,
    status: &str,
    description: &Option<String>,
    summary: &Option<String>,
) {
    if let Some(MessagePart::TaskStatus {
        status: existing_status,
        description: existing_description,
        summary: existing_summary,
        ..
    }) = parts.iter_mut().rev().find(|part| {
        matches!(
            part,
            MessagePart::TaskStatus {
                task_tool_use_id: existing_id,
                ..
            } if existing_id == task_tool_use_id
        )
    }) {
        *existing_status = status.to_string();
        if description.is_some() {
            *existing_description = description.clone();
        }
        if summary.is_some() {
            *existing_summary = summary.clone();
        }
        return;
    }
    parts.push(MessagePart::TaskStatus {
        task_tool_use_id: task_tool_use_id.to_string(),
        status: status.to_string(),
        description: description.clone(),
        summary: summary.clone(),
    });
}

fn push_todo_snapshot(parts: &mut Vec<MessagePart>, items: Vec<TodoListItem>) {
    let completed = items.iter().filter(|item| item.completed).count();
    parts.push(MessagePart::Text {
        content: format!("TODO を更新しました（{completed}/{} 完了）", items.len()),
        parent_tool_use_id: None,
    });
    if let Some(existing) = parts
        .iter_mut()
        .rev()
        .find(|part| matches!(part, MessagePart::TodoListSnapshot { .. }))
    {
        *existing = MessagePart::TodoListSnapshot { items };
    } else {
        parts.push(MessagePart::TodoListSnapshot { items });
    }
}

fn push_or_update_system_notification(
    parts: &mut Vec<MessagePart>,
    notification_type: &SystemNotificationType,
    status: &str,
    label: &str,
    detail: &Option<String>,
    hook_id: &Option<String>,
) {
    if let Some(existing) = parts.iter_mut().rev().find(|part| {
        matches!(
            part,
            MessagePart::SystemNotification {
                notification_type: existing_type,
                status: existing_status,
                ..
            } if existing_type == notification_type && existing_status == "in_progress"
        )
    }) {
        *existing = MessagePart::SystemNotification {
            notification_type: notification_type.clone(),
            status: status.to_string(),
            label: label.to_string(),
            detail: detail.clone(),
            hook_id: hook_id.clone(),
        };
        return;
    }
    parts.push(MessagePart::SystemNotification {
        notification_type: notification_type.clone(),
        status: status.to_string(),
        label: label.to_string(),
        detail: detail.clone(),
        hook_id: hook_id.clone(),
    });
}

fn project_status(
    events: &[AgentSessionEvent],
    session_closed: bool,
    terminal_by_turn: &HashMap<TurnId, TerminalEvent>,
) -> ProjectedStatus {
    if session_closed {
        return ProjectedStatus {
            session_state: SessionState::Closed,
            turn_phase: TurnPhase::Idle,
        };
    }
    let Some(turn_id) = events.iter().rev().find_map(|event| match event {
        AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
        _ => None,
    }) else {
        return ProjectedStatus {
            session_state: SessionState::Idle,
            turn_phase: TurnPhase::Idle,
        };
    };

    if let Some(terminal) = terminal_by_turn.get(&turn_id) {
        return match terminal {
            TerminalEvent::Completed { exit_code, .. } if *exit_code == 0 => ProjectedStatus {
                session_state: SessionState::Done,
                turn_phase: TurnPhase::Idle,
            },
            TerminalEvent::Completed { .. } => ProjectedStatus {
                session_state: SessionState::Error,
                turn_phase: TurnPhase::Idle,
            },
            TerminalEvent::Interrupted {
                reason: InterruptReason::Abort,
                ..
            } => ProjectedStatus {
                session_state: SessionState::Idle,
                turn_phase: TurnPhase::Idle,
            },
            TerminalEvent::Interrupted { .. } => ProjectedStatus {
                session_state: SessionState::Error,
                turn_phase: TurnPhase::Idle,
            },
        };
    }

    if has_unresolved_permissions(events, turn_id) {
        return ProjectedStatus {
            session_state: SessionState::Active,
            turn_phase: TurnPhase::WaitingPermission,
        };
    }

    ProjectedStatus {
        session_state: SessionState::Active,
        turn_phase: TurnPhase::Streaming,
    }
}

fn project_workflow_turn_complete(
    turns: &[TurnProjection],
    terminal_by_turn: &HashMap<TurnId, TerminalEvent>,
) -> Option<WorkflowTurnCompleteInput> {
    let turn = turns
        .iter()
        .rev()
        .find(|turn| terminal_by_turn.contains_key(&turn.turn_id))?;
    let terminal = terminal_by_turn.get(&turn.turn_id)?;
    let final_text_parts = turn
        .assistant_parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    match terminal {
        TerminalEvent::Completed {
            exit_code,
            stop_reason,
            token_usage,
        } => Some(WorkflowTurnCompleteInput {
            turn_id: turn.turn_id,
            exit_code: *exit_code,
            final_text_parts,
            failure_signal: workflow_failure_signal_from_stop_reason(*stop_reason),
            token_usage: *token_usage,
            interrupted: false,
        }),
        TerminalEvent::Interrupted { exit_code, .. } => Some(WorkflowTurnCompleteInput {
            turn_id: turn.turn_id,
            exit_code: *exit_code,
            final_text_parts,
            failure_signal: None,
            token_usage: None,
            interrupted: true,
        }),
    }
}

fn workflow_failure_signal_from_stop_reason(
    stop_reason: Option<TurnStopReason>,
) -> Option<AgentTurnFailureSignal> {
    matches!(stop_reason, Some(TurnStopReason::Refusal))
        .then_some(AgentTurnFailureSignal::ModelRefusal)
}
