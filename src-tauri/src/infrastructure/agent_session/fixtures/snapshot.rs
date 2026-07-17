use serde::Serialize;

use crate::domain::agent_session::entities::{
    Attachment, InterruptReason, MessagePart, PermissionDecision, PermissionRequest,
    PermissionRequestBody, PermissionRequestStatus, TokenUsage, TurnResult, TurnStopReason,
};
use crate::domain::agent_session::gateway::{AgentRuntimeEvent, ResumeOutcome};
use crate::domain::agent_session::value_objects::{
    PermissionMode, SlashCommand, SystemNotificationType, TodoListItem, ToolOutputRef,
    ToolOutputSummary,
};

#[derive(Serialize)]
pub(super) enum RuntimeEventSnapshot {
    SessionEstablished {
        backend_session_id: String,
        resume: ResumeOutcomeSnapshot,
    },
    BackendSessionCleared,
    PartsMerged(Vec<MessagePartSnapshot>),
    PermissionRequested(PermissionRequestSnapshot),
    PermissionModeChanged(PermissionModeSnapshot),
    SlashCommandsUpdated(Vec<SlashCommandSnapshot>),
    TokenUsageUpdated(TokenUsageSnapshot),
    KeepAlive,
    TurnCompleted(TurnResultSnapshot),
    Fatal {
        message: String,
    },
}

impl From<&AgentRuntimeEvent> for RuntimeEventSnapshot {
    fn from(event: &AgentRuntimeEvent) -> Self {
        match event {
            AgentRuntimeEvent::SessionEstablished {
                backend_session_id,
                resume,
            } => Self::SessionEstablished {
                backend_session_id: backend_session_id.clone(),
                resume: resume.into(),
            },
            AgentRuntimeEvent::BackendSessionCleared => Self::BackendSessionCleared,
            AgentRuntimeEvent::PartsMerged(parts) => {
                Self::PartsMerged(parts.iter().map(MessagePartSnapshot::from).collect())
            }
            AgentRuntimeEvent::PermissionRequested(request) => {
                Self::PermissionRequested(request.into())
            }
            AgentRuntimeEvent::PermissionModeChanged(mode) => {
                Self::PermissionModeChanged((*mode).into())
            }
            AgentRuntimeEvent::SlashCommandsUpdated(commands) => Self::SlashCommandsUpdated(
                commands
                    .iter()
                    .map(|command| {
                        let SlashCommand {
                            name,
                            description,
                            argument_hint,
                        } = command;
                        SlashCommandSnapshot {
                            name: name.clone(),
                            description: description.clone(),
                            argument_hint: argument_hint.clone(),
                        }
                    })
                    .collect(),
            ),
            AgentRuntimeEvent::TokenUsageUpdated(usage) => Self::TokenUsageUpdated(usage.into()),
            AgentRuntimeEvent::KeepAlive => Self::KeepAlive,
            AgentRuntimeEvent::TurnCompleted(result) => Self::TurnCompleted(result.into()),
            AgentRuntimeEvent::Fatal { message } => Self::Fatal {
                message: message.clone(),
            },
        }
    }
}

#[derive(Serialize)]
pub(super) enum ResumeOutcomeSnapshot {
    NotRequested,
    Resumed,
    Mismatch { actual: String },
}

impl From<&ResumeOutcome> for ResumeOutcomeSnapshot {
    fn from(outcome: &ResumeOutcome) -> Self {
        match outcome {
            ResumeOutcome::NotRequested => Self::NotRequested,
            ResumeOutcome::Resumed => Self::Resumed,
            ResumeOutcome::Mismatch { actual } => Self::Mismatch {
                actual: actual.clone(),
            },
        }
    }
}

#[derive(Serialize)]
pub(super) enum MessagePartSnapshot {
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
        input: String,
        parent_tool_use_id: Option<String>,
    },
    ToolResult {
        content: String,
        is_error: bool,
        tool_use_id: Option<String>,
        parent_tool_use_id: Option<String>,
        content_ref: Option<ToolOutputRefSnapshot>,
        summary: Option<ToolOutputSummarySnapshot>,
    },
    Error {
        content: String,
        parent_tool_use_id: Option<String>,
    },
    Permission {
        request: PermissionRequestSnapshot,
    },
    TaskStatus {
        task_tool_use_id: String,
        status: String,
        description: Option<String>,
        summary: Option<String>,
    },
    TodoListSnapshot {
        items: Vec<TodoListItemSnapshot>,
    },
    SystemNotification {
        notification_type: SystemNotificationTypeSnapshot,
        status: String,
        label: String,
        detail: Option<String>,
        hook_id: Option<String>,
    },
    Image {
        data: String,
        media_type: String,
    },
    ImageRef {
        attachment: AttachmentSnapshot,
    },
}

impl From<&MessagePart> for MessagePartSnapshot {
    fn from(part: &MessagePart) -> Self {
        match part {
            MessagePart::Thinking {
                content,
                parent_tool_use_id,
            } => Self::Thinking {
                content: content.clone(),
                parent_tool_use_id: parent_tool_use_id.clone(),
            },
            MessagePart::Text {
                content,
                parent_tool_use_id,
            } => Self::Text {
                content: content.clone(),
                parent_tool_use_id: parent_tool_use_id.clone(),
            },
            MessagePart::ToolUse {
                id,
                tool,
                input,
                parent_tool_use_id,
            } => Self::ToolUse {
                id: id.clone(),
                tool: tool.clone(),
                input: input.as_str().to_string(),
                parent_tool_use_id: parent_tool_use_id.clone(),
            },
            MessagePart::ToolResult {
                content,
                is_error,
                tool_use_id,
                parent_tool_use_id,
                content_ref,
                summary,
            } => Self::ToolResult {
                content: content.clone(),
                is_error: *is_error,
                tool_use_id: tool_use_id.clone(),
                parent_tool_use_id: parent_tool_use_id.clone(),
                content_ref: content_ref.as_ref().map(|output| {
                    let ToolOutputRef { id, byte_size } = output;
                    ToolOutputRefSnapshot {
                        id: id.clone(),
                        byte_size: *byte_size,
                    }
                }),
                summary: summary.as_ref().map(|summary| {
                    let ToolOutputSummary {
                        line_count,
                        byte_size,
                        is_error,
                        truncated,
                    } = summary;
                    ToolOutputSummarySnapshot {
                        line_count: *line_count,
                        byte_size: *byte_size,
                        is_error: *is_error,
                        truncated: *truncated,
                    }
                }),
            },
            MessagePart::Error {
                content,
                parent_tool_use_id,
            } => Self::Error {
                content: content.clone(),
                parent_tool_use_id: parent_tool_use_id.clone(),
            },
            MessagePart::Permission { request } => Self::Permission {
                request: request.into(),
            },
            MessagePart::TaskStatus {
                task_tool_use_id,
                status,
                description,
                summary,
            } => Self::TaskStatus {
                task_tool_use_id: task_tool_use_id.clone(),
                status: status.clone(),
                description: description.clone(),
                summary: summary.clone(),
            },
            MessagePart::TodoListSnapshot { items } => Self::TodoListSnapshot {
                items: items
                    .iter()
                    .map(|item| {
                        let TodoListItem { text, completed } = item;
                        TodoListItemSnapshot {
                            text: text.clone(),
                            completed: *completed,
                        }
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
                notification_type: (*notification_type).into(),
                status: status.clone(),
                label: label.clone(),
                detail: detail.clone(),
                hook_id: hook_id.clone(),
            },
            MessagePart::Image { data, media_type } => Self::Image {
                data: data.clone(),
                media_type: media_type.clone(),
            },
            MessagePart::ImageRef { attachment } => {
                let Attachment {
                    id,
                    media_type,
                    byte_size,
                } = attachment;
                Self::ImageRef {
                    attachment: AttachmentSnapshot {
                        id: id.clone(),
                        media_type: media_type.clone(),
                        byte_size: *byte_size,
                    },
                }
            }
        }
    }
}

#[derive(Serialize)]
pub(super) struct PermissionRequestSnapshot {
    id: String,
    tool_use_id: Option<String>,
    parent_tool_use_id: Option<String>,
    tool_name: String,
    body: PermissionRequestBodySnapshot,
    title: Option<String>,
    display_name: Option<String>,
    description: Option<String>,
    decision_reason: Option<String>,
    status: PermissionRequestStatusSnapshot,
}

impl From<&PermissionRequest> for PermissionRequestSnapshot {
    fn from(request: &PermissionRequest) -> Self {
        let PermissionRequest {
            id,
            tool_use_id,
            parent_tool_use_id,
            tool_name,
            body,
            title,
            display_name,
            description,
            decision_reason,
            status,
        } = request;
        Self {
            id: id.clone(),
            tool_use_id: tool_use_id.clone(),
            parent_tool_use_id: parent_tool_use_id.clone(),
            tool_name: tool_name.clone(),
            body: body.into(),
            title: title.clone(),
            display_name: display_name.clone(),
            description: description.clone(),
            decision_reason: decision_reason.clone(),
            status: status.into(),
        }
    }
}

#[derive(Serialize)]
pub(super) enum PermissionRequestBodySnapshot {
    ToolApproval {
        input: String,
    },
    PlanApproval {
        plan: String,
        allowed_prompts: Vec<PermissionAllowedPromptSnapshot>,
    },
    Question {
        questions: Vec<PermissionQuestionSnapshot>,
    },
    PermissionGrant {
        requested: String,
    },
}

impl From<&PermissionRequestBody> for PermissionRequestBodySnapshot {
    fn from(body: &PermissionRequestBody) -> Self {
        match body {
            PermissionRequestBody::ToolApproval { input } => Self::ToolApproval {
                input: input.as_str().to_string(),
            },
            PermissionRequestBody::PlanApproval {
                plan,
                allowed_prompts,
            } => Self::PlanApproval {
                plan: plan.clone(),
                allowed_prompts: allowed_prompts
                    .iter()
                    .map(|prompt| PermissionAllowedPromptSnapshot {
                        tool: prompt.tool.clone(),
                        prompt: prompt.prompt.clone(),
                    })
                    .collect(),
            },
            PermissionRequestBody::Question { questions } => Self::Question {
                questions: questions
                    .iter()
                    .map(|question| PermissionQuestionSnapshot {
                        question: question.question.clone(),
                        header: question.header.clone(),
                        options: question
                            .options
                            .iter()
                            .map(|option| PermissionQuestionOptionSnapshot {
                                label: option.label.clone(),
                                description: option.description.clone(),
                            })
                            .collect(),
                        multi_select: question.multi_select,
                    })
                    .collect(),
            },
            PermissionRequestBody::PermissionGrant { requested } => Self::PermissionGrant {
                requested: requested.as_str().to_string(),
            },
        }
    }
}

#[derive(Serialize)]
pub(super) struct PermissionAllowedPromptSnapshot {
    tool: String,
    prompt: String,
}

#[derive(Serialize)]
pub(super) struct PermissionQuestionSnapshot {
    question: String,
    header: Option<String>,
    options: Vec<PermissionQuestionOptionSnapshot>,
    multi_select: bool,
}

#[derive(Serialize)]
pub(super) struct PermissionQuestionOptionSnapshot {
    label: String,
    description: Option<String>,
}

#[derive(Serialize)]
pub(super) enum PermissionRequestStatusSnapshot {
    Pending,
    Resolved {
        decision: PermissionDecisionSnapshot,
        answers: Option<String>,
    },
}

impl From<&PermissionRequestStatus> for PermissionRequestStatusSnapshot {
    fn from(status: &PermissionRequestStatus) -> Self {
        match status {
            PermissionRequestStatus::Pending => Self::Pending,
            PermissionRequestStatus::Resolved { decision, answers } => Self::Resolved {
                decision: (*decision).into(),
                answers: answers.as_ref().map(|value| value.as_str().to_string()),
            },
        }
    }
}

#[derive(Serialize)]
pub(super) enum PermissionDecisionSnapshot {
    Allowed,
    Denied,
    Cancelled,
}

impl From<PermissionDecision> for PermissionDecisionSnapshot {
    fn from(decision: PermissionDecision) -> Self {
        match decision {
            PermissionDecision::Allowed => Self::Allowed,
            PermissionDecision::Denied => Self::Denied,
            PermissionDecision::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Serialize)]
pub(super) enum PermissionModeSnapshot {
    Ask,
    Edit,
    Full,
}

impl From<PermissionMode> for PermissionModeSnapshot {
    fn from(mode: PermissionMode) -> Self {
        match mode {
            PermissionMode::Ask => Self::Ask,
            PermissionMode::Edit => Self::Edit,
            PermissionMode::Full => Self::Full,
        }
    }
}

#[derive(Serialize)]
pub(super) struct SlashCommandSnapshot {
    name: String,
    description: String,
    argument_hint: Option<String>,
}

#[derive(Serialize)]
pub(super) enum TurnResultSnapshot {
    Completed {
        stop_reason: Option<TurnStopReasonSnapshot>,
        token_usage: Option<TokenUsageSnapshot>,
    },
    Failed {
        error: String,
        token_usage: Option<TokenUsageSnapshot>,
    },
    Interrupted {
        reason: InterruptReasonSnapshot,
        error: Option<String>,
    },
}

impl From<&TurnResult> for TurnResultSnapshot {
    fn from(result: &TurnResult) -> Self {
        match result {
            TurnResult::Completed {
                stop_reason,
                token_usage,
            } => Self::Completed {
                stop_reason: (*stop_reason).map(TurnStopReasonSnapshot::from),
                token_usage: token_usage.as_ref().map(TokenUsageSnapshot::from),
            },
            TurnResult::Failed { error, token_usage } => Self::Failed {
                error: error.clone(),
                token_usage: token_usage.as_ref().map(TokenUsageSnapshot::from),
            },
            TurnResult::Interrupted { reason, error } => Self::Interrupted {
                reason: (*reason).into(),
                error: error.clone(),
            },
        }
    }
}

#[derive(Serialize)]
pub(super) enum TurnStopReasonSnapshot {
    Refusal,
}

impl From<TurnStopReason> for TurnStopReasonSnapshot {
    fn from(reason: TurnStopReason) -> Self {
        match reason {
            TurnStopReason::Refusal => Self::Refusal,
        }
    }
}

#[derive(Serialize)]
pub(super) enum InterruptReasonSnapshot {
    Abort,
    Timeout,
    Crash,
}

impl From<InterruptReason> for InterruptReasonSnapshot {
    fn from(reason: InterruptReason) -> Self {
        match reason {
            InterruptReason::Abort => Self::Abort,
            InterruptReason::Timeout => Self::Timeout,
            InterruptReason::Crash => Self::Crash,
        }
    }
}

#[derive(Serialize)]
pub(super) struct TokenUsageSnapshot {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: Option<u64>,
    context_window_tokens: Option<u64>,
}

impl From<&TokenUsage> for TokenUsageSnapshot {
    fn from(usage: &TokenUsage) -> Self {
        let TokenUsage {
            input_tokens,
            output_tokens,
            total_tokens,
            context_window_tokens,
        } = usage;
        Self {
            input_tokens: *input_tokens,
            output_tokens: *output_tokens,
            total_tokens: *total_tokens,
            context_window_tokens: *context_window_tokens,
        }
    }
}

#[derive(Serialize)]
pub(super) struct ToolOutputRefSnapshot {
    id: String,
    byte_size: u64,
}

#[derive(Serialize)]
pub(super) struct ToolOutputSummarySnapshot {
    line_count: u64,
    byte_size: u64,
    is_error: bool,
    truncated: bool,
}

#[derive(Serialize)]
pub(super) struct TodoListItemSnapshot {
    text: String,
    completed: bool,
}

#[derive(Serialize)]
pub(super) enum SystemNotificationTypeSnapshot {
    Compaction,
    SessionRecovery,
}

impl From<SystemNotificationType> for SystemNotificationTypeSnapshot {
    fn from(notification_type: SystemNotificationType) -> Self {
        match notification_type {
            SystemNotificationType::Compaction => Self::Compaction,
            SystemNotificationType::SessionRecovery => Self::SessionRecovery,
        }
    }
}

#[derive(Serialize)]
pub(super) struct AttachmentSnapshot {
    id: String,
    media_type: String,
    byte_size: u64,
}
