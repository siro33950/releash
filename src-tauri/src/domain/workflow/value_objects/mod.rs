mod contract;
mod definition;
pub(crate) use definition::{InputsMap, InputsMapSeed};
mod execution;
mod execution_metadata;
mod facet;
mod failure;
mod field_path;
mod ids;
mod node_execution;
mod node_fact;
mod predicate;
mod runtime_event;
mod runtime_projection;
mod state;
mod worktree_origin;

pub use contract::{ContractType, ContractValidationResult, ContractViolation};
pub use definition::{
    is_reserved_node_name, ChildEntry, CommandSpec, CompletionRequirement, EffectiveRules,
    EnvironmentVariableName, EnvironmentVariableNameError, FacetRefs, FanoutSpec, InputParam,
    InputParameterRef, InputSourceRef, ItemsSource, NodeCompletion, NodeDefinition, NodeKind,
    NodeKindName, NodeNamespace, NodeNamespaceError, Rule, SchemaDef, SequenceSpec,
    SessionDelegate, SessionPermission, SessionSpec, WorkflowDefinition, WorkflowSourceFormat,
    WorkflowSummary, WorktreeMode, MAIN_ENTRY_NODE_NAME, MAX_FANOUT_CHILDREN,
    MAX_NODES_PER_WORKFLOW,
};
pub use execution::{
    ApprovalTarget, Artifact, ExecutionOrigin, ExecutionStatus, ExecutionTree, Fanout,
};
pub use execution_metadata::{
    ExecutionStatusFilter, WorkflowExecutionSummary, WorkflowPageRequest,
};
pub use facet::{FacetContents, FacetKey, FacetKind, FacetSummary, WorkflowFacetContents};
pub use failure::{
    FailureClassification, FailureDisposition, NodeExecutionFailureKind, TimeoutKind,
};
pub use field_path::FieldPath;
pub use ids::{
    ExecutionTreeId, NodeDefinitionName, WorkflowDefinitionName, WorkflowExecutionId,
    WorkspaceWorktreePath,
};
pub use node_execution::{
    startup_restart_delay, ExecutionParentRef, FanoutSlot, NodeCompletionSignal,
    NodeCompletionSignalState, NodeExecution, NodeExecutionStatus, NodeProcessPresence,
};
pub use node_fact::{
    AbortRequestedFact, AgentActivityObservedFact, AgentSessionActivity, ApprovalGrantedFact,
    ArchiveRequestedFact, ArtifactProducedFact, CommandSpawnedFact, ExecutionTreeLaunch, NodeFact,
    NodeFactMeta, NodeFactRecord, ProcessExitedFact, ProviderSessionTitleObservedFact,
    RuntimeFailureObservedFact, SessionAttachedFact, SessionContinuationAdmittedFact,
    SessionExecutionTreeRootFacts, SessionNodeRenamedFact, StartedFact, StopReceivedFact,
    SubmitReceivedFact, SubmitRejectedFact, TreeRootFact,
};
pub use predicate::{Predicate, PredicateError};
pub use runtime_event::{ContractViolationRecord, WorkflowEvent};
#[cfg(test)]
pub use runtime_projection::FanoutChildSnapshot;
pub use runtime_projection::{
    NodeHistoryEntry, RuntimeArtifact, TokenUsage, NODE_STATUS_ABORTED, NODE_STATUS_COMPLETED,
};
pub use state::RuntimeExecutionState;
pub use worktree_origin::{
    isolated_worktree_owner, IsolatedWorktree, WorktreeInheritance, WorktreeInventoryEntry,
};

#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
pub use state::WorkflowRuntimeSnapshot;
