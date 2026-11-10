use serde::{Deserialize, Serialize};

use super::{
    ExecutionParentRef, NodeExecutionFailureKind, NodeKindName, TokenUsage,
    WorkflowDefinitionContent,
};

/// Canonical facts emitted by a workflow execution.
///
/// The event belongs to the workflow domain, and its serde representation is
/// also the persisted wire shape: gateway codecs serialize this type directly,
/// so serde attribute changes here change stored logs. Compatibility is pinned
/// by the stored-event contract tests in the gateway.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum WorkflowEvent {
    ExecutionStarted {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        repository_root: Option<String>,
        execution_id: String,
        workflow_name: String,
        worktree_path: String,
        request: String,
        definition: WorkflowDefinitionContent,
        timestamp: f64,
    },
    NodeStarted {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        worktree: Option<super::IsolatedWorktree>,
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        parent: Option<ExecutionParentRef>,
        timestamp: f64,
    },
    SessionAttached {
        execution_id: String,
        node_execution_id: String,
        session_id: String,
        timestamp: f64,
    },
    DelegateResultInjected {
        execution_id: String,
        node_execution_id: String,
        child_execution_id: String,
        timestamp: f64,
    },
    NodeSubmitReceived {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    NodeStopReceived {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    NodeRetryRequested {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    CommandSpawned {
        execution_id: String,
        node_execution_id: String,
        display_command: String,
        shell: Option<String>,
        cwd: Option<String>,
        timestamp: f64,
    },
    ArtifactProduced {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        contract: Option<String>,
        value: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        request_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        submitted_at: Option<f64>,
        timestamp: f64,
    },
    NodeCompleted {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        attempt: u32,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        result_summary: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        token_usage: Option<TokenUsage>,
        timestamp: f64,
    },
    NodeFailed {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        attempt: u32,
        reason: String,
        failure_kind: NodeExecutionFailureKind,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        retry_count: Option<u32>,
        timestamp: f64,
    },
    ApprovalRequested {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        result_summary: Option<String>,
        timestamp: f64,
    },
    ApprovalResolved {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        comment: Option<String>,
        timestamp: f64,
    },
    ExecutionCompleted {
        execution_id: String,
        total_token_usage: TokenUsage,
        timestamp: f64,
    },
    ExecutionAborted {
        execution_id: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        aborted_node: Option<String>,
        timestamp: f64,
    },
}
