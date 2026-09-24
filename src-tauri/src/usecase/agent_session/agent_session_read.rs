use std::sync::Arc;

use super::{
    AgentSessionGarbageCollectionOutcome, AgentSessionItemDto, AgentSessionLifecycleUsecase,
    AgentSessionLifecycleUsecaseError, AgentSessionQueryError, AgentSessionQueryService,
};

#[async_trait::async_trait]
pub(crate) trait AgentSessionGarbageCollectionPort: Send + Sync {
    async fn reconcile_garbage_collection(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<AgentSessionGarbageCollectionOutcome, AgentSessionLifecycleUsecaseError>;
}

#[async_trait::async_trait]
impl AgentSessionGarbageCollectionPort for AgentSessionLifecycleUsecase {
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
    Store(crate::domain::failure::FailureKind),
    InvalidRequest,
    StorageUnavailable,
    TerminalUnavailable,
    Corrupt,
}

pub(crate) struct AgentSessionReadUsecase {
    query: Arc<dyn AgentSessionQueryService>,
    garbage_collection: Arc<dyn AgentSessionGarbageCollectionPort>,
}

impl AgentSessionReadUsecase {
    pub(crate) fn new(
        query: Arc<dyn AgentSessionQueryService>,
        garbage_collection: Arc<dyn AgentSessionGarbageCollectionPort>,
    ) -> Self {
        Self {
            query,
            garbage_collection,
        }
    }

    pub(crate) async fn get(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<AgentSessionItemDto>, AgentSessionReadUsecaseError> {
        let Some(item) = self
            .query
            .get(agent_session_id)
            .await
            .map_err(map_query_error)?
        else {
            return Ok(None);
        };
        match self
            .garbage_collection
            .reconcile_garbage_collection(agent_session_id, &gc_request_id())
            .await
        {
            Ok(AgentSessionGarbageCollectionOutcome::Retained) => Ok(Some(item)),
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

fn gc_request_id() -> String {
    format!(
        "agent-session-read-gc-{}",
        crate::other::id::unique_simple_id()
    )
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
        | AgentSessionLifecycleUsecaseError::Conflict(_)) => {
            AgentSessionReadUsecaseError::Lifecycle(error)
        }
        AgentSessionLifecycleUsecaseError::TerminalUnavailable => {
            AgentSessionReadUsecaseError::TerminalUnavailable
        }
        AgentSessionLifecycleUsecaseError::StorageUnavailable => {
            AgentSessionReadUsecaseError::StorageUnavailable
        }
        AgentSessionLifecycleUsecaseError::Store(kind) => AgentSessionReadUsecaseError::Store(kind),
        AgentSessionLifecycleUsecaseError::Corrupt => AgentSessionReadUsecaseError::Corrupt,
        AgentSessionLifecycleUsecaseError::NotFound
        | AgentSessionLifecycleUsecaseError::InvalidOperation
        | AgentSessionLifecycleUsecaseError::LaunchUnavailable => {
            AgentSessionReadUsecaseError::Corrupt
        }
    }
}

impl crate::domain::failure::ClassifiedFailure for AgentSessionReadUsecaseError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind;
        match self {
            Self::Lifecycle(error) => error.failure_kind(),
            Self::Store(kind) => *kind,
            Self::InvalidRequest => FailureKind::InvalidInput,
            Self::StorageUnavailable => FailureKind::Temporary,
            Self::TerminalUnavailable => FailureKind::StateRequired,
            Self::Corrupt => FailureKind::Corrupt,
        }
    }
}
