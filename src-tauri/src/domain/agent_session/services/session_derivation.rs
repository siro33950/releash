use crate::domain::agent_session::aggregates::{
    AgentSessionLifecycle, AgentSessionTreeLocation, AgentSessionTreeLocationError,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::services::fact_replay::{derive_session_facts, SessionFactsView};
use crate::domain::workflow::{NodeFact, NodeFactRecord, NodeKind};

#[cfg(test)]
#[path = "session_derivation_test.rs"]
mod session_derivation_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionDerivationError {
    MissingTreeRoot,
    SessionTreeRootIdentityMismatch,
    SessionProviderMissing,
    InvalidTreeLocation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DerivedAgentSessionFields {
    pub(crate) tree_location: AgentSessionTreeLocation,
    pub(crate) provider: ProviderKind,
    pub(crate) workspace_identity: String,
    pub(crate) worktree_path: String,
    pub(crate) lifecycle: AgentSessionLifecycle,
    pub(crate) session_facts: SessionFactsView,
}

pub(crate) fn derive_agent_session_fields(
    records: &[NodeFactRecord],
    tree_id: &str,
    node_execution_id: &str,
    node_name: &str,
    session_id: &str,
) -> Result<DerivedAgentSessionFields, AgentSessionDerivationError> {
    let root = records
        .first()
        .and_then(|record| match &record.fact {
            NodeFact::Started(started) => started.root.as_ref(),
            _ => None,
        })
        .ok_or(AgentSessionDerivationError::MissingTreeRoot)?;
    let tree_location = AgentSessionTreeLocation::for_agent_session(
        tree_id,
        node_execution_id,
        root.launched_as,
        session_id,
    )
    .map_err(|error| match error {
        AgentSessionTreeLocationError::SessionTreeRootIdentityMismatch => {
            AgentSessionDerivationError::SessionTreeRootIdentityMismatch
        }
        AgentSessionTreeLocationError::EmptyTreeId
        | AgentSessionTreeLocationError::EmptyNodeExecutionId => {
            AgentSessionDerivationError::InvalidTreeLocation
        }
    })?;
    let provider = root
        .definition
        .node_by_name(node_name)
        .and_then(|node| match &node.kind {
            NodeKind::Session(spec) => Some(spec.provider),
            _ => None,
        })
        .ok_or(AgentSessionDerivationError::SessionProviderMissing)?;
    let session_facts = derive_session_facts(records, node_execution_id, session_id);
    let lifecycle = if session_facts.archived {
        AgentSessionLifecycle::Archived
    } else if session_facts.exited {
        AgentSessionLifecycle::Paused
    } else {
        AgentSessionLifecycle::Open
    };

    Ok(DerivedAgentSessionFields {
        tree_location,
        provider,
        workspace_identity: root.workspace_identity.clone(),
        worktree_path: root.worktree_path.clone(),
        lifecycle,
        session_facts,
    })
}
