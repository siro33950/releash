//! Persistence ports for workflow.
//!
//! Implementations live in `adaptor/gateway/workflow/`; durable execution reads
//! are provided by the canonical workspace query port.

use crate::domain::workflow::value_objects::{
    FacetKind, FacetSummary, IsolatedWorktreeLedgerSnapshot, NodeFact, NodeFactMeta,
    WorkflowDefinition, WorkflowExecutionId, WorkflowSummary,
};
use crate::domain::workflow::WorkflowError;

pub const WORKFLOW_ARCHIVE_REASON_MANUAL: &str = "manual";

/// `node_events` を正本として隔離 worktree 台帳を復元・追記する port。
pub trait IsolatedWorktreeLedgerRepository: Send + Sync {
    fn snapshot(&self) -> Result<IsolatedWorktreeLedgerSnapshot, WorkflowError>;
    fn snapshot_for_tree(
        &self,
        tree_id: &str,
    ) -> Result<IsolatedWorktreeLedgerSnapshot, WorkflowError>;
    fn append(
        &self,
        meta: &NodeFactMeta,
        fact: &NodeFact,
        timestamp_ms: i64,
    ) -> Result<(), WorkflowError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExecutionManualArchiveRecord {
    pub execution_id: String,
    pub archived_at: f64,
}

pub trait WorkflowExecutionArchiveRepository: Send + Sync {
    fn archive_manual(
        &self,
        execution_id: &WorkflowExecutionId,
        archived_at: f64,
    ) -> Result<(), WorkflowError>;
    fn restore_manual(
        &self,
        execution_id: &WorkflowExecutionId,
        restored_at: f64,
    ) -> Result<(), WorkflowError>;
    /// Returns archive state for only the requested execution identities while
    /// preserving the canonical binding of the same process-local snapshot.
    fn manual_archive_snapshot_for(
        &self,
        execution_ids: &[String],
    ) -> Result<WorkflowExecutionArchiveSnapshot, WorkflowError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExecutionArchiveSnapshot {
    pub records: Vec<WorkflowExecutionManualArchiveRecord>,
}

pub trait WorkflowDefinitionRepository: Send + Sync {
    fn list(&self, running_names: &[String]) -> Result<Vec<WorkflowSummary>, WorkflowError>;
    fn get(&self, file_stem: &str) -> Result<Option<WorkflowDefinition>, WorkflowError>;
    fn save(
        &self,
        definition: WorkflowDefinition,
        original_name: Option<&str>,
    ) -> Result<(), WorkflowError>;
    fn delete(&self, name: &str) -> Result<(), WorkflowError>;
}

pub trait FacetRepository: Send + Sync {
    fn list(&self, kind: FacetKind) -> Result<Vec<String>, WorkflowError>;
    fn get(&self, kind: FacetKind, key: &str) -> Result<String, WorkflowError>;
    fn save(
        &self,
        kind: FacetKind,
        key: &str,
        content: &str,
        is_new: bool,
    ) -> Result<(), WorkflowError>;
    fn delete(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError>;
    fn list_summaries(&self, kind: FacetKind) -> Result<Vec<FacetSummary>, WorkflowError>;
}
