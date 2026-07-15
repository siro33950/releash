use std::collections::HashMap;

use super::definition::WorkflowDefinition;
use super::execution::ExecutionOrigin;
use super::failure::NodeExecutionFailureKind;
use super::node_execution::NodeExecution;
use super::runtime_projection::{
    NodeHistoryEntry, RuntimeArtifact, TokenUsage, NODE_STATUS_ABORTED, NODE_STATUS_COMPLETED,
    NODE_STATUS_FAILED, NODE_STATUS_INTERRUPTED, NODE_STATUS_RUNNING, NODE_STATUS_WAITING_APPROVAL,
};

/// Private runtime transition state. Public lifecycle state is `ExecutionStatus`.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeExecutionState {
    Running,
    WaitingApproval,
    Completed,
    Failed {
        reason: String,
        kind: NodeExecutionFailureKind,
        retry_count: Option<u32>,
    },
    Aborted,
    Interrupted,
}

impl RuntimeExecutionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => NODE_STATUS_RUNNING,
            Self::WaitingApproval => NODE_STATUS_WAITING_APPROVAL,
            Self::Completed => NODE_STATUS_COMPLETED,
            Self::Failed { .. } => NODE_STATUS_FAILED,
            Self::Aborted => NODE_STATUS_ABORTED,
            Self::Interrupted => NODE_STATUS_INTERRUPTED,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Running | Self::WaitingApproval)
    }
}

/// Internal transition snapshot used by the runtime and restore path.
///
/// This is deliberately separate from the public `WorkflowExecution` read model.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRuntimeSnapshot {
    pub execution_id: String,
    pub workflow_name: String,
    pub worktree_path: String,
    pub created_from: ExecutionOrigin,
    pub request: String,
    pub error_reason: Option<String>,
    pub state: RuntimeExecutionState,
    pub current_node_index: usize,
    pub current_node_name: String,
    pub current_session_id: Option<String>,
    pub node_history: Vec<NodeHistoryEntry>,
    pub node_execution_counts: HashMap<String, u32>,
    pub workflow_definition: WorkflowDefinition,
    pub total_token_usage: TokenUsage,
    pub artifacts: HashMap<String, RuntimeArtifact>,
    pub node_executions: Vec<NodeExecution>,
    pub started_at: f64,
    pub updated_at: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_execution_state_active_is_derived() {
        assert!(RuntimeExecutionState::Running.is_active());
        assert!(RuntimeExecutionState::WaitingApproval.is_active());
        assert!(!RuntimeExecutionState::Aborted.is_active());
    }
}
