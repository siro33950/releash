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

pub use error::WorkflowError;
pub use gateway::{ManagedWorktreeGateway, SecretSourceGateway, WorktreeInventoryGateway};
pub use repository::{
    FacetRepository, IsolatedWorktreeLedgerRepository, WorkflowDefinitionRepository,
    WorkflowExecutionArchiveRepository, WorkflowExecutionArchiveSnapshot,
    WorkflowExecutionManualArchiveRecord, WORKFLOW_ARCHIVE_REASON_MANUAL,
};
pub use services::{contract, secret_masker, validation};
#[cfg(test)]
pub use services::{TimeoutContext, TimeoutPolicy};
#[cfg(test)]
pub use value_objects::ExecutionListFilter;
#[cfg(test)]
pub use value_objects::WorkflowExecutionRecord;
pub use value_objects::{
    is_reserved_node_name, AgentActivityObservedFact, AgentSessionActivity, ApprovalGrantedFact,
    ApprovalTarget, Artifact, ArtifactProducedFact, ChildEntry, CommandSpawnedFact, CommandSpec,
    ContractType, ContractValidationResult, EnvironmentVariableName, EnvironmentVariableNameError,
    ExecutionInterruptionReason, ExecutionOrigin, ExecutionParentRef, ExecutionStatus,
    ExecutionStatusFilter, ExecutionTreeLaunch, FacetContents, FacetKey, FacetKind, FacetRefs,
    FacetSummary, FailureClassification, FailureDisposition, Fanout, FanoutSlot, FanoutSpec,
    InputParam, InputParameterRef, IsolatedWorktreeIdentity, IsolatedWorktreeLedgerEntry,
    IsolatedWorktreeLedgerSnapshot, IsolatedWorktreeLifecycle, IsolatedWorktreeRecoveryCause,
    ItemsSource, NodeCompletion, NodeCompletionSignal, NodeCompletionSignalState, NodeDefinition,
    NodeDefinitionName, NodeExecution, NodeExecutionFailure, NodeExecutionFailureKind,
    NodeExecutionStatus, NodeFact, NodeFactMeta, NodeFactRecord, NodeHistoryEntry, NodeKind,
    NodeKindName, OnFailure, ProcessExitedFact, ProviderSessionTitleObservedFact,
    RepositoryWorktreeInventory, Rule, RuntimeArtifact, RuntimeExecutionState,
    RuntimeFailureObservedFact, SchemaDef, SequenceSpec, SessionAttachedFact,
    SessionExecutionTreeRootFacts, SessionNodeRenamedFact, SessionPermission, SessionSpec,
    StartedFact, StopReceivedFact, SubmitReceivedFact, SubmitRejectedFact, TimeoutKind, TokenUsage,
    TreeRootFact, WorkflowDefinition, WorkflowDefinitionName, WorkflowEvent, WorkflowExecution,
    WorkflowExecutionId, WorkflowExecutionSummary, WorkflowFacetContents, WorkflowPageRequest,
    WorkflowRuntimeSnapshot, WorkflowSourceFormat, WorkflowSummary, WorkspaceWorktreePath,
    WorktreeInventoryEntry, WorktreeManagementKind, NODE_STATUS_COMPLETED, NODE_STATUS_FAILED,
};
