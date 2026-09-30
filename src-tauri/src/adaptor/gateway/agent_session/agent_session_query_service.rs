use std::sync::Arc;

use super::session_facts::{
    locate_session, read_session_context, read_session_records, SessionLocation,
};
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::adaptor::gateway::workflow::fact_log::FactLogReadBackend;
use crate::domain::agent_session::aggregates::{
    derive_agent_session_operations, AgentSessionLifecycle, AgentSessionOperations,
};
use crate::domain::agent_session::services::{derive_session_fields, SessionExecutionContext};
use crate::domain::workflow::NodeFactRecord;
use crate::usecase::agent_session::{
    AgentSessionItemDto, AgentSessionLifecycleDto, AgentSessionOperationsDto,
    AgentSessionQueryError, AgentSessionQueryService, AgentSessionTreeLocationDto,
};

/// 統一 Node 事実ログから session を読む query service。
///
/// 一覧は Session の起動として作られた実行木を対象にする。workflow の実行として
/// 作られた木の session は実行木の view（workspace_tree）で観測する。
pub(crate) struct LocalAgentSessionQueryService {
    backend: FactLogReadBackend,
}

impl LocalAgentSessionQueryService {
    pub(crate) fn new(store: Arc<LocalEventStore>) -> Self {
        Self {
            backend: FactLogReadBackend::Live(store),
        }
    }

    pub(crate) fn new_read_only(store: Arc<LocalEventReadStore>) -> Self {
        Self {
            backend: FactLogReadBackend::ReadOnly(store),
        }
    }

    async fn get_derived_from(
        backend: &FactLogReadBackend,
        agent_session_id: &str,
    ) -> Result<Option<AgentSessionItemDto>, AgentSessionQueryError> {
        if agent_session_id.trim().is_empty() {
            return Err(AgentSessionQueryError::InvalidRequest);
        }
        let Some(location) = locate_session(backend, agent_session_id)
            .await
            .map_err(AgentSessionQueryError::from)?
        else {
            return Ok(None);
        };
        let context = read_session_context(backend, &location)
            .await
            .map_err(AgentSessionQueryError::from)?;
        let records = read_session_records(backend, &location)
            .await
            .map_err(AgentSessionQueryError::from)?;
        Ok(Some(agent_session_item_from_facts(
            agent_session_id,
            &location,
            &context,
            &records,
        )?))
    }
}

#[async_trait::async_trait]
impl AgentSessionQueryService for LocalAgentSessionQueryService {
    async fn get(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<AgentSessionItemDto>, AgentSessionQueryError> {
        Self::get_derived_from(&self.backend, agent_session_id).await
    }
}

fn agent_session_item_from_facts(
    session_id: &str,
    location: &SessionLocation,
    context: &SessionExecutionContext,
    records: &[NodeFactRecord],
) -> Result<AgentSessionItemDto, AgentSessionQueryError> {
    let derived = derive_session_fields(
        records,
        context,
        &location.tree_id,
        &location.node_execution_id,
        session_id,
    )
    .map_err(|_| AgentSessionQueryError::Corrupt)?;
    let view = derived.session_facts;
    let operations: AgentSessionOperations = derive_agent_session_operations(
        derived.tree_location.launched_as(),
        derived.lifecycle == AgentSessionLifecycle::Archived,
    );
    Ok(AgentSessionItemDto {
        id: session_id.to_string(),
        workspace_identity: derived.workspace_identity,
        worktree_path: derived.worktree_path,
        workspace_worktree_path: context.workspace_worktree_path.clone(),
        provider: derived.provider.into(),
        tree_location: AgentSessionTreeLocationDto {
            tree_id: derived.tree_location.tree_id().to_string(),
            node_execution_id: derived.tree_location.node_execution_id().to_string(),
        },
        lifecycle: match derived.lifecycle {
            AgentSessionLifecycle::Open => AgentSessionLifecycleDto::Open,
            AgentSessionLifecycle::Paused => AgentSessionLifecycleDto::Paused,
            AgentSessionLifecycle::Archived => AgentSessionLifecycleDto::Archived,
        },
        provider_session_id: view.provider_session_id,
        transcript_ref: view.transcript_ref,
        operations: AgentSessionOperationsDto {
            can_archive: operations.can_archive,
            can_restore: operations.can_restore,
            can_delete: operations.can_delete,
        },
        last_exit_abnormal: view.last_exit_abnormal,
    })
}
