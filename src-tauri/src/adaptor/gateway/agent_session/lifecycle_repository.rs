use std::sync::Arc;

use crate::domain::agent_session::aggregates::session::{
    QueueItem, QueueState, Session, SessionRestore,
};
use crate::domain::agent_session::events::AgentSessionDomainEvent;
use crate::domain::agent_session::repository::{
    AgentSessionLifecycleRepository, AgentSessionLifecycleRepositoryError, BackendSelection,
    PreparedSessionChange,
};
use crate::domain::agent_session::services::classify_recovery_fact;
use crate::domain::agent_session::value_objects::SessionState;
use crate::domain::local_event::{
    AgentSessionProjectionRecord, AgentSessionStateRecord, LocalEventQuery, LocalEventQueryResult,
    LocalEventTransactionRepository, SessionProjectionRecord,
};
use crate::usecase::agent_session::session::SessionStore;

pub(crate) struct LocalAgentSessionLifecycleRepository {
    repository: Arc<dyn LocalEventTransactionRepository>,
    session_store: Arc<SessionStore>,
}

impl LocalAgentSessionLifecycleRepository {
    pub(crate) fn new(
        repository: Arc<dyn LocalEventTransactionRepository>,
        session_store: Arc<SessionStore>,
    ) -> Self {
        Self {
            repository,
            session_store,
        }
    }
}

fn session_state(state: AgentSessionStateRecord) -> SessionState {
    match state {
        AgentSessionStateRecord::Active => SessionState::Active,
        AgentSessionStateRecord::Idle => SessionState::Idle,
        AgentSessionStateRecord::Done => SessionState::Done,
        AgentSessionStateRecord::Error => SessionState::Error,
        AgentSessionStateRecord::Closed => SessionState::Closed,
        AgentSessionStateRecord::Archived => SessionState::Archived,
    }
}

fn restore(
    projection: AgentSessionProjectionRecord,
    obligations: &[(String, crate::domain::local_event::ObligationRecord)],
) -> Result<Session, AgentSessionLifecycleRepositoryError> {
    let state = session_state(projection.meta.state);
    let recovery_fact = classify_recovery_fact(
        projection.meta.pending_recovery_message.is_some(),
        obligations
            .iter()
            .map(|(identity, record)| (identity.as_str(), record)),
    );
    let current_turn =
        Session::current_turn_from_events(&projection.reducer_events, state.is_closed());
    let queue = projection
        .pending_send_queue
        .into_iter()
        .map(|item| QueueItem {
            id: item.queue_item_id,
            operation_id: item.input_ref,
            reserved_turn_id: Some(item.reserved_turn_id),
            human_message_id: Some(item.human_message_id),
        })
        .collect();
    Session::restore(SessionRestore {
        id: projection.meta.id,
        revision: projection.meta.state_revision,
        state,
        has_messages: projection.meta.message_count != 0,
        has_provider_session: projection.meta.agent_session_id.is_some(),
        current_turn,
        last_terminal: None,
        queue: QueueState::restore(queue, projection.queue_paused_at_bits.is_some()),
        recovery_fact,
    })
    .map_err(|error| AgentSessionLifecycleRepositoryError::Corrupt(format!("{error:?}")))
}

#[async_trait::async_trait]
impl AgentSessionLifecycleRepository for LocalAgentSessionLifecycleRepository {
    async fn restore_session(
        &self,
        session_id: &str,
    ) -> Result<Session, AgentSessionLifecycleRepositoryError> {
        let result = self
            .repository
            .query(LocalEventQuery::AgentSessionLifecycleSnapshot {
                session_id: session_id.to_string(),
            })
            .await
            .map_err(|error| {
                AgentSessionLifecycleRepositoryError::Unavailable(error.to_string())
            })?;
        let LocalEventQueryResult::AgentSessionLifecycleSnapshot(snapshot) = result else {
            return Err(AgentSessionLifecycleRepositoryError::Corrupt(
                "agent session lifecycle query returned the wrong result".into(),
            ));
        };
        let snapshot = snapshot.ok_or(AgentSessionLifecycleRepositoryError::NotFound)?;
        let SessionProjectionRecord::AgentSession(projection) = snapshot.session.projection else {
            return Err(AgentSessionLifecycleRepositoryError::Corrupt(
                "agent session lifecycle projection has the wrong owner kind".into(),
            ));
        };
        restore(*projection, &snapshot.pending_obligations)
    }

    async fn prepare_session_change(
        &self,
        session_id: &str,
        expected_revision: u64,
        events: &[AgentSessionDomainEvent],
    ) -> Result<Option<PreparedSessionChange>, AgentSessionLifecycleRepositoryError> {
        self.session_store
            .prepare_event_projection_mutations_if_current_revision(
                session_id,
                expected_revision,
                events,
            )
            .map(|participant| participant.map(PreparedSessionChange::from_atomic_participant))
            .map_err(AgentSessionLifecycleRepositoryError::Unavailable)
    }

    async fn prepare_lifecycle_change(
        &self,
        session_id: &str,
        expected_revision: u64,
        final_state: SessionState,
        backend_selection: Option<&BackendSelection>,
        events: &[AgentSessionDomainEvent],
    ) -> Result<Option<PreparedSessionChange>, AgentSessionLifecycleRepositoryError> {
        self.session_store
            .prepare_lifecycle_acceptance_mutations(
                session_id,
                expected_revision,
                events,
                final_state,
                backend_selection
                    .map(|selection| (selection.backend_id.as_str(), selection.model_id.as_str())),
            )
            .map(|participant| participant.map(PreparedSessionChange::from_atomic_participant))
            .map_err(AgentSessionLifecycleRepositoryError::Unavailable)
    }
}
