//! Identity mappings between domain-owned runtime values and usecase commit
//! material.
//!
//! These functions intentionally contain no wire or storage conversion. Wire
//! mapping remains in gateway modules.

use std::collections::{BTreeMap, HashMap};

use crate::domain::workflow::{
    NodeHistoryEntry, NodeKind, RuntimeArtifact, RuntimeExecutionState, SchemaDef, TokenUsage,
    WorkflowDefinition,
};

pub(crate) fn workflow_definition_to_domain(workflow: &WorkflowDefinition) -> WorkflowDefinition {
    workflow.clone()
}

pub(crate) fn workflow_schemas_to_domain(
    schemas: &BTreeMap<String, SchemaDef>,
) -> BTreeMap<String, SchemaDef> {
    schemas.clone()
}

pub(crate) fn runtime_execution_state_to_domain(
    state: &RuntimeExecutionState,
) -> RuntimeExecutionState {
    state.clone()
}

pub(crate) fn artifacts_to_domain(
    artifacts: &HashMap<String, RuntimeArtifact>,
) -> HashMap<String, RuntimeArtifact> {
    artifacts.clone()
}

pub(crate) fn runtime_artifact_from_domain(output: RuntimeArtifact) -> RuntimeArtifact {
    output
}

pub(crate) fn node_history_entries_to_domain(
    entries: &[NodeHistoryEntry],
) -> Vec<NodeHistoryEntry> {
    entries.to_vec()
}

pub(crate) fn node_history_entry_from_domain(entry: NodeHistoryEntry) -> NodeHistoryEntry {
    entry
}

pub(crate) fn token_usage_to_domain(usage: &TokenUsage) -> TokenUsage {
    usage.clone()
}

pub(crate) fn node_kind_to_domain(kind: &NodeKind) -> NodeKind {
    kind.clone()
}
