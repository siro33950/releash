use std::sync::Arc;

use crate::domain::agent_session::ProviderAgentTerminalObservationGateway;
use crate::domain::terminal_surface::{TerminalActivity, TerminalSurfaceOwner};
use crate::domain::workspace_tree::WorkspaceIdentity;

use super::{
    ProviderAgentSessionActivityDto, ProviderAgentSessionGarbageCollectionOutcome,
    ProviderAgentSessionItemDto, ProviderAgentSessionLifecycleDto,
    ProviderAgentSessionLifecycleUsecase, ProviderAgentSessionLifecycleUsecaseError,
    ProviderAgentSessionListPageDto, ProviderAgentSessionListRequest,
    ProviderAgentSessionQueryError, ProviderAgentSessionQueryService,
};

#[async_trait::async_trait]
pub(crate) trait ProviderAgentSessionGarbageCollectionPort: Send + Sync {
    async fn reconcile_garbage_collection(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<
        ProviderAgentSessionGarbageCollectionOutcome,
        ProviderAgentSessionLifecycleUsecaseError,
    >;
}

#[async_trait::async_trait]
impl ProviderAgentSessionGarbageCollectionPort for ProviderAgentSessionLifecycleUsecase {
    async fn reconcile_garbage_collection(
        &self,
        agent_session_id: &str,
        caller_request_id: &str,
    ) -> Result<
        ProviderAgentSessionGarbageCollectionOutcome,
        ProviderAgentSessionLifecycleUsecaseError,
    > {
        ProviderAgentSessionLifecycleUsecase::reconcile_garbage_collection(
            self,
            agent_session_id,
            caller_request_id,
        )
        .await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAgentSessionReadUsecaseError {
    InvalidRequest,
    StorageUnavailable,
    TerminalUnavailable,
    Corrupt,
}

pub(crate) struct ProviderAgentSessionReadUsecase {
    query: Arc<dyn ProviderAgentSessionQueryService>,
    garbage_collection: Arc<dyn ProviderAgentSessionGarbageCollectionPort>,
    terminal: Arc<dyn ProviderAgentTerminalObservationGateway>,
}

impl ProviderAgentSessionReadUsecase {
    pub(crate) fn new(
        query: Arc<dyn ProviderAgentSessionQueryService>,
        garbage_collection: Arc<dyn ProviderAgentSessionGarbageCollectionPort>,
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
    ) -> Result<Option<ProviderAgentSessionItemDto>, ProviderAgentSessionReadUsecaseError> {
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
            Ok(ProviderAgentSessionGarbageCollectionOutcome::Retained) => {
                Ok(Some(self.with_activity(item)))
            }
            Ok(ProviderAgentSessionGarbageCollectionOutcome::GarbageCollected)
            | Err(ProviderAgentSessionLifecycleUsecaseError::NotFound) => Ok(self
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
        request: ProviderAgentSessionListRequest,
    ) -> Result<ProviderAgentSessionListPageDto, ProviderAgentSessionReadUsecaseError> {
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
                    Ok(ProviderAgentSessionGarbageCollectionOutcome::Retained) => {}
                    Ok(ProviderAgentSessionGarbageCollectionOutcome::GarbageCollected)
                    | Err(ProviderAgentSessionLifecycleUsecaseError::NotFound) => {
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
    fn with_activity(&self, mut item: ProviderAgentSessionItemDto) -> ProviderAgentSessionItemDto {
        item.activity = match item.lifecycle {
            ProviderAgentSessionLifecycleDto::Open => {
                match TerminalSurfaceOwner::session(
                    WorkspaceIdentity::new(item.workspace_identity.clone()),
                    &item.id,
                ) {
                    Ok(owner) => match self.terminal.session_activity(&owner) {
                        TerminalActivity::Running => ProviderAgentSessionActivityDto::Running,
                        TerminalActivity::Idle => ProviderAgentSessionActivityDto::Idle,
                    },
                    Err(_) => ProviderAgentSessionActivityDto::Idle,
                }
            }
            ProviderAgentSessionLifecycleDto::Paused
            | ProviderAgentSessionLifecycleDto::Archived => ProviderAgentSessionActivityDto::Idle,
        };
        item
    }
}

fn gc_request_id() -> String {
    format!(
        "provider-agent-session-read-gc-{}",
        crate::other::id::unique_simple_id()
    )
}

fn map_query_error(error: ProviderAgentSessionQueryError) -> ProviderAgentSessionReadUsecaseError {
    match error {
        ProviderAgentSessionQueryError::InvalidRequest => {
            ProviderAgentSessionReadUsecaseError::InvalidRequest
        }
        ProviderAgentSessionQueryError::Unavailable => {
            ProviderAgentSessionReadUsecaseError::StorageUnavailable
        }
        ProviderAgentSessionQueryError::Corrupt => ProviderAgentSessionReadUsecaseError::Corrupt,
    }
}

fn map_lifecycle_error(
    error: ProviderAgentSessionLifecycleUsecaseError,
) -> ProviderAgentSessionReadUsecaseError {
    match error {
        ProviderAgentSessionLifecycleUsecaseError::TerminalUnavailable => {
            ProviderAgentSessionReadUsecaseError::TerminalUnavailable
        }
        ProviderAgentSessionLifecycleUsecaseError::StorageUnavailable => {
            ProviderAgentSessionReadUsecaseError::StorageUnavailable
        }
        ProviderAgentSessionLifecycleUsecaseError::Corrupt => {
            ProviderAgentSessionReadUsecaseError::Corrupt
        }
        ProviderAgentSessionLifecycleUsecaseError::NotFound
        | ProviderAgentSessionLifecycleUsecaseError::InvalidOperation
        | ProviderAgentSessionLifecycleUsecaseError::Conflict
        | ProviderAgentSessionLifecycleUsecaseError::LaunchUnavailable => {
            ProviderAgentSessionReadUsecaseError::Corrupt
        }
    }
}
