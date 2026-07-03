use serde::Serialize;
use tauri::Emitter;

use crate::adaptor::protocol::{AgentSupportedCommandMsg, AgentSupportedCommandsUpdated};
use crate::usecase::agent_session::runtime::ports::{
    AgentSessionEventNotifier, AgentSessionStateChangedPayload, AgentStreamingDeltaPayload,
};
use crate::usecase::agent_session::session::{
    project_tool_output_parts_for_stream, ChatMessage, ChatSession, ContextCarryState, ModelInfo,
    PermissionRequestMsg, SessionState, TokenUsage,
};
use crate::usecase::agent_session::status::TurnPhase;

pub(crate) struct TauriAgentSessionEventNotifier {
    app: tauri::AppHandle,
}

impl TauriAgentSessionEventNotifier {
    pub(crate) fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

#[derive(Clone, Serialize)]
struct AgentTurnPreparedPayload<'a> {
    chat_session_id: &'a str,
    session: &'a ChatSession,
    human_message: &'a ChatMessage,
    agent_message: &'a ChatMessage,
}

#[derive(Clone, Serialize)]
struct AgentSessionStateChangedEventPayload {
    chat_session_id: String,
    turn_phase: TurnPhase,
    exit_code: Option<i64>,
    completed_at: Option<f64>,
    interrupted: bool,
    session_state: Option<SessionState>,
    pending_permission_request: Option<PermissionRequestMsg>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentTurnUsageUpdatedPayload {
    #[serde(rename = "chatSessionId")]
    chat_session_id: String,
    token_usage: TokenUsage,
}

#[derive(Clone, Serialize)]
struct AgentPermissionModeChangedPayload<'a> {
    chat_session_id: &'a str,
    permission_mode: &'a str,
}

#[derive(Clone, Serialize)]
struct AgentModelsUpdatedPayload {
    chat_session_id: String,
    available_models: Vec<ModelInfo>,
    selected_model: String,
}

#[derive(Clone, Serialize)]
struct AgentSessionContextCarryUpdatedPayload {
    chat_session_id: String,
    agent_session_id: Option<String>,
    context_carry: Option<ContextCarryState>,
    updated_at: f64,
}

#[derive(Clone, Serialize)]
struct AgentPendingMessageConsumedPayload {
    chat_session_id: String,
    queued_turn_id: Option<String>,
    human_message: Option<ChatMessage>,
    agent_message: ChatMessage,
}

impl AgentSessionEventNotifier for TauriAgentSessionEventNotifier {
    fn session_state_changed(&self, payload: AgentSessionStateChangedPayload) {
        let _ = self.app.emit(
            "agent-session-state-changed",
            AgentSessionStateChangedEventPayload {
                chat_session_id: payload.chat_session_id,
                turn_phase: payload.turn_phase,
                exit_code: payload.exit_code,
                completed_at: payload.completed_at,
                interrupted: payload.interrupted,
                session_state: payload.session_state,
                pending_permission_request: payload.pending_permission_request,
            },
        );
    }

    fn streaming_delta(&self, payload: AgentStreamingDeltaPayload) -> bool {
        let parts = project_tool_output_parts_for_stream(&payload.parts);
        self.app
            .emit(
                "agent-streaming-delta",
                serde_json::json!({
                    "chat_session_id": payload.chat_session_id,
                    "message_id": payload.message_id,
                    "seq": payload.seq,
                    "snapshot": payload.snapshot,
                    "parts": parts,
                }),
            )
            .is_ok()
    }

    fn supported_commands_updated(
        &self,
        session_id: &str,
        commands: Vec<crate::domain::agent_session::value_objects::SlashCommand>,
    ) {
        let payload = AgentSupportedCommandsUpdated {
            chat_session_id: session_id.to_string(),
            commands: commands
                .into_iter()
                .map(|command| AgentSupportedCommandMsg {
                    name: command.name,
                    description: command.description,
                    argument_hint: command.argument_hint,
                })
                .collect(),
        };
        let _ = self.app.emit("agent-supported-commands-updated", payload);
    }

    fn token_usage_updated(&self, session_id: &str, token_usage: TokenUsage) {
        let _ = self.app.emit(
            "agent-turn-usage-updated",
            AgentTurnUsageUpdatedPayload {
                chat_session_id: session_id.to_string(),
                token_usage,
            },
        );
    }

    fn permission_mode_changed(&self, session_id: &str, permission_mode: &str) {
        let _ = self.app.emit(
            "agent-permission-mode-changed",
            AgentPermissionModeChangedPayload {
                chat_session_id: session_id,
                permission_mode,
            },
        );
    }

    fn models_updated(
        &self,
        session_id: &str,
        available_models: Vec<ModelInfo>,
        selected_model: String,
    ) {
        let _ = self.app.emit(
            "agent-models-updated",
            AgentModelsUpdatedPayload {
                chat_session_id: session_id.to_string(),
                available_models,
                selected_model,
            },
        );
    }

    fn context_carry_updated(
        &self,
        session_id: &str,
        agent_session_id: Option<String>,
        context_carry: Option<ContextCarryState>,
        updated_at: f64,
    ) {
        let _ = self.app.emit(
            "agent-session-context-carry-updated",
            AgentSessionContextCarryUpdatedPayload {
                chat_session_id: session_id.to_string(),
                agent_session_id,
                context_carry,
                updated_at,
            },
        );
    }

    fn pending_message_consumed(
        &self,
        session_id: &str,
        queued_turn_id: Option<String>,
        human_message: Option<ChatMessage>,
        agent_message: ChatMessage,
    ) {
        let _ = self.app.emit(
            "agent-pending-message-consumed",
            AgentPendingMessageConsumedPayload {
                chat_session_id: session_id.to_string(),
                queued_turn_id,
                human_message,
                agent_message,
            },
        );
    }

    fn turn_prepared(
        &self,
        session: &ChatSession,
        human_message: &ChatMessage,
        agent_message: &ChatMessage,
    ) {
        let _ = self.app.emit(
            "agent-turn-prepared",
            AgentTurnPreparedPayload {
                chat_session_id: &session.id,
                session,
                human_message,
                agent_message,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::session::{MessageRole, PermissionRequestKindMsg};
    use serde_json::json;

    fn message(id: &str, role: MessageRole) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role,
            content: "content".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            streaming_final_seq: 0,
            timestamp: 1.0,
            mentions: None,
        }
    }

    fn session() -> ChatSession {
        ChatSession {
            id: "session-1".to_string(),
            worktree_path: "/worktree".to_string(),
            messages: Vec::new(),
            state: SessionState::Idle,
            created_at: 1.0,
            updated_at: 2.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "ask".to_string(),
            plan_mode: false,
            selected_model: None,
            permission_profile_id: None,
            backend_id: Some("codex".to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        }
    }

    #[test]
    fn serializes_turn_prepared_payload_with_snake_case_keys() {
        let session = session();
        let human = message("human-1", MessageRole::Human);
        let agent = message("agent-1", MessageRole::Agent);

        let value = serde_json::to_value(AgentTurnPreparedPayload {
            chat_session_id: &session.id,
            session: &session,
            human_message: &human,
            agent_message: &agent,
        })
        .unwrap();

        assert_eq!(value["chat_session_id"], "session-1");
        assert_eq!(value["human_message"]["id"], "human-1");
        assert_eq!(value["agent_message"]["id"], "agent-1");
    }

    #[test]
    fn serializes_session_state_changed_payload() {
        let value = serde_json::to_value(AgentSessionStateChangedEventPayload {
            chat_session_id: "session-1".to_string(),
            turn_phase: TurnPhase::WaitingPermission,
            exit_code: Some(1),
            completed_at: Some(2.0),
            interrupted: true,
            session_state: Some(SessionState::Error),
            pending_permission_request: Some(PermissionRequestMsg {
                id: "req-1".to_string(),
                tool_use_id: None,
                tool_name: "Bash".to_string(),
                kind: PermissionRequestKindMsg::ToolApproval,
                input: Some(json!({"command": "test"})),
                plan: None,
                allowed_prompts: Vec::new(),
                questions: Vec::new(),
                title: None,
                display_name: None,
                description: None,
                decision_reason: None,
            }),
        })
        .unwrap();

        assert_eq!(value["chat_session_id"], "session-1");
        assert_eq!(value["turn_phase"], "waiting_permission");
        assert_eq!(value["session_state"], "error");
        assert_eq!(value["pending_permission_request"]["kind"], "tool_approval");
    }

    #[test]
    fn serializes_usage_payload_with_legacy_chat_session_id_key() {
        let value = serde_json::to_value(AgentTurnUsageUpdatedPayload {
            chat_session_id: "session-1".to_string(),
            token_usage: TokenUsage {
                input_tokens: 1,
                output_tokens: 2,
                total_tokens: Some(3),
                context_window_tokens: None,
            },
        })
        .unwrap();

        assert_eq!(value["chatSessionId"], "session-1");
        assert_eq!(value["tokenUsage"]["inputTokens"], 1);
        assert!(value["tokenUsage"].get("contextWindowTokens").is_none());
    }

    #[test]
    fn serializes_permission_mode_and_models_payloads() {
        let permission = serde_json::to_value(AgentPermissionModeChangedPayload {
            chat_session_id: "session-1",
            permission_mode: "ask",
        })
        .unwrap();
        let models = serde_json::to_value(AgentModelsUpdatedPayload {
            chat_session_id: "session-1".to_string(),
            available_models: vec![ModelInfo {
                id: "codex:gpt".to_string(),
                display_name: "GPT".to_string(),
                backend: "codex".to_string(),
                model_id: "gpt".to_string(),
            }],
            selected_model: "codex:gpt".to_string(),
        })
        .unwrap();

        assert_eq!(permission["chat_session_id"], "session-1");
        assert_eq!(permission["permission_mode"], "ask");
        assert_eq!(models["available_models"][0]["displayName"], "GPT");
        assert_eq!(models["selected_model"], "codex:gpt");
    }

    #[test]
    fn serializes_context_carry_and_pending_message_payloads() {
        let context = serde_json::to_value(AgentSessionContextCarryUpdatedPayload {
            chat_session_id: "session-1".to_string(),
            agent_session_id: Some("backend-1".to_string()),
            context_carry: Some(ContextCarryState::Resumed),
            updated_at: 3.0,
        })
        .unwrap();
        let pending = serde_json::to_value(AgentPendingMessageConsumedPayload {
            chat_session_id: "session-1".to_string(),
            queued_turn_id: Some("queued-1".to_string()),
            human_message: Some(message("human-1", MessageRole::Human)),
            agent_message: message("agent-1", MessageRole::Agent),
        })
        .unwrap();

        assert_eq!(context["context_carry"], "resumed");
        assert_eq!(context["agent_session_id"], "backend-1");
        assert_eq!(pending["queued_turn_id"], "queued-1");
        assert_eq!(pending["agent_message"]["id"], "agent-1");
    }
}
