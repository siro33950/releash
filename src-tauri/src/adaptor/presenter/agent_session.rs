use serde::Serialize;
use tauri::Emitter;

use crate::adaptor::protocol::agent::MessagePartDtoV1;
use crate::adaptor::protocol::agent_session_v1::{ChatMessageDtoV1, ChatSessionDtoV1};
use crate::adaptor::protocol::{AgentSupportedCommandMsg, AgentSupportedCommandsUpdated};
use crate::usecase::agent_session::runtime::ports::{
    AgentSessionEventNotifier, AgentSessionStateChangedPayload, AgentStallObservedPayload,
    AgentStreamingDeltaPayload,
};
use crate::usecase::agent_session::session::{
    project_tool_output_parts_for_stream, ChatMessage, ChatSession, ContextCarryState, ModelInfo,
    PermissionRequestMsg, SessionState, TokenUsage,
};
use crate::usecase::agent_session::status::{SessionNotice, TurnPhase};

pub(crate) fn permission_response_outcome(
    value: crate::usecase::agent_session::operation::PermissionResponseCommandOutcome,
) -> crate::adaptor::protocol::agent_session_v1::PermissionResponseCommandOutcomeDtoV1 {
    value.into()
}

pub(crate) fn permission_response_operation(
    value: crate::usecase::agent_session::operation::AcceptedPermissionResponseOperation,
) -> crate::adaptor::protocol::agent_session_v1::PermissionResponseOperationViewDtoV1 {
    value.into()
}

pub(crate) fn permission_response_command_error(
    value: crate::usecase::agent_session::operation::PermissionResponseOperationError,
) -> crate::adaptor::protocol::agent_session_v1::PermissionResponseCommandErrorDtoV1 {
    use crate::adaptor::protocol::agent_session_v1::PermissionResponseCommandErrorDtoV1 as D;
    use crate::usecase::agent_session::operation::PermissionResponseOperationError as E;
    match value {
        E::InvalidRequest => D::InvalidRequest,
        E::PayloadConflict => D::PayloadConflict,
        E::ShutdownInProgress => D::ShutdownInProgress,
        E::NotFound => D::NotFound,
        E::CapacityExceeded => D::CapacityExceeded,
        E::Internal { correlation_id } => D::Internal { correlation_id },
    }
}

pub(crate) fn permission_response_lookup_error(
    value: crate::usecase::agent_session::operation::GetPermissionResponseOperationError,
) -> crate::adaptor::protocol::agent_session_v1::PermissionResponseLookupErrorDtoV1 {
    use crate::adaptor::protocol::agent_session_v1::PermissionResponseLookupErrorDtoV1 as D;
    use crate::usecase::agent_session::operation::GetPermissionResponseOperationError as E;
    match value {
        E::InvalidRequest => D::InvalidRequest,
        E::NotFound => D::NotFound,
        E::QueryBusy => D::QueryBusy,
        E::DeadlineExceeded => D::DeadlineExceeded,
        E::StorageUnavailable { failure } => D::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => D::Internal { correlation_id },
    }
}

pub(crate) struct TauriAgentSessionEventNotifier {
    app: tauri::AppHandle,
}

impl TauriAgentSessionEventNotifier {
    pub(crate) fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

#[derive(Clone, Serialize)]
struct AgentTurnPreparedPayload {
    chat_session_id: String,
    session: ChatSessionDtoV1,
    human_message: ChatMessageDtoV1,
    agent_message: ChatMessageDtoV1,
}

#[derive(Clone, Serialize)]
struct AgentSessionStateChangedEventPayload {
    chat_session_id: String,
    turn_phase: TurnPhase,
    exit_code: Option<i64>,
    completed_at: Option<f64>,
    interrupted: bool,
    session_state: Option<SessionState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    queue_paused: Option<bool>,
    pending_permission_request: Option<PermissionRequestMsg>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_permission_state_revision: Option<String>,
}

#[derive(Clone, Serialize)]
struct AgentStallObservedEventPayload {
    chat_session_id: String,
    turn_phase: TurnPhase,
    idle_secs: String,
    signal_count: String,
    cap_reached: bool,
}

#[derive(Clone, Serialize)]
struct AgentStallClearedEventPayload<'a> {
    chat_session_id: &'a str,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentTurnUsageUpdatedPayload {
    #[serde(rename = "chatSessionId")]
    chat_session_id: String,
    token_usage: AgentTurnUsageDtoV1,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentTurnUsageDtoV1 {
    input_tokens: String,
    output_tokens: String,
    total_tokens: Option<String>,
    context_window_tokens: Option<String>,
}

impl From<TokenUsage> for AgentTurnUsageDtoV1 {
    fn from(value: TokenUsage) -> Self {
        Self {
            input_tokens: value.input_tokens.to_string(),
            output_tokens: value.output_tokens.to_string(),
            total_tokens: value.total_tokens.map(|tokens| tokens.to_string()),
            context_window_tokens: value.context_window_tokens.map(|tokens| tokens.to_string()),
        }
    }
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
    human_message: Option<ChatMessageDtoV1>,
    agent_message: ChatMessageDtoV1,
}

#[derive(Clone, Serialize)]
struct AgentStreamingDeltaEventPayload {
    chat_session_id: String,
    message_id: String,
    seq: String,
    snapshot: bool,
    parts: Vec<MessagePartDtoV1>,
    message: Option<ChatMessageDtoV1>,
}

impl AgentSessionEventNotifier for TauriAgentSessionEventNotifier {
    fn persist_notice(&self, notice: SessionNotice) {
        let _ = self.app.emit("agent-session-notice", notice);
    }

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
                queue_paused: payload.queue_paused,
                pending_permission_request: payload.pending_permission_request,
                pending_permission_state_revision: payload
                    .pending_permission_state_revision
                    .map(|revision| revision.to_string()),
            },
        );
    }

    fn stall_observed(&self, payload: AgentStallObservedPayload) {
        let _ = self.app.emit(
            "agent-stall-observed",
            AgentStallObservedEventPayload {
                chat_session_id: payload.chat_session_id,
                turn_phase: payload.turn_phase,
                idle_secs: payload.idle_secs.to_string(),
                signal_count: payload.signal_count.to_string(),
                cap_reached: payload.cap_reached,
            },
        );
    }

    fn stall_cleared(&self, session_id: &str) {
        let _ = self.app.emit(
            "agent-stall-cleared",
            AgentStallClearedEventPayload {
                chat_session_id: session_id,
            },
        );
    }

    fn streaming_delta(&self, payload: AgentStreamingDeltaPayload) -> bool {
        let parts = project_tool_output_parts_for_stream(&payload.parts)
            .iter()
            .map(MessagePartDtoV1::from)
            .collect();
        self.app
            .emit(
                "agent-streaming-delta",
                AgentStreamingDeltaEventPayload {
                    chat_session_id: payload.chat_session_id,
                    message_id: payload.message_id,
                    seq: payload.seq.to_string(),
                    snapshot: payload.snapshot,
                    parts,
                    message: payload.message.map(Into::into),
                },
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
                token_usage: token_usage.into(),
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
                human_message: human_message.map(Into::into),
                agent_message: agent_message.into(),
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
                chat_session_id: session.id.clone(),
                session: session.into(),
                human_message: human_message.into(),
                agent_message: agent_message.into(),
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
            error_reason: None,
            created_at: 1.0,
            updated_at: 2.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "ask".to_string(),
            plan_mode: false,
            selected_model: None,
            permission_profile_id: None,
            backend_id: Some("codex".to_string()),
            workflow_node_session: false,
            workflow_node_context: None,
            context_epoch: None,
        }
    }

    #[test]
    fn serializes_turn_prepared_payload_with_snake_case_keys() {
        let session = session();
        let human = message("human-1", MessageRole::Human);
        let agent = message("agent-1", MessageRole::Agent);

        let value = serde_json::to_value(AgentTurnPreparedPayload {
            chat_session_id: session.id.clone(),
            session: (&session).into(),
            human_message: (&human).into(),
            agent_message: (&agent).into(),
        })
        .unwrap();

        assert_eq!(value["chat_session_id"], "session-1");
        assert_eq!(value["human_message"]["id"], "human-1");
        assert_eq!(value["agent_message"]["id"], "agent-1");
    }

    #[test]
    fn b075_serializes_session_state_changed_revision_as_a_decimal_string() {
        let value = serde_json::to_value(AgentSessionStateChangedEventPayload {
            chat_session_id: "session-1".to_string(),
            turn_phase: TurnPhase::WaitingPermission,
            exit_code: Some(1),
            completed_at: Some(2.0),
            interrupted: true,
            session_state: Some(SessionState::Error),
            queue_paused: Some(true),
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
            pending_permission_state_revision: Some(i64::MAX.to_string()),
        })
        .unwrap();

        assert_eq!(value["chat_session_id"], "session-1");
        assert_eq!(value["queue_paused"], true);
        assert_eq!(value["turn_phase"], "waiting_permission");
        assert_eq!(value["session_state"], "error");
        assert_eq!(value["pending_permission_request"]["kind"], "tool_approval");
        assert_eq!(
            value["pending_permission_state_revision"],
            i64::MAX.to_string()
        );
        assert_eq!(value["exit_code"], 1);
    }

    #[test]
    fn b075_serializes_stall_counts_as_decimal_strings() {
        let value = serde_json::to_value(AgentStallObservedEventPayload {
            chat_session_id: "session-1".to_string(),
            turn_phase: TurnPhase::Streaming,
            idle_secs: i64::MAX.to_string(),
            signal_count: i64::MAX.to_string(),
            cap_reached: true,
        })
        .unwrap();

        assert_eq!(value["chat_session_id"], "session-1");
        assert_eq!(value["turn_phase"], "streaming");
        assert_eq!(value["idle_secs"], i64::MAX.to_string());
        assert_eq!(value["signal_count"], i64::MAX.to_string());
        assert_eq!(value["cap_reached"], true);
    }

    #[test]
    fn serializes_stall_cleared_payload() {
        let value = serde_json::to_value(AgentStallClearedEventPayload {
            chat_session_id: "session-1",
        })
        .unwrap();

        assert_eq!(value["chat_session_id"], "session-1");
    }

    #[test]
    fn b075_serializes_usage_counts_as_decimal_strings_with_legacy_keys() {
        let value = serde_json::to_value(AgentTurnUsageUpdatedPayload {
            chat_session_id: "session-1".to_string(),
            token_usage: AgentTurnUsageDtoV1::from(TokenUsage {
                input_tokens: i64::MAX as u64,
                output_tokens: i64::MAX as u64,
                total_tokens: Some(i64::MAX as u64),
                context_window_tokens: Some(i64::MAX as u64),
            }),
        })
        .unwrap();

        assert_eq!(value["chatSessionId"], "session-1");
        for field in [
            "inputTokens",
            "outputTokens",
            "totalTokens",
            "contextWindowTokens",
        ] {
            assert_eq!(value["tokenUsage"][field], i64::MAX.to_string(), "{field}");
        }
    }

    #[test]
    fn b075_serializes_stream_sequence_as_a_decimal_string() {
        let value = serde_json::to_value(AgentStreamingDeltaEventPayload {
            chat_session_id: "session-1".to_string(),
            message_id: "message-1".to_string(),
            seq: i64::MAX.to_string(),
            snapshot: false,
            parts: Vec::new(),
            message: None,
        })
        .unwrap();

        assert_eq!(value["seq"], i64::MAX.to_string());
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
            human_message: Some(message("human-1", MessageRole::Human).into()),
            agent_message: message("agent-1", MessageRole::Agent).into(),
        })
        .unwrap();

        assert_eq!(context["context_carry"], "resumed");
        assert_eq!(context["agent_session_id"], "backend-1");
        assert_eq!(pending["queued_turn_id"], "queued-1");
        assert_eq!(pending["agent_message"]["id"], "agent-1");
    }
}
