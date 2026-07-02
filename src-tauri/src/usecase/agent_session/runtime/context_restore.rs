use crate::usecase::agent_session::session::{
    parts_to_legacy, ChatMessage, ChatSession, ContextCarryState, MessageRole, SessionMeta,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RestoreContextMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RestoreContextPayload {
    pub prompt_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ContextRestorePlan {
    NoContext,
    Resume { session_id: String },
    Reinject { payload: RestoreContextPayload },
}

impl ContextRestorePlan {
    pub(crate) fn carry_state(&self) -> Option<ContextCarryState> {
        match self {
            Self::NoContext => None,
            Self::Resume { .. } => Some(ContextCarryState::Resumed),
            Self::Reinject { .. } => Some(ContextCarryState::Reinjected),
        }
    }

    #[allow(dead_code)] // issues-1301 D-2: retained for resume-plan callers that only need the backend session id.
    pub(crate) fn resume_session_id(&self) -> Option<&str> {
        match self {
            Self::Resume { session_id } => Some(session_id.as_str()),
            _ => None,
        }
    }

    pub(crate) fn restore_context(&self) -> Option<&RestoreContextPayload> {
        match self {
            Self::Reinject { payload } => Some(payload),
            _ => None,
        }
    }
}

fn reinjectable_content(message: &ChatMessage) -> String {
    let content = if message.content.is_empty() {
        message.parts.as_deref().map_or_else(String::new, |parts| {
            let (content, _, _) = parts_to_legacy(parts);
            content
        })
    } else {
        message.content.clone()
    };
    content.trim().to_string()
}

pub(crate) fn restore_context_messages(messages: &[ChatMessage]) -> Vec<RestoreContextMessage> {
    messages
        .iter()
        .filter_map(|message| match message.role {
            MessageRole::Human | MessageRole::Agent => {
                let content = reinjectable_content(message);
                (!content.is_empty()).then(|| RestoreContextMessage {
                    role: message.role.clone(),
                    content,
                })
            }
            MessageRole::System => None,
        })
        .collect()
}

fn build_restore_context_prompt_prefix(messages: &[RestoreContextMessage]) -> String {
    let transcript = messages
        .iter()
        .map(|message| {
            let role = match message.role {
                MessageRole::Human => "Human",
                MessageRole::Agent => "Agent",
                MessageRole::System => "System",
            };
            format!("{role}: {}", message.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "The following conversation history was restored by Releash. Use it as prior context for this session.\n\n<releash_restored_conversation>\n{transcript}\n</releash_restored_conversation>\n\nContinue from this restored context. The user's next message follows."
    )
}

pub(crate) fn restore_context_payload(messages: &[ChatMessage]) -> Option<RestoreContextPayload> {
    let messages = restore_context_messages(messages);
    if messages.is_empty() {
        return None;
    }
    Some(RestoreContextPayload {
        prompt_prefix: build_restore_context_prompt_prefix(&messages),
    })
}

#[allow(dead_code)] // issues-1301 D-2: meta-only restore planning is used by storage/query callers outside the current runtime path.
pub(crate) fn context_restore_plan_from_meta(meta: &SessionMeta) -> Option<ContextRestorePlan> {
    if let Some(session_id) = meta
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(ContextRestorePlan::Resume {
            session_id: session_id.to_string(),
        });
    }
    if meta.context_carry == Some(ContextCarryState::Failed) || meta.message_count == 0 {
        return Some(ContextRestorePlan::NoContext);
    }
    None
}

#[allow(dead_code)] // issues-1301 D-2: full-session planning is kept for non-turn startup paths.
pub(crate) fn context_restore_plan_for_session(
    session: Option<&ChatSession>,
) -> ContextRestorePlan {
    let Some(session) = session else {
        return ContextRestorePlan::NoContext;
    };
    if let Some(session_id) = session
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return ContextRestorePlan::Resume {
            session_id: session_id.to_string(),
        };
    }
    if session.context_carry == Some(ContextCarryState::Failed) {
        return ContextRestorePlan::NoContext;
    }
    restore_context_payload(&session.messages)
        .map(|payload| ContextRestorePlan::Reinject { payload })
        .unwrap_or(ContextRestorePlan::NoContext)
}

pub(crate) fn context_restore_plan_for_session_before_turn(
    session: Option<&ChatSession>,
    streaming_agent_message_id: &str,
) -> ContextRestorePlan {
    let Some(session) = session else {
        return ContextRestorePlan::NoContext;
    };
    if let Some(session_id) = session
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return ContextRestorePlan::Resume {
            session_id: session_id.to_string(),
        };
    }
    if session.context_carry == Some(ContextCarryState::Failed) {
        return ContextRestorePlan::NoContext;
    }
    let end = session
        .messages
        .iter()
        .position(|message| message.id == streaming_agent_message_id)
        .map(|agent_index| {
            if agent_index > 0 && session.messages[agent_index - 1].role == MessageRole::Human {
                agent_index - 1
            } else {
                agent_index
            }
        })
        .unwrap_or(session.messages.len());
    restore_context_payload(&session.messages[..end])
        .map(|payload| ContextRestorePlan::Reinject { payload })
        .unwrap_or(ContextRestorePlan::NoContext)
}

pub(crate) fn apply_restore_prompt_prefix(prompt: String, plan: &ContextRestorePlan) -> String {
    let Some(payload) = plan.restore_context() else {
        return prompt;
    };
    format!("{}\n\n{}", payload.prompt_prefix, prompt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::session::{MessagePart, SessionState};

    fn session_with_messages(messages: Vec<ChatMessage>) -> ChatSession {
        ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages,
            state: SessionState::Active,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some("claude".to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        }
    }

    fn human(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: MessageRole::Human,
            content: content.to_string(),
            thinking: None,
            activities: None,
            parts: None,
            streaming_final_seq: 0,
            timestamp: 1.0,
            mentions: None,
        }
    }

    fn agent(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: MessageRole::Agent,
            content: content.to_string(),
            thinking: None,
            activities: None,
            parts: None,
            streaming_final_seq: 0,
            timestamp: 1.0,
            mentions: None,
        }
    }

    #[test]
    fn test_context_restore_planはagent_session_idを優先しなければreinjectする() {
        let mut session = session_with_messages(vec![human("m1", "remember alpha")]);
        session.agent_session_id = Some("backend-session".to_string());

        assert!(matches!(
            context_restore_plan_for_session(Some(&session)),
            ContextRestorePlan::Resume { ref session_id } if session_id == "backend-session"
        ));

        session.agent_session_id = None;
        assert!(matches!(
            context_restore_plan_for_session(Some(&session)),
            ContextRestorePlan::Reinject { .. }
        ));
    }

    #[test]
    fn test_context_restore_plan_before_turnは現在turnを除外する() {
        let session = session_with_messages(vec![
            human("prior", "remember gamma"),
            human("human-current", "what was it?"),
            agent("agent-current", ""),
        ]);

        let ContextRestorePlan::Reinject { payload } =
            context_restore_plan_for_session_before_turn(Some(&session), "agent-current")
        else {
            panic!("expected reinjection plan");
        };
        assert!(payload.prompt_prefix.contains("remember gamma"));
        assert!(!payload.prompt_prefix.contains("what was it?"));
    }

    #[test]
    fn test_restore_context_messagesはsystemを除外しparts文字列を使う() {
        let messages = vec![
            ChatMessage {
                id: "sys".to_string(),
                role: MessageRole::System,
                content: "internal note".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 1.0,
                mentions: None,
            },
            ChatMessage {
                id: "human".to_string(),
                role: MessageRole::Human,
                content: String::new(),
                thinking: None,
                activities: None,
                parts: Some(vec![MessagePart::Text {
                    content: "remember beta".to_string(),
                    parent_tool_use_id: None,
                }]),
                streaming_final_seq: 0,
                timestamp: 2.0,
                mentions: None,
            },
            agent("agent", "beta acknowledged"),
        ];

        let restore_messages = restore_context_messages(&messages);

        assert_eq!(restore_messages.len(), 2);
        assert_eq!(restore_messages[0].role, MessageRole::Human);
        assert_eq!(restore_messages[0].content, "remember beta");
        assert_eq!(restore_messages[1].role, MessageRole::Agent);
        assert_eq!(restore_messages[1].content, "beta acknowledged");
    }

    #[test]
    fn test_apply_restore_prompt_prefixはreinjectだけprefixする() {
        let plan = ContextRestorePlan::Reinject {
            payload: RestoreContextPayload {
                prompt_prefix: "history".to_string(),
            },
        };

        assert_eq!(
            apply_restore_prompt_prefix("next".to_string(), &plan),
            "history\n\nnext"
        );
        assert_eq!(
            apply_restore_prompt_prefix("next".to_string(), &ContextRestorePlan::NoContext),
            "next"
        );
    }
}
