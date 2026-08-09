use crate::domain::agent_session::aggregates::session::Session;
use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionRemovalAuthorization};
use crate::domain::agent_session::events::AgentSessionDomainEvent;
use crate::domain::agent_session::value_objects::SessionState;
use crate::domain::local_event::LocalStateMutation;
use crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSessionLifecycleRepositoryError {
    NotFound,
    Corrupt(String),
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAgentSessionRepositoryError {
    AlreadyExists,
    Conflict,
    ProviderSessionAlreadyOwned { agent_session_id: String },
    InvalidRequest,
    Corrupt,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedProviderAgentSession {
    session: AgentSession,
    revision: u64,
}

impl VersionedProviderAgentSession {
    pub(crate) fn restored(session: AgentSession, revision: u64) -> Self {
        Self { session, revision }
    }

    pub(crate) fn session(&self) -> &AgentSession {
        &self.session
    }

    pub(crate) fn session_mut(&mut self) -> &mut AgentSession {
        &mut self.session
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn into_session(self) -> AgentSession {
        self.session
    }
}

#[async_trait::async_trait]
pub trait ProviderAgentSessionRepository: Send + Sync {
    async fn create(
        &self,
        session: AgentSession,
        caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError>;

    async fn create_with_lifecycle_events(
        &self,
        session: AgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError>;

    async fn find(
        &self,
        session_id: &str,
    ) -> Result<Option<VersionedProviderAgentSession>, ProviderAgentSessionRepositoryError>;

    async fn save(
        &self,
        session: VersionedProviderAgentSession,
        caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError>;

    async fn remove(
        &self,
        session: VersionedProviderAgentSession,
        authorization: AgentSessionRemovalAuthorization,
        caller_request_id: &str,
    ) -> Result<(), ProviderAgentSessionRepositoryError>;
}

/// Domain value selected for a backend-switch lifecycle change.
///
/// The gateway resolves provider configuration into this value before the
/// repository maps the accepted aggregate change to projection participants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSelection {
    pub backend_id: String,
    pub model_id: String,
}

/// Opaque atomic-batch participant prepared from one aggregate change.
///
/// The repository port exposes this domain concept instead of leaking the
/// local-event projection/storage vocabulary in its method signatures. Only
/// the transaction-owning usecase opens it when assembling the existing
/// `LocalEventTransactionRepository` batch.
#[derive(Debug, Clone)]
pub struct PreparedSessionChange {
    participant: Vec<LocalStateMutation>,
}

impl PreparedSessionChange {
    pub(crate) fn from_atomic_participant(participant: Vec<LocalStateMutation>) -> Self {
        Self { participant }
    }

    pub(crate) fn into_atomic_participant(self) -> Vec<LocalStateMutation> {
        self.participant
    }
}

/// Domain-language boundary for bounded Session restore and CAS preparation.
///
/// Implementations consume storage projections and obligation views inside the
/// adapter. This port never exposes persistence vocabulary and never creates a
/// second commit authority.
#[async_trait::async_trait]
pub trait AgentSessionLifecycleRepository: Send + Sync {
    async fn restore_session(
        &self,
        session_id: &str,
    ) -> Result<Session, AgentSessionLifecycleRepositoryError>;

    /// Maps one aggregate event change to the existing atomic-CAS
    /// participants. Implementations never commit independently.
    async fn prepare_session_change(
        &self,
        session_id: &str,
        expected_revision: u64,
        events: &[AgentSessionDomainEvent],
    ) -> Result<Option<PreparedSessionChange>, AgentSessionLifecycleRepositoryError>;

    /// Maps an accepted lifecycle aggregate change to the existing session
    /// projection CAS participant. The default keeps lightweight test
    /// repositories source-compatible; production repositories must preserve
    /// the supplied final state and backend selection.
    async fn prepare_lifecycle_change(
        &self,
        session_id: &str,
        expected_revision: u64,
        final_state: SessionState,
        backend_selection: Option<&BackendSelection>,
        events: &[AgentSessionDomainEvent],
    ) -> Result<Option<PreparedSessionChange>, AgentSessionLifecycleRepositoryError> {
        let _ = (final_state, backend_selection);
        self.prepare_session_change(session_id, expected_revision, events)
            .await
    }
}
