use std::sync::Arc;

use super::{
    AgentSessionGarbageCollectionOutcome, AgentSessionItemDto, AgentSessionLifecycleUsecase,
    AgentSessionLifecycleUsecaseError, AgentSessionQueryError, AgentSessionQueryService,
};

#[async_trait::async_trait]
pub(crate) trait AgentSessionGarbageCollectionPort: Send + Sync {
    async fn terminal_presence(
        &self,
        agent_session_id: &str,
    ) -> Result<
        crate::domain::agent_session::aggregates::ManagedPtyPresence,
        AgentSessionLifecycleUsecaseError,
    >;

    async fn reconcile_garbage_collection(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<AgentSessionGarbageCollectionOutcome, AgentSessionLifecycleUsecaseError>;
}

#[async_trait::async_trait]
impl AgentSessionGarbageCollectionPort for AgentSessionLifecycleUsecase {
    async fn terminal_presence(
        &self,
        id: &str,
    ) -> Result<
        crate::domain::agent_session::aggregates::ManagedPtyPresence,
        AgentSessionLifecycleUsecaseError,
    > {
        self.terminal_presence(id).await
    }
    async fn reconcile_garbage_collection(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<AgentSessionGarbageCollectionOutcome, AgentSessionLifecycleUsecaseError> {
        AgentSessionLifecycleUsecase::reconcile_garbage_collection(
            self,
            agent_session_id,
            caller_request_id,
        )
        .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSessionReadUsecaseError {
    Lifecycle(AgentSessionLifecycleUsecaseError),
    Store(crate::domain::failure::StorageFailure),
    InvalidRequest,
    StorageUnavailable,
    Corrupt,
}

pub(crate) struct AgentSessionReadUsecase {
    identities: Arc<dyn crate::domain::identity::IdentityIssuer>,
    query: Arc<dyn AgentSessionQueryService>,
    garbage_collection: Arc<dyn AgentSessionGarbageCollectionPort>,
}

impl AgentSessionReadUsecase {
    pub(crate) fn new(
        identities: Arc<dyn crate::domain::identity::IdentityIssuer>,
        query: Arc<dyn AgentSessionQueryService>,
        garbage_collection: Arc<dyn AgentSessionGarbageCollectionPort>,
    ) -> Self {
        Self {
            identities,
            query,
            garbage_collection,
        }
    }

    pub(crate) async fn get(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<AgentSessionItemDto>, AgentSessionReadUsecaseError> {
        let Some(mut item) = self
            .query
            .get(agent_session_id)
            .await
            .map_err(map_query_error)?
        else {
            return Ok(None);
        };
        match self
            .garbage_collection
            .reconcile_garbage_collection(
                agent_session_id,
                &format!("agent-session-read-gc-{}", self.identities.issue()),
            )
            .await
        {
            Ok(AgentSessionGarbageCollectionOutcome::Retained) => {
                item.terminal_presence = Some(match self.garbage_collection.terminal_presence(agent_session_id).await.map_err(map_lifecycle_error)? {
                    crate::domain::agent_session::aggregates::ManagedPtyPresence::Live => "live",
                    crate::domain::agent_session::aggregates::ManagedPtyPresence::ConfirmedAbsent => "absent",
                    crate::domain::agent_session::aggregates::ManagedPtyPresence::Unknown => "unknown",
                }.into());
                Ok(Some(item))
            }
            Ok(AgentSessionGarbageCollectionOutcome::GarbageCollected)
            | Err(AgentSessionLifecycleUsecaseError::NotFound) => Ok(self
                .query
                .get(agent_session_id)
                .await
                .map_err(map_query_error)?),
            Err(error) => Err(map_lifecycle_error(error)),
        }
    }
}

fn map_query_error(error: AgentSessionQueryError) -> AgentSessionReadUsecaseError {
    match error {
        AgentSessionQueryError::InvalidRequest => AgentSessionReadUsecaseError::InvalidRequest,
        AgentSessionQueryError::Unavailable => AgentSessionReadUsecaseError::StorageUnavailable,
        AgentSessionQueryError::Store(kind) => AgentSessionReadUsecaseError::Store(kind),
        AgentSessionQueryError::Corrupt => AgentSessionReadUsecaseError::Corrupt,
    }
}

fn map_lifecycle_error(error: AgentSessionLifecycleUsecaseError) -> AgentSessionReadUsecaseError {
    match error {
        error @ (AgentSessionLifecycleUsecaseError::Workflow(_)
        | AgentSessionLifecycleUsecaseError::Conflict(_)
        | AgentSessionLifecycleUsecaseError::Launch(_)
        | AgentSessionLifecycleUsecaseError::Terminal(_)) => {
            AgentSessionReadUsecaseError::Lifecycle(error)
        }
        AgentSessionLifecycleUsecaseError::StorageUnavailable => {
            AgentSessionReadUsecaseError::StorageUnavailable
        }
        AgentSessionLifecycleUsecaseError::Store(kind) => AgentSessionReadUsecaseError::Store(kind),
        AgentSessionLifecycleUsecaseError::ProviderUnavailable
        | AgentSessionLifecycleUsecaseError::Corrupt
        | AgentSessionLifecycleUsecaseError::NotFound
        | AgentSessionLifecycleUsecaseError::InvalidOperation => {
            AgentSessionReadUsecaseError::Corrupt
        }
    }
}
