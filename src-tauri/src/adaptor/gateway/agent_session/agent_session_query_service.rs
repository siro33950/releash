use std::sync::Arc;

use crate::domain::agent_session::aggregates::{
    AgentSessionLifecycle, AgentSessionOperations, AgentSessionOrigin,
};
use crate::domain::local_event::{
    AgentSessionLifecycleRecord, AgentSessionOriginKind, AgentSessionOriginRecord,
    AgentSessionProviderRecord, LocalEventQuery, LocalEventQueryResult,
    LocalEventTransactionRepository, SessionProjectionRecord,
};
use crate::usecase::agent_session::{
    AgentSessionActivityDto, AgentSessionItemDto, AgentSessionLifecycleDto,
    AgentSessionListPageDto, AgentSessionListRequest, AgentSessionOperationsDto,
    AgentSessionOriginDto, AgentSessionOriginFilter, AgentSessionProviderDto,
    AgentSessionQueryError, AgentSessionQueryService,
};

pub(crate) struct LocalAgentSessionQueryService {
    repository: Arc<dyn LocalEventTransactionRepository>,
}

impl LocalAgentSessionQueryService {
    pub(crate) fn new(repository: Arc<dyn LocalEventTransactionRepository>) -> Self {
        Self { repository }
    }

    pub(crate) fn get_blocking(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<AgentSessionItemDto>, AgentSessionQueryError> {
        if agent_session_id.trim().is_empty() {
            return Err(AgentSessionQueryError::InvalidRequest);
        }
        let result = self
            .repository
            .query_blocking(agent_session_query(agent_session_id))
            .map_err(map_query_error)?;
        agent_session_from_query(result)
    }
}

#[async_trait::async_trait]
impl AgentSessionQueryService for LocalAgentSessionQueryService {
    async fn get(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<AgentSessionItemDto>, AgentSessionQueryError> {
        if agent_session_id.trim().is_empty() {
            return Err(AgentSessionQueryError::InvalidRequest);
        }
        let result = self
            .repository
            .query(agent_session_query(agent_session_id))
            .await
            .map_err(map_query_error)?;
        agent_session_from_query(result)
    }

    async fn list(
        &self,
        request: AgentSessionListRequest,
    ) -> Result<AgentSessionListPageDto, AgentSessionQueryError> {
        let result = self
            .repository
            .query(LocalEventQuery::AgentSessionProjectionPage {
                workspace_identity: request.workspace.as_str().to_string(),
                lifecycle: request.lifecycle.map(lifecycle_record),
                origin: request.origin.map(|origin| match origin {
                    AgentSessionOriginFilter::Standalone => AgentSessionOriginKind::Standalone,
                    AgentSessionOriginFilter::WorkflowNode => AgentSessionOriginKind::WorkflowNode,
                }),
                limit: request.limit,
                after_agent_session_id: request.after_session_id,
            })
            .await
            .map_err(map_query_error)?;
        let LocalEventQueryResult::AgentSessionProjectionPage(page) = result else {
            return Err(AgentSessionQueryError::Corrupt);
        };
        let items = page
            .sessions
            .into_iter()
            .map(|view| agent_session_item(view.projection))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(AgentSessionListPageDto {
            items,
            next_after_session_id: page.next_after_agent_session_id,
        })
    }
}

fn agent_session_query(agent_session_id: &str) -> LocalEventQuery {
    LocalEventQuery::SessionProjectionByIdentity {
        session_id: format!("agent-session:{agent_session_id}"),
    }
}

fn agent_session_from_query(
    result: LocalEventQueryResult,
) -> Result<Option<AgentSessionItemDto>, AgentSessionQueryError> {
    let LocalEventQueryResult::SessionProjectionByIdentity(view) = result else {
        return Err(AgentSessionQueryError::Corrupt);
    };
    view.map(|view| agent_session_item(view.projection))
        .transpose()
}

fn agent_session_item(
    projection: SessionProjectionRecord,
) -> Result<AgentSessionItemDto, AgentSessionQueryError> {
    let SessionProjectionRecord::AgentSession(session) = projection else {
        return Err(AgentSessionQueryError::Corrupt);
    };
    let origin = domain_origin(&session.origin)?;
    let lifecycle = domain_lifecycle(session.lifecycle);
    let available_operations = AgentSessionOperations::for_state(&origin, lifecycle);
    let operations = AgentSessionOperationsDto {
        can_archive: available_operations.can_archive,
        can_restore: available_operations.can_restore,
        can_delete: available_operations.can_delete,
    };
    Ok(AgentSessionItemDto {
        id: session.id,
        workspace_identity: session.workspace_identity,
        worktree_path: session.worktree_path,
        provider: match session.provider {
            AgentSessionProviderRecord::Claude => AgentSessionProviderDto::Claude,
            AgentSessionProviderRecord::Codex => AgentSessionProviderDto::Codex,
        },
        origin: match session.origin {
            AgentSessionOriginRecord::Standalone => AgentSessionOriginDto::Standalone,
            AgentSessionOriginRecord::WorkflowNode {
                workflow_execution_id,
                node_execution_id,
            } => AgentSessionOriginDto::WorkflowNode {
                workflow_execution_id,
                node_execution_id,
            },
        },
        lifecycle: match lifecycle {
            AgentSessionLifecycle::Open => AgentSessionLifecycleDto::Open,
            AgentSessionLifecycle::Paused => AgentSessionLifecycleDto::Paused,
            AgentSessionLifecycle::Archived => AgentSessionLifecycleDto::Archived,
        },
        provider_session_id: session.provider_session_id,
        transcript_ref: session.transcript_ref,
        operations,
        activity: AgentSessionActivityDto::Idle,
        last_exit_abnormal: session.last_exit_abnormal,
    })
}

fn domain_origin(
    origin: &AgentSessionOriginRecord,
) -> Result<AgentSessionOrigin, AgentSessionQueryError> {
    match origin {
        AgentSessionOriginRecord::Standalone => Ok(AgentSessionOrigin::Standalone),
        AgentSessionOriginRecord::WorkflowNode {
            workflow_execution_id,
            node_execution_id,
        } => AgentSessionOrigin::workflow_node(workflow_execution_id, node_execution_id)
            .map_err(|_| AgentSessionQueryError::Corrupt),
    }
}

fn domain_lifecycle(lifecycle: AgentSessionLifecycleRecord) -> AgentSessionLifecycle {
    match lifecycle {
        AgentSessionLifecycleRecord::Open => AgentSessionLifecycle::Open,
        AgentSessionLifecycleRecord::Paused => AgentSessionLifecycle::Paused,
        AgentSessionLifecycleRecord::Archived => AgentSessionLifecycle::Archived,
    }
}

fn map_query_error(
    error: crate::domain::local_event::LocalEventQueryError,
) -> AgentSessionQueryError {
    match error {
        crate::domain::local_event::LocalEventQueryError::InvalidRequest => {
            AgentSessionQueryError::InvalidRequest
        }
        crate::domain::local_event::LocalEventQueryError::StorageUnavailable { .. }
        | crate::domain::local_event::LocalEventQueryError::QueryBusy
        | crate::domain::local_event::LocalEventQueryError::DeadlineExceeded => {
            AgentSessionQueryError::Unavailable
        }
        _ => AgentSessionQueryError::Corrupt,
    }
}

fn lifecycle_record(lifecycle: AgentSessionLifecycleDto) -> AgentSessionLifecycleRecord {
    match lifecycle {
        AgentSessionLifecycleDto::Open => AgentSessionLifecycleRecord::Open,
        AgentSessionLifecycleDto::Paused => AgentSessionLifecycleRecord::Paused,
        AgentSessionLifecycleDto::Archived => AgentSessionLifecycleRecord::Archived,
    }
}
