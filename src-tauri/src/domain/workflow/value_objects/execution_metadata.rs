use super::{ExecutionOrigin, ExecutionStatus, TokenUsage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatusFilter {
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowPageRequest {
    pub offset: usize,
    pub limit: usize,
}

#[cfg(any(test, feature = "test-support"))]
impl WorkflowPageRequest {
    pub const fn new(offset: usize, limit: usize) -> Self {
        Self { offset, limit }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExecutionSummary {
    pub execution_id: String,
    pub workflow_name: String,
    pub status: ExecutionStatus,
    pub worktree_path: String,
    pub current_node: Option<String>,
    pub created_from: ExecutionOrigin,
    pub started_at: f64,
    pub updated_at: f64,
    pub completed_at: Option<f64>,
    pub error_reason: Option<String>,
    pub total_token_usage: TokenUsage,
}
