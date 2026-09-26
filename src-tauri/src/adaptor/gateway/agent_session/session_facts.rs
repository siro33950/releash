use crate::adaptor::gateway::workflow::fact_log::{self, FactLogReadBackend};
use crate::domain::agent_session::repository::AgentSessionRepositoryError;
use crate::domain::agent_session::services::SessionExecutionContext;
use crate::domain::local_event::LocalEventQueryError;
use crate::domain::workflow::NodeFactRecord;
use crate::domain::workflow::{NodeFactMeta, NodeKindName};
use crate::usecase::agent_session::AgentSessionQueryError;

#[derive(Debug)]
pub(crate) enum SessionContextReadError {
    Read(LocalEventQueryError),
    Corrupt(String),
}

impl From<crate::adaptor::gateway::workflow::worktree_context::WorktreeContextReadError>
    for SessionContextReadError
{
    fn from(
        error: crate::adaptor::gateway::workflow::worktree_context::WorktreeContextReadError,
    ) -> Self {
        use crate::adaptor::gateway::workflow::worktree_context::WorktreeContextReadError;
        match error {
            WorktreeContextReadError::Read(error) => Self::Read(error),
            WorktreeContextReadError::Corrupt(reason) => Self::Corrupt(reason),
        }
    }
}

impl std::fmt::Display for SessionContextReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(error) => write!(f, "session context read failed: {error}"),
            Self::Corrupt(reason) => f.write_str(reason),
        }
    }
}

impl From<SessionContextReadError> for AgentSessionRepositoryError {
    fn from(error: SessionContextReadError) -> Self {
        match error {
            SessionContextReadError::Corrupt(_) => Self::Corrupt,
            SessionContextReadError::Read(error) => error.into(),
        }
    }
}

impl From<SessionContextReadError> for AgentSessionQueryError {
    fn from(error: SessionContextReadError) -> Self {
        match error {
            SessionContextReadError::Corrupt(_) => Self::Corrupt,
            SessionContextReadError::Read(error) => error.into(),
        }
    }
}

pub(crate) async fn read_session_context(
    backend: &FactLogReadBackend,
    location: &SessionLocation,
) -> Result<SessionExecutionContext, SessionContextReadError> {
    let tree_id = location.tree_id.clone();
    let row = backend
        .run_indexed(move |connection| {
            crate::adaptor::gateway::local_event_store::node_events::first_row_of_tree(
                connection, &tree_id,
            )
            .map_err(|error| {
                crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error)
            })
        })
        .await
        .map_err(SessionContextReadError::Read)?
        .ok_or_else(|| SessionContextReadError::Corrupt("session tree root is missing".into()))?;
    let root = crate::adaptor::gateway::workflow::stored_definition::read_tree_context(&row.detail)
        .map_err(SessionContextReadError::Corrupt)?
        .ok_or_else(|| {
            SessionContextReadError::Corrupt("session tree root metadata is missing".into())
        })?;
    let provider = root
        .definition
        .get("nodes")
        .and_then(|nodes| nodes.get(&location.node_name))
        .and_then(|node| node.get("session"))
        .and_then(|session| session.get("provider"))
        .ok_or_else(|| {
            SessionContextReadError::Corrupt("session provider is unavailable".into())
        })?;
    let provider = match provider.as_str() {
        Some("claude") => crate::domain::provider_lifecycle::ProviderKind::Claude,
        Some("codex") => crate::domain::provider_lifecycle::ProviderKind::Codex,
        _ => {
            return Err(SessionContextReadError::Corrupt(
                "session provider is unsupported".into(),
            ))
        }
    };
    let root_meta = fact_log::node_meta_from_row(&row).map_err(SessionContextReadError::Corrupt)?;
    let workspace_identity = root.header.workspace_identity.clone();
    let workspace_worktree_path = root.header.worktree_path.clone();
    let launched_as = root.header.launched_as;
    let worktree_path =
        crate::adaptor::gateway::workflow::worktree_context::execution_worktree_path(
            backend,
            location.meta(),
            root_meta,
            root,
        )
        .await
        .map_err(SessionContextReadError::from)?;
    Ok(SessionExecutionContext {
        workspace_identity,
        workspace_worktree_path,
        worktree_path,
        launched_as,
        provider,
    })
}

pub(crate) async fn read_session_records(
    backend: &FactLogReadBackend,
    location: &SessionLocation,
) -> Result<Vec<NodeFactRecord>, fact_log::FactReadError> {
    let tree_id = location.tree_id.clone();
    let node_id = location.node_execution_id.clone();
    let rows = backend
        .run_indexed(move |connection| {
            crate::adaptor::gateway::local_event_store::node_events::read_node(
                connection, &tree_id, &node_id,
            )
            .map_err(|error| {
                crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error)
            })
        })
        .await
        .map_err(fact_log::FactReadError::Query)?;
    let mut records = rows
        .iter()
        .filter(|row| row.event_type != "started")
        .filter_map(|row| fact_log::record_from_row(row).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    if location.parent_id.is_some() {
        records.extend(fact_log::read_tree_archive_records(backend, &location.tree_id).await?);
        records.sort_by_key(|record| record.seq);
    }
    Ok(records)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionLocation {
    pub(crate) tree_id: String,
    pub(crate) node_execution_id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) node_name: String,
    pub(crate) attempt: u32,
}

impl SessionLocation {
    pub(crate) fn from_meta(meta: &NodeFactMeta) -> Self {
        Self {
            tree_id: meta.tree_id.clone(),
            node_execution_id: meta.node_execution_id.clone(),
            parent_id: meta.parent_id.clone(),
            node_name: meta.node_name.clone(),
            attempt: meta.attempt,
        }
    }

    pub(crate) fn meta(&self) -> NodeFactMeta {
        NodeFactMeta {
            tree_id: self.tree_id.clone(),
            node_execution_id: self.node_execution_id.clone(),
            parent_id: self.parent_id.clone(),
            node_name: self.node_name.clone(),
            kind: NodeKindName::Session,
            attempt: self.attempt,
        }
    }
}

pub(crate) async fn locate_session(
    backend: &FactLogReadBackend,
    session_id: &str,
) -> Result<Option<SessionLocation>, fact_log::FactReadError> {
    let Some(record) = fact_log::find_session_attachment_record(backend, session_id).await? else {
        return Ok(None);
    };
    Ok(Some(SessionLocation::from_meta(&record.meta)))
}

#[cfg(test)]
#[path = "session_facts_test.rs"]
mod session_facts_tests;

impl From<fact_log::FactReadError> for AgentSessionRepositoryError {
    fn from(error: fact_log::FactReadError) -> Self {
        match error {
            fact_log::FactReadError::Query(error) => error.into(),
            fact_log::FactReadError::Corrupt(_) => Self::Corrupt,
        }
    }
}

impl From<fact_log::FactReadError> for AgentSessionQueryError {
    fn from(error: fact_log::FactReadError) -> Self {
        Self::Store(error.into())
    }
}
