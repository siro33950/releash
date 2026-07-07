//! workflow domain.
//!
//! This module owns workflow meaning: definitions, execution state, approvals,
//! contracts, facets, and run lifecycle vocabulary. External resources such as
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
    compute_step_states, ParallelChildRun, ParallelChildState, ParallelRunState,
};
pub use error::WorkflowError;
pub use gateway::{ManagedWorktreeGateway, SecretSourceGateway};
pub use repository::{
    FacetRepository, WorkflowDefinitionRepository, WorkflowRunArchiveRepository,
    WorkflowRunManualArchiveRecord, WorkflowRunRepository, WORKFLOW_ARCHIVE_REASON_MANUAL,
};
pub use services::{
    approval_rules, contract, secret_masker, validation, variable_renderer, ApprovalInputError,
    RetryPolicy, TimeoutContext, TimeoutPolicy,
};
#[cfg(test)]
pub use value_objects::WorkflowRunRecord;
pub use value_objects::{
    ApprovalDecision, ApprovalOperations, ChildOutputSnapshot, CollectConfig, CommandSpec,
    ContractType, ContractValidationResult, CycleGuard, FacetKey, FacetKind, FacetRefs,
    FacetSummary, FailureClassification, FailureDisposition, FanoutSpec, InterimChild,
    NodeDefinition, NodeKind, NodeKindName, NodeName, OutcomeCommitMode, ParallelAggregate,
    ParallelStepState, ReduceStrategy, RunId, RunListFilter, RunStatus, RunStatusFilter,
    SessionGate, SessionSpec, StepHistoryEntry, StepOutput, TimeoutKind, TokenUsage,
    TransitionRule, TriggerSource, WorkflowDefinition, WorkflowExecutionState, WorkflowName,
    WorkflowRunSummary, WorkflowStallObservation, WorkflowStateSnapshot, WorkflowStepContext,
    WorkflowStepFailureKind, WorkflowSummary, WorktreePath, STEP_STATE_ABORTED,
    STEP_STATE_COMPLETED, STEP_STATE_FAILED, STEP_STATE_INTERRUPTED, STEP_STATE_PENDING,
    STEP_STATE_RUNNING, STEP_STATE_WAITING_APPROVAL,
};
