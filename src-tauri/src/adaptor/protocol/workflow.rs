use std::collections::{BTreeMap, HashMap};

use crate::domain::workflow::{FailureDisposition, WorkflowStepFailureKind};

use serde::{Deserialize, Serialize};

const STEP_STATE_COMPLETED_VIEW: &str = "completed";
#[cfg(test)]
const STEP_STATE_FAILED_VIEW: &str = "failed";

fn default_step_entry_state_view() -> String {
    STEP_STATE_COMPLETED_VIEW.to_string()
}

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stall_observations: Vec<WorkflowStallObservationView>,
    pub started_at: f64,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStallObservationView {
    pub chat_session_id: String,
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
pub enum WorkflowExecutionStateView {
    Running,
    WaitingApproval,
    Completed,
    Failed {
        reason: String,
        #[serde(rename = "failureKind")]
        failure_kind: WorkflowStepFailureKind,
        #[serde(
            rename = "retryCount",
            skip_serializing_if = "Option::is_none",
            default
        )]
        retry_count: Option<u32>,
    },
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schemas: BTreeMap<String, serde_json::Value>,
    pub nodes: Vec<WorkflowNodeDefinitionView>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowNodeKindView {
    Command,
    Session,
    Fanout,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowSessionGateView {
    #[default]
    Auto,
    Approval,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct WorkflowFacetRefsView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

impl WorkflowFacetRefsView {
    fn is_empty(&self) -> bool {
        self.policy.is_none() && self.knowledge.is_none() && self.instruction.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct WorkflowSessionSpecView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    #[serde(default)]
    pub gate: WorkflowSessionGateView,
    #[serde(default, skip_serializing_if = "WorkflowFacetRefsView::is_empty")]
    pub facets: WorkflowFacetRefsView,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct WorkflowFanoutSpecView {
    pub parallel_children: Vec<WorkflowInterimChildView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<WorkflowAggregateConfigView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowNodeDefinitionView {
    pub name: String,
    pub kind: WorkflowNodeKindView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<WorkflowSessionSpecView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fanout: Option<WorkflowFanoutSpecView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_previous_response: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_output_from: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collect: Option<WorkflowCollectConfigView>,
    // 共通: rules は空配列でも送る（frontend では非 optional として扱う）
    #[serde(default)]
    pub rules: Vec<WorkflowTransitionRuleView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_guard: Option<WorkflowCycleGuardView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_cycle_for: Option<Vec<String>>,
}

/// fanout 配下の暫定 child API 表現。子は暗黙に session 扱い。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowInterimChildView {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    #[serde(default, skip_serializing_if = "WorkflowFacetRefsView::is_empty")]
    pub facets: WorkflowFacetRefsView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_previous_response: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_output_from: Option<Vec<String>>,
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
    /// step entry の終端状態。`"completed"`（既定）/ `"failed"` / `"aborted"`。
    #[serde(default = "default_step_entry_state_view")]
    pub state: String,
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
    pub artifact_contract: Option<String>,
    /// child snapshot の終端状態。`"completed"`（既定）/ `"failed"` / `"aborted"`。
    #[serde(default = "default_step_entry_state_view")]
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failure_kind: Option<WorkflowStepFailureKind>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failure_disposition: Option<FailureDisposition>,
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
    pub artifact_contract: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failure_kind: Option<WorkflowStepFailureKind>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failure_disposition: Option<FailureDisposition>,
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
    pub artifact_contract: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token_usage: Option<TokenUsageView>,
    pub completed_at: f64,
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
                schemas: Default::default(),
                nodes: Vec::new(),
            },
            total_token_usage: TokenUsageView::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: Vec::new(),
            workflow_variables: HashMap::new(),
            approval_operations: None,
            stall_observations: Vec::new(),
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

    #[test]
    fn workflow_execution_state_view_failed_tagged_enum_format() {
        let state = WorkflowExecutionStateView::Failed {
            reason: "exit code 1".to_string(),
            failure_kind: WorkflowStepFailureKind::InfrastructureCrash,
            retry_count: Some(2),
        };
        let json = serde_json::to_string(&state).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "failed");
        assert_eq!(v["reason"], "exit code 1");
        assert_eq!(v["failureKind"], "infrastructure_crash");
        assert_eq!(v["retryCount"], 2);
        let back: WorkflowExecutionStateView = serde_json::from_str(&json).unwrap();
        assert_eq!(back, state);
    }

    #[test]
    fn workflow_execution_state_view_all_variants_serde() {
        let variants = vec![
            WorkflowExecutionStateView::Running,
            WorkflowExecutionStateView::WaitingApproval,
            WorkflowExecutionStateView::Completed,
            WorkflowExecutionStateView::Failed {
                reason: "err".to_string(),
                failure_kind: WorkflowStepFailureKind::ValidationFailure,
                retry_count: None,
            },
            WorkflowExecutionStateView::Aborted,
        ];
        for state in variants {
            let json = serde_json::to_string(&state).unwrap();
            let back: WorkflowExecutionStateView = serde_json::from_str(&json).unwrap();
            assert_eq!(back, state);
        }
    }

    #[test]
    fn child_output_snapshot_view_exposes_failed_child_contract() {
        let view = ChildOutputSnapshotView {
            step_name: "review-a".to_string(),
            session_id: Some("session-a".to_string()),
            result: Some("model_refusal".to_string()),
            run_index: 1,
            completed_at: 1.0,
            structured_output: None,
            artifact_contract: None,
            state: STEP_STATE_FAILED_VIEW.to_string(),
            failure_kind: Some(WorkflowStepFailureKind::ModelRefusal),
            failure_disposition: Some(FailureDisposition::Partial),
        };

        let value = serde_json::to_value(view).expect("child snapshot serializes");

        assert_eq!(value["state"], "failed");
        assert_eq!(value["failureKind"], "model_refusal");
        assert_eq!(value["failureDisposition"], "partial");
    }

    #[test]
    fn parallel_step_state_view_exposes_live_partial_failure_contract() {
        let view = ParallelStepStateView {
            step_name: "review-a".to_string(),
            state: STEP_STATE_FAILED_VIEW.to_string(),
            session_id: Some("session-a".to_string()),
            result: Some("model_refusal".to_string()),
            run_index: 1,
            completed_at: None,
            structured_output: None,
            artifact_contract: None,
            failure_kind: Some(WorkflowStepFailureKind::ModelRefusal),
            failure_disposition: Some(FailureDisposition::Partial),
        };

        let value = serde_json::to_value(view).expect("parallel step serializes");

        assert_eq!(value["state"], "failed");
        assert_eq!(value["failureKind"], "model_refusal");
        assert_eq!(value["failureDisposition"], "partial");
    }

    /// [02] schema 境界: 旧表現（`workflowDefinition.steps`）を含む WorkflowState JSON は
    /// 新 `Workflow` schema（`nodes` 必須 + `deny_unknown_fields`）として deserialize に失敗する。
    /// これにより旧表現の進行中状態は新バージョンに引き継がれない。
    #[test]
    fn legacy_workflow_state_with_steps_fails_to_deserialize() {
        let json = r#"{
            "executionId": "exec-1",
            "workflowName": "legacy",
            "state": { "type": "running" },
            "currentStepIndex": 0,
            "currentStepName": "x",
            "totalSteps": 1,
            "stepHistory": [],
            "stepExecutionCounts": {},
            "workflowDefinition": {
                "name": "legacy",
                "description": "",
                "builtin": false,
                "steps": [{"name":"x","mode":"auto","instruction":"x"}]
            },
            "totalTokenUsage": { "inputTokens": 0, "outputTokens": 0 },
            "stepStates": {},
            "startedAt": 1.0,
            "updatedAt": 1.0
        }"#;
        let result: Result<WorkflowStateFieldsView, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "旧 workflowDefinition.steps を含む WorkflowState は新 schema で deserialize 失敗する"
        );
    }
}
