//! Persistence ports for workflow.
//!
//! Implementations live in `adaptor/gateway/workflow/`; durable execution reads
//! are provided by the canonical workspace query port.

use crate::domain::workflow::value_objects::{ExecutionTreeId, FacetKind};
use crate::domain::workflow::{Facet, WorkflowDefinition, WorkflowError};

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionTreeArchiveRecord {
    pub execution_id: String,
    pub archived_at: f64,
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
    async fn target(&self, execution_id: &str)
        -> Result<ExecutionTreeArchiveTarget, WorkflowError>;
    async fn archive(
        &self,
        execution_id: &ExecutionTreeId,
        archived_at: f64,
    ) -> Result<(), WorkflowError>;
    async fn restore(
        &self,
        execution_id: &ExecutionTreeId,
        restored_at: f64,
    ) -> Result<(), WorkflowError>;
}

/// workflow 定義を取得し、Entity が認めた変更を保存する。
pub trait WorkflowDefinitionRepository: Send + Sync {
    /// 指定された定義を取得し、存在しなければ失敗を返す。
    fn get(&self, file_stem: &str) -> Result<WorkflowDefinition, WorkflowError>;
    /// 指定があれば元の定義を取得する。
    fn find(&self, file_stem: Option<&str>) -> Result<Option<WorkflowDefinition>, WorkflowError>;
}

/// Facet を取得し、Entity が認めた変更を保存する。
pub trait FacetRepository: Send + Sync {
    /// 指定された Facet を取得する。
    fn get(&self, kind: FacetKind, key: &str) -> Result<Facet, WorkflowError>;
}

pub struct WorkflowStartupRecord {
    pub execution: crate::domain::workflow::entities::workflow_execution::ExecutionTree,
    pub definition_error: Option<String>,
}

#[async_trait::async_trait]
pub trait WorkflowStartupRepository: Send + Sync {
    async fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError>;
    async fn load(&self, tree_id: &str) -> Result<Option<WorkflowStartupRecord>, WorkflowError>;
}
