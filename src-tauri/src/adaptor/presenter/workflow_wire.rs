//! Public workflow execution wire model.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSubmitArtifactInput {
    pub contract: String,
    pub value: serde_json::Value,
}

pub use crate::usecase::workflow::diagnostic_dto::{
    DiagnosticItem, DiagnosticReport, DiagnosticSpan, DiagnosticStage, DiagnosticSummary,
    FacetUsageEntry, Severity,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorkflowValidateOutputResponse {
    Valid,
    Invalid { reason: String, details: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorkflowGetOutputResponse {
    Submitted {
        contract: Option<String>,
        structured_output: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        submitted_at: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        timestamp: f64,
    },
    NotSubmitted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatusView {
    Running,
    Completed,
    Aborted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionOriginView {
    DesktopUi,
    Cli,
    Agent,
    Api,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NodeKindView {
    Command,
    Session,
    Fanout,
    Sequence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeExecutionStatusView {
    Running,
    WaitingApproval,
    Succeeded,
    Aborted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeCompletionSignalView {
    Submit,
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageView {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactView {
    pub node_name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub contract: Option<String>,
    pub value: serde_json::Value,
    pub produced_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionParentRefView {
    pub parent_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub item_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub child_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeExecutionView {
    pub process_presence: String,
    pub can_resume_session: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<crate::usecase::workflow::NodeWorktreeDto>,
    pub id: String,
    pub execution_id: String,
    pub node_name: String,
    pub kind: NodeKindView,
    pub attempt: u32,
    pub status: NodeExecutionStatusView,
    pub submit_received: bool,
    pub stop_received: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub waiting_for: Option<NodeCompletionSignalView>,
    pub can_approve: bool,
    pub can_retry: bool,
    pub has_artifact: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub display_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub artifact: Option<ArtifactView>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token_usage: Option<TokenUsageView>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parent: Option<ExecutionParentRefView>,
    pub started_at: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub completed_at: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FanoutView {
    pub parent: NodeExecutionView,
    pub children: Vec<NodeExecutionView>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub artifact: Option<ArtifactView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalTargetView {
    pub node_execution_id: String,
    pub node_name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowExecutionView {
    pub id: String,
    pub workflow_name: String,
    pub status: ExecutionStatusView,
    pub current_node: Option<String>,
    pub worktree_path: String,
    pub created_from: ExecutionOriginView,
    pub started_at: f64,
    pub updated_at: f64,
    pub completed_at: Option<f64>,
    pub error_reason: Option<String>,
    pub total_token_usage: TokenUsageView,
    pub node_executions: Vec<NodeExecutionView>,
    pub artifacts: Vec<ArtifactView>,
    pub fanouts: Vec<FanoutView>,
    pub approval_target: Option<ApprovalTargetView>,
}

#[cfg(test)]
#[path = "workflow_wire_test.rs"]
mod workflow_wire_tests;
