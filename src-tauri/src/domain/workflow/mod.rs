//! workflow domain.
//!
//! This module owns workflow meaning: definitions, execution state, approvals,
//! contracts, facets, and execution lifecycle vocabulary. External resources such as
//! Tauri events, git2, file I/O, and agent runtime handles
//! are represented only by traits in `repository` / `gateway`.

pub mod entities;
pub mod error;
pub mod events;
pub mod gateway;
#[allow(clippy::module_inception)]
pub mod repository;
pub mod services;
pub mod status_aggregation;
pub mod value_objects;

pub use entities::workflow_execution::FanoutRuntimeState;
pub use entities::workflow_execution::OutputSubmissionRollback;
#[cfg(test)]
pub use entities::workflow_execution::{FanoutChildRuntime, FanoutChildRuntimeState};
pub use error::WorkflowError;
pub use events::{WorkflowContractViolation, WorkflowDomainEvent, WorkflowJsonPayload};
pub use gateway::{ManagedWorktreeGateway, SecretSourceGateway};
pub use repository::{
    FacetRepository, WorkflowDefinitionRepository, WorkflowExecutionArchiveRepository,
    WorkflowExecutionArchiveSnapshot, WorkflowExecutionManualArchiveRecord,
    WORKFLOW_ARCHIVE_REASON_MANUAL,
};
pub use services::{
    approval_rules, contract, secret_masker, validation, RetryPolicy, TimeoutContext, TimeoutPolicy,
};
#[cfg(test)]
pub use value_objects::ExecutionListFilter;
#[cfg(test)]
pub use value_objects::WorkflowExecutionRecord;
pub use value_objects::{
    ApprovalTarget, Artifact, CommandSpec, ContractType, ContractValidationResult,
    ContractViolationRecord, ExecutionInterruptionReason, ExecutionOrigin, ExecutionStatus,
    ExecutionStatusFilter, FacetContents, FacetKey, FacetKind, FacetRefs, FacetSummary,
    FailureClassification, FailureDisposition, Fanout, FanoutChildSnapshot, FanoutParentRef,
    FanoutSpec, ItemsSource, NodeDefinition, NodeDefinitionName, NodeExecution,
    NodeExecutionFailure, NodeExecutionFailureKind, NodeExecutionStatus, NodeHistoryEntry,
    NodeKind, NodeKindName, Rule, RuntimeArtifact, RuntimeExecutionState, SchemaDef, SessionGate,
    SessionSpec, TimeoutKind, TokenUsage, WorkflowDefinition, WorkflowDefinitionName,
    WorkflowEvent, WorkflowExecution, WorkflowExecutionId, WorkflowExecutionSummary,
    WorkflowFacetContents, WorkflowNodeContext, WorkflowPageRequest, WorkflowRuntimeSnapshot,
    WorkflowSummary, WorkspaceWorktreePath, NODE_STATUS_ABORTED, NODE_STATUS_COMPLETED,
    NODE_STATUS_FAILED, NODE_STATUS_INTERRUPTED, NODE_STATUS_RUNNING,
};
