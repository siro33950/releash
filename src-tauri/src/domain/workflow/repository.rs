//! Persistence ports for workflow.
//!
//! Implementations live in `adaptor/gateway/workflow/`; durable execution reads
//! are provided by the canonical workspace query port.

use crate::domain::workflow::value_objects::{
    ExecutionTreeId, FacetKind, FacetSummary, WorkflowDefinition, WorkflowSummary,
};
use crate::domain::workflow::WorkflowError;

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionTreeArchiveRecord {
    pub execution_id: String,
    pub archived_at: f64,
    pub archive_reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionTreeArchiveCandidate {
    pub execution_id: String,
    pub worktree_path: String,
    pub workspace_identity: String,
    pub repository_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionTreeArchiveTarget {
    pub execution_id: String,
    pub worktree_path: String,
    pub workspace_identity: String,
    pub repository_root: Option<String>,
    pub status: crate::domain::workflow::ExecutionStatus,
}

#[async_trait::async_trait]
pub trait ExecutionTreeArchiveRepository: Send + Sync {
    async fn location(
        &self,
        execution_id: &str,
    ) -> Result<ExecutionTreeArchiveCandidate, WorkflowError>;
    fn worktree_identity(&self, path: &str) -> Result<String, WorkflowError>;
    async fn worktree_target_page(
        &self,
        worktree_path: &str,
        after: Option<&str>,
    ) -> Result<Vec<ExecutionTreeArchiveCandidate>, WorkflowError>;
    async fn candidate_page(
        &self,
        after: Option<&str>,
    ) -> Result<Vec<ExecutionTreeArchiveCandidate>, WorkflowError>;
    async fn record_repository_root(
        &self,
        execution_id: &str,
        repository_root: &str,
        timestamp: f64,
    ) -> Result<(), WorkflowError>;
    async fn legacy_session_archive_page(
        &self,
        after: Option<&str>,
    ) -> Result<Vec<ExecutionTreeArchiveRecord>, WorkflowError>;
    async fn target(&self, execution_id: &str)
        -> Result<ExecutionTreeArchiveTarget, WorkflowError>;
    fn legacy_archives(&self) -> Result<Vec<ExecutionTreeArchiveRecord>, WorkflowError>;
    fn finish_legacy_migration(&self) -> Result<(), WorkflowError>;
    async fn archive(
        &self,
        execution_id: &ExecutionTreeId,
        archived_at: f64,
        reason: &str,
    ) -> Result<(), WorkflowError>;
    async fn restore(
        &self,
        execution_id: &ExecutionTreeId,
        restored_at: f64,
    ) -> Result<(), WorkflowError>;
    /// Returns archive state for only the requested execution identities while
    /// preserving the canonical binding of the same process-local snapshot.
    async fn archive_snapshot_for(
        &self,
        execution_ids: &[String],
    ) -> Result<ExecutionTreeArchiveSnapshot, WorkflowError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionTreeArchiveSnapshot {
    pub records: Vec<ExecutionTreeArchiveRecord>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRevision(pub String);

pub struct WorkflowStartupRecord {
    pub execution: crate::domain::workflow::entities::workflow_execution::ExecutionTree,
    pub root: crate::domain::workflow::NodeFactMeta,
    pub definition_error: Option<String>,
    pub revision: WorkflowRevision,
}

#[async_trait::async_trait]
pub trait WorkflowStartupRepository: Send + Sync {
    async fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError>;
    async fn load(&self, tree_id: &str) -> Result<Option<WorkflowStartupRecord>, WorkflowError>;
    async fn append(
        &self,
        root: &crate::domain::workflow::NodeFactMeta,
        fact: &crate::domain::workflow::NodeFact,
        timestamp: f64,
        expected_revision: Option<&WorkflowRevision>,
    ) -> Result<(), WorkflowError>;
}
