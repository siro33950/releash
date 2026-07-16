//! Public workflow execution wire model.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatusView {
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
    Interrupted,
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
#[serde(rename_all = "snake_case")]
pub enum ExecutionInterruptionReasonView {
    Crash,
    Stale,
    Stop,
    Orphan,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NodeKindView {
    Command,
    Session,
    Fanout,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeExecutionStatusView {
    Running,
    WaitingApproval,
    Succeeded,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeExecutionFailureKindView {
    StartupTimeout,
    StaleRuntimeTimeout,
    ModelRefusal,
    StructuredOutputMismatch,
    ValidationFailure,
    UserAbort,
    InfrastructureCrash,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FanoutParentRefView {
    pub parent_node: String,
    pub parent_attempt: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub item_index: Option<usize>,
    pub child_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NodeExecutionFailureView {
    pub reason: String,
    pub kind: NodeExecutionFailureKindView,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeExecutionView {
    pub id: String,
    pub execution_id: String,
    pub node_name: String,
    pub kind: NodeKindView,
    pub attempt: u32,
    pub status: NodeExecutionStatusView,
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
    pub failure: Option<NodeExecutionFailureView>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fanout_parent: Option<FanoutParentRefView>,
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
    pub interruption_reason: Option<ExecutionInterruptionReasonView>,
    pub resume_from_node: Option<String>,
    pub total_token_usage: TokenUsageView,
    pub node_executions: Vec<NodeExecutionView>,
    pub artifacts: Vec<ArtifactView>,
    pub fanouts: Vec<FanoutView>,
    pub approval_target: Option<ApprovalTargetView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowExecutionChangedPayloadView {
    pub worktree_path: String,
    pub workflow_execution: WorkflowExecutionView,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execution() -> WorkflowExecutionView {
        WorkflowExecutionView {
            id: "execution-1".to_string(),
            workflow_name: "review".to_string(),
            status: ExecutionStatusView::Interrupted,
            current_node: Some("review".to_string()),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOriginView::Cli,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
            interruption_reason: Some(ExecutionInterruptionReasonView::Stop),
            resume_from_node: Some("review".to_string()),
            total_token_usage: TokenUsageView::default(),
            node_executions: Vec::new(),
            artifacts: vec![ArtifactView {
                node_name: "request".to_string(),
                contract: None,
                value: serde_json::Value::String("review".to_string()),
                produced_at: 1.0,
            }],
            fanouts: Vec::new(),
            approval_target: None,
        }
    }

    #[test]
    fn workflow_execution_uses_canonical_camel_case_boundary() {
        let value = serde_json::to_value(execution()).unwrap();
        assert_eq!(value["id"], "execution-1");
        assert_eq!(value["status"], "interrupted");
        assert_eq!(value["createdFrom"], "cli");
        assert_eq!(value["interruptionReason"], "stop");
        assert_eq!(value["resumeFromNode"], "review");
        assert_eq!(value["artifacts"][0]["nodeName"], "request");
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                "approvalTarget",
                "artifacts",
                "completedAt",
                "createdFrom",
                "currentNode",
                "errorReason",
                "fanouts",
                "id",
                "interruptionReason",
                "nodeExecutions",
                "resumeFromNode",
                "startedAt",
                "status",
                "totalTokenUsage",
                "updatedAt",
                "workflowName",
                "worktreePath",
            ]
        );
    }

    #[test]
    fn execution_changed_payload_names_the_execution() {
        let value = serde_json::to_value(WorkflowExecutionChangedPayloadView {
            worktree_path: "/repo".to_string(),
            workflow_execution: execution(),
        })
        .unwrap();
        assert_eq!(value["worktreePath"], "/repo");
        assert_eq!(value["workflowExecution"]["id"], "execution-1");
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["workflowExecution", "worktreePath"]);
    }
}
