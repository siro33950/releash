//! Persistence ports for workflow.
//!
//! Implementations live in `adaptor/gateway/workflow/` and preserve the current
//! `workflow_runs/` JSON and event log shapes through mapper types.

#[cfg(test)]
use crate::domain::workflow::value_objects::WorkflowRunRecord;
use crate::domain::workflow::value_objects::{
    FacetKind, FacetSummary, RunId, RunListFilter, WorkflowDefinition, WorkflowRunSummary,
    WorkflowSummary,
};
use crate::domain::workflow::WorkflowError;

pub const WORKFLOW_ARCHIVE_REASON_MANUAL: &str = "manual";

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRunManualArchiveRecord {
    pub run_id: String,
    pub archived_at: f64,
}

pub trait WorkflowRunRepository: Send + Sync {
    #[cfg(test)]
    fn register_active(&self, run: WorkflowRunRecord) -> Result<(), WorkflowError>;
    #[cfg(test)]
    fn complete_run(
        &self,
        run_id: &RunId,
        completed: WorkflowRunRecord,
    ) -> Result<(), WorkflowError>;
    fn list_runs(&self, filter: RunListFilter) -> Result<Vec<WorkflowRunSummary>, WorkflowError>;
    fn get_run(&self, run_id: &RunId) -> Result<Option<WorkflowRunSummary>, WorkflowError>;
    #[cfg(test)]
    fn resolve_active_run_by_worktree(
        &self,
        worktree_path: &str,
    ) -> Result<Option<RunId>, WorkflowError>;
    fn resolve_worktree_by_run(&self, run_id: &RunId) -> Result<Option<String>, WorkflowError>;
}

pub trait WorkflowRunArchiveRepository: Send + Sync {
    fn archive_manual(&self, run_id: &RunId, archived_at: f64) -> Result<(), WorkflowError>;
    fn restore_manual(&self, run_id: &RunId, restored_at: f64) -> Result<(), WorkflowError>;
    fn manual_archive_records(&self) -> Result<Vec<WorkflowRunManualArchiveRecord>, WorkflowError>;
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
