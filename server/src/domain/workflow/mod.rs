//! workflow domain.
//!
//! This module owns workflow meaning: definitions, execution state, approvals,
//! contracts, facets, and execution lifecycle vocabulary. External resources such as
//! Tauri events, git2, file I/O, and agent runtime handles
//! are represented only by traits in `repository` / `gateway`.

pub mod entities;
pub mod error;
pub mod gateway;
#[allow(clippy::module_inception)]
pub mod repository;
pub mod services;
pub mod value_objects;

pub use entities::{Facet, WorkflowDefinition};
pub use error::WorkflowError;
pub use gateway::{
    IsolatedWorktreeGateway, ManagedWorktreeGateway, NodeProcessReader, WorkflowDirectoryService,
};
pub use repository::{
    ExecutionTreeArchiveCandidate, ExecutionTreeArchiveRecord, ExecutionTreeArchiveRepository,
    ExecutionTreeArchiveSnapshot, ExecutionTreeArchiveTarget, FacetRepository,
    WorkflowDefinitionRepository,
};
pub use services::{contract, validation};
pub use value_objects::{
    is_reserved_node_name, isolated_worktree_owner, AbortRequestedFact, AgentActivityObservedFact,
    AgentSessionActivity, ApprovalGrantedFact, ApprovalTarget, ArchiveRequestedFact, Artifact,
    ArtifactProducedFact, ChildEntry, CommandSpawnedFact, CommandSpec, CompletionRequirement,
    ContractType, ContractValidationResult, EnvironmentVariableName, EnvironmentVariableNameError,
    ExecutionOrigin, ExecutionParentRef, ExecutionStatus, ExecutionStatusFilter, ExecutionTree,
    ExecutionTreeId, ExecutionTreeLaunch, FacetContents, FacetKey, FacetKind, FacetRefs,
    FailureClassification, FailureDisposition, Fanout, FanoutSlot, FanoutSpec, FieldPath,
    InputParam, InputParameterRef, IsolatedWorktree, ItemsSource, NodeCompletion,
    NodeCompletionSignal, NodeCompletionSignalState, NodeDefinition, NodeDefinitionName,
    NodeExecution, NodeExecutionFailureKind, NodeExecutionStatus, NodeFact, NodeFactMeta,
    NodeFactRecord, NodeKind, NodeKindName, NodeProcessPresence, Predicate, ProcessExitedFact,
    ProviderSessionTitleObservedFact, Rule, RuntimeExecutionState, RuntimeFailureObservedFact,
    SchemaDef, SequenceSpec, SessionAttachedFact, SessionContinuationAdmittedFact, SessionDelegate,
    SessionExecutionTreeRootFacts, SessionNodeRenamedFact, SessionPermission, SessionSpec,
    StartedFact, StopReceivedFact, SubmitReceivedFact, SubmitRejectedFact, TimeoutKind, TokenUsage,
    TreeRootFact, WorkflowDefinitionContent, WorkflowDefinitionName, WorkflowEvent,
    WorkflowExecutionId, WorkflowExecutionSummary, WorkflowFacetContents, WorkflowSourceFormat,
    WorkspaceWorktreePath, WorktreeInheritance, WorktreeInventoryEntry, WorktreeMode,
    NODE_STATUS_COMPLETED,
};

#[cfg(any(test, feature = "test-support"))]
pub use value_objects::WorkflowRuntimeSnapshot;

#[cfg(any(test, feature = "test-support"))]
pub use value_objects::{NodeHistoryEntry, RuntimeArtifact};

pub use value_objects::{WorkflowDirectory, WorkflowSource};

pub use gateway::{WorkflowDefinitionSourceService, WorkflowSourceSaveError};
