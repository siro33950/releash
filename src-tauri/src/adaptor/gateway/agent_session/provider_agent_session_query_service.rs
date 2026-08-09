use std::sync::Arc;

use crate::domain::agent_session::aggregates::{
    AgentSessionLifecycle, AgentSessionOperations, AgentSessionOrigin,
};
use crate::domain::local_event::{
    LocalEventQuery, LocalEventQueryResult, LocalEventTransactionRepository,
    ProviderAgentSessionLifecycleRecord, ProviderAgentSessionOriginKind,
    ProviderAgentSessionOriginRecord, ProviderAgentSessionProviderRecord, SessionProjectionRecord,
};
use crate::usecase::agent_session::{
    ProviderAgentSessionActivityDto, ProviderAgentSessionItemDto, ProviderAgentSessionLifecycleDto,
    ProviderAgentSessionListPageDto, ProviderAgentSessionListRequest,
    ProviderAgentSessionOperationsDto, ProviderAgentSessionOriginDto,
    ProviderAgentSessionOriginFilter, ProviderAgentSessionProviderDto,
    ProviderAgentSessionQueryError, ProviderAgentSessionQueryService,
};

pub(crate) struct LocalProviderAgentSessionQueryService {
    repository: Arc<dyn LocalEventTransactionRepository>,
}

impl LocalProviderAgentSessionQueryService {
    pub(crate) fn new(repository: Arc<dyn LocalEventTransactionRepository>) -> Self {
        Self { repository }
    }

    pub(crate) fn get_blocking(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<ProviderAgentSessionItemDto>, ProviderAgentSessionQueryError> {
        if agent_session_id.trim().is_empty() {
            return Err(ProviderAgentSessionQueryError::InvalidRequest);
        }
        let result = self
            .repository
            .query_blocking(provider_agent_session_query(agent_session_id))
            .map_err(map_query_error)?;
        provider_agent_session_from_query(result)
    }
}

#[async_trait::async_trait]
impl ProviderAgentSessionQueryService for LocalProviderAgentSessionQueryService {
    async fn get(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<ProviderAgentSessionItemDto>, ProviderAgentSessionQueryError> {
        if agent_session_id.trim().is_empty() {
            return Err(ProviderAgentSessionQueryError::InvalidRequest);
        }
        let result = self
            .repository
            .query(provider_agent_session_query(agent_session_id))
            .await
            .map_err(map_query_error)?;
        provider_agent_session_from_query(result)
    }

    async fn list(
        &self,
        request: ProviderAgentSessionListRequest,
    ) -> Result<ProviderAgentSessionListPageDto, ProviderAgentSessionQueryError> {
        let result = self
            .repository
            .query(LocalEventQuery::ProviderAgentSessionProjectionPage {
                workspace_identity: request.workspace.as_str().to_string(),
                lifecycle: request.lifecycle.map(lifecycle_record),
                origin: request.origin.map(|origin| match origin {
                    ProviderAgentSessionOriginFilter::Standalone => {
                        ProviderAgentSessionOriginKind::Standalone
                    }
                    ProviderAgentSessionOriginFilter::WorkflowNode => {
                        ProviderAgentSessionOriginKind::WorkflowNode
                    }
                }),
                limit: request.limit,
                after_agent_session_id: request.after_session_id,
            })
            .await
            .map_err(map_query_error)?;
        let LocalEventQueryResult::ProviderAgentSessionProjectionPage(page) = result else {
            return Err(ProviderAgentSessionQueryError::Corrupt);
        };
        let items = page
            .sessions
            .into_iter()
            .map(|view| provider_agent_session_item(view.projection))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ProviderAgentSessionListPageDto {
            items,
            next_after_session_id: page.next_after_agent_session_id,
        })
    }
}

fn provider_agent_session_query(agent_session_id: &str) -> LocalEventQuery {
    LocalEventQuery::SessionProjectionByIdentity {
        session_id: format!("provider-agent-session:{agent_session_id}"),
    }
}

fn provider_agent_session_from_query(
    result: LocalEventQueryResult,
) -> Result<Option<ProviderAgentSessionItemDto>, ProviderAgentSessionQueryError> {
    let LocalEventQueryResult::SessionProjectionByIdentity(view) = result else {
        return Err(ProviderAgentSessionQueryError::Corrupt);
    };
    view.map(|view| provider_agent_session_item(view.projection))
        .transpose()
}

fn provider_agent_session_item(
    projection: SessionProjectionRecord,
) -> Result<ProviderAgentSessionItemDto, ProviderAgentSessionQueryError> {
    let SessionProjectionRecord::ProviderAgentSession(session) = projection else {
        return Err(ProviderAgentSessionQueryError::Corrupt);
    };
    let origin = domain_origin(&session.origin)?;
    let lifecycle = domain_lifecycle(session.lifecycle);
    let available_operations = AgentSessionOperations::for_state(&origin, lifecycle);
    let operations = ProviderAgentSessionOperationsDto {
        can_archive: available_operations.can_archive,
        can_restore: available_operations.can_restore,
        can_delete: available_operations.can_delete,
    };
    Ok(ProviderAgentSessionItemDto {
        id: session.id,
        workspace_identity: session.workspace_identity,
        worktree_path: session.worktree_path,
        provider: match session.provider {
            ProviderAgentSessionProviderRecord::Claude => ProviderAgentSessionProviderDto::Claude,
            ProviderAgentSessionProviderRecord::Codex => ProviderAgentSessionProviderDto::Codex,
        },
        origin: match session.origin {
            ProviderAgentSessionOriginRecord::Standalone => {
                ProviderAgentSessionOriginDto::Standalone
            }
            ProviderAgentSessionOriginRecord::WorkflowNode {
                workflow_execution_id,
                node_execution_id,
            } => ProviderAgentSessionOriginDto::WorkflowNode {
                workflow_execution_id,
                node_execution_id,
            },
        },
        lifecycle: match lifecycle {
            AgentSessionLifecycle::Open => ProviderAgentSessionLifecycleDto::Open,
            AgentSessionLifecycle::Paused => ProviderAgentSessionLifecycleDto::Paused,
            AgentSessionLifecycle::Archived => ProviderAgentSessionLifecycleDto::Archived,
        },
        provider_session_id: session.provider_session_id,
        transcript_ref: session.transcript_ref,
        operations,
        activity: ProviderAgentSessionActivityDto::Idle,
        last_exit_abnormal: session.last_exit_abnormal,
    })
}

fn domain_origin(
    origin: &ProviderAgentSessionOriginRecord,
) -> Result<AgentSessionOrigin, ProviderAgentSessionQueryError> {
    match origin {
        ProviderAgentSessionOriginRecord::Standalone => Ok(AgentSessionOrigin::Standalone),
        ProviderAgentSessionOriginRecord::WorkflowNode {
            workflow_execution_id,
            node_execution_id,
        } => AgentSessionOrigin::workflow_node(workflow_execution_id, node_execution_id)
            .map_err(|_| ProviderAgentSessionQueryError::Corrupt),
    }
}

fn domain_lifecycle(lifecycle: ProviderAgentSessionLifecycleRecord) -> AgentSessionLifecycle {
    match lifecycle {
        ProviderAgentSessionLifecycleRecord::Open => AgentSessionLifecycle::Open,
        ProviderAgentSessionLifecycleRecord::Paused => AgentSessionLifecycle::Paused,
        ProviderAgentSessionLifecycleRecord::Archived => AgentSessionLifecycle::Archived,
    }
}

fn map_query_error(
    error: crate::domain::local_event::LocalEventQueryError,
) -> ProviderAgentSessionQueryError {
    match error {
        crate::domain::local_event::LocalEventQueryError::InvalidRequest => {
            ProviderAgentSessionQueryError::InvalidRequest
        }
        crate::domain::local_event::LocalEventQueryError::StorageUnavailable { .. }
        | crate::domain::local_event::LocalEventQueryError::QueryBusy
        | crate::domain::local_event::LocalEventQueryError::DeadlineExceeded => {
            ProviderAgentSessionQueryError::Unavailable
        }
        _ => ProviderAgentSessionQueryError::Corrupt,
    }
}

fn lifecycle_record(
    lifecycle: ProviderAgentSessionLifecycleDto,
) -> ProviderAgentSessionLifecycleRecord {
    match lifecycle {
        ProviderAgentSessionLifecycleDto::Open => ProviderAgentSessionLifecycleRecord::Open,
        ProviderAgentSessionLifecycleDto::Paused => ProviderAgentSessionLifecycleRecord::Paused,
        ProviderAgentSessionLifecycleDto::Archived => ProviderAgentSessionLifecycleRecord::Archived,
    }
}
