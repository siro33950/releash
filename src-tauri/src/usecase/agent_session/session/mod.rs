pub(crate) mod errors;
pub(crate) mod image_attachment;
pub(crate) mod lifecycle_controller;
mod message_window;
mod open_tabs;
mod prompt_suggestion;
mod read_paths;
mod store;
mod stored_lifecycle;
mod stream_resync;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::domain::agent_session::services::{
    DefaultToolOutputExternalizationPolicy, ToolOutputExternalizationPolicy,
};
use crate::domain::workflow::WorkflowStepContext;
use crate::usecase::agent_session::context_meta::ContextEpochMeta;

pub use crate::usecase::agent_session::status::TurnPhase;
pub(crate) use image_attachment::validate_image_bytes;
pub(crate) use message_window::{
    plan_agent_chat_eviction, AgentChatEvictionPlan, AgentChatEvictionPlanRequest,
};
pub use open_tabs::OpenTabRegistry;
pub(crate) use prompt_suggestion::{
    AgentPromptGitStatusGateway, AgentPromptSuggestion, AgentPromptSuggestionUsecase,
    GitSuggestionContext,
};
pub(crate) use read_paths::{
    agent_read_paths_from_message, agent_read_paths_from_messages, agent_read_paths_from_parts,
    merge_agent_read_paths,
};
pub use store::{SessionReaderPort, SessionReviewContextReader, SessionStore};
pub(crate) use stored_lifecycle::{
    AgentSessionRuntimeCloser, CodexThreadForkRequest, CodexThreadLifecycleGateway,
    StoredSessionLifecycleUsecase,
};
pub(crate) use stream_resync::{
    resync_streaming_message, AgentStreamResyncReadModel, StreamResyncSnapshot,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoListItem {
    pub text: String,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemNotificationType {
    Compaction,
}

impl SystemNotificationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Compaction => "compaction",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePart {
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
        content_ref: Option<ToolOutputRef>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        summary: Option<ToolOutputSummary>,
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
        items: Vec<TodoListItem>,
    },
    SystemNotification {
        #[serde(rename = "notificationType")]
        notification_type: SystemNotificationType,
        status: String,
        label: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default, rename = "hookId")]
        hook_id: Option<String>,
    },
    Image {
        /// Base64-encoded image data
        data: String,
        /// MIME type (e.g. "image/png", "image/jpeg")
        #[serde(rename = "mediaType")]
        media_type: String,
    },
    ImageRef {
        attachment: AttachmentRef,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    Human,
    Agent,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Active,
    Idle,
    Done,
    Error,
    Closed,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextCarryState {
    Resumed,
    Reinjected,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageMention {
    pub file_path: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub end_line: Option<u32>,
}

impl MessageMention {
    pub fn from_domain(mention: crate::domain::code::MentionReference) -> Self {
        Self {
            file_path: mention.file_path,
            start_line: mention.start_line,
            end_line: mention.end_line,
        }
    }

    pub fn into_domain(self) -> crate::domain::code::MentionReference {
        crate::domain::code::MentionReference {
            file_path: self.file_path,
            start_line: self.start_line,
            end_line: self.end_line,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepContextDto {
    pub run_id: String,
    pub workflow_name: String,
    pub step_name: String,
    pub run_index: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parent_step_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parent_run_index: Option<u32>,
    pub order: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub startup_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub startup_max_retries: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stale_timeout_secs: Option<u64>,
}

pub(crate) mod workflow_step_context_mapper {
    use super::WorkflowStepContextDto;
    use crate::domain::workflow::WorkflowStepContext;

    pub(crate) fn to_dto(context: WorkflowStepContext) -> WorkflowStepContextDto {
        WorkflowStepContextDto {
            run_id: context.run_id,
            workflow_name: context.workflow_name,
            step_name: context.step_name,
            run_index: context.run_index,
            parent_step_name: context.parent_step_name,
            parent_run_index: context.parent_run_index,
            order: context.order,
            startup_timeout_secs: context.startup_timeout_secs,
            startup_max_retries: context.startup_max_retries,
            stale_timeout_secs: context.stale_timeout_secs,
        }
    }

    pub(crate) fn to_domain(context: WorkflowStepContextDto) -> WorkflowStepContext {
        WorkflowStepContext {
            run_id: context.run_id,
            workflow_name: context.workflow_name,
            step_name: context.step_name,
            run_index: context.run_index,
            parent_step_name: context.parent_step_name,
            parent_run_index: context.parent_run_index,
            order: context.order,
            startup_timeout_secs: context.startup_timeout_secs,
            startup_max_retries: context.startup_max_retries,
            stale_timeout_secs: context.stale_timeout_secs,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivityEntry {
    ToolUse {
        tool: String,
        input: serde_json::Value,
        id: String,
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
            rename = "contentRef"
        )]
        content_ref: Option<ToolOutputRef>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        summary: Option<ToolOutputSummary>,
    },
    PermissionResult {
        #[serde(rename = "toolName")]
        tool_name: String,
        status: String,
        summary: String,
    },
}

pub trait SessionBackendResolver {
    #[cfg(test)]
    fn resolve_backend_id(&self, backend_id: Option<String>) -> Result<String, String>;
    fn default_model_for(&self, backend_id: &str) -> Result<String, String>;
    fn backend_exists(&self, backend_id: &str) -> bool;
    fn resolve_default_id(&self) -> Result<String, String>;
}

impl<T> SessionBackendResolver for std::sync::Arc<T>
where
    T: SessionBackendResolver + ?Sized,
{
    #[cfg(test)]
    fn resolve_backend_id(&self, backend_id: Option<String>) -> Result<String, String> {
        self.as_ref().resolve_backend_id(backend_id)
    }

    fn default_model_for(&self, backend_id: &str) -> Result<String, String> {
        self.as_ref().default_model_for(backend_id)
    }

    fn backend_exists(&self, backend_id: &str) -> bool {
        self.as_ref().backend_exists(backend_id)
    }

    fn resolve_default_id(&self) -> Result<String, String> {
        self.as_ref().resolve_default_id()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub thinking: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub activities: Option<Vec<ActivityEntry>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parts: Option<Vec<MessagePart>>,
    /// Final streaming delta seq that produced the persisted parts for this
    /// message. Older sessions omit the field and deserialize to 0.
    #[serde(default)]
    pub streaming_final_seq: u64,
    pub timestamp: f64,
    /// usecase 内の保存・転送用値型。serialize 表現（camelCase・行範囲省略）は
    /// controller protocol 境界の入力型と等価に保つ。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mentions: Option<Vec<MessageMention>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSession {
    pub id: String,
    pub worktree_path: String,
    pub messages: Vec<ChatMessage>,
    pub state: SessionState,
    pub created_at: f64,
    pub updated_at: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_carry: Option<ContextCarryState>,
    /// 抽象モード文字列（"ask" / "edit" / "full"）。
    /// serde の default を意図的に付けない: 保存済みセッションで欠落していた場合は
    /// デシリアライズエラーで起動を拒否する（破壊的変更、Spec issues-947 参照）。
    pub permission_mode: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plan_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub selected_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub permission_profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backend_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub workflow_step_session: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub workflow_step_context: Option<WorkflowStepContextDto>,
    /// Backend-internal freshness meta. Persist it through [`SessionMeta`], but
    /// never expose it through flattened ChatSession command responses.
    #[serde(default, skip_serializing)]
    pub context_epoch: Option<ContextEpochMeta>,
}

pub const SESSION_BODY_FORMAT_VERSION: u32 = 1;
pub const INITIAL_SESSION_PAGE_LIMIT: usize = 50;
pub const DEFAULT_SESSION_PAGE_LIMIT: usize = INITIAL_SESSION_PAGE_LIMIT;
pub const MAX_SESSION_PAGE_LIMIT: usize = 200;
#[cfg(test)]
pub const MAX_TOOL_OUTPUT_BYTES: usize =
    crate::domain::agent_session::services::MAX_TOOL_OUTPUT_BYTES;
#[cfg(test)]
pub const MAX_TOOL_OUTPUT_LINES: usize =
    crate::domain::agent_session::services::MAX_TOOL_OUTPUT_LINES;
#[cfg(test)]
pub const TOOL_OUTPUT_PREVIEW_BYTES: usize =
    crate::domain::agent_session::services::TOOL_OUTPUT_PREVIEW_BYTES;

pub fn should_externalize_tool_output(content: &str) -> bool {
    DefaultToolOutputExternalizationPolicy.should_externalize_tool_output(content)
}

pub fn tool_output_summary(content: &str, is_error: bool, truncated: bool) -> ToolOutputSummary {
    let summary =
        DefaultToolOutputExternalizationPolicy.tool_output_summary(content, is_error, truncated);
    ToolOutputSummary {
        line_count: summary.line_count,
        byte_size: summary.byte_size,
        is_error,
        truncated,
    }
}

pub fn tool_output_preview(content: &str) -> String {
    DefaultToolOutputExternalizationPolicy.tool_output_preview(content)
}

pub fn project_tool_output_part_for_stream(part: &MessagePart) -> MessagePart {
    let MessagePart::ToolResult {
        content,
        is_error,
        tool_use_id,
        parent_tool_use_id,
        content_ref,
        summary,
    } = part
    else {
        return part.clone();
    };
    if content_ref.is_some() || !should_externalize_tool_output(content) {
        return part.clone();
    }
    let projected_summary = summary
        .clone()
        .unwrap_or_else(|| tool_output_summary(content, *is_error, true));
    MessagePart::ToolResult {
        content: tool_output_preview(content),
        is_error: *is_error,
        tool_use_id: tool_use_id.clone(),
        parent_tool_use_id: parent_tool_use_id.clone(),
        content_ref: None,
        summary: Some(projected_summary),
    }
}

pub fn project_tool_output_parts_for_stream(parts: &[MessagePart]) -> Vec<MessagePart> {
    parts
        .iter()
        .map(project_tool_output_part_for_stream)
        .collect()
}

/// meta.json。message body を含まない session 単位の保存正典。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub id: String,
    pub worktree_path: String,
    pub state: SessionState,
    pub created_at: f64,
    pub updated_at: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_carry: Option<ContextCarryState>,
    pub permission_mode: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plan_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub selected_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub permission_profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backend_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub workflow_step_session: bool,
    /// workflow step session の context（step 名等）。message body と異なり軽量な
    /// メタ情報なので、Workflow View のヘッダー表示のため meta に保持する。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub workflow_step_context: Option<WorkflowStepContextDto>,
    #[serde(default, skip_serializing, skip_deserializing)]
    pub workflow_instructions: Vec<String>,
    #[serde(default, skip_serializing, skip_deserializing)]
    pub agent_read_paths: Option<Vec<PathBuf>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_epoch: Option<ContextEpochMeta>,
    pub first_message_preview: String,
    pub message_count: usize,
    pub body_format_version: u32,
}

/// review CLI が `--session-id` から actor / worktree を解決するための read model。
///
/// review 解決では preview/count を使わないため、dir 形式 meta から必要なフィールドだけを返す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionReviewContext {
    pub id: String,
    pub worktree_path: String,
    pub state: SessionState,
    pub selected_model: Option<String>,
    pub backend_id: Option<String>,
}

impl From<SessionMeta> for SessionReviewContext {
    fn from(meta: SessionMeta) -> Self {
        Self {
            id: meta.id,
            worktree_path: meta.worktree_path,
            state: meta.state,
            selected_model: meta.selected_model,
            backend_id: meta.backend_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentRef {
    pub id: String,
    pub media_type: String,
    pub byte_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutputRef {
    pub id: String,
    pub byte_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutputSummary {
    pub line_count: u64,
    pub byte_size: u64,
    pub is_error: bool,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolResultMergeDecision {
    Merge,
    Replace,
    AppendSeparate,
    Skip,
}

pub(crate) fn decide_tool_result_merge(
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

#[derive(Debug, Clone)]
pub(crate) struct ToolResultUpdate {
    pub content: String,
    pub is_error: bool,
    pub tool_use_id: Option<String>,
    pub parent_tool_use_id: Option<String>,
    pub content_ref: Option<ToolOutputRef>,
    pub summary: Option<ToolOutputSummary>,
}

impl ToolResultUpdate {
    pub(crate) fn into_part(self) -> MessagePart {
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

fn tool_result_delta_from_update(
    update: &ToolResultUpdate,
    parent_tool_use_id: Option<String>,
) -> MessagePart {
    MessagePart::ToolResult {
        content: update.content.clone(),
        is_error: update.is_error,
        tool_use_id: update.tool_use_id.clone(),
        parent_tool_use_id,
        content_ref: update.content_ref.clone(),
        summary: update.summary.clone(),
    }
}

pub(crate) fn apply_tool_result_update(
    parts: &mut Vec<MessagePart>,
    update: ToolResultUpdate,
) -> Option<MessagePart> {
    let Some(tool_use_id) = update.tool_use_id.as_deref() else {
        parts.push(update.into_part());
        return None;
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
        return None;
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
        return None;
    };

    let decision = decide_tool_result_merge(
        existing_content_ref,
        *existing_error,
        &update.content_ref,
        update.is_error,
        &update.content,
    );
    match decision {
        ToolResultMergeDecision::Skip => None,
        ToolResultMergeDecision::AppendSeparate => {
            parts.push(update.into_part());
            None
        }
        ToolResultMergeDecision::Replace => {
            if existing_parent_tool_use_id.is_none() {
                *existing_parent_tool_use_id = update.parent_tool_use_id.clone();
            }
            *existing_content = update.content.clone();
            *existing_error = update.is_error;
            *existing_content_ref = update.content_ref.clone();
            *existing_summary = update.summary.clone();
            Some(tool_result_delta_from_update(
                &update,
                existing_parent_tool_use_id.clone(),
            ))
        }
        ToolResultMergeDecision::Merge => {
            if existing_parent_tool_use_id.is_none() {
                *existing_parent_tool_use_id = update.parent_tool_use_id.clone();
            }
            if *existing_error && !update.is_error && update.content_ref.is_none() {
                *existing_content = update.content.clone();
                *existing_error = false;
                *existing_content_ref = None;
                *existing_summary = None;
            } else {
                if update.content.contains(existing_content.as_str()) || existing_content.is_empty()
                {
                    *existing_content = update.content.clone();
                    *existing_summary = update.summary.clone();
                } else {
                    existing_content.push_str(&update.content);
                    *existing_summary = None;
                }
                *existing_content_ref = update.content_ref.clone();
                *existing_error = *existing_error || update.is_error;
            }
            Some(tool_result_delta_from_update(
                &update,
                existing_parent_tool_use_id.clone(),
            ))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionToolOutput {
    pub content: String,
    pub byte_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageIndexEntry {
    pub id: String,
    pub seq: u64,
    pub role: MessageRole,
    pub timestamp: f64,
    pub content_hash: String,
    #[serde(default)]
    pub attachment_refs: Vec<AttachmentRef>,
    #[serde(default)]
    pub tool_output_refs: Vec<ToolOutputRef>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token_meta: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageCursor(pub u64);

impl serde::Serialize for PageCursor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for PageCursor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct CursorVisitor;

        impl serde::de::Visitor<'_> for CursorVisitor {
            type Value = PageCursor;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a page cursor string or integer")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                value
                    .parse::<u64>()
                    .map(PageCursor)
                    .map_err(|_| E::custom("invalid page cursor"))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(PageCursor(value))
            }
        }

        deserializer.deserialize_any(CursorVisitor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePageMetadata {
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token_meta: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub run_meta: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPage {
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub message_metadata: Vec<MessagePageMetadata>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub next_cursor: Option<PageCursor>,
    pub has_more: bool,
    pub total_count: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub latest_token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionAttachment {
    pub data: String,
    pub media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InitialSessionPage {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub next_cursor: Option<PageCursor>,
    pub has_more: bool,
    pub total_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetSessionResponse {
    #[serde(flatten)]
    pub session: ChatSession,
    pub turn_phase: TurnPhase,
    pub available_models: Vec<ModelInfo>,
    pub pending_queue: Vec<QueuedAgentTurn>,
    pub pending_queue_count: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub initial_page: Option<InitialSessionPage>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub latest_token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueuedAgentTurn {
    pub id: String,
    pub content_preview: String,
    pub created_at: f64,
    pub permission_mode: String,
    pub image_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub backend: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub worktree_path: String,
    pub state: SessionState,
    pub created_at: f64,
    pub updated_at: f64,
    pub first_message: String,
    pub message_count: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_carry: Option<ContextCarryState>,
    pub permission_mode: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plan_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub permission_profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backend_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub workflow_step_session: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub workflow_step_context: Option<WorkflowStepContextDto>,
}

pub(crate) fn first_message_preview(messages: &[ChatMessage]) -> String {
    let first_message = messages
        .first()
        .map(|m| {
            let content = if m.content.is_empty() {
                if let Some(parts) = m.parts.as_ref() {
                    if parts.iter().any(|p| {
                        matches!(p, MessagePart::Image { .. } | MessagePart::ImageRef { .. })
                    }) {
                        "[Image]".to_string()
                    } else {
                        let (legacy_content, legacy_thinking, _) = parts_to_legacy(parts);
                        if !legacy_content.is_empty() {
                            legacy_content
                        } else {
                            legacy_thinking.unwrap_or_default()
                        }
                    }
                } else {
                    String::new()
                }
            } else {
                m.content.clone()
            };
            match content.char_indices().nth(100) {
                Some((byte_pos, _)) => format!("{}…", &content[..byte_pos]),
                None => content,
            }
        })
        .unwrap_or_default();
    first_message
}

impl SessionMeta {
    /// 軽量 meta 経路での workflow step session 判定。
    /// `workflow_step_session` フラグは context を持つ場合も必ず立つ正典だが、
    /// context の有無も合わせて見て ChatSession 側の判定と一致させる。
    pub fn is_workflow_step_session(&self) -> bool {
        self.workflow_step_session || self.workflow_step_context.is_some()
    }

    pub fn from_session(session: &ChatSession) -> Self {
        Self {
            id: session.id.clone(),
            worktree_path: session.worktree_path.clone(),
            state: session.state.clone(),
            created_at: session.created_at,
            updated_at: session.updated_at,
            agent_session_id: session.agent_session_id.clone(),
            context_carry: session.context_carry.clone(),
            permission_mode: session.permission_mode.clone(),
            plan_mode: session.plan_mode,
            selected_model: session.selected_model.clone(),
            permission_profile_id: session.permission_profile_id.clone(),
            backend_id: session.backend_id.clone(),
            workflow_step_session: session.is_workflow_step_session(),
            workflow_step_context: session.workflow_step_context.clone(),
            workflow_instructions: Vec::new(),
            agent_read_paths: Some(agent_read_paths_from_messages(&session.messages)),
            context_epoch: session.context_epoch.clone(),
            first_message_preview: first_message_preview(&session.messages),
            message_count: session.messages.len(),
            body_format_version: SESSION_BODY_FORMAT_VERSION,
        }
    }

    pub fn to_session(&self, messages: Vec<ChatMessage>) -> ChatSession {
        ChatSession {
            id: self.id.clone(),
            worktree_path: self.worktree_path.clone(),
            messages,
            state: self.state.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            agent_session_id: self.agent_session_id.clone(),
            context_carry: self.context_carry.clone(),
            permission_mode: self.permission_mode.clone(),
            plan_mode: self.plan_mode,
            selected_model: self.selected_model.clone(),
            permission_profile_id: self.permission_profile_id.clone(),
            backend_id: self.backend_id.clone(),
            workflow_step_session: self.is_workflow_step_session(),
            workflow_step_context: self.workflow_step_context.clone(),
            context_epoch: self.context_epoch.clone(),
        }
    }

    pub fn to_summary(&self) -> SessionSummary {
        SessionSummary {
            id: self.id.clone(),
            worktree_path: self.worktree_path.clone(),
            state: self.state.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            first_message: self.first_message_preview.clone(),
            message_count: self.message_count,
            agent_session_id: self.agent_session_id.clone(),
            context_carry: self.context_carry.clone(),
            permission_mode: self.permission_mode.clone(),
            plan_mode: self.plan_mode,
            permission_profile_id: self.permission_profile_id.clone(),
            backend_id: self.backend_id.clone(),
            workflow_step_session: self.is_workflow_step_session(),
            workflow_step_context: self.workflow_step_context.clone(),
        }
    }
}

impl ChatSession {
    pub fn is_workflow_step_session(&self) -> bool {
        self.workflow_step_session || self.workflow_step_context.is_some()
    }

    pub fn to_summary(&self) -> SessionSummary {
        SessionSummary {
            id: self.id.clone(),
            worktree_path: self.worktree_path.clone(),
            state: self.state.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            first_message: first_message_preview(&self.messages),
            message_count: self.messages.len(),
            agent_session_id: self.agent_session_id.clone(),
            context_carry: self.context_carry.clone(),
            permission_mode: self.permission_mode.clone(),
            plan_mode: self.plan_mode,
            permission_profile_id: self.permission_profile_id.clone(),
            backend_id: self.backend_id.clone(),
            workflow_step_session: self.is_workflow_step_session(),
            workflow_step_context: self.workflow_step_context.clone(),
        }
    }
}

impl SessionSummary {
    pub fn is_workflow_step_session(&self) -> bool {
        self.workflow_step_session || self.workflow_step_context.is_some()
    }
}

pub(crate) fn now_timestamp() -> f64 {
    crate::other::utils::unix_timestamp_seconds()
}

pub fn parts_to_legacy(
    parts: &[MessagePart],
) -> (String, Option<String>, Option<Vec<ActivityEntry>>) {
    let mut content = String::new();
    let mut thinking = String::new();
    let mut activities: Vec<ActivityEntry> = Vec::new();
    for part in parts {
        match part {
            MessagePart::Text { content: c, .. } => content.push_str(c),
            MessagePart::Error { content: c, .. } => content.push_str(c),
            MessagePart::Thinking { content: c, .. } => thinking.push_str(c),
            MessagePart::ToolUse {
                tool, input, id, ..
            } => {
                activities.push(ActivityEntry::ToolUse {
                    tool: tool.clone(),
                    input: input.clone(),
                    id: id.clone(),
                });
            }
            MessagePart::ToolResult {
                content: c,
                is_error,
                tool_use_id,
                content_ref,
                summary,
                ..
            } => {
                activities.push(ActivityEntry::ToolResult {
                    content: c.clone(),
                    is_error: *is_error,
                    tool_use_id: tool_use_id.clone(),
                    content_ref: content_ref.clone(),
                    summary: summary.clone(),
                });
            }
            MessagePart::Permission {
                request,
                status,
                answers,
                ..
            } => {
                if status != "pending" {
                    let tool_name = request
                        .get("tool_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let summary = answers
                        .as_ref()
                        .and_then(|a| a.as_object())
                        .map(|obj| {
                            obj.values()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_else(|| status.clone());
                    activities.push(ActivityEntry::PermissionResult {
                        tool_name,
                        status: status.clone(),
                        summary,
                    });
                }
            }
            MessagePart::TaskStatus { .. } => {}
            MessagePart::TodoListSnapshot { .. } => {}
            MessagePart::SystemNotification { .. } => {}
            MessagePart::Image { .. } | MessagePart::ImageRef { .. } => {}
        }
    }
    let thinking = if thinking.is_empty() {
        None
    } else {
        Some(thinking)
    };
    let activities = if activities.is_empty() {
        None
    } else {
        Some(activities)
    };
    (content, thinking, activities)
}

/// Internal (non-command) version of create_session, callable from agent_sdk.
/// `permission_mode` 未指定の経路（ワークフロー engine 起点の step session 等）向けに
/// `PermissionMode::Edit` を既定値として用いる。検証済み抽象モードを保有する経路
/// （WS handler / message → 新規 session）は [`create_session_internal_with_permission`] を呼ぶこと。
#[cfg(test)]
pub fn create_session_internal(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    worktree_path: &str,
    backend_id: Option<String>,
) -> Result<ChatSession, String> {
    create_session_internal_with_permission(
        session_store,
        data_dir,
        worktree_path,
        backend_id,
        crate::permission::PermissionMode::Edit,
    )
}

/// 検証済みの抽象 [`crate::permission::PermissionMode`] を初回保存で確定するセッション生成 API。
/// WS handler や message → 新規 session 経路から呼び、edit デフォルトで保存→update の二段階を回避する
/// （Spec issues-947: セッション保存層が permission_mode の正典）。
#[allow(dead_code)]
pub fn create_session_internal_with_permission(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    worktree_path: &str,
    backend_id: Option<String>,
    permission_mode: crate::permission::PermissionMode,
) -> Result<ChatSession, String> {
    create_session_internal_with_attributes(
        session_store,
        data_dir,
        worktree_path,
        backend_id,
        permission_mode,
        SessionCreationAttributes::default(),
    )
}

#[derive(Debug, Clone, Default)]
pub struct SessionCreationAttributes {
    pub selected_model: Option<String>,
    pub plan_mode: bool,
    pub workflow_step_session: bool,
    pub workflow_step_context: Option<WorkflowStepContext>,
}

/// 検証済み抽象モード・selected_model・workflow_step_session フラグを初回保存で確定する内部 API。
/// ワークフロー engine の step session 生成経路から呼び、edit デフォルトで保存→属性上書きの
/// 二段階保存を回避する（Spec issues-947: 途中失敗時の不正中間状態の排除）。
pub fn create_session_internal_with_attributes(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    worktree_path: &str,
    backend_id: Option<String>,
    permission_mode: crate::permission::PermissionMode,
    attributes: SessionCreationAttributes,
) -> Result<ChatSession, String> {
    let workflow_step_session =
        attributes.workflow_step_session || attributes.workflow_step_context.is_some();
    let workflow_step_context = attributes
        .workflow_step_context
        .map(workflow_step_context_mapper::to_dto);
    let session = build_new_session(
        worktree_path,
        backend_id,
        permission_mode,
        attributes.selected_model,
        attributes.plan_mode,
        workflow_step_session,
        workflow_step_context,
    );
    session_store.save_full_session_for_migration_or_restore(data_dir, &session)?;
    Ok(session)
}

fn build_new_session(
    worktree_path: &str,
    backend_id: Option<String>,
    permission_mode: crate::permission::PermissionMode,
    selected_model: Option<String>,
    plan_mode: bool,
    workflow_step_session: bool,
    workflow_step_context: Option<WorkflowStepContextDto>,
) -> ChatSession {
    let now = now_timestamp();
    ChatSession {
        id: uuid::Uuid::new_v4().to_string(),
        worktree_path: worktree_path.to_string(),
        messages: Vec::new(),
        state: SessionState::Active,
        created_at: now,
        updated_at: now,
        agent_session_id: None,
        context_carry: None,
        permission_mode: permission_mode.as_str().to_string(),
        plan_mode,
        selected_model,
        permission_profile_id: None,
        backend_id,
        workflow_step_session,
        workflow_step_context,
        context_epoch: None,
    }
}

/// 新規セッションを作成し、当該 backend の既定モデルを `selected_model` に永続化する。
///
/// モデル「未選択（None）」状態は廃止したため、新規セッションは常に backend の既定モデル
/// （[`SessionBackendResolver::default_model_for`] = 固定リスト先頭）を
/// `selected_model` に持つ。既定モデルが解決できない場合はセッション作成エラーとする。
///
/// `permission_mode` は検証済みの抽象 [`crate::permission::PermissionMode`] を要求し、
/// 初回保存で確定する（Spec issues-947: セッション保存層が permission_mode の正典）。
#[cfg(test)]
pub fn create_session_with_initial_model(
    session_store: &SessionStore,
    registry: &impl SessionBackendResolver,
    data_dir: &std::path::Path,
    worktree_path: &str,
    backend_id: String,
    permission_mode: crate::permission::PermissionMode,
) -> Result<ChatSession, String> {
    create_session_with_initial_model_and_plan_mode(
        session_store,
        registry,
        data_dir,
        worktree_path,
        backend_id,
        permission_mode,
        false,
    )
}

#[cfg(test)]
pub fn create_session_with_initial_model_and_plan_mode(
    session_store: &SessionStore,
    registry: &impl SessionBackendResolver,
    data_dir: &std::path::Path,
    worktree_path: &str,
    backend_id: String,
    permission_mode: crate::permission::PermissionMode,
    plan_mode: bool,
) -> Result<ChatSession, String> {
    create_session_with_model_and_plan_mode(
        session_store,
        registry,
        data_dir,
        worktree_path,
        backend_id,
        permission_mode,
        None,
        plan_mode,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn create_session_with_model_and_plan_mode(
    session_store: &SessionStore,
    registry: &impl SessionBackendResolver,
    data_dir: &std::path::Path,
    worktree_path: &str,
    backend_id: String,
    permission_mode: crate::permission::PermissionMode,
    selected_model: Option<String>,
    plan_mode: bool,
) -> Result<ChatSession, String> {
    // 永続化される selected_model は bare model_id に統一する（set_agent_model / spawn と同一規約）。
    // 応答層（get_session）が backend スコープ付き entry id へ変換してフロントへ返す。
    let selected_model = match selected_model {
        Some(model) => model,
        None => registry.default_model_for(&backend_id)?,
    };
    let session = build_new_session(
        worktree_path,
        Some(backend_id),
        permission_mode,
        Some(selected_model),
        plan_mode,
        false,
        None,
    );
    session_store.save_full_session_for_migration_or_restore(data_dir, &session)?;
    Ok(session)
}

/// Internal (non-command) version of add_message, callable from agent_sdk.
pub fn add_message_internal(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    session_id: &str,
    role: MessageRole,
    content: &str,
    parts: Option<Vec<MessagePart>>,
    mentions: Option<Vec<crate::domain::code::MentionReference>>,
) -> Result<ChatMessage, String> {
    let now = now_timestamp();
    let mentions_for_persist = mentions.map(|v| {
        v.into_iter()
            .map(MessageMention::from_domain)
            .collect::<Vec<_>>()
    });
    let message = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        content: content.to_string(),
        thinking: None,
        activities: None,
        parts,
        streaming_final_seq: 0,
        timestamp: now,
        mentions: mentions_for_persist,
    };
    session_store.append_message(data_dir, session_id, &message)?;
    Ok(message)
}

#[cfg(test)]
pub(crate) fn create_session_command_inner(
    session_store: &SessionStore,
    registry: &impl SessionBackendResolver,
    data_dir: &std::path::Path,
    worktree_path: &str,
    permission_mode: &str,
    backend_id: Option<String>,
) -> Result<ChatSession, String> {
    let permission_mode =
        crate::permission::PermissionMode::parse(permission_mode).map_err(|e| e.to_string())?;
    let resolved_backend_id = registry.resolve_backend_id(backend_id)?;
    create_session_with_initial_model(
        session_store,
        registry,
        data_dir,
        worktree_path,
        resolved_backend_id,
        permission_mode,
    )
}

pub(crate) fn update_session_state_in_data_dir(
    state: &SessionStore,
    data_dir: &std::path::Path,
    session_id: &str,
    new_state: SessionState,
) -> Result<(), String> {
    let meta = state
        .get_session_meta(data_dir, session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    if meta.workflow_step_session && meta.state == SessionState::Closed {
        return Ok(());
    }
    state.set_session_state(data_dir, session_id, new_state)?;
    Ok(())
}

/// セッション復元時の backend_id 検証・解決ロジック。
/// - backend_id が Some かつ registry に存在 → Ok
/// - backend_id が Some だが registry に不在 → Err
/// - backend_id が None → デフォルトを代入して Ok
pub fn resolve_session_backend(
    session: &mut ChatSession,
    registry: &impl SessionBackendResolver,
) -> Result<(), String> {
    if let Some(ref bid) = session.backend_id {
        if !registry.backend_exists(bid) {
            return Err(format!(
                "バックエンド '{}' がレジストリに登録されていません",
                bid
            ));
        }
    } else {
        let default_id = registry.resolve_default_id()?;
        session.backend_id = Some(default_id);
    }
    Ok(())
}

/// セッション起動時の permission_mode 検証。
/// 対象外の値（旧語彙 acceptEdits / bypassPermissions / plan / default、未知語彙、空文字）が
/// 保存されていた場合はバリデーションエラーで拒否し、ユーザに手動更新を求める（破壊的変更）。
pub fn validate_session_permission_mode(session: &ChatSession) -> Result<(), String> {
    crate::permission::PermissionMode::parse(&session.permission_mode)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSessionResponse {
    pub restored_workflow_step: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Default)]
    struct TestBackendResolver {
        default_id: Option<String>,
        models: BTreeMap<String, String>,
        existing: BTreeSet<String>,
    }

    impl TestBackendResolver {
        fn with_backend(mut self, backend_id: &str, default_model: &str) -> Self {
            self.existing.insert(backend_id.to_string());
            self.models
                .insert(backend_id.to_string(), default_model.to_string());
            self
        }

        fn with_default(mut self, backend_id: &str) -> Self {
            self.default_id = Some(backend_id.to_string());
            self
        }
    }

    impl SessionBackendResolver for TestBackendResolver {
        fn resolve_backend_id(&self, backend_id: Option<String>) -> Result<String, String> {
            match backend_id {
                Some(id) if self.backend_exists(&id) => Ok(id),
                Some(id) => Err(format!(
                    "Backend '{}' not found. Available backends: claude, codex",
                    id
                )),
                None => self.resolve_default_id(),
            }
        }

        fn default_model_for(&self, backend_id: &str) -> Result<String, String> {
            self.models
                .get(backend_id)
                .cloned()
                .ok_or_else(|| format!("No models configured for backend '{backend_id}'"))
        }

        fn backend_exists(&self, backend_id: &str) -> bool {
            self.existing.contains(backend_id)
        }

        fn resolve_default_id(&self) -> Result<String, String> {
            self.default_id
                .clone()
                .ok_or_else(|| "No default backend configured".to_string())
        }
    }

    #[test]
    fn tool_output_threshold_externalizes_only_when_limit_is_exceeded() {
        let at_byte_limit = "a".repeat(MAX_TOOL_OUTPUT_BYTES);
        let over_byte_limit = "a".repeat(MAX_TOOL_OUTPUT_BYTES + 1);
        let at_line_limit = "x\n".repeat(MAX_TOOL_OUTPUT_LINES);
        let over_line_limit = "x\n".repeat(MAX_TOOL_OUTPUT_LINES + 1);

        assert!(!should_externalize_tool_output(&at_byte_limit));
        assert!(should_externalize_tool_output(&over_byte_limit));
        assert!(!should_externalize_tool_output(&at_line_limit));
        assert!(should_externalize_tool_output(&over_line_limit));
    }

    #[test]
    fn tool_output_preview_is_bounded_and_summary_is_metadata_only() {
        let secret_tail = "USER_SECRET_TAIL";
        let content = format!("{}{}", "a".repeat(MAX_TOOL_OUTPUT_BYTES + 128), secret_tail);

        let preview = tool_output_preview(&content);
        let summary = tool_output_summary(&content, true, true);

        assert!(preview.len() <= TOOL_OUTPUT_PREVIEW_BYTES);
        assert!(!preview.contains(secret_tail));
        assert_eq!(summary.byte_size, content.len() as u64);
        assert_eq!(summary.is_error, true);
        assert!(summary.truncated);
    }

    #[test]
    fn stream_projection_keeps_small_tool_result_inline_and_bounds_large_result() {
        let small = MessagePart::ToolResult {
            content: "ok".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
            content_ref: None,
            summary: None,
        };
        assert_eq!(project_tool_output_part_for_stream(&small), small);

        let large_content = "z".repeat(MAX_TOOL_OUTPUT_BYTES + 1);
        let large = MessagePart::ToolResult {
            content: large_content.clone(),
            is_error: true,
            tool_use_id: Some("tool-2".to_string()),
            parent_tool_use_id: None,
            content_ref: None,
            summary: None,
        };

        let projected = project_tool_output_part_for_stream(&large);
        match projected {
            MessagePart::ToolResult {
                content,
                is_error,
                tool_use_id,
                content_ref,
                summary,
                ..
            } => {
                assert!(content.len() <= TOOL_OUTPUT_PREVIEW_BYTES);
                assert_ne!(content, large_content);
                assert_eq!(is_error, true);
                assert_eq!(tool_use_id.as_deref(), Some("tool-2"));
                assert!(content_ref.is_none());
                let summary = summary.expect("large output should keep summary");
                assert_eq!(summary.byte_size, large_content.len() as u64);
                assert!(summary.truncated);
            }
            other => panic!("expected tool result projection, got {other:?}"),
        }
    }

    fn workflow_step_context_for_test() -> WorkflowStepContext {
        WorkflowStepContext {
            run_id: "run-1".to_string(),
            workflow_name: "workflow".to_string(),
            step_name: "review".to_string(),
            run_index: 1,
            parent_step_name: None,
            parent_run_index: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        }
    }

    fn context_epoch_meta_for_test(payload: &str) -> ContextEpochMeta {
        ContextEpochMeta {
            epoch_id: 1,
            backend_id: Some("claude".to_string()),
            model_id: Some("sonnet".to_string()),
            worktree_path: "/repo".to_string(),
            source_revisions: vec![
                crate::usecase::agent_session::context_meta::ContextSourceRevisionMeta {
                    kind: "repo_summary".to_string(),
                    revision: 2,
                    fingerprint: Some("repo-fingerprint".to_string()),
                    payload: Some(payload.to_string()),
                },
            ],
        }
    }

    #[test]
    fn workflow_step_session_predicate_uses_flag_or_context() {
        let mut session = build_new_session(
            "/repo",
            None,
            crate::permission::PermissionMode::Edit,
            None,
            false,
            false,
            None,
        );
        assert!(!session.is_workflow_step_session());
        assert!(!session.to_summary().is_workflow_step_session());

        session.workflow_step_context = Some(workflow_step_context_mapper::to_dto(
            workflow_step_context_for_test(),
        ));
        assert!(session.is_workflow_step_session());
        assert!(session.to_summary().is_workflow_step_session());
        assert!(session.to_summary().workflow_step_session);

        session.workflow_step_context = None;
        session.workflow_step_session = true;
        assert!(session.is_workflow_step_session());
        assert!(session.to_summary().is_workflow_step_session());
    }

    #[test]
    fn workflow_step_context_persist_serializeは移行前のcamelcase等価() {
        let mut session = build_new_session(
            "/repo",
            None,
            crate::permission::PermissionMode::Edit,
            None,
            false,
            true,
            Some(workflow_step_context_mapper::to_dto(
                workflow_step_context_for_test(),
            )),
        );
        let value = serde_json::to_value(&session).unwrap();
        let context = &value["workflowStepContext"];
        assert_eq!(context["runId"], serde_json::json!("run-1"));
        assert_eq!(context["workflowName"], serde_json::json!("workflow"));
        assert_eq!(context["stepName"], serde_json::json!("review"));
        assert_eq!(context["runIndex"], serde_json::json!(1));
        assert!(context.get("parentStepName").is_none());
        assert!(context.get("parentRunIndex").is_none());
        assert_eq!(context["order"], serde_json::json!(0));

        let summary = session.to_summary();
        let summary_value = serde_json::to_value(&summary).unwrap();
        assert_eq!(
            summary_value["workflowStepContext"],
            value["workflowStepContext"]
        );

        session.workflow_step_context = None;
        let value = serde_json::to_value(&session).unwrap();
        assert!(value.get("workflowStepContext").is_none());
    }

    #[test]
    fn get_session_response_does_not_expose_context_epoch() {
        let mut session = build_new_session(
            "/repo",
            None,
            crate::permission::PermissionMode::Edit,
            None,
            false,
            false,
            None,
        );
        session.context_epoch = Some(context_epoch_meta_for_test("repo payload"));
        let meta = SessionMeta::from_session(&session);
        let response = GetSessionResponse {
            session,
            turn_phase: TurnPhase::Idle,
            available_models: Vec::new(),
            pending_queue: Vec::new(),
            pending_queue_count: 0,
            initial_page: None,
            latest_token_usage: None,
        };

        let response_value = serde_json::to_value(&response).unwrap();
        let meta_value = serde_json::to_value(&meta).unwrap();

        assert!(response_value.get("contextEpoch").is_none());
        assert!(meta_value.get("contextEpoch").is_some());
    }

    #[test]
    fn chat_message_mentions_persist_serializeは移行前のcamelcase等価() {
        // usecase の保存モデルが adaptor/protocol へ逆依存せず、serialize 表現だけを
        // controller 境界と等価に保つことを担保する。
        let msg = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Human,
            content: "hello".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            streaming_final_seq: 0,
            timestamp: 1.0,
            mentions: Some(vec![
                MessageMention {
                    file_path: "src/a.rs".to_string(),
                    start_line: None,
                    end_line: None,
                },
                MessageMention {
                    file_path: "src/b.rs".to_string(),
                    start_line: Some(3),
                    end_line: Some(5),
                },
            ]),
        };
        let v = serde_json::to_value(&msg).unwrap();
        let mentions = &v["mentions"];
        assert_eq!(mentions[0]["filePath"], serde_json::json!("src/a.rs"));
        assert!(mentions[0].get("startLine").is_none());
        assert!(mentions[0].get("endLine").is_none());
        assert_eq!(mentions[1]["filePath"], serde_json::json!("src/b.rs"));
        assert_eq!(mentions[1]["startLine"], serde_json::json!(3));
        assert_eq!(mentions[1]["endLine"], serde_json::json!(5));

        // None の場合は mentions キー自体が省略される（移行前と等価）。
        let msg_none = ChatMessage {
            mentions: None,
            ..msg
        };
        let v = serde_json::to_value(&msg_none).unwrap();
        assert!(v.get("mentions").is_none());
    }

    #[test]
    fn chat_session_missing_permission_mode_rejected_on_deserialize() {
        // Spec issues-947: 保存済みセッションで permissionMode フィールドが欠落していた場合は、
        // serde default で補完せず、デシリアライズエラーで起動を拒否する（破壊的変更）。
        let json = r#"{"id":"s1","worktreePath":"/repo","messages":[],"state":"active","createdAt":1000.0,"updatedAt":1000.0}"#;
        let err = serde_json::from_str::<ChatSession>(json).unwrap_err();
        assert!(
            err.to_string().contains("permissionMode"),
            "missing permissionMode must be rejected, got: {err}"
        );
    }

    #[test]
    fn chat_session_legacy_permission_mode_rejected_by_validation() {
        // 保存済みセッションが旧語彙や未知語彙を持っていた場合、validate_session_permission_mode が拒否する。
        for legacy in [
            "acceptEdits",
            "bypassPermissions",
            "plan",
            "default",
            "unknown",
            "",
        ] {
            let session = ChatSession {
                id: "s1".to_string(),
                worktree_path: "/repo".to_string(),
                messages: vec![],
                state: SessionState::Active,
                created_at: 1000.0,
                updated_at: 1000.0,
                agent_session_id: None,
                context_carry: None,
                permission_mode: legacy.to_string(),
                plan_mode: false,
                permission_profile_id: None,
                selected_model: None,
                backend_id: None,
                workflow_step_session: false,
                workflow_step_context: None,
                context_epoch: None,
            };
            let err = validate_session_permission_mode(&session).unwrap_err();
            assert!(
                err.contains("ask, edit, full"),
                "legacy '{legacy}' must be rejected with allowed list, got: {err}"
            );
        }
    }

    #[test]
    fn chat_session_to_summary_basic() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::Human,
                content: "Hello agent".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            context_carry: Some(ContextCarryState::Reinjected),
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        };
        let summary = session.to_summary();
        assert_eq!(summary.id, "s1");
        assert_eq!(summary.first_message, "Hello agent");
        assert_eq!(summary.context_carry, Some(ContextCarryState::Reinjected));
        // Verify selected_model not in summary (summary doesn't expose model)
        assert_eq!(summary.message_count, 1);
    }

    #[test]
    fn chat_session_to_summary_truncates_long_message() {
        let long_content = "a".repeat(200);
        let session = ChatSession {
            id: "s2".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::Human,
                content: long_content,
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: SessionState::Idle,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        };
        let summary = session.to_summary();
        assert_eq!(summary.first_message.len(), 100 + "…".len());
        assert!(summary.first_message.ends_with('…'));
    }

    #[test]
    fn chat_session_to_summary_truncates_multibyte_message() {
        // 200 Japanese characters (3 bytes each in UTF-8)
        let long_content = "あ".repeat(200);
        let session = ChatSession {
            id: "s2mb".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::Human,
                content: long_content,
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: SessionState::Idle,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        };
        let summary = session.to_summary();
        // 100 chars of "あ" (300 bytes) + "…" (3 bytes)
        assert_eq!(summary.first_message.chars().count(), 101); // 100 + 1 for "…"
        assert!(summary.first_message.ends_with('…'));
        assert!(summary.first_message.starts_with("あ"));
    }

    #[test]
    fn chat_session_to_summary_empty_messages() {
        let session = ChatSession {
            id: "s3".to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: SessionState::Done,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        };
        let summary = session.to_summary();
        assert_eq!(summary.first_message, "");
        assert_eq!(summary.message_count, 0);
    }

    #[test]
    fn generic_state_update_ignores_closed_workflow_step_session_but_updates_regular_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = crate::test_support::build_session_store();

        let workflow_step_id = uuid::Uuid::new_v4().to_string();
        let regular_id = uuid::Uuid::new_v4().to_string();
        let mut workflow_step = ChatSession {
            id: workflow_step_id.clone(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: SessionState::Closed,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: Some("agent-session".to_string()),
            context_carry: Some(ContextCarryState::Resumed),
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: None,
            workflow_step_session: true,
            workflow_step_context: None,
            context_epoch: None,
        };
        let mut regular = workflow_step.clone();
        regular.id = regular_id.clone();
        regular.workflow_step_session = false;

        store
            .save_full_session_for_migration_or_restore(tmp.path(), &workflow_step)
            .unwrap();
        store
            .save_full_session_for_migration_or_restore(tmp.path(), &regular)
            .unwrap();

        update_session_state_in_data_dir(&store, tmp.path(), &workflow_step.id, SessionState::Idle)
            .unwrap();
        update_session_state_in_data_dir(&store, tmp.path(), &regular.id, SessionState::Idle)
            .unwrap();

        workflow_step = store
            .load_full_session_for_restore(tmp.path(), &workflow_step_id)
            .unwrap()
            .unwrap();
        let regular = store
            .load_full_session_for_restore(tmp.path(), &regular_id)
            .unwrap()
            .unwrap();
        assert_eq!(workflow_step.state, SessionState::Closed);
        assert_eq!(regular.state, SessionState::Idle);
    }

    #[test]
    fn update_session_state_does_not_read_message_body() {
        let tmp = tempfile::tempdir().unwrap();
        let store = crate::test_support::build_session_store();
        let session = ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::Human,
                content: "hello".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        };
        store
            .save_full_session_for_migration_or_restore(tmp.path(), &session)
            .unwrap();
        std::fs::write(
            tmp.path()
                .join("sessions")
                .join(&session.id)
                .join("messages")
                .join("1.json"),
            "{",
        )
        .unwrap();

        update_session_state_in_data_dir(&store, tmp.path(), &session.id, SessionState::Idle)
            .unwrap();

        let meta = store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.state, SessionState::Idle);
    }

    #[test]
    fn meta_update_after_shell_load_preserves_all_message_chunks() {
        let tmp = tempfile::tempdir().unwrap();
        let store = crate::test_support::build_session_store();
        let mut session = ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![
                ChatMessage {
                    id: "m1".to_string(),
                    role: MessageRole::Human,
                    content: "first".to_string(),
                    thinking: None,
                    activities: None,
                    parts: None,
                    streaming_final_seq: 0,
                    timestamp: 1000.0,
                    mentions: None,
                },
                ChatMessage {
                    id: "m2".to_string(),
                    role: MessageRole::Agent,
                    content: "second".to_string(),
                    thinking: None,
                    activities: None,
                    parts: None,
                    streaming_final_seq: 0,
                    timestamp: 1001.0,
                    mentions: None,
                },
            ],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1001.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        };
        store
            .save_full_session_for_migration_or_restore(tmp.path(), &session)
            .unwrap();

        let shell = store
            .get_session_shell(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert!(shell.messages.is_empty());

        store
            .set_session_state(tmp.path(), &session.id, SessionState::Idle)
            .unwrap();

        session.state = SessionState::Idle;
        let loaded = store
            .load_full_session_for_restore(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded
                .messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        assert_eq!(loaded.state, session.state);
    }

    #[test]
    fn message_role_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&MessageRole::Human).unwrap(),
            "\"human\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::Agent).unwrap(),
            "\"agent\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::System).unwrap(),
            "\"system\""
        );
    }

    #[test]
    fn session_state_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&SessionState::Active).unwrap(),
            "\"active\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Idle).unwrap(),
            "\"idle\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Done).unwrap(),
            "\"done\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Error).unwrap(),
            "\"error\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Closed).unwrap(),
            "\"closed\""
        );
    }

    #[test]
    fn chat_message_thinking_field_serialization() {
        let msg_with = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Agent,
            content: "response".to_string(),
            thinking: Some("deep thought".to_string()),
            activities: None,
            parts: None,
            streaming_final_seq: 0,
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg_with).unwrap();
        assert!(json.contains("\"thinking\":\"deep thought\""));
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.thinking, Some("deep thought".to_string()));

        let msg_without = ChatMessage {
            id: "m2".to_string(),
            role: MessageRole::Agent,
            content: "response".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            streaming_final_seq: 0,
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg_without).unwrap();
        assert!(!json.contains("thinking"));
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.thinking, None);
    }

    #[test]
    fn chat_message_without_thinking_field_deserializes() {
        let json = r#"{"id":"m1","role":"agent","content":"hello","timestamp":1000.0}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.thinking, None);
    }

    #[test]
    fn chat_session_roundtrip() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![
                ChatMessage {
                    id: "m1".to_string(),
                    role: MessageRole::Human,
                    content: "Hello".to_string(),
                    thinking: None,
                    activities: None,
                    parts: None,
                    streaming_final_seq: 0,
                    timestamp: 1000.0,
                    mentions: None,
                },
                ChatMessage {
                    id: "m2".to_string(),
                    role: MessageRole::Agent,
                    content: "Hi there!".to_string(),
                    thinking: None,
                    activities: None,
                    parts: None,
                    streaming_final_seq: 0,
                    timestamp: 1001.0,
                    mentions: None,
                },
            ],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1001.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        };
        let json = serde_json::to_string(&session).unwrap();
        let back: ChatSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "s1");
        assert_eq!(back.messages.len(), 2);
        assert_eq!(back.messages[0].role, MessageRole::Human);
        assert_eq!(back.messages[1].role, MessageRole::Agent);
    }

    #[test]
    fn chat_session_without_selected_model_deserializes() {
        let json = r#"{"id":"s1","worktreePath":"/repo","messages":[],"state":"active","createdAt":1000.0,"updatedAt":1000.0,"permissionMode":"edit"}"#;
        let session: ChatSession = serde_json::from_str(json).unwrap();
        assert_eq!(session.selected_model, None);
        assert_eq!(session.context_carry, None);
    }

    #[test]
    fn chat_session_roundtrip_with_selected_model() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1001.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: Some("claude-opus-4-6".to_string()),
            backend_id: None,
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("selectedModel"));
        let back: ChatSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.selected_model, Some("claude-opus-4-6".to_string()));
    }

    #[test]
    fn activity_entry_tool_use_serialization() {
        let entry = ActivityEntry::ToolUse {
            tool: "Read".to_string(),
            input: serde_json::json!({"file_path": "/src/main.ts"}),
            id: "toolu_001".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "tool_use");
        assert_eq!(v["tool"], "Read");
        assert_eq!(v["id"], "toolu_001");
        assert_eq!(v["input"]["file_path"], "/src/main.ts");
    }

    #[test]
    fn activity_entry_tool_result_serialization() {
        let entry = ActivityEntry::ToolResult {
            content: "file contents".to_string(),
            is_error: false,
            tool_use_id: Some("toolu_001".into()),
            content_ref: None,
            summary: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "tool_result");
        assert_eq!(v["content"], "file contents");
        assert_eq!(v["isError"], false);
        assert_eq!(v["toolUseId"], "toolu_001");
    }

    #[test]
    fn activity_entry_permission_result_serialization() {
        let entry = ActivityEntry::PermissionResult {
            tool_name: "Bash".to_string(),
            status: "allowed".to_string(),
            summary: "Bash: allowed".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "permission_result");
        assert_eq!(v["toolName"], "Bash");
        assert_eq!(v["status"], "allowed");
        assert_eq!(v["summary"], "Bash: allowed");

        let back: ActivityEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn activity_entry_permission_result_backward_compat() {
        // Existing session files without permission_result should still deserialize
        let json = r#"{"type":"tool_use","tool":"Read","input":{},"id":"t1"}"#;
        let entry: ActivityEntry = serde_json::from_str(json).unwrap();
        assert!(matches!(entry, ActivityEntry::ToolUse { .. }));
    }

    #[test]
    fn chat_message_without_activities_field_deserializes() {
        let json = r#"{"id":"m1","role":"agent","content":"hello","timestamp":1000.0}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.activities, None);
    }

    #[test]
    fn chat_message_with_activities_roundtrip() {
        let msg = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Agent,
            content: "done".to_string(),
            thinking: None,
            activities: Some(vec![
                ActivityEntry::ToolUse {
                    tool: "Read".to_string(),
                    input: serde_json::json!({}),
                    id: "t1".to_string(),
                },
                ActivityEntry::ToolResult {
                    content: "ok".to_string(),
                    is_error: false,
                    tool_use_id: None,
                    content_ref: None,
                    summary: None,
                },
            ]),
            parts: None,
            streaming_final_seq: 0,
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.activities.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn message_part_serde_roundtrip() {
        let parts = vec![
            MessagePart::Thinking {
                content: "hmm".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "hello".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Error {
                content: "something went wrong".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Read".to_string(),
                input: serde_json::json!({"file_path": "/a.ts"}),
                id: "t1".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolResult {
                content: "ok".to_string(),
                is_error: false,
                tool_use_id: None,
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            },
            MessagePart::Permission {
                request: serde_json::json!({"request_id": "r1", "tool_name": "Bash"}),
                status: "allowed".to_string(),
                answers: Some(serde_json::json!({"q1": "yes"})),
                parent_tool_use_id: None,
            },
        ];
        let json = serde_json::to_string(&parts).unwrap();
        let back: Vec<MessagePart> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 6);
        assert_eq!(back, parts);
    }

    #[test]
    fn message_part_error_serialization() {
        let part = MessagePart::Error {
            content: "fail".to_string(),
            parent_tool_use_id: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "error");
        assert_eq!(v["content"], "fail");
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn chat_message_with_parts_roundtrip() {
        let msg = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Agent,
            content: "hi".to_string(),
            thinking: Some("think".to_string()),
            activities: None,
            parts: Some(vec![
                MessagePart::Thinking {
                    content: "think".to_string(),
                    parent_tool_use_id: None,
                },
                MessagePart::Text {
                    content: "hi".to_string(),
                    parent_tool_use_id: None,
                },
            ]),
            streaming_final_seq: 12,
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.parts.as_ref().unwrap().len(), 2);
        assert_eq!(back.streaming_final_seq, 12);
    }

    #[test]
    fn old_json_without_parts_deserializes_to_none() {
        let json = r#"{"id":"m1","role":"agent","content":"hello","timestamp":1000.0}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.parts, None);
        assert_eq!(msg.streaming_final_seq, 0);
    }

    #[test]
    fn message_part_permission_without_answers_serializes() {
        let part = MessagePart::Permission {
            request: serde_json::json!({"request_id": "r1"}),
            status: "pending".to_string(),
            answers: None,
            parent_tool_use_id: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(!json.contains("answers"));
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        if let MessagePart::Permission { answers, .. } = back {
            assert_eq!(answers, None);
        } else {
            panic!("Expected Permission variant");
        }
    }

    #[test]
    fn tool_result_without_tool_use_id_deserializes() {
        let json = r#"{"type":"tool_result","content":"ok","isError":false}"#;
        let part: MessagePart = serde_json::from_str(json).unwrap();
        if let MessagePart::ToolResult { tool_use_id, .. } = part {
            assert_eq!(tool_use_id, None);
        } else {
            panic!("Expected ToolResult variant");
        }

        let entry: ActivityEntry = serde_json::from_str(json).unwrap();
        if let ActivityEntry::ToolResult { tool_use_id, .. } = entry {
            assert_eq!(tool_use_id, None);
        } else {
            panic!("Expected ToolResult variant");
        }
    }

    #[test]
    fn task_status_serde_roundtrip() {
        let part = MessagePart::TaskStatus {
            task_tool_use_id: "toolu_task_001".to_string(),
            status: "completed".to_string(),
            description: Some("Search codebase".to_string()),
            summary: Some("Found 3 files".to_string()),
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "task_status");
        assert_eq!(v["taskToolUseId"], "toolu_task_001");
        assert_eq!(v["status"], "completed");
        assert_eq!(v["description"], "Search codebase");
        assert_eq!(v["summary"], "Found 3 files");
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn task_status_without_optional_fields_deserializes() {
        let json = r#"{"type":"task_status","taskToolUseId":"t1","status":"started"}"#;
        let part: MessagePart = serde_json::from_str(json).unwrap();
        if let MessagePart::TaskStatus {
            task_tool_use_id,
            status,
            description,
            summary,
        } = part
        {
            assert_eq!(task_tool_use_id, "t1");
            assert_eq!(status, "started");
            assert_eq!(description, None);
            assert_eq!(summary, None);
        } else {
            panic!("Expected TaskStatus variant");
        }
    }

    #[test]
    fn parent_tool_use_id_serde() {
        let part = MessagePart::Text {
            content: "sub-agent text".to_string(),
            parent_tool_use_id: Some("toolu_parent".to_string()),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("parentToolUseId"));
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        if let MessagePart::Text {
            parent_tool_use_id, ..
        } = back
        {
            assert_eq!(parent_tool_use_id, Some("toolu_parent".to_string()));
        } else {
            panic!("Expected Text variant");
        }
    }

    #[test]
    fn system_notification_serde_roundtrip() {
        let part = MessagePart::SystemNotification {
            notification_type: SystemNotificationType::Compaction,
            status: "completed".to_string(),
            label: "Conversation compacted".to_string(),
            detail: Some("trigger=auto, 50000 tokens".to_string()),
            hook_id: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "system_notification");
        assert_eq!(v["notificationType"], "compaction");
        assert_eq!(v["status"], "completed");
        assert_eq!(v["label"], "Conversation compacted");
        assert_eq!(v["detail"], "trigger=auto, 50000 tokens");
        assert!(v.get("hookId").is_none());
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn system_notification_rejects_non_compaction_type() {
        let json = r#"{
            "type":"system_notification",
            "notificationType":"hook",
            "status":"in_progress",
            "label":"SessionEnd",
            "hookId":"hook-001"
        }"#;
        let parsed = serde_json::from_str::<MessagePart>(json);
        assert!(parsed.is_err());
    }

    #[test]
    fn old_json_without_system_notification_deserializes() {
        // Backward compat: old session JSON without system_notification parts
        let json = r#"[{"type":"text","content":"hello"},{"type":"task_status","taskToolUseId":"t1","status":"started"}]"#;
        let parts: Vec<MessagePart> = serde_json::from_str(json).unwrap();
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], MessagePart::Text { .. }));
        assert!(matches!(&parts[1], MessagePart::TaskStatus { .. }));
    }

    #[test]
    fn old_json_without_parent_tool_use_id_deserializes() {
        let json = r#"{"type":"text","content":"hello"}"#;
        let part: MessagePart = serde_json::from_str(json).unwrap();
        if let MessagePart::Text {
            parent_tool_use_id, ..
        } = part
        {
            assert_eq!(parent_tool_use_id, None);
        } else {
            panic!("Expected Text variant");
        }
    }

    #[test]
    fn message_part_image_serde_roundtrip() {
        let part = MessagePart::Image {
            data: "iVBORw0KGgoAAAA==".to_string(),
            media_type: "image/png".to_string(),
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "image");
        assert_eq!(v["data"], "iVBORw0KGgoAAAA==");
        assert_eq!(v["mediaType"], "image/png");
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn message_part_image_ref_serde_roundtrip() {
        let part = MessagePart::ImageRef {
            attachment: AttachmentRef {
                id: "abc123".to_string(),
                media_type: "image/png".to_string(),
                byte_size: 42,
            },
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "image_ref");
        assert_eq!(v["attachment"]["id"], "abc123");
        assert_eq!(v["attachment"]["mediaType"], "image/png");
        assert_eq!(v["attachment"]["byteSize"], 42);
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn chat_message_with_image_parts_roundtrip() {
        let msg = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Human,
            content: "Check this image".to_string(),
            thinking: None,
            activities: None,
            parts: Some(vec![
                MessagePart::Text {
                    content: "Check this image".to_string(),
                    parent_tool_use_id: None,
                },
                MessagePart::Image {
                    data: "base64data".to_string(),
                    media_type: "image/jpeg".to_string(),
                },
            ]),
            streaming_final_seq: 0,
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.parts.as_ref().unwrap().len(), 2);
        assert!(matches!(
            &back.parts.as_ref().unwrap()[1],
            MessagePart::Image { .. }
        ));
    }

    #[test]
    fn parts_to_legacy_ignores_image() {
        let parts = vec![
            MessagePart::Text {
                content: "hello".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Image {
                data: "base64".to_string(),
                media_type: "image/png".to_string(),
            },
        ];
        let (content, thinking, activities) = parts_to_legacy(&parts);
        assert_eq!(content, "hello");
        assert_eq!(thinking, None);
        assert_eq!(activities, None);
    }

    /// parent ChatSession 廃止後の在庫 JSON 互換性: 旧 `ChatSession.workflowState` フィールドを
    /// 含む JSON は serde の unknown_fields 既定挙動で silently 読み捨てられ、deserialize は
    /// 成功する（破棄前提でロスレスではないが、起動の阻害にならない）。
    #[test]
    fn legacy_chat_session_with_old_workflow_state_is_silently_ignored() {
        let json = r#"{
            "id": "s1",
            "worktreePath": "/repo",
            "messages": [],
            "state": "active",
            "createdAt": 1.0,
            "updatedAt": 1.0,
            "permissionMode": "edit",
            "workflowStepSession": false,
            "workflowState": {
                "executionId": "exec-1",
                "workflowName": "legacy"
            }
        }"#;
        let result: Result<ChatSession, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "ChatSession.workflowState は撤去フィールド扱いで silently 読み捨てられる"
        );
    }

    #[test]
    fn create_session_internal_with_backend_id() {
        let store = crate::test_support::build_session_store();
        let dir = tempfile::tempdir().unwrap();
        let session =
            create_session_internal(&store, dir.path(), "/repo", Some("claude".to_string()))
                .unwrap();
        assert_eq!(session.backend_id, Some("claude".to_string()));
        assert_eq!(session.state, SessionState::Active);
        assert_eq!(session.worktree_path, "/repo");
    }

    #[test]
    fn create_session_internal_without_backend_id() {
        let store = crate::test_support::build_session_store();
        let dir = tempfile::tempdir().unwrap();
        let session = create_session_internal(&store, dir.path(), "/repo", None).unwrap();
        assert_eq!(session.backend_id, None);
    }

    // Spec issues-947: セッション開始経路は `create_session_internal_with_permission`
    // で session を生成し、検証済み抽象モードを
    // 初回保存で確定する。ask / edit / full それぞれが保存済みセッションの
    // permission_mode として選択値どおりに記録されることを確認する。
    #[test]
    fn create_session_with_permission_persists_selected_abstract_mode() {
        for mode in [
            crate::permission::PermissionMode::Ask,
            crate::permission::PermissionMode::Edit,
            crate::permission::PermissionMode::Full,
        ] {
            let store = crate::test_support::build_session_store();
            let dir = tempfile::tempdir().unwrap();
            let created = create_session_internal_with_permission(
                &store,
                dir.path(),
                "/repo",
                Some("claude".to_string()),
                mode,
            )
            .unwrap();
            assert_eq!(created.permission_mode, mode.as_str());

            let loaded = store
                .load_full_session_for_restore(dir.path(), &created.id)
                .unwrap()
                .unwrap();
            assert_eq!(loaded.permission_mode, mode.as_str());
        }
    }

    fn test_backend_registry() -> TestBackendResolver {
        TestBackendResolver::default()
            .with_backend(
                "claude",
                crate::domain::agent_session::CLAUDE_FIXED_MODELS[0],
            )
            .with_default("claude")
    }

    #[test]
    fn create_session_command_inner_persists_valid_permission_modes() {
        for mode in ["ask", "edit", "full"] {
            let store = crate::test_support::build_session_store();
            let dir = tempfile::tempdir().unwrap();
            let registry = test_backend_registry();

            let created = create_session_command_inner(
                &store,
                &registry,
                dir.path(),
                "/repo",
                mode,
                Some("claude".to_string()),
            )
            .unwrap();

            assert_eq!(created.permission_mode, mode);
            let loaded = store
                .load_full_session_for_restore(dir.path(), &created.id)
                .unwrap()
                .unwrap();
            assert_eq!(loaded.permission_mode, mode);
        }
    }

    #[test]
    fn create_session_command_inner_rejects_invalid_permission_without_creating_session() {
        for invalid in [
            "acceptEdits",
            "bypassPermissions",
            "plan",
            "default",
            "unknown",
            "",
        ] {
            let store = crate::test_support::build_session_store();
            let dir = tempfile::tempdir().unwrap();
            let registry = test_backend_registry();

            let err = create_session_command_inner(
                &store,
                &registry,
                dir.path(),
                "/repo",
                invalid,
                Some("claude".to_string()),
            )
            .unwrap_err();
            assert!(
                err.contains("ask, edit, full"),
                "invalid mode '{invalid}' must include allowed list, got: {err}"
            );
            assert!(store
                .list_worktree_sessions(dir.path(), "/repo")
                .unwrap()
                .is_empty());
        }
    }

    fn fixed_model_registry() -> TestBackendResolver {
        TestBackendResolver::default()
            .with_backend(
                "claude",
                crate::domain::agent_session::CLAUDE_FIXED_MODELS[0],
            )
            .with_backend("codex", crate::domain::agent_session::CODEX_FIXED_MODELS[0])
            .with_default("claude")
    }

    #[test]
    fn create_session_with_initial_model_persists_default_for_claude() {
        // spec: モデル未選択状態は廃止。新規セッションは常に backend の既定モデル
        // （固定リスト先頭）を selected_model に持ち、永続化される。
        let store = crate::test_support::build_session_store();
        let dir = tempfile::tempdir().unwrap();
        let registry = fixed_model_registry();

        // 永続化される selected_model は bare model_id（entry id ではない）。
        let default_model = crate::domain::agent_session::CLAUDE_FIXED_MODELS[0].to_string();

        let session = create_session_with_initial_model(
            &store,
            &registry,
            dir.path(),
            "/repo",
            "claude".to_string(),
            crate::permission::PermissionMode::Edit,
        )
        .unwrap();
        assert_eq!(session.selected_model, Some(default_model.clone()));

        // 永続化されている (on-disk から再ロードしても保持される)
        let reloaded = store
            .load_full_session_for_restore(dir.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.selected_model, Some(default_model));
    }

    #[test]
    fn create_session_with_initial_model_persists_default_for_codex() {
        // spec: codex バックエンドも固定リスト先頭が既定モデルになる。
        let store = crate::test_support::build_session_store();
        let dir = tempfile::tempdir().unwrap();
        let registry = fixed_model_registry();

        // 永続化される selected_model は bare model_id（entry id ではない）。
        let default_model = crate::domain::agent_session::CODEX_FIXED_MODELS[0].to_string();

        let session = create_session_with_initial_model(
            &store,
            &registry,
            dir.path(),
            "/repo",
            "codex".to_string(),
            crate::permission::PermissionMode::Edit,
        )
        .unwrap();
        assert_eq!(session.selected_model, Some(default_model));
    }

    #[test]
    fn create_session_with_model_and_plan_mode_preserves_explicit_selected_model() {
        let store = crate::test_support::build_session_store();
        let dir = tempfile::tempdir().unwrap();
        let registry = fixed_model_registry();
        // 本番呼び出し元（controller / ws / bridge）は entry id を resolve 済みの
        // bare model_id を渡す。usecase は受け取った値をそのまま bare で永続化する。
        let selected_model = "claude-sonnet-4-5".to_string();

        let session = create_session_with_model_and_plan_mode(
            &store,
            &registry,
            dir.path(),
            "/repo",
            "claude".to_string(),
            crate::permission::PermissionMode::Edit,
            Some(selected_model.clone()),
            false,
        )
        .unwrap();

        assert_eq!(session.selected_model, Some(selected_model));
    }

    #[test]
    fn chat_session_without_backend_id_deserializes() {
        let json = r#"{"id":"s1","worktreePath":"/repo","messages":[],"state":"active","createdAt":1000.0,"updatedAt":1000.0,"permissionMode":"edit"}"#;
        let session: ChatSession = serde_json::from_str(json).unwrap();
        assert_eq!(session.backend_id, None);
    }

    #[test]
    fn chat_session_with_backend_id_roundtrip() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1001.0,
            agent_session_id: None,
            context_carry: Some(ContextCarryState::Resumed),
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some("claude".to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("\"backendId\":\"claude\""));
        let back: ChatSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.backend_id, Some("claude".to_string()));
    }

    #[test]
    fn session_summary_includes_backend_id() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::Human,
                content: "Hello".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            context_carry: Some(ContextCarryState::Resumed),
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some("claude".to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        };
        let summary = session.to_summary();
        assert_eq!(summary.backend_id, Some("claude".to_string()));
        assert_eq!(summary.context_carry, Some(ContextCarryState::Resumed));
    }

    // --- resolve_session_backend テスト ---

    mod resolve_session_backend_tests {
        use super::*;

        fn make_session(backend_id: Option<&str>) -> ChatSession {
            ChatSession {
                id: "s_test".to_string(),
                worktree_path: "/repo".to_string(),
                messages: vec![],
                state: SessionState::Closed,
                created_at: 1000.0,
                updated_at: 1000.0,
                agent_session_id: None,
                context_carry: None,
                permission_mode: "edit".to_string(),
                plan_mode: false,
                permission_profile_id: None,
                selected_model: None,
                backend_id: backend_id.map(str::to_string),
                workflow_step_session: false,
                workflow_step_context: None,
                context_epoch: None,
            }
        }

        fn make_registry_with_claude() -> TestBackendResolver {
            TestBackendResolver::default()
                .with_backend(
                    "claude",
                    crate::domain::agent_session::CLAUDE_FIXED_MODELS[0],
                )
                .with_default("claude")
        }

        #[test]
        fn restore_with_valid_backend_id_succeeds() {
            let registry = make_registry_with_claude();
            let mut session = make_session(Some("claude"));
            let result = resolve_session_backend(&mut session, &registry);
            assert!(result.is_ok());
            assert_eq!(session.backend_id, Some("claude".to_string()));
        }

        #[test]
        fn restore_with_invalid_backend_id_returns_error() {
            let registry = make_registry_with_claude();
            let mut session = make_session(Some("codex"));
            let result = resolve_session_backend(&mut session, &registry);
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("codex"));
        }

        #[test]
        fn restore_without_backend_id_assigns_default() {
            let registry = make_registry_with_claude();
            let mut session = make_session(None);
            assert_eq!(session.backend_id, None);

            let result = resolve_session_backend(&mut session, &registry);

            assert!(result.is_ok());
            assert_eq!(session.backend_id, Some("claude".to_string()));
        }
    }
}

/// workflow step session の context が meta 経路（meta.json 永続化を含む）を通じて
/// SessionSummary / ChatSession まで保持されることを保証する回帰テスト。
/// context が meta で None に落ちると Workflow View の step ヘッダー（`step.title` =
/// step 名）が消えるため、その退行を防ぐ。
#[cfg(test)]
mod workflow_step_context_meta_tests {
    use super::*;

    fn step_context_dto() -> WorkflowStepContextDto {
        WorkflowStepContextDto {
            run_id: "run-1".to_string(),
            workflow_name: "wf".to_string(),
            step_name: "step-a".to_string(),
            run_index: 0,
            parent_step_name: None,
            parent_run_index: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: Some(2),
            stale_timeout_secs: None,
        }
    }

    fn session_with_context(context: Option<WorkflowStepContextDto>) -> ChatSession {
        ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: SessionState::Active,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            selected_model: None,
            permission_profile_id: None,
            backend_id: Some("claude".to_string()),
            workflow_step_session: false,
            workflow_step_context: context,
            context_epoch: None,
        }
    }

    #[test]
    fn workflow_step_context_is_workflow_state_only() {
        let dto = workflow_step_context_mapper::to_dto(WorkflowStepContext {
            run_id: "run-1".to_string(),
            workflow_name: "wf".to_string(),
            step_name: "step-a".to_string(),
            run_index: 0,
            parent_step_name: None,
            parent_run_index: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        });

        let dto_json = serde_json::to_string(&dto).expect("serialize dto");
        let restored = workflow_step_context_mapper::to_domain(dto.clone());
        let session = session_with_context(Some(dto));
        let session_json = serde_json::to_string(&session).expect("serialize session");
        let meta = SessionMeta::from_session(&session);
        let meta_json = serde_json::to_string(&meta).expect("serialize meta");
        let summary_json = serde_json::to_string(&session.to_summary()).expect("serialize summary");
        let restored_from_meta = meta.to_session(Vec::new());

        assert!(!dto_json.contains("workflowInstruction"));
        assert!(!session_json.contains("workflowInstruction"));
        assert_eq!(restored.step_name, "step-a");
        assert_eq!(
            meta.workflow_step_context
                .as_ref()
                .map(|context| context.step_name.as_str()),
            Some("step-a")
        );
        assert!(!meta_json.contains("workflowInstruction"));
        assert!(!summary_json.contains("workflowInstruction"));
        assert_eq!(
            restored_from_meta
                .workflow_step_context
                .map(|context| context.step_name),
            Some("step-a".to_string())
        );
    }

    #[test]
    fn meta_round_trip_preserves_workflow_step_context() {
        let meta = SessionMeta::from_session(&session_with_context(Some(step_context_dto())));

        // from_session で context が meta に乗る。
        assert!(meta.workflow_step_session);
        assert_eq!(
            meta.workflow_step_context
                .as_ref()
                .map(|c| c.step_name.as_str()),
            Some("step-a")
        );

        // to_summary / to_session の両経路で context が届く（ヘッダー表示の正典経路）。
        let summary = meta.to_summary();
        assert!(summary.workflow_step_session);
        assert_eq!(summary.workflow_step_context, Some(step_context_dto()));
        let restored_session = meta.to_session(Vec::new());
        assert!(restored_session.workflow_step_session);
        assert_eq!(
            restored_session.workflow_step_context,
            Some(step_context_dto())
        );

        // meta.json の serde round-trip でも context を保持する。
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("\"startupMaxRetries\":2"));
        let restored: SessionMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.workflow_step_context, Some(step_context_dto()));
        assert!(restored.is_workflow_step_session());
    }

    #[test]
    fn meta_without_context_stays_none() {
        let meta = SessionMeta::from_session(&session_with_context(None));
        assert!(meta.workflow_step_context.is_none());
        assert_eq!(meta.to_summary().workflow_step_context, None);
    }
}
