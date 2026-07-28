use std::collections::HashMap;

use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution;
use crate::domain::workflow::{
    ExecutionOrigin, NodeHistoryEntry, RuntimeArtifact, RuntimeExecutionState, TokenUsage,
    WorkflowDefinition,
};

/// Immutable usecase commit material derived from a `WorkflowExecution`
/// aggregate.
#[derive(Debug, Clone)]
pub(crate) struct RuntimeCommitSnapshot {
    pub(crate) execution_id: String,
    pub(crate) workflow_name: String,
    pub(crate) worktree_path: String,
    pub(crate) created_from: ExecutionOrigin,
    pub(crate) request: String,
    pub(crate) error_reason: Option<String>,
    pub(crate) state: RuntimeExecutionState,
    pub(crate) current_node_index: usize,
    pub(crate) current_node_name: String,
    pub(crate) current_session_id: Option<String>,
    pub(crate) node_history: Vec<NodeHistoryEntry>,
    pub(crate) node_execution_counts: HashMap<String, u32>,
    pub(crate) workflow_definition: WorkflowDefinition,
    pub(crate) total_token_usage: TokenUsage,
    pub(crate) artifacts: HashMap<String, RuntimeArtifact>,
    pub(crate) node_executions: Vec<RuntimeNodeExecution>,
    pub(crate) started_at: f64,
    pub(crate) updated_at: f64,
}

impl RuntimeCommitSnapshot {
    pub(crate) fn apply_lifecycle_projection(
        &mut self,
        state: RuntimeExecutionState,
        updated_at: f64,
    ) {
        self.state = state;
        self.updated_at = updated_at;
    }
}

pub(crate) fn runtime_commit_snapshot_to_domain_snapshot(
    state: RuntimeCommitSnapshot,
) -> crate::domain::workflow::WorkflowRuntimeSnapshot {
    crate::domain::workflow::WorkflowRuntimeSnapshot {
        execution_id: state.execution_id,
        workflow_name: state.workflow_name,
        worktree_path: state.worktree_path,
        created_from: state.created_from,
        request: state.request,
        error_reason: state.error_reason,
        state: state.state,
        current_node_index: state.current_node_index,
        current_node_name: state.current_node_name,
        current_session_id: state.current_session_id,
        node_history: state.node_history,
        node_execution_counts: state.node_execution_counts,
        workflow_definition: state.workflow_definition,
        total_token_usage: state.total_token_usage,
        artifacts: state.artifacts,
        node_executions: state
            .node_executions
            .into_iter()
            .map(runtime_node_execution_to_domain)
            .collect(),
        started_at: state.started_at,
        updated_at: state.updated_at,
    }
}

fn runtime_node_execution_to_domain(
    execution: RuntimeNodeExecution,
) -> crate::domain::workflow::NodeExecution {
    use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus;

    let artifact_node_name = execution.node_name.clone();
    let artifact_produced_at = execution.completed_at.unwrap_or(execution.started_at);
    crate::domain::workflow::NodeExecution {
        id: execution.id,
        execution_id: execution.execution_id,
        node_name: execution.node_name,
        kind: execution.kind,
        attempt: execution.attempt,
        status: match execution.status {
            RuntimeNodeExecutionStatus::Running => {
                crate::domain::workflow::NodeExecutionStatus::Running
            }
            RuntimeNodeExecutionStatus::WaitingApproval => {
                crate::domain::workflow::NodeExecutionStatus::WaitingApproval
            }
            RuntimeNodeExecutionStatus::Succeeded => {
                crate::domain::workflow::NodeExecutionStatus::Succeeded
            }
            RuntimeNodeExecutionStatus::Failed => {
                crate::domain::workflow::NodeExecutionStatus::Failed
            }
            RuntimeNodeExecutionStatus::Aborted => {
                crate::domain::workflow::NodeExecutionStatus::Aborted
            }
        },
        session_id: execution.session_id,
        display_command: execution.display_command,
        result_summary: None,
        artifact: execution
            .artifact
            .map(|value| crate::domain::workflow::Artifact {
                node_name: artifact_node_name,
                contract: None,
                value,
                produced_at: artifact_produced_at,
            }),
        token_usage: execution.token_usage,
        failure: execution
            .failure
            .map(|failure| crate::domain::workflow::NodeExecutionFailure {
                reason: failure.reason,
                kind: failure.kind,
            }),
        fanout_parent: execution.fanout_parent,
        started_at: execution.started_at,
        completed_at: execution.completed_at,
    }
}
