use std::future::Future;
use std::pin::Pin;

use crate::domain::agent_session::gateway::AgentRuntimeEvent;
use crate::domain::workflow::WorkflowError;
use crate::usecase::agent_session::session::{
    ChatMessage, ChatSession, ContextCarryState, ModelInfo, PermissionRequestMsg, SessionState,
    TokenUsage,
};
use crate::usecase::agent_session::status::TurnPhase;
use crate::usecase::workflow::ports::{
    WorkflowStallClearedNotification, WorkflowStallObservedNotification,
    WorkflowTurnCompleteNotification,
};

pub(crate) trait AgentTaskSpawner: Send + Sync {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>);
}

#[derive(Debug, Clone)]
pub(crate) struct AgentStreamingDeltaPayload {
    pub chat_session_id: String,
    pub message_id: String,
    pub seq: u64,
    pub snapshot: bool,
    pub parts: Vec<crate::usecase::agent_session::session::MessagePart>,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentSessionStateChangedPayload {
    pub chat_session_id: String,
    pub turn_phase: TurnPhase,
    pub exit_code: Option<i64>,
    pub completed_at: Option<f64>,
    pub interrupted: bool,
    pub session_state: Option<SessionState>,
    pub pending_permission_request: Option<PermissionRequestMsg>,
    pub pending_permission_state_revision: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentStallObservedPayload {
    pub chat_session_id: String,
    pub turn_phase: TurnPhase,
    pub idle_secs: u64,
    pub signal_count: u32,
    pub cap_reached: bool,
}

pub(crate) trait AgentSessionEventNotifier: Send + Sync {
    fn session_state_changed(&self, payload: AgentSessionStateChangedPayload);

    fn stall_observed(&self, payload: AgentStallObservedPayload);

    fn stall_cleared(&self, session_id: &str);

    fn streaming_delta(&self, payload: AgentStreamingDeltaPayload) -> bool;

    fn supported_commands_updated(
        &self,
        session_id: &str,
        commands: Vec<crate::domain::agent_session::value_objects::SlashCommand>,
    );

    fn token_usage_updated(&self, session_id: &str, token_usage: TokenUsage);

    fn permission_mode_changed(&self, session_id: &str, permission_mode: &str);

    fn models_updated(
        &self,
        session_id: &str,
        available_models: Vec<ModelInfo>,
        selected_model: String,
    );

    fn context_carry_updated(
        &self,
        session_id: &str,
        agent_session_id: Option<String>,
        context_carry: Option<ContextCarryState>,
        updated_at: f64,
    );

    fn pending_message_consumed(
        &self,
        session_id: &str,
        queued_turn_id: Option<String>,
        human_message: Option<ChatMessage>,
        agent_message: ChatMessage,
    );

    fn turn_prepared(
        &self,
        session: &ChatSession,
        human_message: &crate::usecase::agent_session::session::ChatMessage,
        agent_message: &crate::usecase::agent_session::session::ChatMessage,
    );

    fn runtime_event_debug(&self, _session_id: &str, _event: &AgentRuntimeEvent) {}
}

#[async_trait::async_trait]
pub(crate) trait WorkflowTurnCompleteNotifier: Send + Sync {
    async fn turn_completed(&self, notification: WorkflowTurnCompleteNotification);
}

#[async_trait::async_trait]
pub(crate) trait WorkflowStallNotifier: Send + Sync {
    async fn stall_observed(&self, notification: WorkflowStallObservedNotification);

    async fn stall_cleared(
        &self,
        notification: WorkflowStallClearedNotification,
    ) -> Result<(), WorkflowError>;
}
