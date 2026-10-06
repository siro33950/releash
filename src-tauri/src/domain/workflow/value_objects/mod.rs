mod contract;
pub(crate) mod definition;
pub(crate) use definition::{InputsMap, InputsMapSeed};
pub(crate) mod execution;
pub(crate) mod execution_metadata;
pub(crate) mod facet;
pub(crate) mod failure;
pub(crate) mod field_path;
pub(crate) mod ids;
pub(crate) mod node_execution;
pub(crate) mod node_fact;
pub(crate) mod predicate;
pub(crate) mod runtime_event;
pub(crate) mod runtime_projection;
pub(crate) mod state;
pub(crate) mod worktree_origin;

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
    ExecutionParentRef, FanoutSlot, NodeCompletionSignal, NodeCompletionSignalState, NodeExecution,
    NodeExecutionStatus, NodeProcessPresence,
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

#[cfg(any(test, feature = "test-support"))]
pub use state::WorkflowRuntimeSnapshot;
