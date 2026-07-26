use std::collections::BTreeMap;
use std::fmt;

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::domain::agent_session::entities::{
    Attachment, MessagePart, PermissionAllowedPrompt, PermissionDecision, PermissionPartStatus,
    PermissionQuestion, PermissionQuestionOption, PermissionRequest, PermissionRequestBody,
    PermissionRequestStatus,
};
use crate::domain::agent_session::value_objects::{
    JsonPayload, SystemNotificationType, TodoListItem, ToolOutputRef, ToolOutputSummary,
};

/// Versioned public JSON algebra.
///
/// The public schema owns this closed representation instead of exposing
/// `serde_json::Value` or borrowing the domain [`JsonPayload`] serialization.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum JsonValueDtoV1 {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl JsonValueDtoV1 {
    fn from_payload(payload: &JsonPayload) -> Self {
        serde_json::from_str(payload.as_str())
            .expect("domain JsonPayload must be validated at its boundary")
    }

    pub(crate) fn from_value(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(value) => Self::Bool(*value),
            serde_json::Value::Number(value) => Self::Number(value.clone()),
            serde_json::Value::String(value) => Self::String(value.clone()),
            serde_json::Value::Array(values) => {
                Self::Array(values.iter().map(Self::from_value).collect())
            }
            serde_json::Value::Object(values) => Self::Object(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), Self::from_value(value)))
                    .collect(),
            ),
        }
    }

    fn into_payload(self) -> JsonPayload {
        JsonPayload::new_unchecked(
            serde_json::to_string(&self).expect("public JSON DTO serialization cannot fail"),
        )
    }
}

impl Serialize for JsonValueDtoV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Number(value) => value.serialize(serializer),
            Self::String(value) => serializer.serialize_str(value),
            Self::Array(values) => values.serialize(serializer),
            Self::Object(values) => values.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for JsonValueDtoV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct JsonValueVisitor;

        impl<'de> Visitor<'de> for JsonValueVisitor {
            type Value = JsonValueDtoV1;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a valid JSON value")
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(JsonValueDtoV1::Null)
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(JsonValueDtoV1::Null)
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(JsonValueDtoV1::Bool(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(JsonValueDtoV1::Number(value.into()))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(JsonValueDtoV1::Number(value.into()))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                serde_json::Number::from_f64(value)
                    .map(JsonValueDtoV1::Number)
                    .ok_or_else(|| E::custom("non-finite number is not valid JSON"))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(JsonValueDtoV1::String(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(JsonValueDtoV1::String(value))
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(JsonValueDtoV1::Array(values))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = BTreeMap::new();
                while let Some((key, value)) = map.next_entry()? {
                    values.insert(key, value);
                }
                Ok(JsonValueDtoV1::Object(values))
            }
        }

        deserializer.deserialize_any(JsonValueVisitor)
    }
}

/// Versioned public representation of the canonical domain [`MessagePart`].
///
/// It intentionally remains distinct from the legacy stored DTO. Both currently
/// preserve the established JSON spelling, but they have different compatibility
/// owners and may evolve independently.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub(crate) enum MessagePartDtoV1 {
    Thinking {
        content: String,
        parent_tool_use_id: Option<String>,
    },
    Text {
        content: String,
        parent_tool_use_id: Option<String>,
    },
    ToolUse {
        tool: String,
        input: JsonValueDtoV1,
        id: String,
        parent_tool_use_id: Option<String>,
    },
    ToolResult {
        content: String,
        is_error: bool,
        tool_use_id: Option<String>,
        parent_tool_use_id: Option<String>,
        content_ref: Option<ToolOutputRefDtoV1>,
        summary: Option<ToolOutputSummaryDtoV1>,
    },
    Error {
        content: String,
        parent_tool_use_id: Option<String>,
    },
    Permission {
        request: PermissionRequestDtoV1,
        status: PermissionPartStatusDtoV1,
        answers: Option<JsonValueDtoV1>,
        parent_tool_use_id: Option<String>,
    },
    TaskStatus {
        task_tool_use_id: String,
        status: String,
        description: Option<String>,
        summary: Option<String>,
    },
    TodoListSnapshot {
        items: Vec<TodoListItemDtoV1>,
    },
    SystemNotification {
        notification_type: SystemNotificationTypeDtoV1,
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
        attachment: AttachmentDtoV1,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttachmentDtoV1 {
    pub id: String,
    pub media_type: String,
    pub byte_size: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolOutputRefDtoV1 {
    pub id: String,
    pub byte_size: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolOutputSummaryDtoV1 {
    pub line_count: String,
    pub byte_size: String,
    pub is_error: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TodoListItemDtoV1 {
    pub text: String,
    pub completed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PermissionPartStatusDtoV1 {
    Pending,
    Allowed,
    Denied,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PermissionRequestKindDtoV1 {
    ToolApproval,
    PlanApproval,
    Question,
    PermissionGrant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionRequestDtoV1 {
    pub id: String,
    pub tool_use_id: Option<String>,
    pub tool_name: String,
    pub kind: PermissionRequestKindDtoV1,
    pub input: Option<JsonValueDtoV1>,
    pub plan: Option<String>,
    pub allowed_prompts: Vec<PermissionAllowedPromptDtoV1>,
    pub questions: Vec<PermissionQuestionDtoV1>,
    pub title: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub decision_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionAllowedPromptDtoV1 {
    pub tool: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionQuestionDtoV1 {
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<PermissionQuestionOptionDtoV1>,
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionQuestionOptionDtoV1 {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SystemNotificationTypeDtoV1 {
    Compaction,
    SessionRecovery,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid public message part: {0}")]
pub(crate) struct MessagePartDtoError(String);

fn decode_u64_decimal(value: &str, field: &str) -> Result<u64, MessagePartDtoError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(MessagePartDtoError(format!(
            "{field} must be a canonical nonnegative decimal string"
        )));
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|parsed| *parsed <= i64::MAX as u64)
        .ok_or_else(|| MessagePartDtoError(format!("{field} exceeds i64::MAX")))
}

impl From<&MessagePart> for MessagePartDtoV1 {
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
                input: payload_value(input),
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
                content_ref: content_ref.as_ref().map(|value| ToolOutputRefDtoV1 {
                    id: value.id.clone(),
                    byte_size: value.byte_size.to_string(),
                }),
                summary: summary.as_ref().map(|value| ToolOutputSummaryDtoV1 {
                    line_count: value.line_count.to_string(),
                    byte_size: value.byte_size.to_string(),
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
                answers: answers.as_ref().map(payload_value),
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
                    .map(|item| TodoListItemDtoV1 {
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
                attachment: AttachmentDtoV1 {
                    id: attachment.id.clone(),
                    media_type: attachment.media_type.clone(),
                    byte_size: attachment.byte_size.to_string(),
                },
            },
        }
    }
}

impl TryFrom<MessagePartDtoV1> for MessagePart {
    type Error = MessagePartDtoError;
    fn try_from(part: MessagePartDtoV1) -> Result<Self, MessagePartDtoError> {
        Ok(match part {
            MessagePartDtoV1::Thinking {
                content,
                parent_tool_use_id,
            } => Self::Thinking {
                content,
                parent_tool_use_id,
            },
            MessagePartDtoV1::Text {
                content,
                parent_tool_use_id,
            } => Self::Text {
                content,
                parent_tool_use_id,
            },
            MessagePartDtoV1::ToolUse {
                tool,
                input,
                id,
                parent_tool_use_id,
            } => Self::ToolUse {
                id,
                tool,
                input: value_payload(input),
                parent_tool_use_id,
            },
            MessagePartDtoV1::ToolResult {
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
                content_ref: content_ref
                    .map(|value| {
                        Ok(ToolOutputRef {
                            id: value.id,
                            byte_size: decode_u64_decimal(
                                &value.byte_size,
                                "content_ref.byte_size",
                            )?,
                        })
                    })
                    .transpose()?,
                summary: summary
                    .map(|value| {
                        Ok(ToolOutputSummary {
                            line_count: decode_u64_decimal(
                                &value.line_count,
                                "summary.line_count",
                            )?,
                            byte_size: decode_u64_decimal(&value.byte_size, "summary.byte_size")?,
                            is_error: value.is_error,
                            truncated: value.truncated,
                        })
                    })
                    .transpose()?,
            },
            MessagePartDtoV1::Error {
                content,
                parent_tool_use_id,
            } => Self::Error {
                content,
                parent_tool_use_id,
            },
            MessagePartDtoV1::Permission {
                request,
                status,
                answers,
                parent_tool_use_id,
            } => {
                let status: PermissionPartStatus = status.into();
                let answers = answers.map(value_payload);
                let mut request = request.into_domain(status, answers.clone())?;
                request.parent_tool_use_id = parent_tool_use_id.clone();
                Self::Permission {
                    request,
                    status,
                    answers,
                    parent_tool_use_id,
                }
            }
            MessagePartDtoV1::TaskStatus {
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
            MessagePartDtoV1::TodoListSnapshot { items } => Self::TodoListSnapshot {
                items: items
                    .into_iter()
                    .map(|item| TodoListItem {
                        text: item.text,
                        completed: item.completed,
                    })
                    .collect(),
            },
            MessagePartDtoV1::SystemNotification {
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
            MessagePartDtoV1::Image { data, media_type } => Self::Image { data, media_type },
            MessagePartDtoV1::ImageRef { attachment } => Self::ImageRef {
                attachment: Attachment {
                    id: attachment.id,
                    media_type: attachment.media_type,
                    byte_size: decode_u64_decimal(&attachment.byte_size, "attachment.byte_size")?,
                },
            },
        })
    }
}

impl From<&PermissionRequest> for PermissionRequestDtoV1 {
    fn from(request: &PermissionRequest) -> Self {
        let (kind, input, plan, allowed_prompts, questions) = match &request.body {
            PermissionRequestBody::ToolApproval { input } => (
                PermissionRequestKindDtoV1::ToolApproval,
                Some(payload_value(input)),
                None,
                Vec::new(),
                Vec::new(),
            ),
            PermissionRequestBody::PlanApproval {
                plan,
                allowed_prompts,
            } => (
                PermissionRequestKindDtoV1::PlanApproval,
                None,
                Some(plan.clone()),
                allowed_prompts
                    .iter()
                    .map(|prompt| PermissionAllowedPromptDtoV1 {
                        tool: prompt.tool.clone(),
                        prompt: prompt.prompt.clone(),
                    })
                    .collect(),
                Vec::new(),
            ),
            PermissionRequestBody::Question { questions } => (
                PermissionRequestKindDtoV1::Question,
                None,
                None,
                Vec::new(),
                questions
                    .iter()
                    .map(|question| PermissionQuestionDtoV1 {
                        question: question.question.clone(),
                        header: question.header.clone(),
                        options: question
                            .options
                            .iter()
                            .map(|option| PermissionQuestionOptionDtoV1 {
                                label: option.label.clone(),
                                description: option.description.clone(),
                            })
                            .collect(),
                        multi_select: question.multi_select,
                    })
                    .collect(),
            ),
            PermissionRequestBody::PermissionGrant { requested } => (
                PermissionRequestKindDtoV1::PermissionGrant,
                Some(payload_value(requested)),
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

impl PermissionRequestDtoV1 {
    fn into_domain(
        self,
        part_status: PermissionPartStatus,
        answers: Option<JsonPayload>,
    ) -> Result<PermissionRequest, MessagePartDtoError> {
        let body = match self.kind {
            PermissionRequestKindDtoV1::ToolApproval => PermissionRequestBody::ToolApproval {
                input: self
                    .input
                    .map(value_payload)
                    .ok_or_else(|| MessagePartDtoError("missing tool_approval.input".into()))?,
            },
            PermissionRequestKindDtoV1::PlanApproval => PermissionRequestBody::PlanApproval {
                plan: self
                    .plan
                    .ok_or_else(|| MessagePartDtoError("missing plan_approval.plan".into()))?,
                allowed_prompts: self
                    .allowed_prompts
                    .into_iter()
                    .map(|prompt| PermissionAllowedPrompt {
                        tool: prompt.tool,
                        prompt: prompt.prompt,
                    })
                    .collect(),
            },
            PermissionRequestKindDtoV1::Question => PermissionRequestBody::Question {
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
            PermissionRequestKindDtoV1::PermissionGrant => PermissionRequestBody::PermissionGrant {
                requested: self
                    .input
                    .map(value_payload)
                    .ok_or_else(|| MessagePartDtoError("missing permission_grant.input".into()))?,
            },
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

fn payload_value(payload: &JsonPayload) -> JsonValueDtoV1 {
    JsonValueDtoV1::from_payload(payload)
}
fn value_payload(value: JsonValueDtoV1) -> JsonPayload {
    value.into_payload()
}
impl From<PermissionPartStatus> for PermissionPartStatusDtoV1 {
    fn from(value: PermissionPartStatus) -> Self {
        match value {
            PermissionPartStatus::Pending => Self::Pending,
            PermissionPartStatus::Allowed => Self::Allowed,
            PermissionPartStatus::Denied => Self::Denied,
            PermissionPartStatus::Cancelled => Self::Cancelled,
        }
    }
}
impl From<PermissionPartStatusDtoV1> for PermissionPartStatus {
    fn from(value: PermissionPartStatusDtoV1) -> Self {
        match value {
            PermissionPartStatusDtoV1::Pending => Self::Pending,
            PermissionPartStatusDtoV1::Allowed => Self::Allowed,
            PermissionPartStatusDtoV1::Denied => Self::Denied,
            PermissionPartStatusDtoV1::Cancelled => Self::Cancelled,
        }
    }
}
impl From<SystemNotificationType> for SystemNotificationTypeDtoV1 {
    fn from(value: SystemNotificationType) -> Self {
        match value {
            SystemNotificationType::Compaction => Self::Compaction,
            SystemNotificationType::SessionRecovery => Self::SessionRecovery,
        }
    }
}
impl From<SystemNotificationTypeDtoV1> for SystemNotificationType {
    fn from(value: SystemNotificationTypeDtoV1) -> Self {
        match value {
            SystemNotificationTypeDtoV1::Compaction => Self::Compaction,
            SystemNotificationTypeDtoV1::SessionRecovery => Self::SessionRecovery,
        }
    }
}

#[cfg(test)]
mod message_part_dto_tests {
    use super::*;

    fn payload(value: serde_json::Value) -> JsonPayload {
        value.into()
    }

    fn permission() -> PermissionRequest {
        PermissionRequest {
            id: "permission-1".into(),
            tool_use_id: Some("tool-1".into()),
            parent_tool_use_id: None,
            tool_name: "Bash".into(),
            body: PermissionRequestBody::ToolApproval {
                input: payload(serde_json::json!({"command": "cargo test"})),
            },
            title: None,
            display_name: None,
            description: None,
            decision_reason: None,
            status: PermissionRequestStatus::Pending,
        }
    }

    fn all_parts() -> Vec<MessagePart> {
        vec![
            MessagePart::Thinking {
                content: "thinking".into(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "text".into(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                id: "tool-1".into(),
                tool: "Read".into(),
                input: payload(serde_json::json!({"path":"src/lib.rs"})),
                parent_tool_use_id: None,
            },
            MessagePart::ToolResult {
                content: "ok".into(),
                is_error: false,
                tool_use_id: Some("tool-1".into()),
                parent_tool_use_id: None,
                content_ref: Some(ToolOutputRef {
                    id: "blob".into(),
                    byte_size: 2,
                }),
                summary: Some(ToolOutputSummary {
                    line_count: 1,
                    byte_size: 2,
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
                status: PermissionPartStatus::Pending,
                answers: None,
                parent_tool_use_id: None,
            },
            MessagePart::TaskStatus {
                task_tool_use_id: "task".into(),
                status: "running".into(),
                description: None,
                summary: None,
            },
            MessagePart::TodoListSnapshot {
                items: vec![TodoListItem {
                    text: "todo".into(),
                    completed: false,
                }],
            },
            MessagePart::SystemNotification {
                notification_type: SystemNotificationType::SessionRecovery,
                status: "in_progress".into(),
                label: "Recovering".into(),
                detail: None,
                hook_id: None,
            },
            MessagePart::Image {
                data: "AA==".into(),
                media_type: "image/png".into(),
            },
            MessagePart::ImageRef {
                attachment: Attachment {
                    id: "image".into(),
                    media_type: "image/png".into(),
                    byte_size: 1,
                },
            },
        ]
    }

    #[test]
    fn all_known_variants_round_trip_through_public_v1() {
        for part in all_parts() {
            let dto = MessagePartDtoV1::from(&part);
            let json = serde_json::to_vec(&dto).unwrap();
            let decoded: MessagePartDtoV1 = serde_json::from_slice(&json).unwrap();
            assert_eq!(MessagePart::try_from(decoded).unwrap(), part);
        }
    }

    #[test]
    fn public_v1_is_snake_case_and_emits_explicit_null() {
        let dto = MessagePartDtoV1::from(&MessagePart::Text {
            content: "hello".into(),
            parent_tool_use_id: None,
        });
        assert_eq!(
            serde_json::to_string(&dto).unwrap(),
            r#"{"type":"text","content":"hello","parent_tool_use_id":null}"#
        );
    }

    #[test]
    fn b075_message_part_semantic_integers_cover_every_canonical_boundary() {
        for (raw, expected) in [
            ("0", Some(0)),
            ("1", Some(1)),
            ("9223372036854775807", Some(i64::MAX as u64)),
            ("", None),
            ("01", None),
            ("+1", None),
            ("-1", None),
            ("1e0", None),
            ("１", None),
            (" 1", None),
            ("1 ", None),
            ("9223372036854775808", None),
        ] {
            assert_eq!(decode_u64_decimal(raw, "field").ok(), expected, "{raw:?}");
        }

        for pointer in [
            "/content_ref/byte_size",
            "/summary/line_count",
            "/summary/byte_size",
        ] {
            let mut raw = serde_json::json!({
                "type": "tool_result",
                "content": "result",
                "is_error": false,
                "tool_use_id": "tool-1",
                "parent_tool_use_id": null,
                "content_ref": { "id": "blob-1", "byte_size": "1" },
                "summary": {
                    "line_count": "1",
                    "byte_size": "1",
                    "is_error": false,
                    "truncated": false
                }
            });
            *raw.pointer_mut(pointer).unwrap() = serde_json::json!(1);
            assert!(
                serde_json::from_value::<MessagePartDtoV1>(raw).is_err(),
                "{pointer} accepted a JSON number"
            );
        }

        let mut image_ref = serde_json::json!({
            "type": "image_ref",
            "attachment": {
                "id": "image-1",
                "media_type": "image/png",
                "byte_size": "1"
            }
        });
        image_ref["attachment"]["byte_size"] = serde_json::json!(1);
        assert!(serde_json::from_value::<MessagePartDtoV1>(image_ref).is_err());
    }

    #[test]
    fn b075_message_part_maximum_counts_round_trip_as_decimal_strings() {
        let maximum = i64::MAX as u64;
        let tool_result = MessagePartDtoV1::from(&MessagePart::ToolResult {
            content: "result".to_string(),
            is_error: false,
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: None,
            content_ref: Some(ToolOutputRef {
                id: "blob-1".to_string(),
                byte_size: maximum,
            }),
            summary: Some(ToolOutputSummary {
                line_count: maximum,
                byte_size: maximum,
                is_error: false,
                truncated: false,
            }),
        });
        let encoded = serde_json::to_value(&tool_result).unwrap();
        for pointer in [
            "/content_ref/byte_size",
            "/summary/line_count",
            "/summary/byte_size",
        ] {
            assert_eq!(
                encoded.pointer(pointer),
                Some(&serde_json::Value::String(maximum.to_string())),
                "{pointer}"
            );
        }
        assert_eq!(
            MessagePart::try_from(tool_result).unwrap(),
            MessagePart::ToolResult {
                content: "result".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: Some(ToolOutputRef {
                    id: "blob-1".to_string(),
                    byte_size: maximum,
                }),
                summary: Some(ToolOutputSummary {
                    line_count: maximum,
                    byte_size: maximum,
                    is_error: false,
                    truncated: false,
                }),
            }
        );

        let image_ref = MessagePartDtoV1::from(&MessagePart::ImageRef {
            attachment: Attachment {
                id: "image-1".to_string(),
                media_type: "image/png".to_string(),
                byte_size: maximum,
            },
        });
        assert_eq!(
            serde_json::to_value(image_ref).unwrap()["attachment"]["byte_size"],
            maximum.to_string()
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSupportedCommandMsg {
    pub name: String,
    pub description: String,
    #[serde(
        rename = "argumentHint",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub argument_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSupportedCommandsUpdated {
    pub chat_session_id: String,
    pub commands: Vec<AgentSupportedCommandMsg>,
}
