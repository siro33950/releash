#[cfg(any(test, feature = "test-support"))]
use std::collections::HashMap;

use crate::domain::workflow::entities::workflow_execution::ExecutionTree as ExecutionTreeAggregate;
use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution;
#[cfg(any(test, feature = "test-support"))]
use crate::domain::workflow::services::projection as workflow_projection;
#[cfg(any(test, feature = "test-support"))]
use crate::domain::workflow::{
    ExecutionOrigin, NodeHistoryEntry, RuntimeArtifact, TokenUsage, WorkflowDefinition,
};

use crate::domain::workflow::RuntimeExecutionState;

/// Immutable usecase commit material derived from a `ExecutionTree`
/// aggregate.
#[derive(Debug, Clone)]
pub struct RuntimeCommitSnapshot {
    pub execution_id: String,
    pub(crate) workflow_name: String,
    pub worktree_path: String,
    pub repository_root: Option<String>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) created_from: ExecutionOrigin,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) request: String,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) error_reason: Option<String>,
    pub state: RuntimeExecutionState,
    /// 表示用の「現在の node」（実行木からの導出値）。
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) current_node_name: Option<String>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) current_session_id: Option<String>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) node_history: Vec<NodeHistoryEntry>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) workflow_definition: WorkflowDefinition,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) total_token_usage: TokenUsage,
    /// 全スコープの Artifact をフラット化した互換 read（CLI / 表示用）。
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) artifacts: HashMap<String, RuntimeArtifact>,
    pub node_executions: Vec<RuntimeNodeExecution>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) started_at: f64,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) updated_at: f64,
}

impl RuntimeCommitSnapshot {
    pub fn from_execution(
        execution: &ExecutionTreeAggregate,
    ) -> Result<Self, crate::usecase::workflow::runtime_error::WorkflowRuntimeError> {
        Ok(Self {
            execution_id: execution.id.clone(),
            workflow_name: execution.workflow_name.clone(),
            worktree_path: execution.worktree_path.clone(),
            repository_root: execution.repository_root.clone(),
            #[cfg(any(test, feature = "test-support"))]
            created_from: execution.created_from,
            #[cfg(any(test, feature = "test-support"))]
            request: execution.request.clone().unwrap_or_default(),
            #[cfg(any(test, feature = "test-support"))]
            error_reason: execution.error_reason.clone(),
            state: execution.state().clone(),
            #[cfg(any(test, feature = "test-support"))]
            current_node_name: execution.display_current_node(),
            #[cfg(any(test, feature = "test-support"))]
            current_session_id: execution.current_session_id.clone(),
            #[cfg(any(test, feature = "test-support"))]
            node_history: execution.node_history.clone(),
            #[cfg(any(test, feature = "test-support"))]
            workflow_definition: execution
                .workflow_definition()
                .map_err(|error| {
                    crate::usecase::workflow::runtime_error::WorkflowRuntimeError::InvalidState(
                        error.to_string(),
                    )
                })?
                .clone(),
            #[cfg(any(test, feature = "test-support"))]
            total_token_usage: workflow_projection::total_token_usage(&execution.node_history),
            #[cfg(any(test, feature = "test-support"))]
            artifacts: execution.flattened_artifacts(),
            node_executions: execution.node_executions.clone(),
            #[cfg(any(test, feature = "test-support"))]
            started_at: execution.started_at,
            #[cfg(any(test, feature = "test-support"))]
            updated_at: execution.updated_at,
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
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

#[cfg(any(test, feature = "test-support"))]
fn runtime_node_execution_to_domain(
    execution: RuntimeNodeExecution,
) -> crate::domain::workflow::NodeExecution {
    use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus;

    let artifact_node_name = execution.node_name.clone();
    let artifact_produced_at = execution.completed_at.unwrap_or(execution.started_at);
    crate::domain::workflow::NodeExecution {
        worktree: execution.worktree.clone(),
        id: execution.id,
        execution_id: execution.execution_id,
        node_name: execution.node_name,
        kind: execution.kind,
        attempt: execution.attempt,
        process_presence: Default::default(),
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
        parent: execution.parent,
        completion_signals: execution.completion_signals,
        started_at: execution.started_at,
        completed_at: execution.completed_at,
    }
}
