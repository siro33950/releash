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

impl SessionContextReadError {
    fn is_corrupt(&self) -> bool {
        matches!(
            self,
            Self::Corrupt(_)
                | Self::Read(
                    LocalEventQueryError::Corrupt { .. }
                        | LocalEventQueryError::IncompatibleStoredEvent { .. }
                )
        )
    }
}

impl std::fmt::Display for SessionContextReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(error) => write!(f, "session root read failed: {error}"),
            Self::Corrupt(reason) => f.write_str(reason),
        }
    }
}

impl From<SessionContextReadError> for AgentSessionRepositoryError {
    fn from(error: SessionContextReadError) -> Self {
        if error.is_corrupt() {
            Self::Corrupt
        } else {
            Self::Unavailable
        }
    }
}

impl From<SessionContextReadError> for AgentSessionQueryError {
    fn from(error: SessionContextReadError) -> Self {
        if error.is_corrupt() {
            Self::Corrupt
        } else {
            Self::Unavailable
        }
    }
}

pub(crate) fn read_session_context(
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
        .map_err(SessionContextReadError::Read)?
        .ok_or_else(|| SessionContextReadError::Corrupt("session tree root is missing".into()))?;
    let header =
        crate::adaptor::gateway::workflow::stored_definition::read_tree_header(&row.detail)
            .map_err(SessionContextReadError::Corrupt)?
            .ok_or_else(|| {
                SessionContextReadError::Corrupt("session tree root metadata is missing".into())
            })?;
    let value: serde_json::Value = serde_json::from_str(&row.detail)
        .map_err(|error| SessionContextReadError::Corrupt(error.to_string()))?;
    let provider = value
        .get("root")
        .and_then(|root| root.get("definition"))
        .and_then(|definition| definition.get("nodes"))
        .and_then(|nodes| nodes.get(&location.node_name))
        .and_then(|node| node.get("session"))
        .and_then(|session| session.get("provider"))
        .ok_or_else(|| {
            SessionContextReadError::Corrupt("session provider is unavailable".into())
        })?;
    Ok(SessionExecutionContext {
        workspace_identity: header.workspace_identity,
        worktree_path: header.worktree_path,
        launched_as: header.launched_as,
        provider: match provider.as_str() {
            Some("claude") => crate::domain::provider_lifecycle::ProviderKind::Claude,
            Some("codex") => crate::domain::provider_lifecycle::ProviderKind::Codex,
            _ => {
                return Err(SessionContextReadError::Corrupt(
                    "session provider is unsupported".into(),
                ))
            }
        },
    })
}

pub(crate) fn read_session_records(
    backend: &FactLogReadBackend,
    location: &SessionLocation,
) -> Result<Vec<NodeFactRecord>, String> {
    let tree_id = location.tree_id.clone();
    let node_id = location.node_execution_id.clone();
    let rows = backend
        .run_indexed(move |connection| {
            crate::adaptor::gateway::local_event_store::node_events::read_node(
                connection, &tree_id, &node_id,
            )
            .map_err(|_| crate::domain::local_event::LocalEventQueryError::InvalidRequest)
        })
        .map_err(|error| format!("session facts read failed: {error:?}"))?;
    rows.iter()
        .filter(|row| row.event_type != "started")
        .map(fact_log::record_from_row)
        .collect()
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

pub(crate) fn locate_session(
    backend: &FactLogReadBackend,
    session_id: &str,
) -> Result<Option<SessionLocation>, String> {
    let Some(record) = fact_log::find_session_attachment_record(backend, session_id)? else {
        return Ok(None);
    };
    Ok(Some(SessionLocation::from_meta(&record.meta)))
}

#[cfg(test)]
#[path = "session_facts_test.rs"]
mod session_facts_tests;
