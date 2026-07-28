//! Closed semantic values stored in local-state rows.
//!
//! These types are the repository-port contract. They deliberately contain
//! no serde attributes, persistence versions, schema/tag strings, SQL row
//! details, or raw JSON. The SQLite gateway owns all `Stored*V1` conversion.

use std::path::PathBuf;

use crate::domain::agent_session::entities::{
    InterruptReason, MessagePart, PermissionRequest, PermissionRequestBody,
    PermissionRequestStatus, PermissionResponse, PermissionResponseDecision, TokenUsage,
    TurnResult, TurnStopReason,
};
use crate::domain::agent_session::events::{
    AgentSessionDomainEvent, BackendSessionRecoveryReason, RecoveryActionKind,
    RecoveryResultClassification, SendDisposition, StopResolution, TurnTokenUsage,
};
use crate::domain::agent_session::value_objects::{
    ContextEpoch, ContextRevision, ContextSourceKind, JsonPayload, ToolOutputRef, ToolOutputSummary,
};
use crate::domain::code::MentionReference;
use crate::domain::workflow::{
    ExecutionInterruptionReason, ExecutionOrigin, ExecutionStatus,
    TokenUsage as WorkflowTokenUsage, WorkflowNodeContext,
};

use super::{
    CommitOperationKind, OperationKind, QuitIntent, SafeOperationFailure, ShutdownPlanKey,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSessionStateRecord {
    Active,
    Idle,
    Done,
    Error,
    Closed,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentContextCarryStateRecord {
    Resumed,
    Reinjected,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPendingRecoveryMessageRecord {
    Notice {
        recovery_id: String,
        message_id: String,
    },
    Error {
        recovery_id: String,
        message_id: String,
        error: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRecoveryPublicationListRecord {
    SessionList,
    ClosedHistory,
    ArchivedHistory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRecoveryPublicationWorkflowOwnerRecord {
    pub execution_id: Option<String>,
    pub node_execution_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRecoveryPublicationClassificationRecord {
    pub list: AgentRecoveryPublicationListRecord,
    pub workflow_owner: Option<AgentRecoveryPublicationWorkflowOwnerRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionSummaryRecord {
    pub id: String,
    pub worktree_path: String,
    pub state: AgentSessionStateRecord,
    pub error_reason: Option<String>,
    pub created_at_bits: u64,
    pub updated_at_bits: u64,
    pub first_message: String,
    pub message_count: u64,
    pub agent_session_id: Option<String>,
    pub context_carry: Option<AgentContextCarryStateRecord>,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
    pub backend_id: Option<String>,
    pub workflow_node_session: bool,
    pub workflow_node_context: Option<WorkflowNodeContext>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRecoveryPublicationSnapshotRecord {
    pub recovery_id: String,
    pub summary: AgentSessionSummaryRecord,
    pub classification: AgentRecoveryPublicationClassificationRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentContextSourceRecord {
    pub kind: ContextSourceKind,
    pub revision: ContextRevision,
    pub fingerprint: Option<String>,
    pub payload: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentContextEpochRecord {
    pub epoch: ContextEpoch,
    pub sources: Vec<AgentContextSourceRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTurnInterruptionRecord {
    pub message_id: String,
    pub reason: crate::domain::agent_session::events::InterruptReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionMetadataRecord {
    pub id: String,
    pub worktree_path: String,
    pub state: AgentSessionStateRecord,
    pub error_reason: Option<String>,
    pub state_revision: u64,
    pub created_at_bits: u64,
    pub updated_at_bits: u64,
    pub agent_session_id: Option<String>,
    pub provider_session_generation: u64,
    pub provider_session_observation_id: Option<String>,
    pub context_reinjection_generation: Option<u64>,
    pub context_carry: Option<AgentContextCarryStateRecord>,
    pub pending_recovery_message: Option<AgentPendingRecoveryMessageRecord>,
    pub recovery_publication_snapshot: Option<AgentRecoveryPublicationSnapshotRecord>,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub selected_model: Option<String>,
    pub permission_profile_id: Option<String>,
    pub backend_id: String,
    pub workflow_node_session: bool,
    pub workflow_node_context: Option<WorkflowNodeContext>,
    pub workflow_instructions: Vec<String>,
    pub agent_read_paths: Option<Vec<PathBuf>>,
    pub context_epoch: Option<AgentContextEpochRecord>,
    pub last_turn_interruption: Option<AgentTurnInterruptionRecord>,
    pub last_turn_id: Option<u64>,
    pub first_message_preview: String,
    pub message_count: u64,
    pub body_format_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentQueuedSendRecord {
    pub queue_item_id: String,
    pub human_message_id: String,
    pub reserved_turn_id: String,
    pub input_ref: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentSessionProjectionRecord {
    pub meta: AgentSessionMetadataRecord,
    pub title: Option<String>,
    pub reducer_events: Vec<AgentSessionDomainEvent>,
    pub queue_paused_at_bits: Option<u64>,
    pub latest_token_usage: Option<TokenUsage>,
    pub pending_send_queue: Vec<AgentQueuedSendRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowExecutionProjectionRecord {
    Present(WorkflowExecutionMetadataRecord),
    Deleted { execution_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowWorktreeOwnerRecord {
    pub worktree_path: String,
    pub execution_id: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionProjectionRecord {
    AgentSession(Box<AgentSessionProjectionRecord>),
    WorkflowExecution(WorkflowExecutionProjectionRecord),
    WorkflowWorktreeOwner(WorkflowWorktreeOwnerRecord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMessageRoleRecord {
    Human,
    Agent,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentMessageActivityRecord {
    ToolUse {
        tool: String,
        input: JsonPayload,
        id: String,
    },
    ToolResult {
        content: String,
        is_error: bool,
        tool_use_id: Option<String>,
        content_ref: Option<ToolOutputRef>,
        summary: Option<ToolOutputSummary>,
    },
    PermissionResult {
        tool_name: String,
        status: String,
        summary: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentMessageProjectionRecord {
    pub id: String,
    pub role: AgentMessageRoleRecord,
    pub content: String,
    pub thinking: Option<String>,
    pub activities: Option<Vec<AgentMessageActivityRecord>>,
    pub parts: Option<Vec<MessagePart>>,
    pub streaming_final_seq: u64,
    pub timestamp_bits: u64,
    pub mentions: Option<Vec<MentionReference>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentContentBlobRecord {
    Attachment {
        id: String,
        media_type: String,
        bytes: Vec<u8>,
    },
    ToolOutput {
        id: String,
        content: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessageProjectionRecord {
    AgentMessage(AgentMessageProjectionRecord),
    AgentContentBlob(AgentContentBlobRecord),
}

impl SessionProjectionRecord {
    /// Conservative decoded-size accounting for bounded writer admission.
    ///
    /// The gateway separately checks the exact Stored V1 byte length before
    /// SQLite writes. This calculation accounts for semantic heap content and
    /// never derives a bound from `Debug` or a persistence representation.
    pub(crate) fn semantic_bytes(&self) -> usize {
        match self {
            Self::AgentSession(projection) => {
                let meta = &projection.meta;
                let mut total = 512usize
                    .saturating_add(strings([
                        meta.id.as_str(),
                        meta.worktree_path.as_str(),
                        meta.permission_mode.as_str(),
                        meta.backend_id.as_str(),
                        meta.first_message_preview.as_str(),
                    ]))
                    .saturating_add(optional_string(&meta.error_reason))
                    .saturating_add(optional_string(&meta.agent_session_id))
                    .saturating_add(optional_string(&meta.selected_model))
                    .saturating_add(optional_string(&meta.permission_profile_id))
                    .saturating_add(
                        meta.workflow_instructions
                            .iter()
                            .fold(0usize, |size, value| size.saturating_add(value.len())),
                    )
                    .saturating_add(meta.agent_read_paths.as_ref().map_or(0, |paths| {
                        paths.iter().fold(0usize, |size, path| {
                            size.saturating_add(path.as_os_str().len())
                        })
                    }))
                    .saturating_add(projection.title.as_ref().map_or(0, String::len));
                if let Some(message) = &meta.pending_recovery_message {
                    total = total.saturating_add(match message {
                        AgentPendingRecoveryMessageRecord::Notice {
                            recovery_id,
                            message_id,
                        } => strings([recovery_id.as_str(), message_id.as_str()]),
                        AgentPendingRecoveryMessageRecord::Error {
                            recovery_id,
                            message_id,
                            error,
                        } => strings([recovery_id.as_str(), message_id.as_str(), error.as_str()]),
                    });
                }
                if let Some(snapshot) = &meta.recovery_publication_snapshot {
                    total = total
                        .saturating_add(snapshot.recovery_id.len())
                        .saturating_add(session_summary_bytes(&snapshot.summary))
                        .saturating_add(snapshot.classification.workflow_owner.as_ref().map_or(
                            0,
                            |owner| {
                                optional_string(&owner.execution_id)
                                    .saturating_add(optional_string(&owner.node_execution_id))
                            },
                        ));
                }
                if let Some(context) = &meta.workflow_node_context {
                    total = total.saturating_add(workflow_node_context_bytes(context));
                }
                if let Some(epoch) = &meta.context_epoch {
                    total = total
                        .saturating_add(optional_string(&epoch.epoch.backend_id))
                        .saturating_add(optional_string(&epoch.epoch.model_id))
                        .saturating_add(epoch.epoch.worktree_path.len());
                    total =
                        total.saturating_add(epoch.sources.iter().fold(0usize, |size, source| {
                            size.saturating_add(32)
                                .saturating_add(optional_string(&source.fingerprint))
                                .saturating_add(optional_string(&source.payload))
                        }));
                }
                if let Some(interruption) = &meta.last_turn_interruption {
                    total = total.saturating_add(interruption.message_id.len());
                }
                total = total
                    .saturating_add(
                        projection
                            .reducer_events
                            .iter()
                            .fold(0usize, |size, event| {
                                size.saturating_add(agent_event_bytes(event))
                            }),
                    )
                    .saturating_add(projection.pending_send_queue.iter().fold(
                        0usize,
                        |size, entry| {
                            size.saturating_add(strings([
                                entry.queue_item_id.as_str(),
                                entry.human_message_id.as_str(),
                                entry.reserved_turn_id.as_str(),
                                entry.input_ref.as_str(),
                            ]))
                        },
                    ));
                total
            }
            Self::WorkflowExecution(WorkflowExecutionProjectionRecord::Present(execution)) => {
                256usize
                    .saturating_add(strings([
                        execution.execution_id.as_str(),
                        execution.workflow_name.as_str(),
                        execution.worktree_path.as_str(),
                    ]))
                    .saturating_add(optional_string(&execution.current_node))
                    .saturating_add(optional_string(&execution.error_reason))
                    .saturating_add(optional_string(&execution.resume_from_node))
            }
            Self::WorkflowExecution(WorkflowExecutionProjectionRecord::Deleted {
                execution_id,
            }) => 64usize.saturating_add(execution_id.len()),
            Self::WorkflowWorktreeOwner(owner) => 96usize.saturating_add(strings([
                owner.worktree_path.as_str(),
                owner.execution_id.as_str(),
            ])),
        }
    }
}

impl MessageProjectionRecord {
    pub(crate) fn semantic_bytes(&self) -> usize {
        match self {
            Self::AgentMessage(message) => 256usize
                .saturating_add(strings([message.id.as_str(), message.content.as_str()]))
                .saturating_add(optional_string(&message.thinking))
                .saturating_add(message.activities.as_ref().map_or(0, |activities| {
                    activities.iter().fold(0usize, |size, activity| {
                        size.saturating_add(message_activity_bytes(activity))
                    })
                }))
                .saturating_add(message.parts.as_ref().map_or(0, |parts| {
                    parts.iter().fold(0usize, |size, part| {
                        size.saturating_add(message_part_bytes(part))
                    })
                }))
                .saturating_add(message.mentions.as_ref().map_or(0, |mentions| {
                    mentions.iter().fold(0usize, |size, mention| {
                        size.saturating_add(16)
                            .saturating_add(mention.file_path.len())
                    })
                })),
            Self::AgentContentBlob(AgentContentBlobRecord::Attachment {
                id,
                media_type,
                bytes,
            }) => 96usize
                .saturating_add(strings([id.as_str(), media_type.as_str()]))
                .saturating_add(bytes.len()),
            Self::AgentContentBlob(AgentContentBlobRecord::ToolOutput { id, content }) => {
                96usize.saturating_add(strings([id.as_str(), content.as_str()]))
            }
        }
    }
}

fn strings<const N: usize>(values: [&str; N]) -> usize {
    values
        .into_iter()
        .fold(0usize, |size, value| size.saturating_add(value.len()))
}

fn optional_string(value: &Option<String>) -> usize {
    value.as_ref().map_or(0, String::len)
}

fn workflow_node_context_bytes(context: &WorkflowNodeContext) -> usize {
    96usize
        .saturating_add(strings([
            context.execution_id.as_str(),
            context.node_execution_id.as_str(),
            context.workflow_name.as_str(),
            context.node_name.as_str(),
        ]))
        .saturating_add(optional_string(&context.parent_node_name))
}

fn session_summary_bytes(summary: &AgentSessionSummaryRecord) -> usize {
    let mut total = 256usize
        .saturating_add(strings([
            summary.id.as_str(),
            summary.worktree_path.as_str(),
            summary.first_message.as_str(),
            summary.permission_mode.as_str(),
        ]))
        .saturating_add(optional_string(&summary.error_reason))
        .saturating_add(optional_string(&summary.agent_session_id))
        .saturating_add(optional_string(&summary.permission_profile_id))
        .saturating_add(optional_string(&summary.backend_id));
    if let Some(context) = &summary.workflow_node_context {
        total = total.saturating_add(workflow_node_context_bytes(context));
    }
    total
}

fn message_activity_bytes(activity: &AgentMessageActivityRecord) -> usize {
    match activity {
        AgentMessageActivityRecord::ToolUse { tool, input, id } => {
            64usize.saturating_add(strings([tool.as_str(), input.as_str(), id.as_str()]))
        }
        AgentMessageActivityRecord::ToolResult {
            content,
            tool_use_id,
            content_ref,
            ..
        } => 64usize
            .saturating_add(content.len())
            .saturating_add(optional_string(tool_use_id))
            .saturating_add(
                content_ref
                    .as_ref()
                    .map_or(0, |reference| reference.id.len()),
            ),
        AgentMessageActivityRecord::PermissionResult {
            tool_name,
            status,
            summary,
        } => 64usize.saturating_add(strings([
            tool_name.as_str(),
            status.as_str(),
            summary.as_str(),
        ])),
    }
}

fn message_part_bytes(part: &MessagePart) -> usize {
    match part {
        MessagePart::Thinking {
            content,
            parent_tool_use_id,
        }
        | MessagePart::Text {
            content,
            parent_tool_use_id,
        }
        | MessagePart::Error {
            content,
            parent_tool_use_id,
        } => 48usize
            .saturating_add(content.len())
            .saturating_add(optional_string(parent_tool_use_id)),
        MessagePart::ToolUse {
            id,
            tool,
            input,
            parent_tool_use_id,
        } => 64usize
            .saturating_add(strings([id.as_str(), tool.as_str(), input.as_str()]))
            .saturating_add(optional_string(parent_tool_use_id)),
        MessagePart::ToolResult {
            content,
            tool_use_id,
            parent_tool_use_id,
            content_ref,
            ..
        } => 64usize
            .saturating_add(content.len())
            .saturating_add(optional_string(tool_use_id))
            .saturating_add(optional_string(parent_tool_use_id))
            .saturating_add(
                content_ref
                    .as_ref()
                    .map_or(0, |reference| reference.id.len()),
            ),
        MessagePart::Permission {
            request,
            answers,
            parent_tool_use_id,
            ..
        } => 64usize
            .saturating_add(permission_request_bytes(request))
            .saturating_add(answers.as_ref().map_or(0, |answers| answers.as_str().len()))
            .saturating_add(optional_string(parent_tool_use_id)),
        MessagePart::TaskStatus {
            task_tool_use_id,
            status,
            description,
            summary,
        } => 64usize
            .saturating_add(strings([task_tool_use_id.as_str(), status.as_str()]))
            .saturating_add(optional_string(description))
            .saturating_add(optional_string(summary)),
        MessagePart::TodoListSnapshot { items } => items.iter().fold(32usize, |size, item| {
            size.saturating_add(16).saturating_add(item.text.len())
        }),
        MessagePart::SystemNotification {
            status,
            label,
            detail,
            hook_id,
            ..
        } => 64usize
            .saturating_add(strings([status.as_str(), label.as_str()]))
            .saturating_add(optional_string(detail))
            .saturating_add(optional_string(hook_id)),
        MessagePart::Image { data, media_type } => {
            64usize.saturating_add(strings([data.as_str(), media_type.as_str()]))
        }
        MessagePart::ImageRef { attachment } => 64usize.saturating_add(strings([
            attachment.id.as_str(),
            attachment.media_type.as_str(),
        ])),
    }
}

fn permission_request_bytes(request: &PermissionRequest) -> usize {
    let mut total = 128usize
        .saturating_add(strings([request.id.as_str(), request.tool_name.as_str()]))
        .saturating_add(optional_string(&request.tool_use_id))
        .saturating_add(optional_string(&request.parent_tool_use_id))
        .saturating_add(optional_string(&request.title))
        .saturating_add(optional_string(&request.display_name))
        .saturating_add(optional_string(&request.description))
        .saturating_add(optional_string(&request.decision_reason));
    total = total.saturating_add(match &request.body {
        PermissionRequestBody::ToolApproval { input } => input.as_str().len(),
        PermissionRequestBody::PlanApproval {
            plan,
            allowed_prompts,
        } => allowed_prompts.iter().fold(plan.len(), |size, prompt| {
            size.saturating_add(strings([prompt.tool.as_str(), prompt.prompt.as_str()]))
        }),
        PermissionRequestBody::Question { questions } => {
            questions.iter().fold(0usize, |size, question| {
                size.saturating_add(question.question.len())
                    .saturating_add(optional_string(&question.header))
                    .saturating_add(question.options.iter().fold(0usize, |size, option| {
                        size.saturating_add(option.label.len())
                            .saturating_add(optional_string(&option.description))
                    }))
            })
        }
        PermissionRequestBody::PermissionGrant { requested } => requested.as_str().len(),
    });
    if let PermissionRequestStatus::Resolved {
        answers: Some(answers),
        ..
    } = &request.status
    {
        total = total.saturating_add(answers.as_str().len());
    }
    total
}

fn prompt_input_bytes(prompt: &crate::domain::agent_session::events::PromptInput) -> usize {
    prompt
        .mentions
        .iter()
        .fold(prompt.content.len(), |size, mention| {
            size.saturating_add(16)
                .saturating_add(mention.file_path.len())
        })
        .saturating_add(
            prompt
                .attachment_refs
                .iter()
                .fold(0usize, |size, attachment| {
                    size.saturating_add(strings([
                        attachment.id.as_str(),
                        attachment.media_type.as_str(),
                    ]))
                }),
        )
        .saturating_add(prompt.parts.iter().fold(0usize, |size, part| {
            size.saturating_add(message_part_bytes(part))
        }))
}

fn goal_reactivation_bytes(
    outcome: &crate::domain::agent_session::events::GoalReactivationOutcome,
) -> usize {
    use crate::domain::agent_session::events::GoalReactivationOutcome;
    match outcome {
        GoalReactivationOutcome::NoCurrentGoal => 16,
        GoalReactivationOutcome::TerminalGoalUnchanged { goal_id, .. }
        | GoalReactivationOutcome::ObservedUnchanged { goal_id, .. } => {
            32usize.saturating_add(goal_id.len())
        }
        GoalReactivationOutcome::Restored {
            goal_id,
            provider_goal_ref,
            ..
        } => 32usize
            .saturating_add(goal_id.len())
            .saturating_add(optional_string(provider_goal_ref)),
    }
}

fn agent_event_bytes(event: &AgentSessionDomainEvent) -> usize {
    use crate::domain::agent_session::events::AgentSessionDomainEvent as Event;
    let base = 64usize;
    match event {
        Event::BackendSessionRecoveryStarted { recovery_id, .. }
        | Event::BackendSessionRecoveryCompleted { recovery_id, .. } => {
            base.saturating_add(recovery_id.len())
        }
        Event::SessionConfigurationReactivated {
            recovery_id,
            consumed_observation_id,
            ..
        } => base
            .saturating_add(recovery_id.len())
            .saturating_add(optional_string(consumed_observation_id)),
        Event::SessionGoalReactivated {
            recovery_id,
            outcome,
            restoring_turn_id,
            consumed_observation_id,
            ..
        } => base
            .saturating_add(recovery_id.len())
            .saturating_add(goal_reactivation_bytes(outcome))
            .saturating_add(optional_string(restoring_turn_id))
            .saturating_add(optional_string(consumed_observation_id)),
        Event::BackendSessionRecoveryFailed {
            recovery_id, error, ..
        } => base.saturating_add(strings([recovery_id.as_str(), error.as_str()])),
        Event::TurnStarted {
            message_id,
            assistant_message_id,
            prompt,
            ..
        } => base
            .saturating_add(message_id.len())
            .saturating_add(optional_string(assistant_message_id))
            .saturating_add(prompt_input_bytes(prompt)),
        Event::TurnInterruptRequested { .. }
        | Event::QueuePaused { .. }
        | Event::QueueResumed { .. }
        | Event::TurnCompleted { .. }
        | Event::SessionClosed { .. } => base,
        Event::TextRecorded {
            message_id,
            content,
            parent_tool_use_id,
            ..
        }
        | Event::ReasoningRecorded {
            message_id,
            content,
            parent_tool_use_id,
            ..
        }
        | Event::ErrorRecorded {
            message_id,
            content,
            parent_tool_use_id,
            ..
        } => base
            .saturating_add(strings([message_id.as_str(), content.as_str()]))
            .saturating_add(optional_string(parent_tool_use_id)),
        Event::ToolCallStarted {
            tool_use_id,
            tool,
            input,
            parent_tool_use_id,
            ..
        } => base
            .saturating_add(strings([
                tool_use_id.as_str(),
                tool.as_str(),
                input.as_str(),
            ]))
            .saturating_add(optional_string(parent_tool_use_id)),
        Event::ToolCallSucceeded {
            tool_use_id,
            content,
            content_ref,
            ..
        }
        | Event::ToolCallFailed {
            tool_use_id,
            content,
            content_ref,
            ..
        } => base
            .saturating_add(strings([tool_use_id.as_str(), content.as_str()]))
            .saturating_add(
                content_ref
                    .as_ref()
                    .map_or(0, |reference| reference.id.len()),
            ),
        Event::ToolResultRecorded {
            message_id,
            content,
            content_ref,
            tool_use_id,
            parent_tool_use_id,
            ..
        } => base
            .saturating_add(strings([message_id.as_str(), content.as_str()]))
            .saturating_add(
                content_ref
                    .as_ref()
                    .map_or(0, |reference| reference.id.len()),
            )
            .saturating_add(optional_string(tool_use_id))
            .saturating_add(optional_string(parent_tool_use_id)),
        Event::ToolCallRetried { tool_use_id, .. } => base.saturating_add(tool_use_id.len()),
        Event::PermissionRequested {
            tool_use_id,
            request,
            ..
        } => base
            .saturating_add(optional_string(tool_use_id))
            .saturating_add(permission_request_bytes(request)),
        Event::PermissionResolved {
            tool_use_id,
            request_id,
            answers,
            ..
        } => base
            .saturating_add(optional_string(tool_use_id))
            .saturating_add(optional_string(request_id))
            .saturating_add(answers.as_ref().map_or(0, |answers| answers.as_str().len())),
        Event::TaskStatusChanged {
            message_id,
            task_tool_use_id,
            status,
            description,
            summary,
            ..
        } => base
            .saturating_add(strings([
                message_id.as_str(),
                task_tool_use_id.as_str(),
                status.as_str(),
            ]))
            .saturating_add(optional_string(description))
            .saturating_add(optional_string(summary)),
        Event::TodoListSnapshotRecorded {
            message_id, items, ..
        } => items
            .iter()
            .fold(base.saturating_add(message_id.len()), |size, item| {
                size.saturating_add(16).saturating_add(item.text.len())
            }),
        Event::SystemNotificationRecorded {
            message_id,
            status,
            label,
            detail,
            hook_id,
            ..
        } => base
            .saturating_add(strings([
                message_id.as_str(),
                status.as_str(),
                label.as_str(),
            ]))
            .saturating_add(optional_string(detail))
            .saturating_add(optional_string(hook_id)),
        Event::ImageRecorded {
            message_id,
            data,
            media_type,
            ..
        } => base.saturating_add(strings([
            message_id.as_str(),
            data.as_str(),
            media_type.as_str(),
        ])),
        Event::ImageRefRecorded {
            message_id,
            attachment,
            ..
        } => base.saturating_add(strings([
            message_id.as_str(),
            attachment.id.as_str(),
            attachment.media_type.as_str(),
        ])),
        Event::FinalPartsRecorded {
            message_id, parts, ..
        } => parts
            .iter()
            .fold(base.saturating_add(message_id.len()), |size, part| {
                size.saturating_add(message_part_bytes(part))
            }),
        Event::TurnInterrupted { error, .. } => base.saturating_add(optional_string(error)),
        Event::SessionErrored {
            message_id, reason, ..
        } => base.saturating_add(strings([message_id.as_str(), reason.as_str()])),
        Event::SendOperationAccepted {
            operation_id,
            human_message_id,
            prompt,
            reserved_turn_id,
            ..
        } => base
            .saturating_add(operation_id.len())
            .saturating_add(optional_string(human_message_id))
            .saturating_add(prompt.as_ref().map_or(0, prompt_input_bytes))
            .saturating_add(optional_string(reserved_turn_id)),
        Event::StopOperationAccepted { operation_id, .. }
        | Event::SessionLifecycleOperationAccepted { operation_id, .. }
        | Event::StopResolutionRecorded { operation_id, .. } => {
            base.saturating_add(operation_id.len())
        }
        Event::ObligationRecorded { obligation_id, .. }
        | Event::PendingRecoveryPublished { obligation_id, .. } => {
            base.saturating_add(obligation_id.len())
        }
        Event::RecoveryActionResolved {
            action_id,
            obligation_id,
            ..
        } => base.saturating_add(strings([action_id.as_str(), obligation_id.as_str()])),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordAuthentication {
    pub principal_mac: [u8; 32],
    pub binding_hmac: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLifecycleRecordAction {
    Close,
    ArchiveOpen,
    ArchiveClosed,
    SwitchBackend { backend_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationReceiptRecord {
    Send {
        operation_id: String,
        session_id: String,
        input_ref: String,
        disposition: SendDisposition,
        authentication: RecordAuthentication,
    },
    PermissionResponse {
        operation_id: String,
        session_id: String,
        request_id: String,
        input_ref: String,
        authentication: RecordAuthentication,
    },
    Stop {
        operation_id: String,
        session_id: String,
        turn_id: String,
        accepted_revision: u64,
        authentication: RecordAuthentication,
    },
    SessionLifecycle {
        operation_id: String,
        session_id: String,
        action: SessionLifecycleRecordAction,
        first_accepted_revision: i64,
        commit_operation_kind: CommitOperationKind,
        authentication: RecordAuthentication,
    },
    ApplicationQuit {
        operation_id: String,
        plan: ShutdownPlanKey,
        intent: QuitIntent,
        t0_ms: i64,
        deadline_ms: i64,
        binding_hmac: [u8; 32],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecisionRecord {
    Allowed,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationStatusValue {
    Accepted,
    AwaitingProviderStart {
        dependency_obligation_ids: Vec<String>,
    },
    AwaitingProviderResponse {
        obligation_id: String,
    },
    Queued {
        queue_item_id: String,
        reserved_turn_id: String,
    },
    ProviderStartReserved {
        obligation_id: String,
    },
    Running {
        turn_id: String,
    },
    Completed,
    PermissionCompleted {
        decision: PermissionDecisionRecord,
    },
    StopCompleted {
        resolution: StopResolution,
    },
    Preparing,
    Activated,
    ExitPending,
    Exited,
    OutcomeUnknown {
        operation_id: String,
        plan: ShutdownPlanKey,
        activation_commit_id: String,
    },
    FailedBeforeActivation {
        failure: SafeOperationFailure,
    },
    ReconciliationRequired {
        failure: SafeOperationFailure,
    },
    Failed {
        failure: SafeOperationFailure,
    },
    Terminal {
        result: TurnResult,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationStatusRecord {
    pub kind: OperationKind,
    pub value: OperationStatusValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTerminalKind {
    Completed,
    Abort,
    Timeout,
    Crash,
    SessionClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalInterruptReasonRecord {
    Abort,
    Timeout,
    Crash,
    SessionClosed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentTurnTerminalResultRecord {
    Current(TurnResult),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TerminalResultRecord {
    AgentTurn {
        kind: AgentTerminalKind,
        session_id: String,
        turn_id: String,
        message_id: String,
        streaming_final_sequence: u64,
        completed_at_bits: u64,
        result: AgentTurnTerminalResultRecord,
    },
    SessionClosed {
        operation_id: String,
        reason: TerminalInterruptReasonRecord,
        result: TurnResult,
    },
    Stop {
        operation_id: String,
        /// `None` is the historic `terminal_winner` marker used when an
        /// already committed terminal, rather than a new interruption,
        /// resolves the Stop operation.
        reason: Option<TerminalInterruptReasonRecord>,
        exit_code: Option<i32>,
        result: TurnResult,
    },
    StopSuperseded {
        terminal_identity: String,
        terminal_result_sha256: [u8; 32],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObligationStateRecord {
    Prepared,
    Pending,
    EffectReserved,
    Running,
    WaitingApproval,
    OutcomeUnknown,
    ReconciliationRequired,
    Failed,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendObligationKindRecord {
    ProviderEstablish,
    TurnExecution,
}

/// Acceptance-time routing fact. The historical V1 obligation stored only
/// this tag; queue/turn identities live in their dedicated fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendObligationDispositionRecord {
    StartedTurn,
    Queued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPublicationMessageKindRecord {
    Notice,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSessionNoticeOperationRecord {
    Send,
    LoadSession,
    LoadOlder,
    CancelQueue,
    ResumeQueue,
    CloseSession,
    RestoreSession,
    ArchiveSession,
    ForkSession,
    SetTitle,
    RespondPermission,
    SetBackend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackActionRecord {
    Dismiss,
    RetryResolution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowExecutionMetadataRecord {
    pub execution_id: String,
    pub workflow_name: String,
    pub status: ExecutionStatus,
    pub worktree_path: String,
    pub current_node: Option<String>,
    pub created_from: ExecutionOrigin,
    pub started_at_bits: u64,
    pub updated_at_bits: u64,
    pub completed_at_bits: Option<u64>,
    pub error_reason: Option<String>,
    pub interruption_reason: Option<ExecutionInterruptionReason>,
    pub resume_from_node: Option<String>,
    pub total_token_usage: WorkflowTokenUsage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryPublicationMessageRecord {
    pub kind: RecoveryPublicationMessageKindRecord,
    pub recovery_id: String,
    pub message_id: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTurnFailureSignalRecord {
    ModelRefusal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObligationRecoveryActionRecord {
    pub action_id: String,
    pub origin_revision: u64,
    pub action: RecoveryActionKind,
    pub effect_identity: String,
    pub state: ObligationStateRecord,
    pub classification: Option<RecoveryResultClassification>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoritativeEffectObservationRecord {
    pub effect_identity: String,
    pub origin_revision: u64,
    pub classification: RecoveryResultClassification,
    pub cancellable: bool,
    pub safe_view: String,
    pub result_sha256: [u8; 32],
    pub proof_mac: [u8; 32],
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObligationRecord {
    Send {
        obligation_id: String,
        operation_id: String,
        session_id: String,
        kind: SendObligationKindRecord,
        disposition: SendObligationDispositionRecord,
        human_message_id: Option<String>,
        assistant_message_id: Option<String>,
        reserved_turn_id: Option<String>,
        turn_id: Option<String>,
        dependency_obligation_ids: Vec<String>,
        canonical_payload: String,
        state: ObligationStateRecord,
    },
    PermissionResponse {
        operation_id: String,
        effect_identity: String,
        session_id: String,
        turn_id: String,
        response: PermissionResponse,
        owner_access: bool,
        from_runtime_state: bool,
        state: ObligationStateRecord,
    },
    StopInterrupt {
        operation_id: String,
        session_id: String,
        turn_id: String,
        expected_revision: u64,
        deadline_ms: i64,
        state: ObligationStateRecord,
    },
    SessionClose {
        obligation_id: String,
        operation_id: String,
        session_id: String,
        action: SessionLifecycleRecordAction,
        state: ObligationStateRecord,
    },
    BackendSessionRecovery {
        session_id: String,
        recovery_id: String,
        detail: BackendSessionRecoveryObligationRecord,
        state: ObligationStateRecord,
    },
    WorkflowShutdown {
        operation_id: String,
        effect_identity: String,
        owner_revision: i64,
        execution_id: String,
        state: ObligationStateRecord,
    },
    WorkflowTurnCompletion {
        session_id: String,
        turn_id: String,
        terminal_identity: String,
        notification_sha256: [u8; 32],
        detail: WorkflowTurnCompletionObligationRecord,
        state: ObligationStateRecord,
    },
    RecoveryPublication {
        session_id: String,
        recovery_id: String,
        message_id: String,
        source_obligation_id: String,
        detail: RecoveryPublicationObligationRecord,
        state: ObligationStateRecord,
    },
    ProviderEstablish {
        operation_id: String,
        effect_identity: String,
        session_id: String,
        state: ObligationStateRecord,
    },
    TurnExecution {
        operation_id: String,
        session_id: String,
        turn_id: String,
        state: ObligationStateRecord,
    },
    TerminalCommit {
        operation_id: String,
        session_id: String,
        turn_id: String,
        terminal_identity: String,
        state: ObligationStateRecord,
    },
    RecoveryReserved {
        recovery_id: String,
        effect_identity: String,
        state: ObligationStateRecord,
    },
    RecoveryCompleted {
        recovery_id: String,
        effect_identity: String,
        classification: RecoveryResultClassification,
        state: ObligationStateRecord,
    },
    FeedbackReservation {
        feedback_id: String,
        attempt_id: String,
        session_id: String,
        operation: AgentSessionNoticeOperationRecord,
        process_instance_id: String,
    },
    Feedback {
        feedback_id: String,
        attempt_id: String,
        session_id: String,
        operation: AgentSessionNoticeOperationRecord,
        actions: Vec<FeedbackActionRecord>,
        resolution_identity: Option<String>,
        failure: SafeOperationFailure,
    },
    WorkflowExecution {
        execution: WorkflowExecutionMetadataRecord,
    },
    RecoveryTransition {
        original: Box<ObligationRecord>,
        recovery_action: ObligationRecoveryActionRecord,
    },
    Observed {
        original: Box<ObligationRecord>,
        observation: AuthoritativeEffectObservationRecord,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendSessionRecoveryObligationRecord {
    EffectReserved {
        old_provider_session_generation: u64,
        reason: BackendSessionRecoveryReason,
        reserved_at_bits: u64,
    },
    Completed {
        old_provider_session_generation: u64,
        provider_session_generation: u64,
        backend_session_id: String,
        completed_at_bits: u64,
    },
    Failed {
        error_sha256: [u8; 32],
        failed_at_bits: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowTurnCompletionObligationRecord {
    Pending {
        workflow_context: Box<WorkflowNodeContext>,
        message_id: String,
        exit_code: i64,
        failure_signal: Option<WorkflowTurnFailureSignalRecord>,
        token_usage: Option<TurnTokenUsage>,
        interrupted: bool,
    },
    Applied {
        settled_at_bits: u64,
    },
    AlreadyApplied {
        settled_at_bits: u64,
    },
    Retired {
        reason: WorkflowObligationRetirementReason,
        settled_at_bits: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowObligationRetirementReason {
    Superseded,
    Unrecoverable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowObligationTerminalOutcome {
    Applied,
    AlreadyApplied,
    Retired(WorkflowObligationRetirementReason),
}

impl WorkflowTurnCompletionObligationRecord {
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending { .. })
    }

    pub fn terminal_outcome(&self) -> Option<WorkflowObligationTerminalOutcome> {
        match self {
            Self::Pending { .. } => None,
            Self::Applied { .. } => Some(WorkflowObligationTerminalOutcome::Applied),
            Self::AlreadyApplied { .. } => Some(WorkflowObligationTerminalOutcome::AlreadyApplied),
            Self::Retired { reason, .. } => {
                Some(WorkflowObligationTerminalOutcome::Retired(*reason))
            }
        }
    }

    pub fn settle(
        &self,
        outcome: WorkflowObligationTerminalOutcome,
        settled_at_bits: u64,
    ) -> Result<Self, WorkflowObligationTerminalOutcome> {
        if let Some(existing) = self.terminal_outcome() {
            return Err(existing);
        }
        Ok(match outcome {
            WorkflowObligationTerminalOutcome::Applied => Self::Applied { settled_at_bits },
            WorkflowObligationTerminalOutcome::AlreadyApplied => {
                Self::AlreadyApplied { settled_at_bits }
            }
            WorkflowObligationTerminalOutcome::Retired(reason) => Self::Retired {
                reason,
                settled_at_bits,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryPublicationObligationRecord {
    Pending {
        pending_message: RecoveryPublicationMessageRecord,
    },
    Completed {
        published_at_bits: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAttemptRecord {
    Obligation {
        obligation_id: String,
        origin_revision: u64,
        action: RecoveryActionKind,
        effect_identity: String,
        state: ObligationStateRecord,
        failure: Option<SafeOperationFailure>,
    },
    ShutdownTarget {
        resource_ref: String,
        plan: ShutdownPlanKey,
        ordinal: i64,
        target_key: String,
        origin_revision: u64,
        action: RecoveryActionKind,
        effect_identity_sha256: [u8; 32],
        intent: QuitIntent,
        state: ObligationStateRecord,
        failure: Option<SafeOperationFailure>,
    },
    FeedbackRetry {
        feedback_id: String,
        origin_revision: u64,
        resolution_identity: String,
        state: ObligationStateRecord,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryResultOutcomeRecord {
    Pending,
    Terminal,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Closed recovery read-model vocabulary includes variants adapters may return later.
pub enum RecoveryResourceViewRecord {
    Operation {
        kind: OperationKind,
        operation_id: String,
    },
    Session {
        session_id: String,
    },
    BackendRecovery {
        session_id: String,
        recovery_id: String,
    },
    ShutdownTarget {
        plan: ShutdownPlanKey,
        ordinal: i64,
        target_id: String,
        state: ShutdownTargetStateRecord,
    },
    SafeSummary(String),
    ReconciliationRequired {
        failure: SafeOperationFailure,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryActionResultRecord {
    pub outcome: RecoveryResultOutcomeRecord,
    pub classification: RecoveryResultClassification,
    pub resource_revision: u64,
    pub canonical_result_sha256: [u8; 32],
    pub resource_view: RecoveryResourceViewRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryResultRecord {
    Action(RecoveryActionResultRecord),
    FeedbackRetry {
        feedback_id: String,
        resource_revision: u64,
        resolved: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownTargetKindRecord {
    AgentSession,
    WorkflowExecution,
    WorkflowNode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownTargetStateRecord {
    Prepared,
    EffectReserved,
    Completed,
    Failed,
    ReconciliationRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownOutcomeRecord {
    Completed,
    AbortedBeforeActivation,
    ReconciliationRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownPlanRecord {
    pub operation_id: String,
    pub intent: QuitIntent,
    pub t0_ms: i64,
    pub preparation_cutoff_ms: Option<i64>,
    pub deadline_ms: i64,
    pub target_count: Option<u64>,
    pub prepared_count: Option<u64>,
    pub effect_reserved_count: Option<u64>,
    pub terminal_count: Option<u64>,
    pub completed_count: Option<u64>,
    pub unresolved_count: Option<u64>,
    pub recovery_snapshot_count: Option<u64>,
    pub recovery_snapshot_id: Option<String>,
    pub process_instance_id: String,
    pub outcome: Option<ShutdownOutcomeRecord>,
    pub failure: Option<SafeOperationFailure>,
    pub shutdown_effect_count: Option<u64>,
    pub admission_open: Option<bool>,
    pub retry_quit_same_boot: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownTargetRecoveryRecord {
    pub action_id: String,
    pub origin_revision: u64,
    pub action: RecoveryActionKind,
    pub state: ObligationStateRecord,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShutdownTargetRecord {
    Target {
        target_id: String,
        kind: ShutdownTargetKindRecord,
        state: ShutdownTargetStateRecord,
        effect_identity: String,
        owner_operation_id: Option<String>,
        failure: Option<SafeOperationFailure>,
        recovery_action: Option<ShutdownTargetRecoveryRecord>,
    },
    RecoverySnapshot {
        obligation_id: String,
        ordered_key: String,
        owner: String,
        revision: u64,
        record: Box<ObligationRecord>,
    },
}

fn identity_field(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
    bytes.extend_from_slice(value);
}

fn identity_text(bytes: &mut Vec<u8>, value: &str) {
    identity_field(bytes, value.as_bytes());
}

fn identity_optional_text(bytes: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(value) => {
            bytes.push(1);
            identity_text(bytes, value);
        }
        None => bytes.push(0),
    }
}

fn identity_token_usage(bytes: &mut Vec<u8>, value: &TokenUsage) {
    bytes.extend_from_slice(&value.input_tokens.to_be_bytes());
    bytes.extend_from_slice(&value.output_tokens.to_be_bytes());
    for optional in [value.total_tokens, value.context_window_tokens] {
        match optional {
            Some(value) => {
                bytes.push(1);
                bytes.extend_from_slice(&value.to_be_bytes());
            }
            None => bytes.push(0),
        }
    }
}

fn identity_turn_result(bytes: &mut Vec<u8>, value: &TurnResult) {
    match value {
        TurnResult::Completed {
            stop_reason,
            token_usage,
        } => {
            identity_text(bytes, "completed");
            match stop_reason {
                Some(TurnStopReason::Refusal) => identity_optional_text(bytes, Some("refusal")),
                None => identity_optional_text(bytes, None),
            }
            match token_usage {
                Some(value) => {
                    bytes.push(1);
                    identity_token_usage(bytes, value);
                }
                None => bytes.push(0),
            }
        }
        TurnResult::Failed { error, token_usage } => {
            identity_text(bytes, "failed");
            identity_text(bytes, error);
            match token_usage {
                Some(value) => {
                    bytes.push(1);
                    identity_token_usage(bytes, value);
                }
                None => bytes.push(0),
            }
        }
        TurnResult::Interrupted { reason, error } => {
            identity_text(bytes, "interrupted");
            identity_text(
                bytes,
                match reason {
                    InterruptReason::Abort => "abort",
                    InterruptReason::Timeout => "timeout",
                    InterruptReason::Crash => "crash",
                    InterruptReason::SessionClosed => "session_closed",
                },
            );
            identity_optional_text(bytes, error.as_deref());
        }
    }
}

fn identity_obligation_state(bytes: &mut Vec<u8>, value: ObligationStateRecord) {
    identity_text(
        bytes,
        match value {
            ObligationStateRecord::Prepared => "prepared",
            ObligationStateRecord::Pending => "pending",
            ObligationStateRecord::EffectReserved => "effect_reserved",
            ObligationStateRecord::Running => "running",
            ObligationStateRecord::WaitingApproval => "waiting_approval",
            ObligationStateRecord::OutcomeUnknown => "outcome_unknown",
            ObligationStateRecord::ReconciliationRequired => "reconciliation_required",
            ObligationStateRecord::Failed => "failed",
            ObligationStateRecord::Completed => "completed",
            ObligationStateRecord::Cancelled => "cancelled",
        },
    );
}

fn identity_recovery_action(bytes: &mut Vec<u8>, value: RecoveryActionKind) {
    identity_text(
        bytes,
        match value {
            RecoveryActionKind::ReadAgain => "read_again",
            RecoveryActionKind::RetrySameEffect => "retry_same_effect",
            RecoveryActionKind::UseObservedResult => "use_observed_result",
            RecoveryActionKind::CancelIfSafe => "cancel_if_safe",
            RecoveryActionKind::KeepForManualResolution => "keep_for_manual_resolution",
        },
    );
}

fn identity_classification(bytes: &mut Vec<u8>, value: RecoveryResultClassification) {
    identity_text(
        bytes,
        match value {
            RecoveryResultClassification::Pending => "pending",
            RecoveryResultClassification::Succeeded => "succeeded",
            RecoveryResultClassification::ConfirmedNoEffect => "confirmed_no_effect",
            RecoveryResultClassification::Ambiguous => "ambiguous",
            RecoveryResultClassification::CancelledBeforeEffect => "cancelled_before_effect",
            RecoveryResultClassification::Unchanged => "unchanged",
        },
    );
}

fn identity_failure(bytes: &mut Vec<u8>, value: &SafeOperationFailure) {
    // The explicit discriminant is stable even if Debug names are changed.
    bytes.push(match value.kind {
        super::SessionOperationFailureKind::StorageUnavailable => 0,
        super::SessionOperationFailureKind::StorageCorrupt => 1,
        super::SessionOperationFailureKind::PersistFailure => 3,
        super::SessionOperationFailureKind::ProtocolIncompatible => 4,
        super::SessionOperationFailureKind::ProviderUnavailable => 5,
        super::SessionOperationFailureKind::ExternalEffectFailed => 6,
        super::SessionOperationFailureKind::OutcomeUnknown => 7,
        super::SessionOperationFailureKind::DeadlineExceeded => 8,
        super::SessionOperationFailureKind::CapacityExceeded => 9,
        super::SessionOperationFailureKind::StopCapacityExceeded => 10,
        super::SessionOperationFailureKind::ShutdownAuthorityMismatch => 11,
        super::SessionOperationFailureKind::TargetRevisionChanged => 12,
        super::SessionOperationFailureKind::OwnerRevisionChanged => 13,
        super::SessionOperationFailureKind::RuntimeGenerationChanged => 14,
        super::SessionOperationFailureKind::InvalidEffectIntent => 15,
        super::SessionOperationFailureKind::PreviousShutdownReconciliationRequired => 16,
        super::SessionOperationFailureKind::Internal => 18,
    });
    bytes.push(u8::from(value.retryable));
    identity_text(bytes, value.label.value());
    identity_optional_text(bytes, value.detail.as_ref().map(|value| value.value()));
    identity_text(bytes, &value.correlation_id);
}

impl TerminalResultRecord {
    pub(crate) fn write_canonical_identity_v1(
        &self,
        bytes: &mut Vec<u8>,
    ) -> Result<(), &'static str> {
        match self {
            Self::AgentTurn {
                kind,
                session_id,
                turn_id,
                message_id,
                streaming_final_sequence,
                completed_at_bits,
                result,
            } => {
                identity_text(bytes, "agent_turn");
                bytes.push(match kind {
                    AgentTerminalKind::Completed => 0,
                    AgentTerminalKind::Abort => 1,
                    AgentTerminalKind::Timeout => 2,
                    AgentTerminalKind::Crash => 3,
                    AgentTerminalKind::SessionClosed => 4,
                });
                identity_text(bytes, session_id);
                identity_text(bytes, turn_id);
                identity_text(bytes, message_id);
                bytes.extend_from_slice(&streaming_final_sequence.to_be_bytes());
                bytes.extend_from_slice(&completed_at_bits.to_be_bytes());
                let AgentTurnTerminalResultRecord::Current(result) = result;
                identity_turn_result(bytes, result);
            }
            Self::SessionClosed {
                operation_id,
                reason,
                result,
            } => {
                identity_text(bytes, "session_closed");
                identity_text(bytes, operation_id);
                bytes.push(*reason as u8);
                identity_turn_result(bytes, result);
            }
            Self::Stop {
                operation_id,
                reason,
                exit_code,
                result,
            } => {
                identity_text(bytes, "stop");
                identity_text(bytes, operation_id);
                match reason {
                    Some(value) => {
                        bytes.push(1);
                        bytes.push(*value as u8);
                    }
                    None => bytes.push(0),
                }
                match exit_code {
                    Some(value) => {
                        bytes.push(1);
                        bytes.extend_from_slice(&value.to_be_bytes());
                    }
                    None => bytes.push(0),
                }
                identity_turn_result(bytes, result);
            }
            Self::StopSuperseded {
                terminal_identity,
                terminal_result_sha256,
            } => {
                identity_text(bytes, "stop_superseded");
                identity_text(bytes, terminal_identity);
                identity_field(bytes, terminal_result_sha256);
            }
        }
        Ok(())
    }
}

impl ObligationRecord {
    pub(crate) fn original(&self) -> &Self {
        match self {
            Self::RecoveryTransition { original, .. } | Self::Observed { original, .. } => {
                original.original()
            }
            _ => self,
        }
    }

    pub(crate) fn state(&self) -> Option<ObligationStateRecord> {
        match self.original() {
            Self::Send { state, .. }
            | Self::PermissionResponse { state, .. }
            | Self::StopInterrupt { state, .. }
            | Self::SessionClose { state, .. }
            | Self::BackendSessionRecovery { state, .. }
            | Self::WorkflowShutdown { state, .. }
            | Self::WorkflowTurnCompletion { state, .. }
            | Self::RecoveryPublication { state, .. }
            | Self::ProviderEstablish { state, .. }
            | Self::TurnExecution { state, .. }
            | Self::TerminalCommit { state, .. }
            | Self::RecoveryReserved { state, .. }
            | Self::RecoveryCompleted { state, .. } => Some(*state),
            Self::FeedbackReservation { .. }
            | Self::Feedback { .. }
            | Self::WorkflowExecution { .. }
            | Self::RecoveryTransition { .. }
            | Self::Observed { .. } => None,
        }
    }

    fn recovery_action(&self) -> Option<&ObligationRecoveryActionRecord> {
        match self {
            Self::RecoveryTransition {
                recovery_action, ..
            } => Some(recovery_action),
            Self::Observed { original, .. } => original.recovery_action(),
            _ => None,
        }
    }

    /// Canonical owner-level fence for starting a new external effect.
    ///
    /// Ordinary live work remains queueable. Explicitly ambiguous work,
    /// recovery-owned handoffs, and lifecycle closure remain blockers until
    /// their exact durable identity is resolved.
    pub(crate) fn blocks_effect_admission(&self) -> bool {
        let original = self.original();
        if matches!(original, Self::FeedbackReservation { .. }) {
            return false;
        }
        let state = self.state();
        let action_unresolved = self.recovery_action().is_some_and(|action| {
            matches!(
                action.state,
                ObligationStateRecord::Prepared
                    | ObligationStateRecord::EffectReserved
                    | ObligationStateRecord::OutcomeUnknown
                    | ObligationStateRecord::ReconciliationRequired
            )
        });
        let explicitly_unresolved = matches!(
            state,
            Some(
                ObligationStateRecord::ReconciliationRequired
                    | ObligationStateRecord::Failed
                    | ObligationStateRecord::OutcomeUnknown
            )
        );
        let recovery_owned = match original {
            Self::WorkflowTurnCompletion { detail, .. } => detail.is_pending(),
            Self::WorkflowShutdown { state, .. } => !matches!(
                state,
                ObligationStateRecord::Completed | ObligationStateRecord::Cancelled
            ),
            Self::BackendSessionRecovery { .. }
            | Self::RecoveryPublication { .. }
            | Self::RecoveryReserved { .. }
            | Self::RecoveryCompleted { .. } => true,
            _ => false,
        };
        let closing = matches!(original, Self::SessionClose { .. })
            && state != Some(ObligationStateRecord::Completed);
        // ProviderEstablish belongs to the superseded two-flight Send
        // protocol. An EffectReserved row proves that an older process
        // crossed that protocol's external-effect boundary, so no other send
        // effect may be claimed until startup normalization either retires it
        // with backend-specific proof or leaves it for reconciliation.
        let legacy_provider_establish_reserved = matches!(
            original,
            Self::Send {
                kind: SendObligationKindRecord::ProviderEstablish,
                state: ObligationStateRecord::EffectReserved,
                ..
            } | Self::ProviderEstablish {
                state: ObligationStateRecord::EffectReserved,
                ..
            }
        );
        let known_live = matches!(
            original,
            Self::Send { .. }
                | Self::ProviderEstablish { .. }
                | Self::TurnExecution { .. }
                | Self::PermissionResponse { .. }
                | Self::StopInterrupt { .. }
                | Self::TerminalCommit { .. }
                | Self::SessionClose { .. }
                | Self::WorkflowShutdown { .. }
                | Self::WorkflowTurnCompletion { .. }
        );
        let blocks = action_unresolved
            || explicitly_unresolved
            || recovery_owned
            || closing
            || legacy_provider_establish_reserved;
        blocks || !known_live
    }

    pub(crate) fn write_canonical_identity_v1(
        &self,
        bytes: &mut Vec<u8>,
    ) -> Result<(), &'static str> {
        match self {
            Self::RecoveryTransition {
                original,
                recovery_action,
            } => {
                identity_text(bytes, "recovery_transition");
                original.write_canonical_identity_v1(bytes)?;
                identity_text(bytes, &recovery_action.action_id);
                bytes.extend_from_slice(&recovery_action.origin_revision.to_be_bytes());
                identity_recovery_action(bytes, recovery_action.action);
                identity_text(bytes, &recovery_action.effect_identity);
                identity_obligation_state(bytes, recovery_action.state);
                match recovery_action.classification {
                    Some(value) => {
                        bytes.push(1);
                        identity_classification(bytes, value);
                    }
                    None => bytes.push(0),
                }
            }
            Self::Observed {
                original,
                observation,
            } => {
                identity_text(bytes, "observed");
                original.write_canonical_identity_v1(bytes)?;
                identity_text(bytes, &observation.effect_identity);
                bytes.extend_from_slice(&observation.origin_revision.to_be_bytes());
                identity_classification(bytes, observation.classification);
                bytes.push(u8::from(observation.cancellable));
                identity_text(bytes, &observation.safe_view);
                identity_field(bytes, &observation.result_sha256);
                identity_field(bytes, &observation.proof_mac);
            }
            Self::Send {
                obligation_id,
                operation_id,
                session_id,
                kind,
                disposition,
                human_message_id,
                assistant_message_id,
                reserved_turn_id,
                turn_id,
                dependency_obligation_ids,
                canonical_payload,
                state,
            } => {
                identity_text(bytes, "send");
                for value in [obligation_id, operation_id, session_id] {
                    identity_text(bytes, value);
                }
                bytes.push(match kind {
                    SendObligationKindRecord::ProviderEstablish => 0,
                    SendObligationKindRecord::TurnExecution => 1,
                });
                bytes.push(match disposition {
                    SendObligationDispositionRecord::StartedTurn => 0,
                    SendObligationDispositionRecord::Queued => 1,
                });
                for value in [
                    human_message_id.as_deref(),
                    assistant_message_id.as_deref(),
                    reserved_turn_id.as_deref(),
                    turn_id.as_deref(),
                ] {
                    identity_optional_text(bytes, value);
                }
                bytes.extend_from_slice(&(dependency_obligation_ids.len() as u64).to_be_bytes());
                for dependency in dependency_obligation_ids {
                    identity_text(bytes, dependency);
                }
                identity_text(bytes, canonical_payload);
                identity_obligation_state(bytes, *state);
            }
            Self::PermissionResponse {
                operation_id,
                effect_identity,
                session_id,
                turn_id,
                response,
                owner_access,
                from_runtime_state,
                state,
            } => {
                identity_text(bytes, "permission_response");
                for value in [
                    operation_id,
                    effect_identity,
                    session_id,
                    turn_id,
                    &response.request_id,
                ] {
                    identity_text(bytes, value);
                }
                match &response.decision {
                    PermissionResponseDecision::Allow {
                        updated_input,
                        answers,
                    } => {
                        bytes.push(0);
                        identity_optional_text(
                            bytes,
                            updated_input.as_ref().map(|value| value.as_str()),
                        );
                        identity_optional_text(bytes, answers.as_ref().map(|value| value.as_str()));
                    }
                    PermissionResponseDecision::Deny { message } => {
                        bytes.push(1);
                        identity_optional_text(bytes, message.as_deref());
                    }
                }
                bytes.push(u8::from(*owner_access));
                bytes.push(u8::from(*from_runtime_state));
                identity_obligation_state(bytes, *state);
            }
            Self::StopInterrupt {
                operation_id,
                session_id,
                turn_id,
                expected_revision,
                deadline_ms,
                state,
            } => {
                identity_text(bytes, "stop_interrupt");
                for value in [operation_id, session_id, turn_id] {
                    identity_text(bytes, value);
                }
                bytes.extend_from_slice(&expected_revision.to_be_bytes());
                bytes.extend_from_slice(&deadline_ms.to_be_bytes());
                identity_obligation_state(bytes, *state);
            }
            Self::SessionClose {
                obligation_id,
                operation_id,
                session_id,
                action,
                state,
            } => {
                identity_text(bytes, "session_close");
                for value in [obligation_id, operation_id, session_id] {
                    identity_text(bytes, value);
                }
                match action {
                    SessionLifecycleRecordAction::Close => bytes.push(0),
                    SessionLifecycleRecordAction::ArchiveOpen => bytes.push(1),
                    SessionLifecycleRecordAction::ArchiveClosed => bytes.push(2),
                    SessionLifecycleRecordAction::SwitchBackend { backend_id } => {
                        bytes.push(3);
                        identity_text(bytes, backend_id);
                    }
                }
                identity_obligation_state(bytes, *state);
            }
            Self::BackendSessionRecovery {
                session_id,
                recovery_id,
                detail,
                state,
            } => {
                identity_text(bytes, "backend_session_recovery");
                identity_text(bytes, session_id);
                identity_text(bytes, recovery_id);
                match detail {
                    BackendSessionRecoveryObligationRecord::EffectReserved {
                        old_provider_session_generation,
                        reason,
                        reserved_at_bits,
                    } => {
                        bytes.push(0);
                        bytes.extend_from_slice(&old_provider_session_generation.to_be_bytes());
                        bytes.push(match reason {
                            BackendSessionRecoveryReason::ResumeMismatch => 0,
                            BackendSessionRecoveryReason::BackendSessionLost => 1,
                        });
                        bytes.extend_from_slice(&reserved_at_bits.to_be_bytes());
                    }
                    BackendSessionRecoveryObligationRecord::Completed {
                        old_provider_session_generation,
                        provider_session_generation,
                        backend_session_id,
                        completed_at_bits,
                    } => {
                        bytes.push(1);
                        bytes.extend_from_slice(&old_provider_session_generation.to_be_bytes());
                        bytes.extend_from_slice(&provider_session_generation.to_be_bytes());
                        identity_text(bytes, backend_session_id);
                        bytes.extend_from_slice(&completed_at_bits.to_be_bytes());
                    }
                    BackendSessionRecoveryObligationRecord::Failed {
                        error_sha256,
                        failed_at_bits,
                    } => {
                        bytes.push(2);
                        identity_field(bytes, error_sha256);
                        bytes.extend_from_slice(&failed_at_bits.to_be_bytes());
                    }
                }
                identity_obligation_state(bytes, *state);
            }
            Self::WorkflowShutdown {
                operation_id,
                effect_identity,
                owner_revision,
                execution_id,
                state,
            } => {
                identity_text(bytes, "workflow_shutdown");
                for value in [operation_id, effect_identity, execution_id] {
                    identity_text(bytes, value);
                }
                bytes.extend_from_slice(&owner_revision.to_be_bytes());
                identity_obligation_state(bytes, *state);
            }
            Self::WorkflowTurnCompletion {
                session_id,
                turn_id,
                terminal_identity,
                notification_sha256,
                detail,
                state,
            } => {
                identity_text(bytes, "workflow_turn_completion");
                for value in [session_id, turn_id, terminal_identity] {
                    identity_text(bytes, value);
                }
                identity_field(bytes, notification_sha256);
                match detail {
                    WorkflowTurnCompletionObligationRecord::Pending {
                        workflow_context,
                        message_id,
                        exit_code,
                        failure_signal,
                        token_usage,
                        interrupted,
                    } => {
                        bytes.push(0);
                        for value in [
                            &workflow_context.execution_id,
                            &workflow_context.node_execution_id,
                            &workflow_context.workflow_name,
                            &workflow_context.node_name,
                        ] {
                            identity_text(bytes, value);
                        }
                        bytes.extend_from_slice(&workflow_context.attempt.to_be_bytes());
                        identity_optional_text(bytes, workflow_context.parent_node_name.as_deref());
                        match workflow_context.parent_attempt {
                            Some(value) => {
                                bytes.push(1);
                                bytes.extend_from_slice(&value.to_be_bytes());
                            }
                            None => bytes.push(0),
                        }
                        bytes.extend_from_slice(&workflow_context.order.to_be_bytes());
                        for value in [
                            workflow_context.startup_timeout_secs,
                            workflow_context.stale_timeout_secs,
                        ] {
                            match value {
                                Some(value) => {
                                    bytes.push(1);
                                    bytes.extend_from_slice(&value.to_be_bytes());
                                }
                                None => bytes.push(0),
                            }
                        }
                        match workflow_context.startup_max_retries {
                            Some(value) => {
                                bytes.push(1);
                                bytes.extend_from_slice(&value.to_be_bytes());
                            }
                            None => bytes.push(0),
                        }
                        identity_text(bytes, message_id);
                        bytes.extend_from_slice(&exit_code.to_be_bytes());
                        bytes.push(u8::from(failure_signal.is_some()));
                        match token_usage {
                            Some(value) => {
                                bytes.push(1);
                                bytes.extend_from_slice(&value.input_tokens.to_be_bytes());
                                bytes.extend_from_slice(&value.output_tokens.to_be_bytes());
                            }
                            None => bytes.push(0),
                        }
                        bytes.push(u8::from(*interrupted));
                    }
                    WorkflowTurnCompletionObligationRecord::Applied { settled_at_bits } => {
                        bytes.push(1);
                        bytes.extend_from_slice(&settled_at_bits.to_be_bytes());
                    }
                    WorkflowTurnCompletionObligationRecord::AlreadyApplied { settled_at_bits } => {
                        bytes.push(2);
                        bytes.extend_from_slice(&settled_at_bits.to_be_bytes());
                    }
                    WorkflowTurnCompletionObligationRecord::Retired {
                        reason,
                        settled_at_bits,
                    } => {
                        bytes.push(3);
                        bytes.push(match reason {
                            WorkflowObligationRetirementReason::Superseded => 0,
                            WorkflowObligationRetirementReason::Unrecoverable => 1,
                        });
                        bytes.extend_from_slice(&settled_at_bits.to_be_bytes());
                    }
                }
                identity_obligation_state(bytes, *state);
            }
            Self::RecoveryPublication {
                session_id,
                recovery_id,
                message_id,
                source_obligation_id,
                detail,
                state,
            } => {
                identity_text(bytes, "recovery_publication");
                for value in [session_id, recovery_id, message_id, source_obligation_id] {
                    identity_text(bytes, value);
                }
                match detail {
                    RecoveryPublicationObligationRecord::Pending { pending_message } => {
                        bytes.push(0);
                        bytes.push(match pending_message.kind {
                            RecoveryPublicationMessageKindRecord::Notice => 0,
                            RecoveryPublicationMessageKindRecord::Error => 1,
                        });
                        identity_text(bytes, &pending_message.recovery_id);
                        identity_text(bytes, &pending_message.message_id);
                        identity_optional_text(bytes, pending_message.error.as_deref());
                    }
                    RecoveryPublicationObligationRecord::Completed { published_at_bits } => {
                        bytes.push(1);
                        bytes.extend_from_slice(&published_at_bits.to_be_bytes());
                    }
                }
                identity_obligation_state(bytes, *state);
            }
            Self::ProviderEstablish {
                operation_id,
                effect_identity,
                session_id,
                state,
            } => {
                identity_text(bytes, "provider_establish");
                for value in [operation_id, effect_identity, session_id] {
                    identity_text(bytes, value);
                }
                identity_obligation_state(bytes, *state);
            }
            Self::TurnExecution {
                operation_id,
                session_id,
                turn_id,
                state,
            } => {
                identity_text(bytes, "turn_execution");
                for value in [operation_id, session_id, turn_id] {
                    identity_text(bytes, value);
                }
                identity_obligation_state(bytes, *state);
            }
            Self::TerminalCommit {
                operation_id,
                session_id,
                turn_id,
                terminal_identity,
                state,
            } => {
                identity_text(bytes, "terminal_commit");
                for value in [operation_id, session_id, turn_id, terminal_identity] {
                    identity_text(bytes, value);
                }
                identity_obligation_state(bytes, *state);
            }
            Self::RecoveryReserved {
                recovery_id,
                effect_identity,
                state,
            } => {
                identity_text(bytes, "recovery_reserved");
                identity_text(bytes, recovery_id);
                identity_text(bytes, effect_identity);
                identity_obligation_state(bytes, *state);
            }
            Self::RecoveryCompleted {
                recovery_id,
                effect_identity,
                classification,
                state,
            } => {
                identity_text(bytes, "recovery_completed");
                identity_text(bytes, recovery_id);
                identity_text(bytes, effect_identity);
                identity_classification(bytes, *classification);
                identity_obligation_state(bytes, *state);
            }
            Self::FeedbackReservation {
                feedback_id,
                attempt_id,
                session_id,
                operation,
                process_instance_id,
            } => {
                identity_text(bytes, "feedback_reservation");
                for value in [feedback_id, attempt_id, session_id, process_instance_id] {
                    identity_text(bytes, value);
                }
                bytes.push(*operation as u8);
            }
            Self::Feedback {
                feedback_id,
                attempt_id,
                session_id,
                operation,
                actions,
                resolution_identity,
                failure,
            } => {
                identity_text(bytes, "feedback");
                for value in [feedback_id, attempt_id, session_id] {
                    identity_text(bytes, value);
                }
                bytes.push(*operation as u8);
                bytes.extend_from_slice(&(actions.len() as u64).to_be_bytes());
                for action in actions {
                    bytes.push(*action as u8);
                }
                identity_optional_text(bytes, resolution_identity.as_deref());
                identity_failure(bytes, failure);
            }
            Self::WorkflowExecution { execution } => {
                identity_text(bytes, "workflow_execution");
                for value in [
                    &execution.execution_id,
                    &execution.workflow_name,
                    &execution.worktree_path,
                ] {
                    identity_text(bytes, value);
                }
                identity_text(bytes, execution.status.as_str());
                identity_optional_text(bytes, execution.current_node.as_deref());
                identity_text(bytes, execution.created_from.as_public_value());
                bytes.extend_from_slice(&execution.started_at_bits.to_be_bytes());
                bytes.extend_from_slice(&execution.updated_at_bits.to_be_bytes());
                match execution.completed_at_bits {
                    Some(value) => {
                        bytes.push(1);
                        bytes.extend_from_slice(&value.to_be_bytes());
                    }
                    None => bytes.push(0),
                }
                identity_optional_text(bytes, execution.error_reason.as_deref());
                identity_optional_text(
                    bytes,
                    execution.interruption_reason.map(|value| value.as_str()),
                );
                identity_optional_text(bytes, execution.resume_from_node.as_deref());
                bytes.extend_from_slice(&execution.total_token_usage.input_tokens.to_be_bytes());
                bytes.extend_from_slice(&execution.total_token_usage.output_tokens.to_be_bytes());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod workflow_obligation_tests {
    use super::*;

    fn pending() -> WorkflowTurnCompletionObligationRecord {
        WorkflowTurnCompletionObligationRecord::Pending {
            workflow_context: Box::new(WorkflowNodeContext {
                execution_id: "execution-1".to_string(),
                node_execution_id: "node-execution-1".to_string(),
                workflow_name: "workflow".to_string(),
                node_name: "node".to_string(),
                attempt: 1,
                parent_node_name: None,
                parent_attempt: None,
                order: 0,
                startup_timeout_secs: None,
                startup_max_retries: None,
                stale_timeout_secs: None,
            }),
            message_id: "message-1".to_string(),
            exit_code: 0,
            failure_signal: None,
            token_usage: None,
            interrupted: false,
        }
    }

    #[test]
    fn workflow_turn_completion_obligation_has_closed_terminal_transitions() {
        for outcome in [
            WorkflowObligationTerminalOutcome::Applied,
            WorkflowObligationTerminalOutcome::AlreadyApplied,
            WorkflowObligationTerminalOutcome::Retired(
                WorkflowObligationRetirementReason::Superseded,
            ),
            WorkflowObligationTerminalOutcome::Retired(
                WorkflowObligationRetirementReason::Unrecoverable,
            ),
        ] {
            let terminal = pending().settle(outcome, 42).expect("pending settles");
            assert_eq!(terminal.terminal_outcome(), Some(outcome));
            assert!(!terminal.is_pending());
            assert_eq!(terminal.settle(outcome, 43), Err(outcome));
        }
    }

    #[test]
    fn terminal_workflow_turn_completion_no_longer_blocks_effect_admission() {
        let record = |detail| ObligationRecord::WorkflowTurnCompletion {
            session_id: "session-1".to_string(),
            turn_id: "1".to_string(),
            terminal_identity: "terminal-1".to_string(),
            notification_sha256: [1; 32],
            detail,
            state: ObligationStateRecord::Completed,
        };
        assert!(!record(
            pending()
                .settle(WorkflowObligationTerminalOutcome::Applied, 42)
                .unwrap()
        )
        .blocks_effect_admission());
        assert!(!record(
            pending()
                .settle(
                    WorkflowObligationTerminalOutcome::Retired(
                        WorkflowObligationRetirementReason::Unrecoverable,
                    ),
                    42,
                )
                .unwrap()
        )
        .blocks_effect_admission());
    }
}
