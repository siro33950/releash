use crate::usecase::agent_session::session::{
    AttachmentRef, ChatMessage, MessageMention, MessagePart,
};

pub use crate::domain::agent_session::events::{
    AgentSessionDomainEvent as AgentSessionEvent, BackendSessionRecoveryReason,
    GoalReactivationOutcome, InterruptReason, PermissionDecision, PromptInput, TurnId,
    TurnStopReason, TurnTokenUsage,
};

pub(crate) fn prompt_input_from_human_message(message: &ChatMessage) -> PromptInput {
    let parts = message.parts.clone().unwrap_or_default();
    PromptInput {
        content: message.content.clone(),
        mentions: message
            .mentions
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(MessageMention::into_domain)
            .collect(),
        attachment_refs: attachment_refs_from_parts(&parts),
        parts,
    }
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

pub(super) fn assistant_message_id_for_turn(
    events: &[AgentSessionEvent],
    turn_id: TurnId,
) -> Option<String> {
    events.iter().find_map(|event| match event {
        AgentSessionEvent::TurnStarted {
            turn_id: id,
            message_id,
            assistant_message_id,
            ..
        } if *id == turn_id => Some(assistant_message_id_for_started_turn(
            message_id,
            assistant_message_id.as_deref(),
        )),
        _ => None,
    })
}

pub(super) fn assistant_message_id_for_started_turn(
    message_id: &str,
    assistant_message_id: Option<&str>,
) -> String {
    assistant_message_id
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{message_id}:agent"))
}
