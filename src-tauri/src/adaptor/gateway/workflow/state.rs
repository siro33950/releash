use crate::adaptor::gateway::workflow::domain_mapping::{
    artifacts_to_domain, node_history_entries_to_domain, runtime_execution_state_to_domain,
    token_usage_to_domain, workflow_definition_to_domain,
};
use crate::adaptor::gateway::workflow::schema::NodeKindName;
pub use crate::domain::workflow::entities::workflow_execution::{
    RuntimeNodeExecution as NodeExecution, RuntimeNodeExecutionStatus as NodeExecutionStatus,
};
pub use crate::domain::workflow::{
    FanoutChildSnapshot, NodeHistoryEntry, RuntimeArtifact, RuntimeExecutionState, TokenUsage,
};

pub(crate) use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

pub(crate) fn runtime_commit_snapshot_to_domain_snapshot(
    state: RuntimeCommitSnapshot,
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
        node_history: node_history_entries_to_domain(&state.node_history),
        node_execution_counts: state.node_execution_counts,
        workflow_definition,
        total_token_usage: token_usage_to_domain(&state.total_token_usage),
        artifacts: artifacts_to_domain(&state.artifacts),
        node_executions: state
            .node_executions
            .into_iter()
            .map(node_execution_to_domain)
            .collect(),
        started_at: state.started_at,
        updated_at: state.updated_at,
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
        token_usage: execution.token_usage.as_ref().map(token_usage_to_domain),
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
