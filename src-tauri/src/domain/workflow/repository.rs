//! Persistence ports for workflow.
//!
//! Implementations live in `adaptor/gateway/workflow/` and preserve the current
//! `workflow_runs/` JSON and event log shapes through mapper types.

use crate::domain::workflow::value_objects::{
    FacetKind, FacetSummary, RunId, RunListFilter, WorkflowDefinition, WorkflowRunRecord,
    WorkflowRunSummary, WorkflowSummary,
};
use crate::domain::workflow::WorkflowError;

pub trait WorkflowRunRepository: Send + Sync {
    fn register_active(&self, run: WorkflowRunRecord) -> Result<(), WorkflowError>;
    fn complete_run(
        &self,
        run_id: &RunId,
        completed: WorkflowRunRecord,
    ) -> Result<(), WorkflowError>;
    fn cancel_reservation(&self, run_id: &RunId) -> Result<(), WorkflowError>;
    fn list_runs(&self, filter: RunListFilter) -> Result<Vec<WorkflowRunSummary>, WorkflowError>;
    fn get_run(&self, run_id: &RunId) -> Result<Option<WorkflowRunSummary>, WorkflowError>;
    fn resolve_active_run_by_worktree(
        &self,
        worktree_path: &str,
    ) -> Result<Option<RunId>, WorkflowError>;
    fn resolve_worktree_by_run(&self, run_id: &RunId) -> Result<Option<String>, WorkflowError>;
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
