use std::collections::HashMap;

use super::definition::WorkflowDefinition;
use super::failure::WorkflowStepFailureKind;
use super::step_output::{
    ParallelStepState, StepHistoryEntry, StepOutput, TokenUsage, STEP_STATE_ABORTED,
    STEP_STATE_COMPLETED, STEP_STATE_FAILED, STEP_STATE_RUNNING, STEP_STATE_WAITING_APPROVAL,
};

#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowExecutionState {
    Running,
    WaitingApproval,
    Completed,
    Failed {
        reason: String,
        kind: WorkflowStepFailureKind,
        retry_count: Option<u32>,
    },
    Aborted,
}

impl WorkflowExecutionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => STEP_STATE_RUNNING,
            Self::WaitingApproval => STEP_STATE_WAITING_APPROVAL,
            Self::Completed => STEP_STATE_COMPLETED,
            Self::Failed { .. } => STEP_STATE_FAILED,
            Self::Aborted => STEP_STATE_ABORTED,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Running | Self::WaitingApproval)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApprovalOperations {
    pub can_reject: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowStateSnapshot {
    pub execution_id: String,
    pub workflow_name: String,
    pub state: WorkflowExecutionState,
    pub current_step_index: usize,
    pub current_step_name: String,
    pub current_session_id: Option<String>,
    pub total_steps: usize,
    pub step_history: Vec<StepHistoryEntry>,
    pub step_execution_counts: HashMap<String, u32>,
    pub workflow_definition: WorkflowDefinition,
    pub total_token_usage: TokenUsage,
    pub step_states: HashMap<String, String>,
    pub step_outputs: HashMap<String, StepOutput>,
    pub active_parallel_steps: Vec<ParallelStepState>,
    pub workflow_variables: HashMap<String, String>,
    pub approval_operations: Option<ApprovalOperations>,
    pub started_at: f64,
    pub updated_at: f64,
}

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn test_execution_state_activeを判定する() {
        assert!(WorkflowExecutionState::Running.is_active());
        assert!(WorkflowExecutionState::WaitingApproval.is_active());
        assert!(!WorkflowExecutionState::Aborted.is_active());
    }
}
