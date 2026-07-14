use std::collections::HashMap;

use crate::adaptor::gateway::workflow::domain_mapping::{
    node_history_entries_to_domain, runtime_execution_state_to_domain,
    workflow_definition_to_domain,
};
pub use crate::adaptor::gateway::workflow::event::{FanoutParentRef, TokenUsage};
use crate::adaptor::gateway::workflow::schema::{NodeKindName, Workflow};
use crate::domain::workflow::{
    ExecutionOrigin, FailureDisposition, NodeExecutionFailureKind, NODE_STATUS_ABORTED,
    NODE_STATUS_COMPLETED, NODE_STATUS_FAILED, NODE_STATUS_RUNNING, NODE_STATUS_WAITING_APPROVAL,
};

#[derive(Debug, Clone)]
pub struct WorkflowState {
    pub execution_id: String,
    pub workflow_name: String,
    pub worktree_path: String,
    pub created_from: ExecutionOrigin,
    pub request: String,
    pub error_reason: Option<String>,
    pub state: RuntimeExecutionState,
    pub current_node_index: usize,
    pub current_node_name: String,
    pub current_session_id: Option<String>,
    pub total_nodes: usize,
    pub node_history: Vec<NodeHistoryEntry>,
    pub node_execution_counts: HashMap<String, u32>,
    pub workflow_definition: Workflow,
    pub total_token_usage: TokenUsage,
    pub node_statuses: HashMap<String, String>,
    pub artifacts: HashMap<String, RuntimeArtifact>,
    pub node_executions: Vec<NodeExecution>,
    pub approval_operations: Option<ApprovalOperations>,
    pub stall_observations: Vec<NodeStallObservation>,
    pub started_at: f64,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ApprovalOperations {
    pub can_approve: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeStallObservation {
    pub session_id: String,
    pub node_name: String,
    pub attempt: u32,
    pub turn_phase: String,
    pub idle_secs: u64,
    pub signal_count: u32,
    pub cap_reached: bool,
    pub observed_at: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeExecutionStatus {
    Running,
    WaitingApproval,
    Succeeded,
    Failed,
    Aborted,
}

impl NodeExecutionStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::WaitingApproval)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeExecutionFailure {
    pub reason: String,
    pub kind: NodeExecutionFailureKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeExecution {
    pub id: String,
    pub execution_id: String,
    pub node_name: String,
    pub kind: NodeKindName,
    pub attempt: u32,
    pub status: NodeExecutionStatus,
    pub session_id: Option<String>,
    pub artifact: Option<serde_json::Value>,
    pub token_usage: Option<TokenUsage>,
    pub failure: Option<NodeExecutionFailure>,
    pub fanout_parent: Option<FanoutParentRef>,
    pub started_at: f64,
    pub completed_at: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeExecutionState {
    Running,
    WaitingApproval,
    Completed,
    Failed {
        reason: String,
        kind: NodeExecutionFailureKind,
        retry_count: Option<u32>,
    },
    Aborted,
    Interrupted,
}

impl RuntimeExecutionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => NODE_STATUS_RUNNING,
            Self::WaitingApproval => NODE_STATUS_WAITING_APPROVAL,
            Self::Completed => NODE_STATUS_COMPLETED,
            Self::Failed { .. } => NODE_STATUS_FAILED,
            Self::Aborted => NODE_STATUS_ABORTED,
            Self::Interrupted => crate::domain::workflow::NODE_STATUS_INTERRUPTED,
        }
    }
}

/// 各 node の表示用状態を計算する。
pub fn compute_node_statuses(
    workflow: &Workflow,
    current_node_index: usize,
    state: &RuntimeExecutionState,
    node_history: &[NodeHistoryEntry],
) -> HashMap<String, String> {
    let domain_workflow = workflow_definition_to_domain(workflow);
    let domain_state = runtime_execution_state_to_domain(state);
    let domain_history = node_history_entries_to_domain(node_history);
    crate::domain::workflow::compute_node_statuses(
        &domain_workflow,
        current_node_index,
        &domain_state,
        &domain_history,
    )
}

#[derive(Debug, Clone)]
pub struct NodeHistoryEntry {
    pub node_name: String,
    pub completed_at: f64,
    pub result: Option<String>,
    pub session_id: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub artifact: Option<serde_json::Value>,
    pub attempt: u32,
    pub fanout_children: Option<Vec<FanoutChildSnapshot>>,
    /// node entry の終端状態。
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct FanoutChildSnapshot {
    pub node_name: String,
    pub session_id: Option<String>,
    pub result: Option<String>,
    pub attempt: u32,
    pub completed_at: f64,
    pub artifact: Option<serde_json::Value>,
    pub contract: Option<String>,
    /// child snapshot の終端状態。
    pub state: String,
    pub failure_kind: Option<NodeExecutionFailureKind>,
    pub failure_disposition: Option<FailureDisposition>,
}

#[derive(Debug, Clone)]
pub struct RuntimeArtifact {
    pub node_name: String,
    pub attempt: u32,
    pub session_id: Option<String>,
    pub result: Option<String>,
    pub artifact: Option<serde_json::Value>,
    pub contract: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub completed_at: f64,
}

pub(crate) fn workflow_state_to_domain_snapshot(
    state: WorkflowState,
) -> crate::domain::workflow::WorkflowRuntimeSnapshot {
    let workflow_definition = workflow_definition_to_domain(&state.workflow_definition);

    crate::domain::workflow::WorkflowRuntimeSnapshot {
        execution_id: state.execution_id,
        workflow_name: state.workflow_name,
        worktree_path: state.worktree_path,
        created_from: state.created_from,
        request: state.request,
        error_reason: state.error_reason,
        state: runtime_execution_state_to_domain(&state.state),
        current_node_index: state.current_node_index,
        current_node_name: state.current_node_name,
        current_session_id: state.current_session_id,
        total_nodes: state.total_nodes,
        node_history: state
            .node_history
            .into_iter()
            .map(node_history_entry_to_domain)
            .collect(),
        node_execution_counts: state.node_execution_counts,
        workflow_definition,
        total_token_usage: token_usage_to_domain(state.total_token_usage),
        node_statuses: state.node_statuses,
        artifacts: state
            .artifacts
            .into_iter()
            .map(|(key, artifact)| (key, runtime_artifact_to_domain(artifact)))
            .collect(),
        node_executions: state
            .node_executions
            .into_iter()
            .map(node_execution_to_domain)
            .collect(),
        approval_operations: state.approval_operations.map(|operations| {
            crate::domain::workflow::RuntimeApprovalOperations {
                can_approve: operations.can_approve,
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
    observation: NodeStallObservation,
) -> crate::domain::workflow::NodeStallObservation {
    crate::domain::workflow::NodeStallObservation {
        session_id: observation.session_id,
        node_name: observation.node_name,
        attempt: observation.attempt,
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

fn node_history_entry_to_domain(
    entry: NodeHistoryEntry,
) -> crate::domain::workflow::NodeHistoryEntry {
    crate::domain::workflow::NodeHistoryEntry {
        node_name: entry.node_name,
        completed_at: entry.completed_at,
        result: entry.result,
        session_id: entry.session_id,
        token_usage: entry.token_usage.map(token_usage_to_domain),
        artifact: entry.artifact,
        attempt: entry.attempt,
        fanout_children: entry
            .fanout_children
            .map(|children| children.into_iter().map(child_output_to_domain).collect()),
        state: entry.state,
    }
}

fn child_output_to_domain(
    output: FanoutChildSnapshot,
) -> crate::domain::workflow::FanoutChildSnapshot {
    crate::domain::workflow::FanoutChildSnapshot {
        node_name: output.node_name,
        session_id: output.session_id,
        result: output.result,
        attempt: output.attempt,
        completed_at: output.completed_at,
        artifact: output.artifact,
        contract: output.contract,
        state: output.state,
        failure_kind: output.failure_kind,
        failure_disposition: output.failure_disposition,
    }
}

fn node_execution_to_domain(execution: NodeExecution) -> crate::domain::workflow::NodeExecution {
    let artifact_node_name = execution.node_name.clone();
    let artifact_produced_at = execution.completed_at.unwrap_or(execution.started_at);
    crate::domain::workflow::NodeExecution {
        id: execution.id,
        execution_id: execution.execution_id,
        node_name: execution.node_name,
        kind: match execution.kind {
            NodeKindName::Command => crate::domain::workflow::NodeKindName::Command,
            NodeKindName::Session => crate::domain::workflow::NodeKindName::Session,
            NodeKindName::Fanout => crate::domain::workflow::NodeKindName::Fanout,
        },
        attempt: execution.attempt,
        status: match execution.status {
            NodeExecutionStatus::Running => crate::domain::workflow::NodeExecutionStatus::Running,
            NodeExecutionStatus::WaitingApproval => {
                crate::domain::workflow::NodeExecutionStatus::WaitingApproval
            }
            NodeExecutionStatus::Succeeded => {
                crate::domain::workflow::NodeExecutionStatus::Succeeded
            }
            NodeExecutionStatus::Failed => crate::domain::workflow::NodeExecutionStatus::Failed,
            NodeExecutionStatus::Aborted => crate::domain::workflow::NodeExecutionStatus::Aborted,
        },
        session_id: execution.session_id,
        result_summary: None,
        artifact: execution
            .artifact
            .map(|value| crate::domain::workflow::Artifact {
                node_name: artifact_node_name,
                contract: None,
                value,
                produced_at: artifact_produced_at,
            }),
        token_usage: execution.token_usage.map(token_usage_to_domain),
        failure: execution
            .failure
            .map(|failure| crate::domain::workflow::NodeExecutionFailure {
                reason: failure.reason,
                kind: failure.kind,
            }),
        fanout_parent: execution.fanout_parent.map(|parent| {
            crate::domain::workflow::FanoutParentRef {
                parent_node: parent.parent_node,
                parent_attempt: parent.parent_attempt,
                item_index: parent.item_index,
                child_index: parent.child_index,
            }
        }),
        started_at: execution.started_at,
        completed_at: execution.completed_at,
    }
}

fn runtime_artifact_to_domain(output: RuntimeArtifact) -> crate::domain::workflow::RuntimeArtifact {
    crate::domain::workflow::RuntimeArtifact {
        node_name: output.node_name,
        attempt: output.attempt,
        session_id: output.session_id,
        result: output.result,
        artifact: output.artifact,
        contract: output.contract,
        token_usage: output.token_usage.map(token_usage_to_domain),
        completed_at: output.completed_at,
    }
}
