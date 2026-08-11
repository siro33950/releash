use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionRemovalAuthorization};
use crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSessionRepositoryError {
    AlreadyExists,
    Conflict,
    ProviderSessionAlreadyOwned { agent_session_id: String },
    InvalidRequest,
    Corrupt,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedAgentSession {
    session: AgentSession,
    revision: u64,
}

impl VersionedAgentSession {
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
pub trait AgentSessionRepository: Send + Sync {
    async fn create(
        &self,
        session: AgentSession,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError>;

    async fn create_with_lifecycle_events(
        &self,
        session: AgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError>;

    async fn find(
        &self,
        session_id: &str,
    ) -> Result<Option<VersionedAgentSession>, AgentSessionRepositoryError>;

    async fn save(
        &self,
        session: VersionedAgentSession,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError>;

    async fn remove(
        &self,
        session: VersionedAgentSession,
        authorization: AgentSessionRemovalAuthorization,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionRepositoryError>;
}
