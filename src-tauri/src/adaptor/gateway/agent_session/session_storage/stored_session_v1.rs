//! Gateway-owned V1 DTOs for persisted session messages and test-only file indexes.

#[cfg(test)]
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::usecase::agent_session::session::MessageIndexEntry;
use crate::usecase::agent_session::session::{
    ActivityEntry, ChatMessage, ChatSession, ContextCarryState, MessageMention, MessageRole,
    SessionState, WorkflowNodeContextDto,
};

use super::stored_message_part_v1::{
    contains_additive_fields, decode_stored_message_part_v1, DecodedStoredMessagePartV1,
    IncompatibleStoredEvent, PreservedStoredPayload, StoredMessagePartV1, StoredPayloadSource,
};

#[derive(Debug, Clone)]
pub(crate) struct DecodedStoredChatMessageV1 {
    pub message: ChatMessage,
    #[cfg_attr(not(test), allow(dead_code))]
    pub preserved_additive_payload: Option<PreservedStoredPayload>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredChatMessageV1 {
    id: String,
    role: StoredMessageRoleV1,
    content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activities: Option<Vec<StoredActivityEntryV1>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parts: Option<Vec<StoredMessagePartV1>>,
    #[serde(default)]
    streaming_final_seq: u64,
    timestamp: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mentions: Option<Vec<StoredMessageMentionV1>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredChatSessionV1 {
    id: String,
    worktree_path: String,
    messages: Vec<StoredChatMessageV1>,
    state: StoredSessionStateV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_reason: Option<String>,
    created_at: f64,
    updated_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_carry: Option<StoredContextCarryStateV1>,
    permission_mode: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    plan_mode: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selected_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    permission_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    backend_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    workflow_node_session: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workflow_node_context: Option<StoredWorkflowNodeContextV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredMessageRoleV1 {
    Human,
    Agent,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredSessionStateV1 {
    Active,
    Idle,
    Done,
    Error,
    Closed,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredContextCarryStateV1 {
    Resumed,
    Reinjected,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredMessageMentionV1 {
    file_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    start_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredWorkflowNodeContextV1 {
    execution_id: String,
    node_execution_id: String,
    workflow_name: String,
    node_name: String,
    attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_node_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_attempt: Option<u32>,
    order: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    startup_timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    startup_max_retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stale_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StoredActivityEntryV1 {
    ToolUse {
        tool: String,
        input: serde_json::Value,
        id: String,
    },
    ToolResult {
        content: String,
        #[serde(rename = "isError")]
        is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "toolUseId")]
        tool_use_id: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "contentRef"
        )]
        content_ref: Option<StoredToolOutputRefV1>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<StoredToolOutputSummaryV1>,
    },
    PermissionResult {
        #[serde(rename = "toolName")]
        tool_name: String,
        status: String,
        summary: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAttachmentV1 {
    id: String,
    media_type: String,
    byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredToolOutputRefV1 {
    id: String,
    byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredToolOutputSummaryV1 {
    line_count: u64,
    byte_size: u64,
    is_error: bool,
    #[serde(default)]
    truncated: bool,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredMessageIndexEntryV1 {
    id: String,
    seq: u64,
    role: StoredMessageRoleV1,
    timestamp: f64,
    content_hash: String,
    #[serde(default)]
    attachment_refs: Vec<StoredAttachmentV1>,
    #[serde(default)]
    tool_output_refs: Vec<StoredToolOutputRefV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_meta: Option<serde_json::Value>,
}

pub(crate) fn decode_chat_message_v1(
    raw: &[u8],
    source: StoredPayloadSource,
) -> Result<DecodedStoredChatMessageV1, IncompatibleStoredEvent> {
    let original: serde_json::Value = serde_json::from_slice(raw)
        .map_err(|error| incompatible("chat_message", format!("invalid JSON: {error}")))?;
    let decoded_parts = if let Some(parts) = original
        .as_object()
        .and_then(|object| object.get("parts"))
        .and_then(serde_json::Value::as_array)
    {
        let mut decoded_parts = Vec::with_capacity(parts.len());
        for (ordinal, part) in parts.iter().enumerate() {
            let part_raw = serde_json::to_vec(part)
                .map_err(|error| incompatible("message_part", error.to_string()))?;
            let DecodedStoredMessagePartV1 {
                part,
                preserved_additive_payload: _,
            } = decode_stored_message_part_v1(
                &part_raw,
                1,
                StoredPayloadSource {
                    source_id: source.source_id.clone(),
                    record_ordinal: u64::try_from(ordinal).ok(),
                },
            )?;
            decoded_parts.push(part);
        }
        Some(decoded_parts)
    } else {
        None
    };
    let stored: StoredChatMessageV1 = serde_json::from_value(original.clone())
        .map_err(|error| incompatible("chat_message", format!("invalid known payload: {error}")))?;
    let canonical = serde_json::to_value(&stored)
        .expect("stored chat message serialization must be deterministic");
    let has_additive = contains_additive_fields(&original, &canonical);
    let mut message: ChatMessage = stored.try_into()?;
    if decoded_parts.is_some() {
        message.parts = decoded_parts;
    }
    Ok(DecodedStoredChatMessageV1 {
        message,
        preserved_additive_payload: has_additive.then(|| PreservedStoredPayload {
            source,
            payload_version: 1,
            type_tag: "chat_message".to_string(),
            raw_bytes: raw.to_vec(),
        }),
    })
}

pub(crate) fn encode_chat_message_v1(message: &ChatMessage) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&StoredChatMessageV1::from(message))
}

#[cfg(test)]
pub(crate) fn encode_chat_message_pretty_v1(
    message: &ChatMessage,
    preserved: Option<&PreservedStoredPayload>,
) -> Result<Vec<u8>, serde_json::Error> {
    let canonical = serde_json::to_value(StoredChatMessageV1::from(message))?;
    let value = if let Some(preserved) = preserved {
        let mut original: serde_json::Value = serde_json::from_slice(&preserved.raw_bytes)?;
        merge_known_fields(&mut original, &canonical);
        original
    } else {
        canonical
    };
    serde_json::to_vec_pretty(&value)
}

#[cfg(test)]
pub(crate) fn encode_chat_session_v1(session: &ChatSession) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&StoredChatSessionV1::from(session))
}

#[cfg(test)]
pub(crate) fn decode_chat_session_v1(raw: &[u8]) -> Result<ChatSession, IncompatibleStoredEvent> {
    let stored: StoredChatSessionV1 = serde_json::from_slice(raw)
        .map_err(|error| incompatible("chat_session", format!("invalid known payload: {error}")))?;
    stored.try_into()
}

#[cfg(test)]
pub(super) fn encode_message_index_v1(
    index: &[MessageIndexEntry],
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec_pretty(
        &index
            .iter()
            .map(StoredMessageIndexEntryV1::from)
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
pub(super) fn decode_message_index_v1(raw: &[u8]) -> Result<Vec<MessageIndexEntry>, String> {
    let stored: Vec<StoredMessageIndexEntryV1> =
        serde_json::from_slice(raw).map_err(|error| error.to_string())?;
    Ok(stored.into_iter().map(Into::into).collect())
}

#[cfg(test)]
pub(crate) fn encode_activity_entry_v1(
    value: &ActivityEntry,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&StoredActivityEntryV1::from(value))
}

#[cfg(test)]
pub(crate) fn decode_activity_entry_v1(raw: &[u8]) -> Result<ActivityEntry, serde_json::Error> {
    serde_json::from_slice::<StoredActivityEntryV1>(raw).map(Into::into)
}

#[cfg(test)]
pub(crate) fn write_message_index_v1(
    path: &Path,
    index: &[MessageIndexEntry],
) -> Result<(), String> {
    let bytes = encode_message_index_v1(index)
        .map_err(|error| format!("Failed to serialize session index: {error}"))?;
    super::layout::write_binary_atomic(path, &bytes, "session index")
}

#[cfg(test)]
pub(super) fn preservation_sidecar_path(path: &Path) -> PathBuf {
    path.with_extension("json.preserved-v1")
}

#[cfg(test)]
pub(super) fn decode_preservation_sidecar(
    raw: &[u8],
) -> Result<PreservedStoredPayload, serde_json::Error> {
    serde_json::from_slice(raw)
}

#[cfg(test)]
pub(super) fn encode_preservation_sidecar(
    value: &PreservedStoredPayload,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec_pretty(value)
}

#[cfg(test)]
fn merge_known_fields(original: &mut serde_json::Value, canonical: &serde_json::Value) {
    match (original, canonical) {
        (serde_json::Value::Object(original), serde_json::Value::Object(canonical)) => {
            for (key, canonical_value) in canonical {
                if let Some(original_value) = original.get_mut(key) {
                    merge_known_fields(original_value, canonical_value);
                } else {
                    original.insert(key.clone(), canonical_value.clone());
                }
            }
        }
        (serde_json::Value::Array(original), serde_json::Value::Array(canonical))
            if original.len() == canonical.len() =>
        {
            for (original, canonical) in original.iter_mut().zip(canonical) {
                merge_known_fields(original, canonical);
            }
        }
        (original, canonical) => *original = canonical.clone(),
    }
}

fn incompatible(type_tag: &str, reason: String) -> IncompatibleStoredEvent {
    IncompatibleStoredEvent {
        type_tag: type_tag.to_string(),
        payload_version: 1,
        reason,
    }
}

impl From<&ChatMessage> for StoredChatMessageV1 {
    fn from(value: &ChatMessage) -> Self {
        Self {
            id: value.id.clone(),
            role: (&value.role).into(),
            content: value.content.clone(),
            thinking: value.thinking.clone(),
            activities: value
                .activities
                .as_ref()
                .map(|items| items.iter().map(Into::into).collect()),
            parts: value
                .parts
                .as_ref()
                .map(|parts| parts.iter().map(Into::into).collect()),
            streaming_final_seq: value.streaming_final_seq,
            timestamp: value.timestamp,
            mentions: value
                .mentions
                .as_ref()
                .map(|items| items.iter().map(Into::into).collect()),
        }
    }
}

impl TryFrom<StoredChatMessageV1> for ChatMessage {
    type Error = IncompatibleStoredEvent;
    fn try_from(value: StoredChatMessageV1) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            role: value.role.into(),
            content: value.content,
            thinking: value.thinking,
            activities: value
                .activities
                .map(|items| items.into_iter().map(Into::into).collect()),
            parts: value
                .parts
                .map(|parts| parts.into_iter().map(TryInto::try_into).collect())
                .transpose()?,
            streaming_final_seq: value.streaming_final_seq,
            timestamp: value.timestamp,
            mentions: value
                .mentions
                .map(|items| items.into_iter().map(Into::into).collect()),
        })
    }
}

impl From<&ChatSession> for StoredChatSessionV1 {
    fn from(value: &ChatSession) -> Self {
        Self {
            id: value.id.clone(),
            worktree_path: value.worktree_path.clone(),
            messages: value.messages.iter().map(Into::into).collect(),
            state: (&value.state).into(),
            error_reason: value.error_reason.clone(),
            created_at: value.created_at,
            updated_at: value.updated_at,
            agent_session_id: value.agent_session_id.clone(),
            context_carry: value.context_carry.as_ref().map(Into::into),
            permission_mode: value.permission_mode.clone(),
            plan_mode: value.plan_mode,
            selected_model: value.selected_model.clone(),
            permission_profile_id: value.permission_profile_id.clone(),
            backend_id: value.backend_id.clone(),
            workflow_node_session: value.workflow_node_session,
            workflow_node_context: value.workflow_node_context.as_ref().map(Into::into),
        }
    }
}

impl TryFrom<StoredChatSessionV1> for ChatSession {
    type Error = IncompatibleStoredEvent;
    fn try_from(value: StoredChatSessionV1) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            worktree_path: value.worktree_path,
            messages: value
                .messages
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
            state: value.state.into(),
            error_reason: value.error_reason,
            created_at: value.created_at,
            updated_at: value.updated_at,
            agent_session_id: value.agent_session_id,
            context_carry: value.context_carry.map(Into::into),
            permission_mode: value.permission_mode,
            plan_mode: value.plan_mode,
            selected_model: value.selected_model,
            permission_profile_id: value.permission_profile_id,
            backend_id: value.backend_id,
            workflow_node_session: value.workflow_node_session,
            workflow_node_context: value.workflow_node_context.map(Into::into),
            context_epoch: None,
        })
    }
}

impl From<&MessageRole> for StoredMessageRoleV1 {
    fn from(value: &MessageRole) -> Self {
        match value {
            MessageRole::Human => Self::Human,
            MessageRole::Agent => Self::Agent,
            MessageRole::System => Self::System,
        }
    }
}
impl From<StoredMessageRoleV1> for MessageRole {
    fn from(value: StoredMessageRoleV1) -> Self {
        match value {
            StoredMessageRoleV1::Human => Self::Human,
            StoredMessageRoleV1::Agent => Self::Agent,
            StoredMessageRoleV1::System => Self::System,
        }
    }
}
impl From<&SessionState> for StoredSessionStateV1 {
    fn from(value: &SessionState) -> Self {
        match value {
            SessionState::Active => Self::Active,
            SessionState::Idle => Self::Idle,
            SessionState::Done => Self::Done,
            SessionState::Error => Self::Error,
            SessionState::Closed => Self::Closed,
            SessionState::Archived => Self::Archived,
        }
    }
}
impl From<StoredSessionStateV1> for SessionState {
    fn from(value: StoredSessionStateV1) -> Self {
        match value {
            StoredSessionStateV1::Active => Self::Active,
            StoredSessionStateV1::Idle => Self::Idle,
            StoredSessionStateV1::Done => Self::Done,
            StoredSessionStateV1::Error => Self::Error,
            StoredSessionStateV1::Closed => Self::Closed,
            StoredSessionStateV1::Archived => Self::Archived,
        }
    }
}
impl From<&ContextCarryState> for StoredContextCarryStateV1 {
    fn from(value: &ContextCarryState) -> Self {
        match value {
            ContextCarryState::Resumed => Self::Resumed,
            ContextCarryState::Reinjected => Self::Reinjected,
            ContextCarryState::Failed => Self::Failed,
        }
    }
}
impl From<StoredContextCarryStateV1> for ContextCarryState {
    fn from(value: StoredContextCarryStateV1) -> Self {
        match value {
            StoredContextCarryStateV1::Resumed => Self::Resumed,
            StoredContextCarryStateV1::Reinjected => Self::Reinjected,
            StoredContextCarryStateV1::Failed => Self::Failed,
        }
    }
}
impl From<&MessageMention> for StoredMessageMentionV1 {
    fn from(value: &MessageMention) -> Self {
        Self {
            file_path: value.file_path.clone(),
            start_line: value.start_line,
            end_line: value.end_line,
        }
    }
}
impl From<StoredMessageMentionV1> for MessageMention {
    fn from(value: StoredMessageMentionV1) -> Self {
        Self {
            file_path: value.file_path,
            start_line: value.start_line,
            end_line: value.end_line,
        }
    }
}
impl From<&WorkflowNodeContextDto> for StoredWorkflowNodeContextV1 {
    fn from(v: &WorkflowNodeContextDto) -> Self {
        Self {
            execution_id: v.execution_id.clone(),
            node_execution_id: v.node_execution_id.clone(),
            workflow_name: v.workflow_name.clone(),
            node_name: v.node_name.clone(),
            attempt: v.attempt,
            parent_node_name: v.parent_node_name.clone(),
            parent_attempt: v.parent_attempt,
            order: v.order,
            startup_timeout_secs: v.startup_timeout_secs,
            startup_max_retries: v.startup_max_retries,
            stale_timeout_secs: v.stale_timeout_secs,
        }
    }
}
impl From<StoredWorkflowNodeContextV1> for WorkflowNodeContextDto {
    fn from(v: StoredWorkflowNodeContextV1) -> Self {
        Self {
            execution_id: v.execution_id,
            node_execution_id: v.node_execution_id,
            workflow_name: v.workflow_name,
            node_name: v.node_name,
            attempt: v.attempt,
            parent_node_name: v.parent_node_name,
            parent_attempt: v.parent_attempt,
            order: v.order,
            startup_timeout_secs: v.startup_timeout_secs,
            startup_max_retries: v.startup_max_retries,
            stale_timeout_secs: v.stale_timeout_secs,
        }
    }
}
impl From<&ActivityEntry> for StoredActivityEntryV1 {
    fn from(value: &ActivityEntry) -> Self {
        match value {
            ActivityEntry::ToolUse { tool, input, id } => Self::ToolUse {
                tool: tool.clone(),
                input: input.clone(),
                id: id.clone(),
            },
            ActivityEntry::ToolResult {
                content,
                is_error,
                tool_use_id,
                content_ref,
                summary,
            } => Self::ToolResult {
                content: content.clone(),
                is_error: *is_error,
                tool_use_id: tool_use_id.clone(),
                content_ref: content_ref.as_ref().map(Into::into),
                summary: summary.as_ref().map(Into::into),
            },
            ActivityEntry::PermissionResult {
                tool_name,
                status,
                summary,
            } => Self::PermissionResult {
                tool_name: tool_name.clone(),
                status: status.clone(),
                summary: summary.clone(),
            },
        }
    }
}
impl From<StoredActivityEntryV1> for ActivityEntry {
    fn from(value: StoredActivityEntryV1) -> Self {
        match value {
            StoredActivityEntryV1::ToolUse { tool, input, id } => Self::ToolUse { tool, input, id },
            StoredActivityEntryV1::ToolResult {
                content,
                is_error,
                tool_use_id,
                content_ref,
                summary,
            } => Self::ToolResult {
                content,
                is_error,
                tool_use_id,
                content_ref: content_ref.map(Into::into),
                summary: summary.map(Into::into),
            },
            StoredActivityEntryV1::PermissionResult {
                tool_name,
                status,
                summary,
            } => Self::PermissionResult {
                tool_name,
                status,
                summary,
            },
        }
    }
}
impl From<&crate::domain::agent_session::entities::Attachment> for StoredAttachmentV1 {
    fn from(v: &crate::domain::agent_session::entities::Attachment) -> Self {
        Self {
            id: v.id.clone(),
            media_type: v.media_type.clone(),
            byte_size: v.byte_size,
        }
    }
}
impl From<StoredAttachmentV1> for crate::domain::agent_session::entities::Attachment {
    fn from(v: StoredAttachmentV1) -> Self {
        Self {
            id: v.id,
            media_type: v.media_type,
            byte_size: v.byte_size,
        }
    }
}
impl From<&crate::domain::agent_session::value_objects::ToolOutputRef> for StoredToolOutputRefV1 {
    fn from(v: &crate::domain::agent_session::value_objects::ToolOutputRef) -> Self {
        Self {
            id: v.id.clone(),
            byte_size: v.byte_size,
        }
    }
}
impl From<StoredToolOutputRefV1> for crate::domain::agent_session::value_objects::ToolOutputRef {
    fn from(v: StoredToolOutputRefV1) -> Self {
        Self {
            id: v.id,
            byte_size: v.byte_size,
        }
    }
}
impl From<&crate::domain::agent_session::value_objects::ToolOutputSummary>
    for StoredToolOutputSummaryV1
{
    fn from(v: &crate::domain::agent_session::value_objects::ToolOutputSummary) -> Self {
        Self {
            line_count: v.line_count,
            byte_size: v.byte_size,
            is_error: v.is_error,
            truncated: v.truncated,
        }
    }
}
impl From<StoredToolOutputSummaryV1>
    for crate::domain::agent_session::value_objects::ToolOutputSummary
{
    fn from(v: StoredToolOutputSummaryV1) -> Self {
        Self {
            line_count: v.line_count,
            byte_size: v.byte_size,
            is_error: v.is_error,
            truncated: v.truncated,
        }
    }
}
#[cfg(test)]
impl From<&MessageIndexEntry> for StoredMessageIndexEntryV1 {
    fn from(v: &MessageIndexEntry) -> Self {
        Self {
            id: v.id.clone(),
            seq: v.seq,
            role: (&v.role).into(),
            timestamp: v.timestamp,
            content_hash: v.content_hash.clone(),
            attachment_refs: v.attachment_refs.iter().map(Into::into).collect(),
            tool_output_refs: v.tool_output_refs.iter().map(Into::into).collect(),
            token_meta: v.token_meta.clone(),
        }
    }
}
#[cfg(test)]
impl From<StoredMessageIndexEntryV1> for MessageIndexEntry {
    fn from(v: StoredMessageIndexEntryV1) -> Self {
        Self {
            id: v.id,
            seq: v.seq,
            role: v.role.into(),
            timestamp: v.timestamp,
            content_hash: v.content_hash,
            attachment_refs: v.attachment_refs.into_iter().map(Into::into).collect(),
            tool_output_refs: v.tool_output_refs.into_iter().map(Into::into).collect(),
            token_meta: v.token_meta,
        }
    }
}
