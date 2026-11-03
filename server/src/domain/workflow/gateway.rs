//! External interaction ports for workflow.

use crate::domain::workflow::WorkflowError;

/// サーバの定義フォルダを利用できる状態にするサービス。
pub trait WorkflowDirectoryService: Send + Sync {
    /// サーバの定義フォルダを利用できる状態にして返す。
    fn prepare(&self) -> Result<crate::domain::workflow::WorkflowDirectory, WorkflowError>;
}

pub trait ManagedWorktreeGateway: Send + Sync {
    fn resolve(&self, worktree_path: &str) -> Result<String, WorkflowError>;
}

pub trait IsolatedWorktreeGateway: Send + Sync {
    fn repository_root(&self, worktree_path: &str) -> Result<String, WorkflowError>;
    fn is_created(
        &self,
        parent_worktree_path: &str,
        worktree: &crate::domain::workflow::IsolatedWorktree,
    ) -> Result<bool, WorkflowError>;
    fn create(
        &self,
        parent_worktree_path: &str,
        worktree: &crate::domain::workflow::IsolatedWorktree,
    ) -> Result<(), WorkflowError>;
}

pub trait NodeProcessReader: Send + Sync {
    fn presence(
        &self,
        workspace: &str,
        node_execution_id: &str,
        kind: crate::domain::workflow::NodeKindName,
        session_id: Option<&str>,
    ) -> Result<crate::domain::workflow::NodeProcessPresence, crate::domain::workflow::WorkflowError>;
}

/// 定義のソースを読み取り、解釈した Entity を返すサービス。
pub trait WorkflowDefinitionSourceService: Send + Sync {
    /// 定義のソースを取得する。
    fn get_source(&self, file_stem: &str) -> Result<Option<String>, WorkflowError>;
    /// 入力された定義を解釈し、保存候補または診断を返す。
    fn parse(
        &self,
        source: &str,
    ) -> Result<crate::domain::workflow::WorkflowDefinition, WorkflowSourceSaveError>;
}

/// 定義の解釈または保存を受け付けられない理由。
#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowSourceSaveError {
    Diagnostics(Vec<crate::domain::workflow::value_objects::diagnostic::DiagnosticItem>),
    Workflow(WorkflowError),
}
