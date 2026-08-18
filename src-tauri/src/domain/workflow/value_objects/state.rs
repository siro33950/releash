use std::collections::HashMap;

use super::definition::WorkflowDefinition;
use super::execution::ExecutionOrigin;
use super::node_execution::NodeExecution;
use super::runtime_projection::{
    NodeHistoryEntry, RuntimeArtifact, TokenUsage, NODE_STATUS_ABORTED, NODE_STATUS_COMPLETED,
    NODE_STATUS_RUNNING,
};
#[cfg(test)]
use super::runtime_projection::{NODE_STATUS_INTERRUPTED, NODE_STATUS_WAITING_APPROVAL};

/// Private runtime transition state. Public lifecycle state is `ExecutionStatus`.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeExecutionState {
    Running,
    #[cfg(test)]
    WaitingApproval,
    Completed,
    Aborted,
    #[cfg(test)]
    Interrupted,
}

impl RuntimeExecutionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => NODE_STATUS_RUNNING,
            #[cfg(test)]
            Self::WaitingApproval => NODE_STATUS_WAITING_APPROVAL,
            Self::Completed => NODE_STATUS_COMPLETED,
            Self::Aborted => NODE_STATUS_ABORTED,
            #[cfg(test)]
            Self::Interrupted => NODE_STATUS_INTERRUPTED,
        }
    }

    #[cfg(test)]
    pub fn is_active(&self) -> bool {
        match self {
            Self::Running => true,
            #[cfg(test)]
            Self::WaitingApproval => true,
            _ => false,
        }
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
    pub current_node_name: Option<String>,
    pub current_session_id: Option<String>,
    pub node_history: Vec<NodeHistoryEntry>,
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
        assert!(!RuntimeExecutionState::Aborted.is_active());
        assert!(!RuntimeExecutionState::Completed.is_active());
    }
}
