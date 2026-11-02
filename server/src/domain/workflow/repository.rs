//! Persistence ports for workflow.
//!
//! Implementations live in `adaptor/gateway/workflow/`; durable execution reads
//! are provided by the canonical workspace query port.

use crate::domain::workflow::value_objects::{
    ExecutionTreeId, FacetKind, FacetSummary, WorkflowSummary,
};
use crate::domain::workflow::{Facet, WorkflowDefinition, WorkflowError};

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

/// workflow 定義を取得し、Entity が認めた変更を保存する。
pub trait WorkflowDefinitionRepository: Send + Sync {
    /// 定義と実行状況を取得し、一覧の項目を返す。
    fn list(
        &self,
        running: &[crate::domain::workflow::WorkflowExecutionSummary],
    ) -> Result<Vec<WorkflowSummary>, WorkflowError>;
    /// 指定された定義を取得し、存在しなければ失敗を返す。
    fn get(&self, file_stem: &str) -> Result<WorkflowDefinition, WorkflowError>;
    /// 検証済みの定義を保存する。
    fn save(
        &self,
        definition: WorkflowDefinition,
        original_name: Option<&str>,
    ) -> Result<(), WorkflowError>;
    /// 指定があれば元の定義を取得する。
    fn find(&self, file_stem: Option<&str>) -> Result<Option<WorkflowDefinition>, WorkflowError>;
    /// 検証済みの定義を元のソース表現で保存する。
    fn save_source(
        &self,
        definition: &WorkflowDefinition,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<(), WorkflowError>;
    /// 利用者の定義を削除する。
    fn delete(&self, definition: &WorkflowDefinition) -> Result<(), WorkflowError>;
}

/// Facet を取得し、Entity が認めた変更を保存する。
pub trait FacetRepository: Send + Sync {
    /// 指定された種類の Facet の識別子を取得する。
    fn list(&self, kind: FacetKind) -> Result<Vec<String>, WorkflowError>;
    /// 指定された Facet を取得する。
    fn get(&self, kind: FacetKind, key: &str) -> Result<Facet, WorkflowError>;
    /// Facet を保存し、新規保存では既存の Facet を上書きしない。
    fn save(&self, facet: Facet, is_new: bool) -> Result<(), WorkflowError>;
    /// 保存対象を取得し、存在しなければ未保存の利用者 Facet を返す。
    fn get_or_new(&self, kind: FacetKind, key: &str) -> Result<Facet, WorkflowError>;
    /// 利用者の Facet を削除する。
    fn delete(&self, facet: &Facet) -> Result<(), WorkflowError>;
    /// 指定された種類の Facet の一覧項目を取得する。
    fn list_summaries(&self, kind: FacetKind) -> Result<Vec<FacetSummary>, WorkflowError>;
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
