use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::adaptor::gateway::workflow::domain_mapping::{
    step_history_entries_to_domain, workflow_definition_to_domain,
    workflow_execution_state_to_domain,
};
pub use crate::adaptor::gateway::workflow::event::TokenUsage;
use crate::adaptor::gateway::workflow::failure_wire::default_failure_kind;
use crate::adaptor::gateway::workflow::schema::Workflow;
use crate::domain::workflow::{
    FailureDisposition, WorkflowStepFailureKind, STEP_STATE_ABORTED, STEP_STATE_COMPLETED,
    STEP_STATE_FAILED, STEP_STATE_RUNNING, STEP_STATE_WAITING_APPROVAL,
};

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stall_observations: Vec<WorkflowStallObservation>,
    pub started_at: f64,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalOperations {
    pub can_reject: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStallObservation {
    pub session_id: String,
    pub step_name: String,
    pub run_index: u32,
    pub turn_phase: String,
    pub idle_secs: u64,
    pub signal_count: u32,
    pub cap_reached: bool,
    pub observed_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum WorkflowExecutionState {
    Running,
    WaitingApproval,
    Completed,
    Failed {
        reason: String,
        #[serde(default = "default_failure_kind")]
        kind: WorkflowStepFailureKind,
        #[serde(skip_serializing_if = "Option::is_none", default)]
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
    let domain_workflow = workflow_definition_to_domain(workflow);
    let domain_state = workflow_execution_state_to_domain(state);
    let domain_history = step_history_entries_to_domain(step_history);
    crate::domain::workflow::compute_step_states(
        &domain_workflow,
        current_step_index,
        &domain_state,
        &domain_history,
    )
}

/// `StepHistoryEntry.state` / `ChildOutputSnapshot.state` の serde default。
///
/// 既存 ndjson event log には `state` フィールドが存在しないため、
/// deserialize 時の欠落値は本関数で `"completed"` にフォールバックする。
/// 新規 entry は engine / projection 側で `"completed"` または `"aborted"` を
/// 明示セットする。
pub fn default_step_entry_state() -> String {
    STEP_STATE_COMPLETED.to_string()
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failure_kind: Option<WorkflowStepFailureKind>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failure_disposition: Option<FailureDisposition>,
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failure_kind: Option<WorkflowStepFailureKind>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failure_disposition: Option<FailureDisposition>,
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

pub(crate) fn workflow_state_to_domain_snapshot(
    state: WorkflowState,
) -> crate::domain::workflow::WorkflowStateSnapshot {
    let workflow_definition = workflow_definition_to_domain(&state.workflow_definition);

    crate::domain::workflow::WorkflowStateSnapshot {
        execution_id: state.execution_id,
        workflow_name: state.workflow_name,
        state: workflow_execution_state_to_domain(&state.state),
        current_step_index: state.current_step_index,
        current_step_name: state.current_step_name,
        current_session_id: state.current_session_id,
        total_steps: state.total_steps,
        step_history: state
            .step_history
            .into_iter()
            .map(step_history_entry_to_domain)
            .collect(),
        step_execution_counts: state.step_execution_counts,
        workflow_definition,
        total_token_usage: token_usage_to_domain(state.total_token_usage),
        step_states: state.step_states,
        step_outputs: state
            .step_outputs
            .into_iter()
            .map(|(key, output)| (key, step_output_to_domain(output)))
            .collect(),
        active_parallel_steps: state
            .active_parallel_steps
            .into_iter()
            .map(parallel_step_state_to_domain)
            .collect(),
        workflow_variables: state.workflow_variables,
        approval_operations: state.approval_operations.map(|operations| {
            crate::domain::workflow::ApprovalOperations {
                can_reject: operations.can_reject,
            }
        }),
        stall_observations: state
            .stall_observations
            .into_iter()
            .map(workflow_stall_observation_to_domain)
            .collect(),
        started_at: state.started_at,
        updated_at: state.updated_at,
    }
}

fn workflow_stall_observation_to_domain(
    observation: WorkflowStallObservation,
) -> crate::domain::workflow::WorkflowStallObservation {
    crate::domain::workflow::WorkflowStallObservation {
        session_id: observation.session_id,
        step_name: observation.step_name,
        run_index: observation.run_index,
        turn_phase: observation.turn_phase,
        idle_secs: observation.idle_secs,
        signal_count: observation.signal_count,
        cap_reached: observation.cap_reached,
        observed_at: observation.observed_at,
    }
}

fn token_usage_to_domain(usage: TokenUsage) -> crate::domain::workflow::TokenUsage {
    crate::domain::workflow::TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

fn step_history_entry_to_domain(
    entry: StepHistoryEntry,
) -> crate::domain::workflow::StepHistoryEntry {
    crate::domain::workflow::StepHistoryEntry {
        step_name: entry.step_name,
        completed_at: entry.completed_at,
        result: entry.result,
        session_id: entry.session_id,
        token_usage: entry.token_usage.map(token_usage_to_domain),
        structured_output: entry.structured_output,
        run_index: entry.run_index,
        child_outputs: entry
            .child_outputs
            .map(|children| children.into_iter().map(child_output_to_domain).collect()),
        state: entry.state,
    }
}

fn child_output_to_domain(
    output: ChildOutputSnapshot,
) -> crate::domain::workflow::ChildOutputSnapshot {
    crate::domain::workflow::ChildOutputSnapshot {
        step_name: output.step_name,
        session_id: output.session_id,
        result: output.result,
        run_index: output.run_index,
        completed_at: output.completed_at,
        structured_output: output.structured_output,
        output_contract: output.output_contract,
        state: output.state,
        failure_kind: output.failure_kind,
        failure_disposition: output.failure_disposition,
    }
}

fn parallel_step_state_to_domain(
    state: ParallelStepState,
) -> crate::domain::workflow::ParallelStepState {
    crate::domain::workflow::ParallelStepState {
        step_name: state.step_name,
        state: state.state,
        session_id: state.session_id,
        result: state.result,
        run_index: state.run_index,
        completed_at: state.completed_at,
        structured_output: state.structured_output,
        output_contract: state.output_contract,
        failure_kind: state.failure_kind,
        failure_disposition: state.failure_disposition,
    }
}

fn step_output_to_domain(output: StepOutput) -> crate::domain::workflow::StepOutput {
    crate::domain::workflow::StepOutput {
        step_name: output.step_name,
        run_index: output.run_index,
        session_id: output.session_id,
        result: output.result,
        structured_output: output.structured_output,
        output_contract: output.output_contract,
        token_usage: output.token_usage.map(token_usage_to_domain),
        completed_at: output.completed_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_state_missing_failure_kind_defaults_to_infrastructure_crash() {
        let state: WorkflowExecutionState = serde_json::from_value(serde_json::json!({
            "type": "failed",
            "reason": "legacy failure"
        }))
        .unwrap();

        assert!(matches!(
            state,
            WorkflowExecutionState::Failed {
                kind: WorkflowStepFailureKind::InfrastructureCrash,
                ..
            }
        ));
    }
}
