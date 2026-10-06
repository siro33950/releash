use super::{ExecutionOrigin, ExecutionStatus, TokenUsage};
use crate::domain::workflow::error::WorkflowError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatusFilter {
    Active,
    Terminal,
}

impl ExecutionStatusFilter {
    pub fn from_public_filter(value: Option<&str>) -> Result<Option<Self>, WorkflowError> {
        match value {
            None | Some("") => Ok(None),
            Some("active") => Ok(Some(Self::Active)),
            Some("terminal") => Ok(Some(Self::Terminal)),
            Some(other) => Err(WorkflowError::validation(format!(
                "invalid execution status filter: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowPageRequest {
    pub offset: usize,
    pub limit: usize,
}

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

#[cfg(test)]
#[path = "execution_metadata_test.rs"]
mod execution_metadata_tests;
