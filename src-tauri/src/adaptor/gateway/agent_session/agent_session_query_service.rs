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
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::{ExecutionTreeLaunch, NodeFact, NodeFactRecord};
use crate::usecase::agent_session::{
    AgentSessionItemDto, AgentSessionLifecycleDto, AgentSessionOperationsDto,
    AgentSessionProviderDto, AgentSessionQueryError, AgentSessionQueryService,
    AgentSessionTreeLocationDto,
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

    fn get_derived_from(
        backend: &FactLogReadBackend,
        agent_session_id: &str,
    ) -> Result<Option<AgentSessionItemDto>, AgentSessionQueryError> {
        if agent_session_id.trim().is_empty() {
            return Err(AgentSessionQueryError::InvalidRequest);
        }
        let Some(location) = locate_session(backend, agent_session_id)
            .map_err(|_| AgentSessionQueryError::Unavailable)?
        else {
            return Ok(None);
        };
        let context =
            read_session_context(backend, &location).map_err(AgentSessionQueryError::from)?;
        let records = read_session_records(backend, &location)
            .map_err(|_| AgentSessionQueryError::Unavailable)?;
        Ok(Some(agent_session_item_from_facts(
            agent_session_id,
            &location,
            &context,
            &records,
        )?))
    }

    pub(crate) fn get_blocking(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<AgentSessionItemDto>, AgentSessionQueryError> {
        Self::get_derived_from(&self.backend, agent_session_id)
    }
}

#[async_trait::async_trait]
impl AgentSessionQueryService for LocalAgentSessionQueryService {
    async fn get(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<AgentSessionItemDto>, AgentSessionQueryError> {
        let backend = self.backend.clone();
        let agent_session_id = agent_session_id.to_string();
        tokio::task::spawn_blocking(move || Self::get_derived_from(&backend, &agent_session_id))
            .await
            .map_err(|_| AgentSessionQueryError::Unavailable)?
    }
}

pub(crate) fn workspace_session_items(
    backend: &FactLogReadBackend,
    tree_ids: &[String],
    workspace: &str,
) -> Result<Vec<AgentSessionItemDto>, AgentSessionQueryError> {
    let mut items = Vec::new();
    for tree_id in tree_ids {
        let requested = tree_id.clone();
        let root = backend
            .run_indexed(move |connection| {
                crate::adaptor::gateway::local_event_store::node_events::first_row_of_tree(
                    connection, &requested,
                )
                .map_err(|_| crate::domain::local_event::LocalEventQueryError::InvalidRequest)
            })
            .map_err(|_| AgentSessionQueryError::Unavailable)?;
        let Some(root) = root else {
            continue;
        };
        let Some(header) =
            crate::adaptor::gateway::workflow::stored_definition::read_tree_header(&root.detail)
                .map_err(|_| AgentSessionQueryError::Corrupt)?
        else {
            continue;
        };
        if header.launched_as != ExecutionTreeLaunch::Session
            || header.workspace_identity != workspace
        {
            continue;
        }
        let location = SessionLocation {
            tree_id: root.tree_id,
            node_execution_id: root.node_execution_id,
            parent_id: root.parent_id,
            node_name: root.node_name,
            attempt: u32::try_from(root.attempt).map_err(|_| AgentSessionQueryError::Corrupt)?,
        };
        let records = read_session_records(backend, &location)
            .map_err(|_| AgentSessionQueryError::Unavailable)?;
        let Some(session_id) = records.iter().find_map(|record| match &record.fact {
            NodeFact::SessionAttached(attached) => Some(attached.session_id.as_str()),
            _ => None,
        }) else {
            continue;
        };
        let context =
            read_session_context(backend, &location).map_err(AgentSessionQueryError::from)?;
        items.push(agent_session_item_from_facts(
            session_id, &location, &context, &records,
        )?);
    }
    items.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(items)
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
        derived.lifecycle == AgentSessionLifecycle::Paused,
        view.provider_session_id.is_some(),
    );
    Ok(AgentSessionItemDto {
        id: session_id.to_string(),
        workspace_identity: derived.workspace_identity,
        worktree_path: derived.worktree_path,
        provider: match derived.provider {
            ProviderKind::Claude => AgentSessionProviderDto::Claude,
            ProviderKind::Codex => AgentSessionProviderDto::Codex,
        },
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
            can_resume: operations.can_resume,
        },
        last_exit_abnormal: view.last_exit_abnormal,
    })
}
