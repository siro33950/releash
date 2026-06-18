use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::definition::WorkflowDefinition;
use super::step_output::{
    ParallelStepState, StepHistoryEntry, StepOutput, TokenUsage, STEP_STATE_ABORTED,
    STEP_STATE_COMPLETED, STEP_STATE_FAILED, STEP_STATE_RUNNING, STEP_STATE_WAITING_APPROVAL,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum WorkflowExecutionState {
    Running,
    WaitingApproval,
    Completed,
    Failed { reason: String },
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

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed { .. } | Self::Aborted)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalOperations {
    pub can_reject: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStateSnapshot {
    pub execution_id: String,
    pub workflow_name: String,
    pub state: WorkflowExecutionState,
    pub current_step_index: usize,
    pub current_step_name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub current_session_id: Option<String>,
    pub total_steps: usize,
    pub step_history: Vec<StepHistoryEntry>,
    pub step_execution_counts: HashMap<String, u32>,
    pub workflow_definition: WorkflowDefinition,
    pub total_token_usage: TokenUsage,
    pub step_states: HashMap<String, String>,
    #[serde(default)]
    pub step_outputs: HashMap<String, StepOutput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_parallel_steps: Vec<ParallelStepState>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub workflow_variables: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_operations: Option<ApprovalOperations>,
    pub started_at: f64,
    pub updated_at: f64,
}

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn test_execution_state_activeとterminalを判定する() {
        assert!(WorkflowExecutionState::Running.is_active());
        assert!(WorkflowExecutionState::WaitingApproval.is_active());
        assert!(WorkflowExecutionState::Completed.is_terminal());
        assert!(WorkflowExecutionState::Failed {
            reason: "boom".to_string()
        }
        .is_terminal());
        assert!(!WorkflowExecutionState::Aborted.is_active());
    }
}
