use std::sync::Arc;

use super::agent_session_repository::{derive_session, locate_session};
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::adaptor::gateway::workflow::fact_log::{self, FactLogReadBackend};
use crate::domain::agent_session::aggregates::{
    AgentSession, AgentSessionLifecycle, AgentSessionOperations,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::{NodeFact, TreeRootFact};
use crate::usecase::agent_session::{
    AgentSessionActivityDto, AgentSessionItemDto, AgentSessionLifecycleDto,
    AgentSessionOperationsDto, AgentSessionProviderDto, AgentSessionQueryError,
    AgentSessionQueryService, AgentSessionTreeParentDto,
};

/// 統一 Node 事実ログから session を読む query service。
///
/// 一覧は「親を持たない session（= session root の実行木）」であり、
/// 出所種別による別系統の一覧 query は存在しない。workflow の子 session は
/// 実行木の view（workspace_tree）で観測する。
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
        let records = fact_log::read_tree_records_from(backend, &location.tree_id)
            .map_err(|_| AgentSessionQueryError::Unavailable)?;
        let session = derive_session(agent_session_id, &location, &records)
            .map_err(|_| AgentSessionQueryError::Corrupt)?;
        Ok(Some(agent_session_item(session.session())))
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
        let Some(root) = fact_log::read_tree_root_from(backend, tree_id)
            .map_err(|_| AgentSessionQueryError::Unavailable)?
        else {
            continue;
        };
        let is_workspace_session = matches!(
            &root.fact,
            NodeFact::Started(started)
                if matches!(
                    &started.root,
                    Some(TreeRootFact::Session(root)) if root.workspace_identity == workspace
                )
        );
        if !is_workspace_session {
            continue;
        }
        let records = fact_log::read_tree_records_from(backend, tree_id)
            .map_err(|_| AgentSessionQueryError::Unavailable)?;
        let Some(first) = records.first() else {
            continue;
        };
        let location = super::agent_session_repository::SessionLocation {
            tree_id: tree_id.clone(),
            node_execution_id: first.meta.node_execution_id.clone(),
            parent_id: first.meta.parent_id.clone(),
            node_name: first.meta.node_name.clone(),
            attempt: first.meta.attempt,
        };
        let session = derive_session(&first.meta.node_execution_id, &location, &records)
            .map_err(|_| AgentSessionQueryError::Corrupt)?;
        items.push(agent_session_item(session.session()));
    }
    items.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(items)
}

pub(crate) fn agent_session_item(session: &AgentSession) -> AgentSessionItemDto {
    let operations = AgentSessionOperations::for_state(session.tree_parent(), session.lifecycle());
    AgentSessionItemDto {
        id: session.id().to_string(),
        workspace_identity: session.workspace().as_str().to_string(),
        worktree_path: session.worktree_path().to_string(),
        provider: match session.provider() {
            ProviderKind::Claude => AgentSessionProviderDto::Claude,
            ProviderKind::Codex => AgentSessionProviderDto::Codex,
        },
        tree_parent: session
            .tree_parent()
            .map(|parent| AgentSessionTreeParentDto {
                tree_id: parent.tree_id.clone(),
                node_execution_id: parent.node_execution_id.clone(),
            }),
        lifecycle: match session.lifecycle() {
            AgentSessionLifecycle::Open => AgentSessionLifecycleDto::Open,
            AgentSessionLifecycle::Paused => AgentSessionLifecycleDto::Paused,
            AgentSessionLifecycle::Archived => AgentSessionLifecycleDto::Archived,
        },
        provider_session_id: session.provider_session_id().map(str::to_string),
        transcript_ref: session.transcript_ref().map(str::to_string),
        operations: AgentSessionOperationsDto {
            can_archive: operations.can_archive,
            can_restore: operations.can_restore,
            can_delete: operations.can_delete,
        },
        activity: AgentSessionActivityDto::Idle,
        last_exit_abnormal: session.last_exit_abnormal(),
    }
}
