use crate::domain::agent_session::aggregates::{AgentSessionAvailability, ManagedPtyPresence};
use std::sync::Arc;

use super::{
    AgentSessionGarbageCollectionOutcome, AgentSessionItemDto, AgentSessionLifecycleUsecase,
    AgentSessionLifecycleUsecaseError, AgentSessionQueryError, AgentSessionQueryService,
};

#[async_trait::async_trait]
pub trait AgentSessionGarbageCollectionPort: Send + Sync {
    async fn terminal_availability(
        &self,
        agent_session_id: &str,
    ) -> Result<(ManagedPtyPresence, AgentSessionAvailability), AgentSessionLifecycleUsecaseError>;

    async fn reconcile_garbage_collection(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<AgentSessionGarbageCollectionOutcome, AgentSessionLifecycleUsecaseError>;
}

#[async_trait::async_trait]
impl AgentSessionGarbageCollectionPort for AgentSessionLifecycleUsecase {
    async fn terminal_availability(
        &self,
        id: &str,
    ) -> Result<(ManagedPtyPresence, AgentSessionAvailability), AgentSessionLifecycleUsecaseError>
    {
        self.terminal_availability(id).await
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
pub enum AgentSessionReadUsecaseError {
    Lifecycle(AgentSessionLifecycleUsecaseError),
    Store(crate::domain::failure::StorageFailure),
    InvalidRequest,
    StorageUnavailable,
    Corrupt,
}

pub struct AgentSessionReadUsecase {
    identities: Arc<dyn crate::domain::identity::IdentityIssuer>,
    query: Arc<dyn AgentSessionQueryService>,
    garbage_collection: Arc<dyn AgentSessionGarbageCollectionPort>,
}

impl AgentSessionReadUsecase {
    pub fn new(
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
    ) -> Result<Option<(AgentSessionItemDto, AgentSessionAvailability)>, AgentSessionReadUsecaseError>
    {
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
            Ok(AgentSessionGarbageCollectionOutcome::Retained) => {}
            Ok(AgentSessionGarbageCollectionOutcome::GarbageCollected)
            | Err(AgentSessionLifecycleUsecaseError::NotFound) => {
                let Some(current) = self
                    .query
                    .get(agent_session_id)
                    .await
                    .map_err(map_query_error)?
                else {
                    return Ok(None);
                };
                item = current;
            }
            Err(error) => return Err(map_lifecycle_error(error)),
        }
        let (presence, availability) = match self
            .garbage_collection
            .terminal_availability(agent_session_id)
            .await
        {
            Ok(value) => value,
            Err(AgentSessionLifecycleUsecaseError::NotFound) => return Ok(None),
            Err(error) => return Err(map_lifecycle_error(error)),
        };
        item.terminal_presence = Some(
            match presence {
                ManagedPtyPresence::Live => "live",
                ManagedPtyPresence::ConfirmedAbsent => "absent",
                ManagedPtyPresence::Unknown => "unknown",
            }
            .into(),
        );
        Ok(Some((item, availability)))
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

#[cfg(test)]
#[path = "agent_session_read_test.rs"]
mod agent_session_read_tests;
