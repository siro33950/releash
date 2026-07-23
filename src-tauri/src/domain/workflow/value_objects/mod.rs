mod contract;
mod definition;
mod execution;
mod execution_metadata;
mod facet;
mod failure;
mod ids;
mod node_execution;
mod runtime_projection;
mod state;
mod workflow_node_context;

pub use contract::{ContractType, ContractValidationResult, ContractViolation};
pub use definition::{
    CommandSpec, FacetRefs, FanoutSpec, ItemsSource, NodeDefinition, NodeKind, NodeKindName, Rule,
    SchemaDef, SessionGate, SessionSpec, WorkflowDefinition, WorkflowSummary, MAX_FANOUT_CHILDREN,
    MAX_NODES_PER_WORKFLOW,
};
pub use execution::{
    ApprovalTarget, Artifact, ExecutionInterruptionReason, ExecutionOrigin, ExecutionStatus,
    Fanout, WorkflowExecution,
};
#[cfg(test)]
pub use execution_metadata::WorkflowExecutionRecord;
pub use execution_metadata::{
    ExecutionListFilter, ExecutionStatusFilter, WorkflowExecutionSummary, WorkflowPageRequest,
};
pub use facet::{FacetKey, FacetKind, FacetSummary};
pub use failure::{
    FailureClassification, FailureDisposition, NodeExecutionFailureKind, TimeoutKind,
};
pub use ids::{
    NodeDefinitionName, WorkflowDefinitionName, WorkflowExecutionId, WorkspaceWorktreePath,
};
pub use node_execution::{
    FanoutParentRef, NodeExecution, NodeExecutionFailure, NodeExecutionStatus,
};
pub use runtime_projection::{
    default_node_history_status, FanoutChildSnapshot, NodeHistoryEntry, RuntimeArtifact,
    TokenUsage, NODE_STATUS_ABORTED, NODE_STATUS_COMPLETED, NODE_STATUS_FAILED,
    NODE_STATUS_INTERRUPTED, NODE_STATUS_RUNNING, NODE_STATUS_WAITING_APPROVAL,
};
pub use state::{RuntimeExecutionState, WorkflowRuntimeSnapshot};
pub use workflow_node_context::WorkflowNodeContext;
