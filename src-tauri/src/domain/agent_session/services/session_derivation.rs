use crate::domain::agent_session::aggregates::{
    AgentSessionLifecycle, AgentSessionTreeLocation, AgentSessionTreeLocationError,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::services::fact_replay::{derive_session_facts, SessionFactsView};
use crate::domain::workflow::NodeFactRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionExecutionContext {
    pub(crate) workspace_identity: String,
    pub(crate) worktree_path: String,
    pub(crate) launched_as: crate::domain::workflow::ExecutionTreeLaunch,
    pub(crate) provider: ProviderKind,
}

#[cfg(test)]
#[path = "session_derivation_test.rs"]
mod session_derivation_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionDerivationError {
    SessionTreeRootIdentityMismatch,
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

pub(crate) fn derive_session_fields(
    records: &[NodeFactRecord],
    context: &SessionExecutionContext,
    tree_id: &str,
    node_execution_id: &str,
    session_id: &str,
) -> Result<DerivedAgentSessionFields, AgentSessionDerivationError> {
    let tree_location = AgentSessionTreeLocation::for_agent_session(
        tree_id,
        node_execution_id,
        context.launched_as,
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
        provider: context.provider,
        workspace_identity: context.workspace_identity.clone(),
        worktree_path: context.worktree_path.clone(),
        lifecycle,
        session_facts,
    })
}
