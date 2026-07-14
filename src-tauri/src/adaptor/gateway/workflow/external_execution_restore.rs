use std::collections::HashMap;

use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata;
use crate::adaptor::gateway::workflow::runtime_state::{
    FanoutChildRuntime, FanoutChildRuntimeState, FanoutRuntimeState, WorkflowExecution,
};
use crate::adaptor::gateway::workflow::schema::{NodeKindName, Workflow};
use crate::adaptor::gateway::workflow::state::{
    FanoutChildSnapshot, NodeExecution, NodeExecutionFailure, NodeExecutionStatus,
    NodeHistoryEntry, RuntimeArtifact, RuntimeExecutionState, TokenUsage,
};
use crate::adaptor::gateway::workflow::step_settings::WorkflowDefaults;
use crate::domain::workflow as workflow_domain;

pub(crate) struct RestoredExternalExecution {
    pub(crate) execution: WorkflowExecution,
    pub(crate) current_session_id: Option<String>,
}

pub(crate) fn validate_execution_metadata_for_external_restore(
    execution_id: &str,
    metadata: &WorkflowExecutionMetadata,
) -> Result<(), WorkflowEngineError> {
    if metadata.status.is_terminal() {
        return Err(WorkflowEngineError::InvalidState(format!(
            "execution {execution_id} is already terminal"
        )));
    }
    Ok(())
}

pub(crate) fn restore_execution_from_projection(
    execution_id: &str,
    metadata: WorkflowExecutionMetadata,
    projection: workflow_domain::WorkflowExecution,
    definition: Workflow,
    request: String,
) -> Result<RestoredExternalExecution, WorkflowEngineError> {
    if !projection.status.is_active() {
        return Err(WorkflowEngineError::InvalidState(format!(
            "execution {execution_id} is already terminal"
        )));
    }
    let current_node_name = projection.current_node.as_deref().ok_or_else(|| {
        WorkflowEngineError::InvalidState(format!(
            "execution {execution_id} has no active current node"
        ))
    })?;
    let current_node_index = definition
        .nodes
        .iter()
        .position(|node| node.name == current_node_name)
        .ok_or_else(|| {
            WorkflowEngineError::InvalidState(format!(
                "execution {execution_id} has unknown current node '{current_node_name}'"
            ))
        })?;
    if current_node_index >= definition.nodes.len() {
        return Err(WorkflowEngineError::InvalidState(format!(
            "execution {execution_id} has invalid current node"
        )));
    }

    let node_execution_counts = projection.node_executions.iter().fold(
        HashMap::<String, u32>::new(),
        |mut counts, node| {
            counts
                .entry(node.node_name.clone())
                .and_modify(|attempt| *attempt = (*attempt).max(node.attempt))
                .or_insert(node.attempt);
            counts
        },
    );
    let node_history = projection
        .node_executions
        .iter()
        .filter(|node| node.fanout_parent.is_none() && !node.status.is_active())
        .map(|node| node_history_entry(node, &projection))
        .collect();
    let artifacts = projection
        .artifacts
        .iter()
        .filter(|artifact| artifact.node_name != "request")
        .filter_map(|artifact| {
            let node = projection.node_executions.iter().rev().find(|node| {
                node.node_name == artifact.node_name
                    && node.fanout_parent.is_none()
                    && node.status == workflow_domain::NodeExecutionStatus::Succeeded
            })?;
            Some((
                artifact.node_name.clone(),
                RuntimeArtifact {
                    node_name: artifact.node_name.clone(),
                    attempt: node.attempt,
                    session_id: node.session_id.clone(),
                    result: node.result_summary.clone(),
                    artifact: Some(artifact.value.clone()),
                    contract: artifact.contract.clone(),
                    token_usage: node.token_usage.as_ref().map(token_usage),
                    completed_at: node.completed_at.unwrap_or(artifact.produced_at),
                },
            ))
        })
        .collect();
    let node_executions = projection
        .node_executions
        .iter()
        .map(node_execution)
        .collect();
    let parallel_run = projection
        .fanouts
        .iter()
        .find(|fanout| {
            fanout.parent.node_name == current_node_name
                && (fanout.parent.status.is_active()
                    || fanout.children.iter().any(|child| child.status.is_active()))
        })
        .map(fanout_runtime_state);
    let current_session_id = projection
        .approval_target
        .as_ref()
        .and_then(|target| target.session_id.clone())
        .or_else(|| {
            projection
                .node_executions
                .iter()
                .rev()
                .find(|node| node.fanout_parent.is_none() && node.status.is_active())
                .and_then(|node| node.session_id.clone())
        });
    let restored_workflow_defaults = WorkflowDefaults {
        backend_id: None,
        permission_mode: crate::domain::agent_session::PermissionMode::EDIT.to_string(),
    };
    let execution = WorkflowExecution {
        id: execution_id.to_string(),
        workflow: definition,
        state: match projection.status {
            workflow_domain::ExecutionStatus::Running => RuntimeExecutionState::Running,
            workflow_domain::ExecutionStatus::WaitingApproval => {
                RuntimeExecutionState::WaitingApproval
            }
            _ => unreachable!("terminal projections were rejected above"),
        },
        current_node_index,
        node_execution_counts,
        node_history,
        workflow_defaults: restored_workflow_defaults,
        worktree_path: metadata.worktree_path,
        created_from: projection.created_from,
        error_reason: projection.error_reason,
        started_at: projection.started_at,
        updated_at: projection.updated_at,
        current_session_id: current_session_id.clone(),
        current_step_token_usage: TokenUsage::default(),
        artifacts,
        node_executions,
        request: Some(request),
        parallel_run,
        current_stall_observations: Vec::new(),
    };

    Ok(RestoredExternalExecution {
        execution,
        current_session_id,
    })
}

fn node_history_entry(
    node: &workflow_domain::NodeExecution,
    execution: &workflow_domain::WorkflowExecution,
) -> NodeHistoryEntry {
    let fanout_children = execution
        .fanouts
        .iter()
        .find(|fanout| fanout.parent.id == node.id)
        .map(|fanout| {
            fanout
                .children
                .iter()
                .map(|child| FanoutChildSnapshot {
                    node_name: child.node_name.clone(),
                    session_id: child.session_id.clone(),
                    result: child.result_summary.clone(),
                    attempt: child.attempt,
                    completed_at: child.completed_at.unwrap_or(child.started_at),
                    artifact: child
                        .artifact
                        .as_ref()
                        .map(|artifact| artifact.value.clone()),
                    contract: child
                        .artifact
                        .as_ref()
                        .and_then(|artifact| artifact.contract.clone()),
                    state: node_history_status(child.status).to_string(),
                    failure_kind: child.failure.as_ref().map(|failure| failure.kind),
                    failure_disposition: child
                        .failure
                        .as_ref()
                        .map(|failure| failure.kind.default_disposition()),
                })
                .collect()
        });
    NodeHistoryEntry {
        node_name: node.node_name.clone(),
        completed_at: node.completed_at.unwrap_or(node.started_at),
        result: node.result_summary.clone(),
        session_id: node.session_id.clone(),
        token_usage: node.token_usage.as_ref().map(token_usage),
        artifact: node
            .artifact
            .as_ref()
            .map(|artifact| artifact.value.clone()),
        attempt: node.attempt,
        fanout_children,
        state: node_history_status(node.status).to_string(),
    }
}

fn node_history_status(status: workflow_domain::NodeExecutionStatus) -> &'static str {
    match status {
        workflow_domain::NodeExecutionStatus::Running => workflow_domain::NODE_STATUS_RUNNING,
        workflow_domain::NodeExecutionStatus::WaitingApproval => {
            workflow_domain::NODE_STATUS_WAITING_APPROVAL
        }
        workflow_domain::NodeExecutionStatus::Succeeded => workflow_domain::NODE_STATUS_COMPLETED,
        workflow_domain::NodeExecutionStatus::Failed => workflow_domain::NODE_STATUS_FAILED,
        workflow_domain::NodeExecutionStatus::Aborted => workflow_domain::NODE_STATUS_ABORTED,
    }
}

fn node_execution(node: &workflow_domain::NodeExecution) -> NodeExecution {
    NodeExecution {
        id: node.id.clone(),
        execution_id: node.execution_id.clone(),
        node_name: node.node_name.clone(),
        kind: match node.kind {
            workflow_domain::NodeKindName::Command => NodeKindName::Command,
            workflow_domain::NodeKindName::Session => NodeKindName::Session,
            workflow_domain::NodeKindName::Fanout => NodeKindName::Fanout,
        },
        attempt: node.attempt,
        status: match node.status {
            workflow_domain::NodeExecutionStatus::Running => NodeExecutionStatus::Running,
            workflow_domain::NodeExecutionStatus::WaitingApproval => {
                NodeExecutionStatus::WaitingApproval
            }
            workflow_domain::NodeExecutionStatus::Succeeded => NodeExecutionStatus::Succeeded,
            workflow_domain::NodeExecutionStatus::Failed => NodeExecutionStatus::Failed,
            workflow_domain::NodeExecutionStatus::Aborted => NodeExecutionStatus::Aborted,
        },
        session_id: node.session_id.clone(),
        artifact: node
            .artifact
            .as_ref()
            .map(|artifact| artifact.value.clone()),
        token_usage: node.token_usage.as_ref().map(token_usage),
        failure: node.failure.as_ref().map(|failure| NodeExecutionFailure {
            reason: failure.reason.clone(),
            kind: failure.kind,
        }),
        fanout_parent: node.fanout_parent.as_ref().map(|parent| {
            crate::adaptor::gateway::workflow::event::FanoutParentRef {
                parent_node: parent.parent_node.clone(),
                parent_attempt: parent.parent_attempt,
                item_index: parent.item_index,
                child_index: parent.child_index,
            }
        }),
        started_at: node.started_at,
        completed_at: node.completed_at,
    }
}

fn fanout_runtime_state(fanout: &workflow_domain::Fanout) -> FanoutRuntimeState {
    FanoutRuntimeState {
        parent_node_name: fanout.parent.node_name.clone(),
        parent_node_execution_id: fanout.parent.id.clone(),
        children: fanout
            .children
            .iter()
            .map(|child| FanoutChildRuntime {
                node_execution_id: child.id.clone(),
                node_name: child.node_name.clone(),
                session_id: child.session_id.clone().unwrap_or_default(),
                state: match child.status {
                    workflow_domain::NodeExecutionStatus::Running
                    | workflow_domain::NodeExecutionStatus::WaitingApproval => {
                        FanoutChildRuntimeState::Running
                    }
                    workflow_domain::NodeExecutionStatus::Succeeded => {
                        FanoutChildRuntimeState::Completed
                    }
                    workflow_domain::NodeExecutionStatus::Failed => FanoutChildRuntimeState::Failed,
                    workflow_domain::NodeExecutionStatus::Aborted => {
                        FanoutChildRuntimeState::Interrupted
                    }
                },
                result: child.result_summary.clone(),
                artifact: child
                    .artifact
                    .as_ref()
                    .map(|artifact| artifact.value.clone()),
                contract: child
                    .artifact
                    .as_ref()
                    .and_then(|artifact| artifact.contract.clone()),
                failure_kind: child.failure.as_ref().map(|failure| failure.kind),
                failure_disposition: child
                    .failure
                    .as_ref()
                    .map(|failure| failure.kind.default_disposition()),
                token_usage: child
                    .token_usage
                    .as_ref()
                    .map(token_usage)
                    .unwrap_or_default(),
                attempt: child.attempt,
                completed_at: child.completed_at,
            })
            .collect(),
    }
}

fn token_usage(usage: &workflow_domain::TokenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{NodeDefinition, Workflow};
    use crate::domain::workflow::{Artifact, ExecutionOrigin, ExecutionStatus};

    fn workflow_execution(status: ExecutionStatus) -> WorkflowExecutionMetadata {
        WorkflowExecutionMetadata {
            execution_id: "execution-1".to_string(),
            workflow_name: "wf".to_string(),
            status,
            worktree_path: "/tmp/wt".to_string(),
            current_node: Some("review".to_string()),
            created_from: ExecutionOrigin::Cli,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
            total_token_usage: Default::default(),
        }
    }

    fn definition() -> Workflow {
        Workflow {
            name: "wf".to_string(),
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                ..NodeDefinition::default()
            }],
            ..Workflow::default()
        }
    }

    fn projection(status: ExecutionStatus) -> workflow_domain::WorkflowExecution {
        workflow_domain::WorkflowExecution {
            id: "execution-1".to_string(),
            workflow_name: "wf".to_string(),
            status,
            current_node: status.is_active().then(|| "review".to_string()),
            created_from: ExecutionOrigin::Cli,
            worktree_path: "/tmp/wt".to_string(),
            started_at: 10.0,
            updated_at: 20.0,
            completed_at: status.is_terminal().then_some(20.0),
            error_reason: None,
            total_token_usage: workflow_domain::TokenUsage::default(),
            node_executions: Vec::new(),
            artifacts: vec![Artifact {
                node_name: "request".to_string(),
                contract: None,
                value: serde_json::Value::String("ship it".to_string()),
                produced_at: 10.0,
            }],
            fanouts: Vec::new(),
            approval_target: None,
        }
    }

    #[test]
    fn validate_execution_metadata_for_external_restore_rejects_terminal_metadata() {
        let err = validate_execution_metadata_for_external_restore(
            "execution-1",
            &workflow_execution(ExecutionStatus::Completed),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            WorkflowEngineError::InvalidState(message) if message == "execution execution-1 is already terminal"
        ));
    }

    #[test]
    fn restore_execution_from_projection_rejects_terminal_projection() {
        let result = restore_execution_from_projection(
            "execution-1",
            workflow_execution(ExecutionStatus::Running),
            projection(ExecutionStatus::Completed),
            definition(),
            "ship it".to_string(),
        );

        assert!(matches!(
            result,
            Err(WorkflowEngineError::InvalidState(message))
                if message == "execution execution-1 is already terminal"
        ));
    }

    #[test]
    fn restore_execution_from_projection_rejects_unknown_current_node() {
        let mut projection = projection(ExecutionStatus::Running);
        projection.current_node = Some("missing".to_string());
        let result = restore_execution_from_projection(
            "execution-1",
            workflow_execution(ExecutionStatus::Running),
            projection,
            definition(),
            "ship it".to_string(),
        );

        assert!(matches!(
            result,
            Err(WorkflowEngineError::InvalidState(message))
                if message.contains("unknown current node")
        ));
    }

    #[test]
    fn restore_execution_from_projection_rebuilds_runtime_execution() {
        let mut projection = projection(ExecutionStatus::WaitingApproval);
        projection
            .node_executions
            .push(workflow_domain::NodeExecution {
                id: "node-execution-1".to_string(),
                execution_id: "execution-1".to_string(),
                node_name: "review".to_string(),
                kind: workflow_domain::NodeKindName::Session,
                attempt: 1,
                status: workflow_domain::NodeExecutionStatus::WaitingApproval,
                session_id: Some("session-1".to_string()),
                result_summary: None,
                artifact: None,
                token_usage: None,
                failure: None,
                fanout_parent: None,
                started_at: 10.0,
                completed_at: None,
            });
        projection.approval_target = Some(workflow_domain::ApprovalTarget {
            node_execution_id: "node-execution-1".to_string(),
            node_name: "review".to_string(),
            session_id: Some("session-1".to_string()),
        });
        let restored = restore_execution_from_projection(
            "execution-1",
            workflow_execution(ExecutionStatus::Running),
            projection,
            definition(),
            "ship it".to_string(),
        )
        .unwrap();

        assert_eq!(restored.current_session_id.as_deref(), Some("session-1"));
        assert_eq!(restored.execution.id, "execution-1");
        assert_eq!(restored.execution.workflow.name, "wf");
        assert!(matches!(
            restored.execution.state,
            RuntimeExecutionState::WaitingApproval
        ));
        assert_eq!(restored.execution.worktree_path, "/tmp/wt");
        assert_eq!(restored.execution.request.as_deref(), Some("ship it"));
        assert_eq!(restored.execution.current_node_index, 0);
        assert!(restored.execution.parallel_run.is_none());
        assert_eq!(restored.execution.node_executions.len(), 1);
        assert_eq!(restored.execution.node_executions[0].id, "node-execution-1");
        assert_eq!(
            restored.execution.node_executions[0].status,
            NodeExecutionStatus::WaitingApproval
        );
        assert_eq!(restored.execution.workflow_defaults.backend_id, None);
        assert_eq!(
            restored.execution.workflow_defaults.permission_mode,
            crate::domain::agent_session::PermissionMode::EDIT.to_string()
        );
        assert!(restored.execution.current_stall_observations.is_empty());
    }
}
