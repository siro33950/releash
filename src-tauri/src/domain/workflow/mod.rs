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
pub mod status_aggregation;
pub mod value_objects;

pub use entities::workflow_execution::{
    compute_node_statuses, FanoutChildRuntime, FanoutChildRuntimeState, FanoutRuntimeState,
};
pub use error::WorkflowError;
pub use gateway::{ManagedWorktreeGateway, SecretSourceGateway};
pub use repository::{
    FacetRepository, WorkflowDefinitionRepository, WorkflowExecutionArchiveRepository,
    WorkflowExecutionManualArchiveRecord, WorkflowExecutionRepository,
    WORKFLOW_ARCHIVE_REASON_MANUAL,
};
pub use services::{
    approval_rules, contract, secret_masker, validation, ApprovalInputError, RetryPolicy,
    TimeoutContext, TimeoutPolicy,
};
#[cfg(test)]
pub use value_objects::WorkflowExecutionRecord;
pub use value_objects::{
    ApprovalTarget, Artifact, CommandSpec, ContractType, ContractValidationResult,
    ExecutionListFilter, ExecutionOrigin, ExecutionStatus, ExecutionStatusFilter, FacetKey,
    FacetKind, FacetRefs, FacetSummary, FailureClassification, FailureDisposition, Fanout,
    FanoutChildSnapshot, FanoutParentRef, FanoutSpec, ItemsSource, NodeDefinition, NodeExecution,
    NodeExecutionFailure, NodeExecutionFailureKind, NodeExecutionStatus, NodeHistoryEntry,
    NodeKind, NodeKindName, NodeName, NodeStallObservation, OutcomeCommitMode, Rule,
    RuntimeApprovalOperations, RuntimeArtifact, RuntimeExecutionState, SchemaDef, SessionGate,
    SessionSpec, TimeoutKind, TokenUsage, WorkflowDefinition, WorkflowExecution,
    WorkflowExecutionId, WorkflowExecutionSummary, WorkflowName, WorkflowNodeContext,
    WorkflowPageRequest, WorkflowRuntimeSnapshot, WorkflowSummary, WorktreePath,
    NODE_STATUS_ABORTED, NODE_STATUS_COMPLETED, NODE_STATUS_FAILED, NODE_STATUS_INTERRUPTED,
    NODE_STATUS_RUNNING, NODE_STATUS_WAITING_APPROVAL,
};
