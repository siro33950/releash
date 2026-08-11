use std::sync::Arc;

use crate::domain::agent_session::ProviderAgentTerminalObservationGateway;
use crate::domain::terminal_surface::{TerminalActivity, TerminalSurfaceOwner};
use crate::domain::workspace_tree::WorkspaceIdentity;

use super::{
    AgentSessionActivityDto, AgentSessionGarbageCollectionOutcome, AgentSessionItemDto,
    AgentSessionLifecycleDto, AgentSessionLifecycleUsecase, AgentSessionLifecycleUsecaseError,
    AgentSessionListPageDto, AgentSessionListRequest, AgentSessionQueryError,
    AgentSessionQueryService,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionReadUsecaseError {
    InvalidRequest,
    StorageUnavailable,
    TerminalUnavailable,
    Corrupt,
}

pub(crate) struct AgentSessionReadUsecase {
    query: Arc<dyn AgentSessionQueryService>,
    garbage_collection: Arc<dyn AgentSessionGarbageCollectionPort>,
    terminal: Arc<dyn ProviderAgentTerminalObservationGateway>,
}

impl AgentSessionReadUsecase {
    pub(crate) fn new(
        query: Arc<dyn AgentSessionQueryService>,
        garbage_collection: Arc<dyn AgentSessionGarbageCollectionPort>,
        terminal: Arc<dyn ProviderAgentTerminalObservationGateway>,
    ) -> Self {
        Self {
            query,
            garbage_collection,
            terminal,
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
            Ok(AgentSessionGarbageCollectionOutcome::Retained) => {
                Ok(Some(self.with_activity(item)))
            }
            Ok(AgentSessionGarbageCollectionOutcome::GarbageCollected)
            | Err(AgentSessionLifecycleUsecaseError::NotFound) => Ok(self
                .query
                .get(agent_session_id)
                .await
                .map_err(map_query_error)?
                .map(|item| self.with_activity(item))),
            Err(error) => Err(map_lifecycle_error(error)),
        }
    }

    pub(crate) async fn list(
        &self,
        request: AgentSessionListRequest,
    ) -> Result<AgentSessionListPageDto, AgentSessionReadUsecaseError> {
        loop {
            let mut page = self
                .query
                .list(request.clone())
                .await
                .map_err(map_query_error)?;
            let mut removed = false;
            for item in &page.items {
                match self
                    .garbage_collection
                    .reconcile_garbage_collection(&item.id, &gc_request_id())
                    .await
                {
                    Ok(AgentSessionGarbageCollectionOutcome::Retained) => {}
                    Ok(AgentSessionGarbageCollectionOutcome::GarbageCollected)
                    | Err(AgentSessionLifecycleUsecaseError::NotFound) => {
                        removed = true;
                    }
                    Err(error) => return Err(map_lifecycle_error(error)),
                }
            }
            if !removed {
                page.items = page
                    .items
                    .into_iter()
                    .map(|item| self.with_activity(item))
                    .collect();
                return Ok(page);
            }
        }
    }

    /// open の session は terminal gateway の activity 分類を参照する。
    /// paused / archived は常に idle。
    fn with_activity(&self, mut item: AgentSessionItemDto) -> AgentSessionItemDto {
        item.activity = match item.lifecycle {
            AgentSessionLifecycleDto::Open => {
                match TerminalSurfaceOwner::session(
                    WorkspaceIdentity::new(item.workspace_identity.clone()),
                    &item.id,
                ) {
                    Ok(owner) => match self.terminal.session_activity(&owner) {
                        TerminalActivity::Running => AgentSessionActivityDto::Running,
                        TerminalActivity::Idle => AgentSessionActivityDto::Idle,
                    },
                    Err(_) => AgentSessionActivityDto::Idle,
                }
            }
            AgentSessionLifecycleDto::Paused | AgentSessionLifecycleDto::Archived => {
                AgentSessionActivityDto::Idle
            }
        };
        item
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
        AgentSessionQueryError::Corrupt => AgentSessionReadUsecaseError::Corrupt,
    }
}

fn map_lifecycle_error(error: AgentSessionLifecycleUsecaseError) -> AgentSessionReadUsecaseError {
    match error {
        AgentSessionLifecycleUsecaseError::TerminalUnavailable => {
            AgentSessionReadUsecaseError::TerminalUnavailable
        }
        AgentSessionLifecycleUsecaseError::StorageUnavailable => {
            AgentSessionReadUsecaseError::StorageUnavailable
        }
        AgentSessionLifecycleUsecaseError::Corrupt => AgentSessionReadUsecaseError::Corrupt,
        AgentSessionLifecycleUsecaseError::NotFound
        | AgentSessionLifecycleUsecaseError::InvalidOperation
        | AgentSessionLifecycleUsecaseError::Conflict
        | AgentSessionLifecycleUsecaseError::LaunchUnavailable => {
            AgentSessionReadUsecaseError::Corrupt
        }
    }
}
