use super::{ExecutionOrigin, ExecutionStatus, TokenUsage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatusFilter {
    Active,
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
