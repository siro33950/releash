use std::collections::HashMap;

use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution;
use crate::domain::workflow::entities::workflow_execution::WorkflowExecution as WorkflowExecutionAggregate;
use crate::domain::workflow::services::projection as workflow_projection;
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
    /// 表示用の「現在の node」（実行木からの導出値）。
    pub(crate) current_node_name: Option<String>,
    pub(crate) current_session_id: Option<String>,
    pub(crate) node_history: Vec<NodeHistoryEntry>,
    pub(crate) workflow_definition: WorkflowDefinition,
    pub(crate) total_token_usage: TokenUsage,
    /// 全スコープの Artifact をフラット化した互換 read（CLI / 表示用）。
    pub(crate) artifacts: HashMap<String, RuntimeArtifact>,
    pub(crate) node_executions: Vec<RuntimeNodeExecution>,
    pub(crate) started_at: f64,
    pub(crate) updated_at: f64,
}

impl RuntimeCommitSnapshot {
    pub(crate) fn from_execution(
        execution: &WorkflowExecutionAggregate,
    ) -> Result<Self, crate::usecase::workflow::runtime_error::WorkflowRuntimeError> {
        Ok(Self {
            execution_id: execution.id.clone(),
            workflow_name: execution.workflow.name.clone(),
            worktree_path: execution.worktree_path.clone(),
            created_from: execution.created_from,
            request: execution.request.clone().unwrap_or_default(),
            error_reason: execution.error_reason.clone(),
            state: execution.state().clone(),
            current_node_name: execution.display_current_node(),
            current_session_id: execution.current_session_id.clone(),
            node_history: execution.node_history.clone(),
            workflow_definition: execution.workflow.clone(),
            total_token_usage: workflow_projection::total_token_usage(&execution.node_history),
            artifacts: execution.flattened_artifacts(),
            node_executions: execution.node_executions.clone(),
            started_at: execution.started_at,
            updated_at: execution.updated_at,
        })
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
        current_node_name: state.current_node_name,
        current_session_id: state.current_session_id,
        node_history: state.node_history,
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
            RuntimeNodeExecutionStatus::Paused => {
                crate::domain::workflow::NodeExecutionStatus::Paused
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
        parent: execution.parent,
        completion_signals: execution.completion_signals,
        started_at: execution.started_at,
        completed_at: execution.completed_at,
    }
}
