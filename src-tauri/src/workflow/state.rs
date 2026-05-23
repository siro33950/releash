use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub use crate::workflow::event::TokenUsage;
use crate::workflow::schema::Workflow;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowState {
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
    pub workflow_definition: Workflow,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalOperations {
    pub can_reject: bool,
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

/// 各 node の表示用状態を計算する。
///
/// 命名は本マイルストーンでは `step_*` を維持する（vocabulary を `node_*` に寄せるのは [04] の責務）。
pub fn compute_step_states(
    workflow: &Workflow,
    current_step_index: usize,
    state: &WorkflowExecutionState,
    step_history: &[StepHistoryEntry],
) -> HashMap<String, String> {
    workflow
        .nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let in_history = step_history.iter().any(|h| h.step_name == node.name);
            let s = if i == current_step_index {
                // ワークフローがfailedでも、この node 自体が完了済みならcompletedとする。
                // cycle_guard超過やcontract violation等のワークフローレベル失敗で、
                // 完了済み node がfailed表示になるのを防ぐ。
                if matches!(state, WorkflowExecutionState::Failed { .. }) && in_history {
                    "completed"
                } else {
                    state.as_str()
                }
            } else if in_history {
                "completed"
            } else {
                "pending"
            };
            (node.name.clone(), s.to_string())
        })
        .collect()
}

/// `StepHistoryEntry.state` / `ChildOutputSnapshot.state` の serde default。
///
/// 既存 ndjson event log には `state` フィールドが存在しないため、
/// deserialize 時の欠落値は本関数で `"completed"` にフォールバックする。
/// 新規 entry は engine / projection 側で `"completed"` または `"aborted"` を
/// 明示セットする。
pub fn default_step_entry_state() -> String {
    "completed".to_string()
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output: Option<serde_json::Value>,
    #[serde(default)]
    pub run_index: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub child_outputs: Option<Vec<ChildOutputSnapshot>>,
    /// step entry の終端状態。`"completed"`（既定）/ `"aborted"`。
    /// 旧 ndjson 互換のため deserialize 時は default で `"completed"` になる。
    #[serde(default = "default_step_entry_state")]
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildOutputSnapshot {
    pub step_name: String,
    pub session_id: Option<String>,
    pub result: Option<String>,
    pub run_index: u32,
    pub completed_at: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub output_contract: Option<String>,
    /// child snapshot の終端状態。`"completed"`（既定）/ `"aborted"`。
    #[serde(default = "default_step_entry_state")]
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParallelStepState {
    pub step_name: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<String>,
    pub run_index: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub completed_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub output_contract: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepOutput {
    pub step_name: String,
    pub run_index: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub output_contract: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token_usage: Option<TokenUsage>,
    pub completed_at: f64,
}
