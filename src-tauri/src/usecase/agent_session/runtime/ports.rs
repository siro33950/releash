use std::future::Future;
use std::pin::Pin;

use crate::domain::agent_session::gateway::AgentRuntimeEvent;
use crate::domain::workflow::WorkflowError;
use crate::usecase::agent_session::event_log::AgentSessionEvent;
use crate::usecase::agent_session::session::{
    ChatMessage, ChatSession, ContextCarryState, ModelInfo, PermissionRequestMsg, SessionState,
    TokenUsage,
};
use crate::usecase::agent_session::status::{SessionNotice, TurnPhase};
use crate::usecase::workflow::ports::{
    WorkflowStallClearedNotification, WorkflowStallObservedNotification,
    WorkflowTurnCompleteNotification,
};

pub(crate) trait AgentTaskSpawner: Send + Sync {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>);
}

/// Process-local lease for the narrow interval after the durable execution
/// claim wins and before runtime state becomes its observable owner. Dropping
/// the lease must release that temporary ownership, including task aborts and
/// unwinds, so recovery never hides an orphaned durable reservation.
pub(crate) struct AcceptedSendExecutionClaim {
    release: Option<Box<dyn FnOnce() + Send + 'static>>,
}

/// A process-local request to wake the app-lifetime accepted-send recovery
/// owner.
///
/// The token is deliberately inert until `publish` is called. Callers must
/// attach it to the execution claim with `wake_after_release`, or explicitly
/// release every other ownership marker before publishing it. This prevents a
/// recovery scan from consuming the sole wake while the dying worker still
/// reports itself as the owner.
#[must_use = "publish only after every process-local accepted-send owner is released"]
pub(crate) struct AcceptedSendRecoveryWake {
    publish: Option<Box<dyn FnOnce() + Send + 'static>>,
}

pub(crate) enum AcceptedQueuedTurnExecutionClaimOutcome {
    Claimed(AcceptedSendExecutionClaim),
    Blocked,
}

impl AcceptedSendRecoveryWake {
    pub(crate) fn new(publish: impl FnOnce() + Send + 'static) -> Self {
        Self {
            publish: Some(Box::new(publish)),
        }
    }

    pub(crate) fn publish(mut self) {
        if let Some(publish) = self.publish.take() {
            publish();
        }
    }
}

impl AcceptedSendExecutionClaim {
    pub(crate) fn new(release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            release: Some(Box::new(release)),
        }
    }

    pub(crate) fn release_then(mut self, release: impl FnOnce() + Send + 'static) -> Self {
        let previous = self.release.take();
        Self::new(move || {
            if let Some(previous) = previous {
                previous();
            }
            release();
        })
    }

    /// Arm one recovery wake behind this claim's complete release chain.
    ///
    /// Existing release callbacks run first, including the queued dispatch
    /// marker callback installed by the production driver. The recovery owner
    /// therefore cannot observe either marker after consuming this wake.
    pub(crate) fn wake_after_release(self, recovery_wake: AcceptedSendRecoveryWake) -> Self {
        self.release_then(move || recovery_wake.publish())
    }
}

impl Drop for AcceptedSendExecutionClaim {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

#[async_trait::async_trait]
pub(crate) trait AcceptedSendObligationDriver: Send + Sync {
    async fn claim_immediate_turn_execution(
        &self,
        operation_id: &str,
        obligation_id: &str,
    ) -> Result<AcceptedSendExecutionClaim, ()>;

    async fn claim_queued_turn_execution(
        &self,
        operation_id: &str,
        obligation_id: &str,
        session_id: &str,
        queue_item_id: &str,
        event: AgentSessionEvent,
    ) -> Result<AcceptedQueuedTurnExecutionClaimOutcome, ()>;

    async fn mark_turn_running(
        &self,
        operation_id: &str,
        obligation_id: &str,
        turn_id: u64,
    ) -> Result<(), ()>;

    async fn reconcile_turn_execution(
        &self,
        operation_id: &str,
        obligation_id: &str,
    ) -> Option<AcceptedSendRecoveryWake>;
}

#[derive(Debug, Clone)]
pub(crate) struct AgentStreamingDeltaPayload {
    pub chat_session_id: String,
    pub message_id: String,
    pub seq: u64,
    pub snapshot: bool,
    pub parts: Vec<crate::usecase::agent_session::session::MessagePart>,
    pub message: Option<ChatMessage>,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentSessionStateChangedPayload {
    pub chat_session_id: String,
    pub turn_phase: TurnPhase,
    pub exit_code: Option<i64>,
    pub completed_at: Option<f64>,
    pub interrupted: bool,
    pub session_state: Option<SessionState>,
    pub queue_paused: Option<bool>,
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
    fn persist_notice(&self, notice: SessionNotice);

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
