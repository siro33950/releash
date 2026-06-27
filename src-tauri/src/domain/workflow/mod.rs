//! workflow domain.
//!
//! This module owns workflow meaning: definitions, execution state, approvals,
//! contracts, facets, and run lifecycle vocabulary. External resources such as
//! Tauri events, git2, file I/O, WebSocket broadcast, and agent runtime handles
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
    compute_step_states, NodeCompletion, ParallelChildCompletion, ParallelChildRun,
    ParallelChildState, ParallelRunState, WorkflowExecution,
};
pub use error::WorkflowError;
pub use gateway::{ManagedWorktreeGateway, SecretSourceGateway};
pub use repository::{
    FacetRepository, WorkflowDefinitionRepository, WorkflowRunArchiveRepository,
    WorkflowRunManualArchiveRecord, WorkflowRunRepository, WORKFLOW_ARCHIVE_REASON_MANUAL,
};
pub use services::{
    approval_rules, contract, failure_policy, parallel, secret_masker, validation,
    variable_renderer, ApprovalInputError, ParallelFailurePolicy, ParallelPropagation,
    ParallelReduceResult, RepairDecision, RetryPolicy, StructuredOutputRepairPolicy,
    TimeoutContext, TimeoutPolicy, ValidationError,
};
pub use value_objects::{
    ApprovalDecision, ApprovalOperations, ChildNodeDefinition, ChildOutputSnapshot, CollectConfig,
    ContractType, ContractValidationResult, ContractViolation, CycleGuard, FacetKey, FacetKind,
    FacetSummary, FailureClassification, FailureDisposition, NodeDefinition, NodeName, NodeType,
    OutcomeCommitMode, ParallelAggregate, ParallelStepState, ReduceStrategy, RunId, RunListFilter,
    RunStatus, RunStatusFilter, StepHistoryEntry, StepOutput, TimeoutKind, TokenUsage,
    TransitionRule, TriggerSource, WorkflowDefinition, WorkflowExecutionState, WorkflowName,
    WorkflowRunRecord, WorkflowRunSummary, WorkflowStateSnapshot, WorkflowStepContext,
    WorkflowStepFailureKind, WorkflowSummary, WorktreePath, STEP_STATE_ABORTED,
    STEP_STATE_COMPLETED, STEP_STATE_FAILED, STEP_STATE_INTERRUPTED, STEP_STATE_PENDING,
    STEP_STATE_RUNNING, STEP_STATE_WAITING_APPROVAL,
};
