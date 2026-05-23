use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepRuntimeState {
    pub runtime_active: bool,
    pub tab_open: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStateView {
    #[serde(flatten)]
    pub state: WorkflowStateFieldsView,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub runtime_states: HashMap<String, WorkflowStepRuntimeState>,
}

impl WorkflowStateView {
    pub fn from_parts(
        state: WorkflowStateFieldsView,
        runtime_states: HashMap<String, WorkflowStepRuntimeState>,
    ) -> Self {
        Self {
            state,
            runtime_states,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStateFieldsView {
    pub execution_id: String,
    pub workflow_name: String,
    pub state: WorkflowExecutionStateView,
    pub current_step_index: usize,
    pub current_step_name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub current_session_id: Option<String>,
    pub total_steps: usize,
    pub step_history: Vec<StepHistoryEntryView>,
    pub step_execution_counts: HashMap<String, u32>,
    pub workflow_definition: WorkflowDefinitionView,
    pub total_token_usage: TokenUsageView,
    pub step_states: HashMap<String, String>,
    #[serde(default)]
    pub step_outputs: HashMap<String, StepOutputView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_parallel_steps: Vec<ParallelStepStateView>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub workflow_variables: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_operations: Option<ApprovalOperationsView>,
    pub started_at: f64,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum WorkflowExecutionStateView {
    Running,
    WaitingApproval,
    Completed,
    Failed { reason: String },
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalOperationsView {
    pub can_reject: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageView {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowDefinitionView {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub builtin: bool,
    pub nodes: Vec<WorkflowNodeDefinitionView>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowNodeTypeView {
    Agent,
    Bash,
    Approval,
    Parallel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowNodeDefinitionView {
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: WorkflowNodeTypeView,
    // agent / approval 系 prompt 設定
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_contract: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_previous_response: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_output_from: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collect: Option<WorkflowCollectConfigView>,
    // bash 系
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    // parallel 系: 子 node は ChildNodeDefinition と同じく top-level 専用フィールドを
    // 構造的に持たない `WorkflowChildNodeDefinitionView` を使用する（[02] schema 境界）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_children: Option<Vec<WorkflowChildNodeDefinitionView>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<WorkflowAggregateConfigView>,
    // 共通: rules は空配列でも送る（frontend では非 optional として扱う）
    #[serde(default)]
    pub rules: Vec<WorkflowTransitionRuleView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_guard: Option<WorkflowCycleGuardView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_cycle_for: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
}

/// 並列 node 配下の子 node の API 表現。
///
/// [02] schema 境界: Rust 側 `ChildNodeDefinition` と同じく、top-level 専用フィールド
/// （`rules` / `cycle_guard` / `resets_cycle_for` / `collect` / `parallel_children` /
///  `aggregate` / `command`）は型レベルで持たない。これにより、protocol 境界の
/// API 表現が backend ドメインモデルと語彙的に一致する。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowChildNodeDefinitionView {
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: WorkflowNodeTypeView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_contract: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_previous_response: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_output_from: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowAggregateConfigView {
    #[serde(default)]
    pub all_match: Option<String>,
    #[serde(default)]
    pub any_match: Option<String>,
    pub then: String,
    pub r#else: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowTransitionRuleView {
    pub r#match: String,
    pub next: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowCycleGuardView {
    pub max_iterations: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_exhausted: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowCollectConfigView {
    pub from: Vec<String>,
    pub reduce: WorkflowReduceStrategyView,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowReduceStrategyView {
    Last,
    Concat,
    Grouped,
    AnyNeedsFix,
    AllPassed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepHistoryEntryView {
    pub step_name: String,
    pub completed_at: f64,
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token_usage: Option<TokenUsageView>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output: Option<serde_json::Value>,
    #[serde(default)]
    pub run_index: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub child_outputs: Option<Vec<ChildOutputSnapshotView>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildOutputSnapshotView {
    pub step_name: String,
    pub session_id: Option<String>,
    pub result: Option<String>,
    pub run_index: u32,
    pub completed_at: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub output_contract: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParallelStepStateView {
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
pub struct StepOutputView {
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
    pub token_usage: Option<TokenUsageView>,
    pub completed_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStateSync {
    pub worktree_path: String,
    pub workflow_state: WorkflowStateView,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn workflow_state(session_id: &str) -> WorkflowStateFieldsView {
        WorkflowStateFieldsView {
            execution_id: "exec-1".to_string(),
            workflow_name: "wf".to_string(),
            state: WorkflowExecutionStateView::Running,
            current_step_index: 0,
            current_step_name: "step".to_string(),
            current_session_id: Some(session_id.to_string()),
            total_steps: 1,
            step_history: Vec::new(),
            step_execution_counts: HashMap::new(),
            workflow_definition: WorkflowDefinitionView {
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                nodes: Vec::new(),
            },
            total_token_usage: TokenUsageView::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: Vec::new(),
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 1.0,
            updated_at: 2.0,
        }
    }

    #[test]
    fn workflow_state_view_serializes_runtime_state_wire_contract_as_camel_case() {
        let session_id = "step-session";
        let mut runtime_states = HashMap::new();
        runtime_states.insert(
            session_id.to_string(),
            WorkflowStepRuntimeState {
                runtime_active: true,
                tab_open: true,
            },
        );

        let view = WorkflowStateView::from_parts(workflow_state(session_id), runtime_states);
        let value = serde_json::to_value(view).expect("workflow state view serializes");

        assert_eq!(
            value["runtimeStates"][session_id]["runtimeActive"],
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            value["runtimeStates"][session_id]["tabOpen"],
            serde_json::Value::Bool(true)
        );
        assert!(value["runtimeStates"][session_id]["runtime_active"].is_null());
        assert!(value["runtimeStates"][session_id]["tab_open"].is_null());
    }
}
