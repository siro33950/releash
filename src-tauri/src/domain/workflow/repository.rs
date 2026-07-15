//! Persistence ports for workflow.
//!
//! Implementations live in `adaptor/gateway/workflow/` and preserve the current
//! `workflow_executions/` JSON and event log shapes through mapper types.

#[cfg(test)]
use crate::domain::workflow::value_objects::WorkflowExecutionRecord;
use crate::domain::workflow::value_objects::{
    ExecutionListFilter, FacetKind, FacetSummary, WorkflowDefinition, WorkflowExecutionId,
    WorkflowExecutionSummary, WorkflowPageRequest, WorkflowSummary,
};
use crate::domain::workflow::WorkflowError;

pub const WORKFLOW_ARCHIVE_REASON_MANUAL: &str = "manual";

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExecutionManualArchiveRecord {
    pub execution_id: String,
    pub archived_at: f64,
}

pub trait WorkflowExecutionRepository: Send + Sync {
    #[cfg(test)]
    fn register_active(&self, execution: WorkflowExecutionRecord) -> Result<(), WorkflowError>;
    #[cfg(test)]
    fn complete_execution(
        &self,
        execution_id: &WorkflowExecutionId,
        completed: WorkflowExecutionRecord,
    ) -> Result<(), WorkflowError>;
    fn list_executions(
        &self,
        filter: ExecutionListFilter,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError>;
    fn list_executions_page(
        &self,
        filter: ExecutionListFilter,
        page: WorkflowPageRequest,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError> {
        self.list_executions(filter).map(|executions| {
            executions
                .into_iter()
                .skip(page.offset)
                .take(page.limit)
                .collect()
        })
    }
    fn get_execution(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError>;
    #[cfg(test)]
    fn resolve_active_execution_by_worktree(
        &self,
        worktree_path: &str,
    ) -> Result<Option<WorkflowExecutionId>, WorkflowError>;
    fn resolve_worktree_by_execution(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<String>, WorkflowError>;
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
    fn manual_archive_records(
        &self,
    ) -> Result<Vec<WorkflowExecutionManualArchiveRecord>, WorkflowError>;
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
