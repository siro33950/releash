use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::workflow::schema::Workflow;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowState {
    pub execution_id: String,
    pub workflow_name: String,
    pub state: WorkflowExecutionState,
    pub current_step_index: usize,
    pub current_step_name: String,
    pub total_steps: usize,
    pub step_history: Vec<StepHistoryEntry>,
    pub step_execution_counts: HashMap<String, u32>,
    pub workflow_definition: Workflow,
    pub total_token_usage: TokenUsage,
    pub step_states: HashMap<String, String>,
    pub started_at: f64,
    pub updated_at: f64,
}

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
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::Completed => "completed",
            Self::Failed { .. } => "failed",
            Self::Aborted => "aborted",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl TokenUsage {
    pub fn add(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

/// 各ステップの表示用状態を計算する。
pub fn compute_step_states(
    workflow: &Workflow,
    current_step_index: usize,
    state: &WorkflowExecutionState,
    step_history: &[StepHistoryEntry],
) -> HashMap<String, String> {
    workflow
        .steps
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let s = if i == current_step_index {
                state.as_str()
            } else if step_history.iter().any(|h| h.step_name == step.name) {
                "completed"
            } else {
                "pending"
            };
            (step.name.clone(), s.to_string())
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepHistoryEntry {
    pub step_name: String,
    pub completed_at: f64,
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token_usage: Option<TokenUsage>,
}
