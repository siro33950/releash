use serde::{Deserialize, Serialize};

use crate::usecase::agent_session::session::{
    AttachmentRef, ChatMessage, MessageMention, MessagePart, SystemNotificationType, TodoListItem,
    ToolOutputRef, ToolOutputSummary,
};

pub type TurnId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStopReason {
    Refusal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptInput {
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<MessageMention>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachment_refs: Vec<AttachmentRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<MessagePart>,
}

impl PromptInput {
    pub fn from_human_message(message: &ChatMessage) -> Self {
        let parts = message.parts.clone().unwrap_or_default();
        Self {
            content: message.content.clone(),
            mentions: message.mentions.clone().unwrap_or_default(),
            attachment_refs: attachment_refs_from_parts(&parts),
            parts,
        }
    }

    pub fn from_content_images<I>(content: &str, images: I) -> Self
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let parts = human_parts_from_content_images(content, images);
        Self {
            content: content.to_string(),
            mentions: Vec::new(),
            attachment_refs: attachment_refs_from_parts(&parts),
            parts,
        }
    }
}

pub fn human_parts_from_content_images<I>(content: &str, images: I) -> Vec<MessagePart>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut images = images.into_iter().peekable();
    if images.peek().is_none() {
        return Vec::new();
    }

    let mut parts = Vec::new();
    if !content.is_empty() {
        parts.push(MessagePart::Text {
            content: content.to_string(),
            parent_tool_use_id: None,
        });
    }
    parts.extend(images.map(|(data, media_type)| MessagePart::Image { data, media_type }));
    parts
}

pub fn attachment_refs_from_parts(parts: &[MessagePart]) -> Vec<AttachmentRef> {
    parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::ImageRef { attachment } => Some(attachment.clone()),
            _ => None,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptReason {
    Abort,
    Timeout,
    BridgeCrash,
}

impl InterruptReason {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Abort => "abort",
            Self::Timeout => "timeout",
            Self::BridgeCrash => "bridge crash",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allowed,
    Denied,
    Cancelled,
}

impl PermissionDecision {
    pub fn from_status(status: &str) -> Option<Self> {
        match status {
            "allowed" | "allow" => Some(Self::Allowed),
            "denied" | "deny" => Some(Self::Denied),
            "cancelled" | "canceled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn status(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentSessionEvent {
    TurnStarted {
        turn_id: TurnId,
        message_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assistant_message_id: Option<String>,
        prompt: PromptInput,
        at: f64,
    },
    TextRecorded {
        turn_id: TurnId,
        message_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    ReasoningRecorded {
        turn_id: TurnId,
        message_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    ErrorRecorded {
        turn_id: TurnId,
        message_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    ToolCallStarted {
        turn_id: TurnId,
        tool_use_id: String,
        tool: String,
        input: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    ToolCallSucceeded {
        turn_id: TurnId,
        tool_use_id: String,
        content: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "contentRef"
        )]
        content_ref: Option<ToolOutputRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<ToolOutputSummary>,
    },
    ToolCallFailed {
        turn_id: TurnId,
        tool_use_id: String,
        content: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "contentRef"
        )]
        content_ref: Option<ToolOutputRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<ToolOutputSummary>,
    },
    ToolResultRecorded {
        turn_id: TurnId,
        message_id: String,
        content: String,
        is_error: bool,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "contentRef"
        )]
        content_ref: Option<ToolOutputRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<ToolOutputSummary>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    ToolCallRetried {
        turn_id: TurnId,
        tool_use_id: String,
        attempt: u32,
    },
    PermissionRequested {
        turn_id: TurnId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
        request: serde_json::Value,
    },
    PermissionResolved {
        turn_id: TurnId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        decision: PermissionDecision,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answers: Option<serde_json::Value>,
    },
    TaskStatusChanged {
        turn_id: TurnId,
        message_id: String,
        task_tool_use_id: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    TodoListSnapshotRecorded {
        turn_id: TurnId,
        message_id: String,
        items: Vec<TodoListItem>,
    },
    SystemNotificationRecorded {
        turn_id: TurnId,
        message_id: String,
        notification_type: SystemNotificationType,
        status: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hook_id: Option<String>,
    },
    ImageRecorded {
        turn_id: TurnId,
        message_id: String,
        data: String,
        media_type: String,
    },
    ImageRefRecorded {
        turn_id: TurnId,
        message_id: String,
        attachment: AttachmentRef,
    },
    FinalPartsRecorded {
        turn_id: TurnId,
        message_id: String,
        parts: Vec<MessagePart>,
    },
    TurnCompleted {
        turn_id: TurnId,
        exit_code: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_reason: Option<TurnStopReason>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_usage: Option<TurnTokenUsage>,
    },
    TurnInterrupted {
        turn_id: TurnId,
        reason: InterruptReason,
        exit_code: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    SessionClosed {
        at: f64,
    },
}
