//! Versioned legacy JSON representation of the canonical agent message part.
//!
//! The domain type owns meaning and deliberately has no serde dependency. This
//! module owns the pre-SQLite JSON shape. Domain values never implement serde;
//! readers and writers must cross this explicit DTO boundary.

use serde::{Deserialize, Serialize};

use crate::domain::agent_session::entities::{
    Attachment, MessagePart, PermissionAllowedPrompt, PermissionDecision, PermissionPartStatus,
    PermissionQuestion, PermissionQuestionOption, PermissionRequest, PermissionRequestBody,
    PermissionRequestStatus,
};
use crate::domain::agent_session::value_objects::{
    JsonPayload, SystemNotificationType, TodoListItem, ToolOutputRef, ToolOutputSummary,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredPayloadSource {
    pub source_id: String,
    pub record_ordinal: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreservedStoredPayload {
    pub source: StoredPayloadSource,
    pub payload_version: u32,
    pub type_tag: String,
    pub raw_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("incompatible stored event type={type_tag} version={payload_version}: {reason}")]
pub(crate) struct IncompatibleStoredEvent {
    pub type_tag: String,
    pub payload_version: u32,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DecodedStoredMessagePartV1 {
    pub part: MessagePart,
    pub preserved_additive_payload: Option<PreservedStoredPayload>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub(super) enum StoredMessagePartV1 {
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
        content_ref: Option<StoredToolOutputRefV1>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        summary: Option<StoredToolOutputSummaryV1>,
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
        request: StoredPermissionRequestV1,
        status: StoredPermissionPartStatusV1,
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
        items: Vec<StoredTodoListItemV1>,
    },
    SystemNotification {
        #[serde(rename = "notificationType")]
        notification_type: StoredSystemNotificationTypeV1,
        status: String,
        label: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default, rename = "hookId")]
        hook_id: Option<String>,
    },
    Image {
        data: String,
        #[serde(rename = "mediaType")]
        media_type: String,
    },
    ImageRef {
        attachment: StoredAttachmentV1,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredAttachmentV1 {
    id: String,
    media_type: String,
    byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredToolOutputRefV1 {
    id: String,
    byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredToolOutputSummaryV1 {
    line_count: u64,
    byte_size: u64,
    is_error: bool,
    #[serde(default)]
    truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredTodoListItemV1 {
    text: String,
    completed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredPermissionPartStatusV1 {
    Pending,
    Allowed,
    Denied,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredPermissionRequestKindV1 {
    ToolApproval,
    PlanApproval,
    Question,
    PermissionGrant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredPermissionRequestV1 {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    tool_use_id: Option<String>,
    tool_name: String,
    kind: StoredPermissionRequestKindV1,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    plan: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_prompts: Vec<StoredPermissionAllowedPromptV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    questions: Vec<StoredPermissionQuestionV1>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    decision_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredPermissionAllowedPromptV1 {
    tool: String,
    prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredPermissionQuestionV1 {
    question: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    header: Option<String>,
    #[serde(default)]
    options: Vec<StoredPermissionQuestionOptionV1>,
    multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredPermissionQuestionOptionV1 {
    label: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredSystemNotificationTypeV1 {
    Compaction,
    SessionRecovery,
}

impl From<&MessagePart> for StoredMessagePartV1 {
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
                tool: tool.clone(),
                input: json_value(input),
                id: id.clone(),
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
                content_ref: content_ref.as_ref().map(|value| StoredToolOutputRefV1 {
                    id: value.id.clone(),
                    byte_size: value.byte_size,
                }),
                summary: summary.as_ref().map(|value| StoredToolOutputSummaryV1 {
                    line_count: value.line_count,
                    byte_size: value.byte_size,
                    is_error: value.is_error,
                    truncated: value.truncated,
                }),
            },
            MessagePart::Error {
                content,
                parent_tool_use_id,
            } => Self::Error {
                content: content.clone(),
                parent_tool_use_id: parent_tool_use_id.clone(),
            },
            MessagePart::Permission {
                request,
                status,
                answers,
                parent_tool_use_id,
            } => Self::Permission {
                request: request.into(),
                status: (*status).into(),
                answers: answers.as_ref().map(json_value),
                parent_tool_use_id: parent_tool_use_id.clone(),
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
                    .map(|item| StoredTodoListItemV1 {
                        text: item.text.clone(),
                        completed: item.completed,
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
            MessagePart::ImageRef { attachment } => Self::ImageRef {
                attachment: StoredAttachmentV1 {
                    id: attachment.id.clone(),
                    media_type: attachment.media_type.clone(),
                    byte_size: attachment.byte_size,
                },
            },
        }
    }
}

impl TryFrom<StoredMessagePartV1> for MessagePart {
    type Error = IncompatibleStoredEvent;

    fn try_from(part: StoredMessagePartV1) -> Result<Self, IncompatibleStoredEvent> {
        Ok(match part {
            StoredMessagePartV1::Thinking {
                content,
                parent_tool_use_id,
            } => Self::Thinking {
                content,
                parent_tool_use_id,
            },
            StoredMessagePartV1::Text {
                content,
                parent_tool_use_id,
            } => Self::Text {
                content,
                parent_tool_use_id,
            },
            StoredMessagePartV1::ToolUse {
                tool,
                input,
                id,
                parent_tool_use_id,
            } => Self::ToolUse {
                id,
                tool,
                input: json_payload(input),
                parent_tool_use_id,
            },
            StoredMessagePartV1::ToolResult {
                content,
                is_error,
                tool_use_id,
                parent_tool_use_id,
                content_ref,
                summary,
            } => Self::ToolResult {
                content,
                is_error,
                tool_use_id,
                parent_tool_use_id,
                content_ref: content_ref.map(|value| ToolOutputRef {
                    id: value.id,
                    byte_size: value.byte_size,
                }),
                summary: summary.map(|value| ToolOutputSummary {
                    line_count: value.line_count,
                    byte_size: value.byte_size,
                    is_error: value.is_error,
                    truncated: value.truncated,
                }),
            },
            StoredMessagePartV1::Error {
                content,
                parent_tool_use_id,
            } => Self::Error {
                content,
                parent_tool_use_id,
            },
            StoredMessagePartV1::Permission {
                request,
                status,
                answers,
                parent_tool_use_id,
            } => {
                let status: PermissionPartStatus = status.into();
                let answers = answers.map(json_payload);
                let mut request = request.into_domain(status, answers.clone())?;
                request.parent_tool_use_id = parent_tool_use_id.clone();
                Self::Permission {
                    request,
                    status,
                    answers,
                    parent_tool_use_id,
                }
            }
            StoredMessagePartV1::TaskStatus {
                task_tool_use_id,
                status,
                description,
                summary,
            } => Self::TaskStatus {
                task_tool_use_id,
                status,
                description,
                summary,
            },
            StoredMessagePartV1::TodoListSnapshot { items } => Self::TodoListSnapshot {
                items: items
                    .into_iter()
                    .map(|item| TodoListItem {
                        text: item.text,
                        completed: item.completed,
                    })
                    .collect(),
            },
            StoredMessagePartV1::SystemNotification {
                notification_type,
                status,
                label,
                detail,
                hook_id,
            } => Self::SystemNotification {
                notification_type: notification_type.into(),
                status,
                label,
                detail,
                hook_id,
            },
            StoredMessagePartV1::Image { data, media_type } => Self::Image { data, media_type },
            StoredMessagePartV1::ImageRef { attachment } => Self::ImageRef {
                attachment: Attachment {
                    id: attachment.id,
                    media_type: attachment.media_type,
                    byte_size: attachment.byte_size,
                },
            },
        })
    }
}

impl From<&PermissionRequest> for StoredPermissionRequestV1 {
    fn from(request: &PermissionRequest) -> Self {
        let (kind, input, plan, allowed_prompts, questions) = match &request.body {
            PermissionRequestBody::ToolApproval { input } => (
                StoredPermissionRequestKindV1::ToolApproval,
                Some(json_value(input)),
                None,
                Vec::new(),
                Vec::new(),
            ),
            PermissionRequestBody::PlanApproval {
                plan,
                allowed_prompts,
            } => (
                StoredPermissionRequestKindV1::PlanApproval,
                None,
                Some(plan.clone()),
                allowed_prompts
                    .iter()
                    .map(|prompt| StoredPermissionAllowedPromptV1 {
                        tool: prompt.tool.clone(),
                        prompt: prompt.prompt.clone(),
                    })
                    .collect(),
                Vec::new(),
            ),
            PermissionRequestBody::Question { questions } => (
                StoredPermissionRequestKindV1::Question,
                None,
                None,
                Vec::new(),
                questions
                    .iter()
                    .map(|question| StoredPermissionQuestionV1 {
                        question: question.question.clone(),
                        header: question.header.clone(),
                        options: question
                            .options
                            .iter()
                            .map(|option| StoredPermissionQuestionOptionV1 {
                                label: option.label.clone(),
                                description: option.description.clone(),
                            })
                            .collect(),
                        multi_select: question.multi_select,
                    })
                    .collect(),
            ),
            PermissionRequestBody::PermissionGrant { requested } => (
                StoredPermissionRequestKindV1::PermissionGrant,
                Some(json_value(requested)),
                None,
                Vec::new(),
                Vec::new(),
            ),
        };
        Self {
            id: request.id.clone(),
            tool_use_id: request.tool_use_id.clone(),
            tool_name: request.tool_name.clone(),
            kind,
            input,
            plan,
            allowed_prompts,
            questions,
            title: request.title.clone(),
            display_name: request.display_name.clone(),
            description: request.description.clone(),
            decision_reason: request.decision_reason.clone(),
        }
    }
}

impl StoredPermissionRequestV1 {
    pub(super) fn into_domain(
        self,
        part_status: PermissionPartStatus,
        answers: Option<JsonPayload>,
    ) -> Result<PermissionRequest, IncompatibleStoredEvent> {
        let body = match self.kind {
            StoredPermissionRequestKindV1::ToolApproval => PermissionRequestBody::ToolApproval {
                input: required_json(self.input, "tool_approval.input")?,
            },
            StoredPermissionRequestKindV1::PlanApproval => PermissionRequestBody::PlanApproval {
                plan: required(self.plan, "plan_approval.plan")?,
                allowed_prompts: self
                    .allowed_prompts
                    .into_iter()
                    .map(|prompt| PermissionAllowedPrompt {
                        tool: prompt.tool,
                        prompt: prompt.prompt,
                    })
                    .collect(),
            },
            StoredPermissionRequestKindV1::Question => PermissionRequestBody::Question {
                questions: self
                    .questions
                    .into_iter()
                    .map(|question| PermissionQuestion {
                        question: question.question,
                        header: question.header,
                        options: question
                            .options
                            .into_iter()
                            .map(|option| PermissionQuestionOption {
                                label: option.label,
                                description: option.description,
                            })
                            .collect(),
                        multi_select: question.multi_select,
                    })
                    .collect(),
            },
            StoredPermissionRequestKindV1::PermissionGrant => {
                PermissionRequestBody::PermissionGrant {
                    requested: required_json(self.input, "permission_grant.input")?,
                }
            }
        };
        let status = match part_status {
            PermissionPartStatus::Pending => PermissionRequestStatus::Pending,
            PermissionPartStatus::Allowed => PermissionRequestStatus::Resolved {
                decision: PermissionDecision::Allowed,
                answers,
            },
            PermissionPartStatus::Denied => PermissionRequestStatus::Resolved {
                decision: PermissionDecision::Denied,
                answers,
            },
            PermissionPartStatus::Cancelled => PermissionRequestStatus::Resolved {
                decision: PermissionDecision::Cancelled,
                answers,
            },
        };
        Ok(PermissionRequest {
            id: self.id,
            tool_use_id: self.tool_use_id,
            parent_tool_use_id: None,
            tool_name: self.tool_name,
            body,
            title: self.title,
            display_name: self.display_name,
            description: self.description,
            decision_reason: self.decision_reason,
            status,
        })
    }
}

fn required<T>(value: Option<T>, field: &str) -> Result<T, IncompatibleStoredEvent> {
    value.ok_or_else(|| incompatible("permission", format!("missing required field {field}")))
}

fn required_json(
    value: Option<serde_json::Value>,
    field: &str,
) -> Result<JsonPayload, IncompatibleStoredEvent> {
    required(value, field).map(json_payload)
}

fn incompatible(type_tag: impl Into<String>, reason: impl Into<String>) -> IncompatibleStoredEvent {
    IncompatibleStoredEvent {
        type_tag: type_tag.into(),
        payload_version: 1,
        reason: reason.into(),
    }
}

fn json_value(payload: &JsonPayload) -> serde_json::Value {
    serde_json::from_str(payload.as_str())
        .expect("domain JsonPayload must be validated at its boundary")
}

fn json_payload(value: serde_json::Value) -> JsonPayload {
    JsonPayload::new_unchecked(
        serde_json::to_string(&value).expect("JSON value serialization cannot fail"),
    )
}

impl From<PermissionPartStatus> for StoredPermissionPartStatusV1 {
    fn from(value: PermissionPartStatus) -> Self {
        match value {
            PermissionPartStatus::Pending => Self::Pending,
            PermissionPartStatus::Allowed => Self::Allowed,
            PermissionPartStatus::Denied => Self::Denied,
            PermissionPartStatus::Cancelled => Self::Cancelled,
        }
    }
}
impl From<StoredPermissionPartStatusV1> for PermissionPartStatus {
    fn from(value: StoredPermissionPartStatusV1) -> Self {
        match value {
            StoredPermissionPartStatusV1::Pending => Self::Pending,
            StoredPermissionPartStatusV1::Allowed => Self::Allowed,
            StoredPermissionPartStatusV1::Denied => Self::Denied,
            StoredPermissionPartStatusV1::Cancelled => Self::Cancelled,
        }
    }
}
impl From<SystemNotificationType> for StoredSystemNotificationTypeV1 {
    fn from(value: SystemNotificationType) -> Self {
        match value {
            SystemNotificationType::Compaction => Self::Compaction,
            SystemNotificationType::SessionRecovery => Self::SessionRecovery,
        }
    }
}
impl From<StoredSystemNotificationTypeV1> for SystemNotificationType {
    fn from(value: StoredSystemNotificationTypeV1) -> Self {
        match value {
            StoredSystemNotificationTypeV1::Compaction => Self::Compaction,
            StoredSystemNotificationTypeV1::SessionRecovery => Self::SessionRecovery,
        }
    }
}

pub(crate) fn decode_stored_message_part_v1(
    raw: &[u8],
    payload_version: u32,
    source: StoredPayloadSource,
) -> Result<DecodedStoredMessagePartV1, IncompatibleStoredEvent> {
    if payload_version != 1 {
        return Err(IncompatibleStoredEvent {
            type_tag: "message_part".to_string(),
            payload_version,
            reason: "unsupported required payload version".to_string(),
        });
    }
    let value: serde_json::Value = serde_json::from_slice(raw)
        .map_err(|error| incompatible("message_part", format!("invalid JSON: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| incompatible("message_part", "expected JSON object"))?;
    let type_tag = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| incompatible("message_part", "missing required type tag"))?
        .to_string();
    let stored: StoredMessagePartV1 = serde_json::from_value(value.clone()).map_err(|error| {
        incompatible(type_tag.clone(), format!("invalid known payload: {error}"))
    })?;
    let canonical = serde_json::to_value(&stored)
        .expect("stored message part serialization must be deterministic");
    let has_additive_fields = contains_additive_fields(&value, &canonical);
    let part = stored.try_into()?;
    Ok(DecodedStoredMessagePartV1 {
        part,
        preserved_additive_payload: has_additive_fields.then(|| PreservedStoredPayload {
            source,
            payload_version,
            type_tag,
            raw_bytes: raw.to_vec(),
        }),
    })
}

#[cfg(test)]
pub(crate) fn encode_stored_message_part_v1(
    part: &MessagePart,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&StoredMessagePartV1::from(part))
}

pub(crate) fn encode_stored_message_parts_v1(
    parts: &[MessagePart],
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(
        &parts
            .iter()
            .map(StoredMessagePartV1::from)
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
pub(crate) fn decode_stored_message_parts_v1(
    raw: &[u8],
) -> Result<Vec<MessagePart>, IncompatibleStoredEvent> {
    let stored: Vec<StoredMessagePartV1> = serde_json::from_slice(raw)
        .map_err(|error| incompatible("message_part", format!("invalid JSON: {error}")))?;
    stored.into_iter().map(TryInto::try_into).collect()
}

pub(super) fn contains_additive_fields(
    original: &serde_json::Value,
    canonical: &serde_json::Value,
) -> bool {
    match (original, canonical) {
        (serde_json::Value::Object(original), serde_json::Value::Object(canonical)) => original
            .iter()
            .any(|(key, value)| match canonical.get(key) {
                Some(canonical_value) => contains_additive_fields(value, canonical_value),
                None => true,
            }),
        (serde_json::Value::Array(original), serde_json::Value::Array(canonical)) => {
            original.len() != canonical.len()
                || original
                    .iter()
                    .zip(canonical)
                    .any(|(value, canonical_value)| {
                        contains_additive_fields(value, canonical_value)
                    })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(value: serde_json::Value) -> JsonPayload {
        json_payload(value)
    }

    fn permission() -> PermissionRequest {
        PermissionRequest {
            id: "permission-1".to_string(),
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: Some("parent-1".to_string()),
            tool_name: "Bash".to_string(),
            body: PermissionRequestBody::ToolApproval {
                input: payload(serde_json::json!({"command": "cargo test"})),
            },
            title: Some("Run command".to_string()),
            display_name: None,
            description: None,
            decision_reason: None,
            status: PermissionRequestStatus::Resolved {
                decision: PermissionDecision::Allowed,
                answers: Some(payload(serde_json::json!({"approved": true}))),
            },
        }
    }

    fn all_parts() -> Vec<MessagePart> {
        vec![
            MessagePart::Thinking {
                content: "thinking".into(),
                parent_tool_use_id: Some("parent".into()),
            },
            MessagePart::Text {
                content: "text".into(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                id: "tool-1".into(),
                tool: "Read".into(),
                input: payload(serde_json::json!({"path": "src/lib.rs"})),
                parent_tool_use_id: None,
            },
            MessagePart::ToolResult {
                content: "result".into(),
                is_error: false,
                tool_use_id: Some("tool-1".into()),
                parent_tool_use_id: None,
                content_ref: Some(ToolOutputRef {
                    id: "blob-1".into(),
                    byte_size: 10,
                }),
                summary: Some(ToolOutputSummary {
                    line_count: 1,
                    byte_size: 10,
                    is_error: false,
                    truncated: false,
                }),
            },
            MessagePart::Error {
                content: "error".into(),
                parent_tool_use_id: None,
            },
            MessagePart::Permission {
                request: permission(),
                status: PermissionPartStatus::Allowed,
                answers: Some(payload(serde_json::json!({"approved": true}))),
                parent_tool_use_id: Some("parent-1".into()),
            },
            MessagePart::TaskStatus {
                task_tool_use_id: "task-1".into(),
                status: "completed".into(),
                description: Some("done".into()),
                summary: None,
            },
            MessagePart::TodoListSnapshot {
                items: vec![TodoListItem {
                    text: "test".into(),
                    completed: true,
                }],
            },
            MessagePart::SystemNotification {
                notification_type: SystemNotificationType::Compaction,
                status: "completed".into(),
                label: "Compacted".into(),
                detail: None,
                hook_id: Some("hook-1".into()),
            },
            MessagePart::Image {
                data: "AA==".into(),
                media_type: "image/png".into(),
            },
            MessagePart::ImageRef {
                attachment: Attachment {
                    id: "attachment-1".into(),
                    media_type: "image/png".into(),
                    byte_size: 1,
                },
            },
        ]
    }

    #[test]
    fn all_known_variants_round_trip_through_stored_v1_without_semantic_loss() {
        for part in all_parts() {
            let stored = StoredMessagePartV1::from(&part);
            let json = serde_json::to_vec(&stored).unwrap();
            let decoded: StoredMessagePartV1 = serde_json::from_slice(&json).unwrap();
            assert_eq!(MessagePart::try_from(decoded).unwrap(), part);
        }
    }

    #[test]
    fn legacy_json_spelling_and_optional_omission_remain_stable() {
        let part = MessagePart::ToolResult {
            content: "ok".into(),
            is_error: false,
            tool_use_id: Some("tool-1".into()),
            parent_tool_use_id: None,
            content_ref: None,
            summary: None,
        };
        assert_eq!(
            String::from_utf8(encode_stored_message_part_v1(&part).unwrap()).unwrap(),
            r#"{"type":"tool_result","content":"ok","isError":false,"toolUseId":"tool-1"}"#
        );
    }

    #[test]
    fn additive_fields_are_raw_preserved_with_source_metadata() {
        let raw = br#"{"type":"text","content":"hello","futureField":{"n":1}}"#;
        let decoded = decode_stored_message_part_v1(
            raw,
            1,
            StoredPayloadSource {
                source_id: "sessions/s-1/messages/1.json".into(),
                record_ordinal: Some(1),
            },
        )
        .unwrap();
        assert!(
            matches!(decoded.part, MessagePart::Text { ref content, .. } if content == "hello")
        );
        let preserved = decoded.preserved_additive_payload.unwrap();
        assert_eq!(preserved.raw_bytes, raw);
        assert_eq!(preserved.source.record_ordinal, Some(1));
        assert_eq!(preserved.type_tag, "text");
    }

    #[test]
    fn nested_additive_fields_are_raw_preserved() {
        let raw = br#"{"type":"image_ref","attachment":{"id":"a","mediaType":"image/png","byteSize":1,"futureNested":true}}"#;
        let decoded = decode_stored_message_part_v1(
            raw,
            1,
            StoredPayloadSource {
                source_id: "fixture".into(),
                record_ordinal: None,
            },
        )
        .unwrap();

        assert_eq!(decoded.preserved_additive_payload.unwrap().raw_bytes, raw);
    }

    #[test]
    fn unknown_required_variant_and_version_fail_closed() {
        let source = StoredPayloadSource {
            source_id: "fixture".into(),
            record_ordinal: None,
        };
        let variant =
            decode_stored_message_part_v1(br#"{"type":"future_required"}"#, 1, source.clone())
                .unwrap_err();
        assert_eq!(variant.type_tag, "future_required");
        let version = decode_stored_message_part_v1(br#"{"type":"text","content":"x"}"#, 2, source)
            .unwrap_err();
        assert_eq!(version.payload_version, 2);
        assert!(version
            .reason
            .contains("unsupported required payload version"));
    }

    #[test]
    fn semantic_domain_types_have_no_serde_contract() {
        for (name, source) in [
            (
                "message_part",
                include_str!("../../../../domain/agent_session/entities/message_part.rs"),
            ),
            (
                "permission_request",
                include_str!("../../../../domain/agent_session/entities/permission_request.rs"),
            ),
            (
                "agent_session_events",
                include_str!("../../../../domain/agent_session/events.rs"),
            ),
            (
                "json_payload",
                include_str!("../../../../domain/agent_session/value_objects/json_payload.rs"),
            ),
            (
                "workflow_events",
                include_str!("../../../../domain/workflow/events.rs"),
            ),
        ] {
            assert!(!source.contains("serde::"), "{name} owns a serde impl");
            assert!(
                !source.contains("Serialize") && !source.contains("Deserialize"),
                "{name} owns a serialization derive or import"
            );
        }
    }
}
