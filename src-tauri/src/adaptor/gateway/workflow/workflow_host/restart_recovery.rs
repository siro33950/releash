//! Canonical event-log hydration for active Workflow restart reconciliation.
//!
//! The implementation is kept in this module so canonical event hydration
//! remains separate from normal provider activation.

use super::resume_projection::ActiveRestartProjection;
use super::*;
use crate::adaptor::gateway::workflow::workflow_host::execution_state::{
    FanoutChildRuntime, FanoutRuntimeState,
};
use crate::domain::workflow::{
    ContractValidationResult, NodeExecution as DomainNodeExecution,
    NodeExecutionStatus as DomainNodeExecutionStatus,
};

fn invalid(message: impl Into<String>) -> WorkflowRuntimeError {
    WorkflowRuntimeError::InvalidState(message.into())
}

fn runtime_result_from_artifact(
    checkpoint: &ActiveRestartProjection,
    contract: Option<&str>,
    value: &serde_json::Value,
) -> Result<Option<String>, WorkflowRuntimeError> {
    let Some(contract) = contract else {
        return Ok(None);
    };
    match workflow_contract::validate_artifact_value(
        &checkpoint.workflow.schemas,
        contract,
        value.clone(),
    ) {
        ContractValidationResult::Valid { result, .. } => Ok(result),
        ContractValidationResult::Invalid(violation) => Err(invalid(format!(
            "canonical workflow artifact for contract '{contract}' is invalid: {}",
            violation.reason
        ))),
    }
}

fn hydrate_runtime_artifacts(
    checkpoint: &ActiveRestartProjection,
) -> Result<HashMap<String, RuntimeArtifact>, WorkflowRuntimeError> {
    let mut artifacts = HashMap::new();
    artifacts.insert(
        crate::domain::workflow::services::reference::REQUEST_ARTIFACT.to_string(),
        workflow_prompt::request_node_artifact(&checkpoint.request, checkpoint.started_at),
    );
    for artifact in checkpoint
        .projected_execution
        .artifacts
        .iter()
        .filter(|artifact| {
            artifact.node_name != crate::domain::workflow::services::reference::REQUEST_ARTIFACT
        })
    {
        let execution = checkpoint
            .projected_execution
            .node_executions
            .iter()
            .rev()
            .find(|node| {
                node.node_name == artifact.node_name
                    && node
                        .artifact
                        .as_ref()
                        .is_some_and(|candidate| candidate.value == artifact.value)
            })
            .ok_or_else(|| {
                invalid(format!(
                    "canonical artifact for node '{}' has no producing node execution",
                    artifact.node_name
                ))
            })?;
        let contract = artifact.contract.clone().or_else(|| {
            checkpoint
                .workflow
                .nodes
                .iter()
                .find(|node| node.name == artifact.node_name)
                .and_then(|node| node.artifact.clone())
        });
        let result =
            runtime_result_from_artifact(checkpoint, contract.as_deref(), &artifact.value)?
                .or_else(|| execution.result_summary.clone());
        artifacts.insert(
            artifact.node_name.clone(),
            RuntimeArtifact {
                node_name: artifact.node_name.clone(),
                attempt: execution.attempt,
                session_id: execution.session_id.clone(),
                result,
                artifact: Some(artifact.value.clone()),
                contract,
                token_usage: execution
                    .token_usage
                    .as_ref()
                    .map(resume_orchestration::runtime_token_usage),
                completed_at: artifact.produced_at,
            },
        );
    }
    Ok(artifacts)
}

fn hydrate_node_history(
    checkpoint: &ActiveRestartProjection,
    parent_position: usize,
) -> Vec<crate::domain::workflow::NodeHistoryEntry> {
    checkpoint.projected_execution.node_executions[..parent_position]
        .iter()
        .filter(|node| {
            node.fanout_parent.is_none() && node.status == DomainNodeExecutionStatus::Succeeded
        })
        .map(|node| {
            let fanout_children = (node.kind == crate::domain::workflow::NodeKindName::Fanout)
                .then(|| {
                    checkpoint
                        .projected_execution
                        .node_executions
                        .iter()
                        .filter_map(|child| {
                            let parent = child.fanout_parent.as_ref()?;
                            if parent.parent_node != node.node_name
                                || parent.parent_attempt != node.attempt
                                || child.status != DomainNodeExecutionStatus::Succeeded
                            {
                                return None;
                            }
                            Some(crate::domain::workflow::FanoutChildSnapshot {
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
                                state: crate::domain::workflow::NODE_STATUS_COMPLETED.to_string(),
                                failure_kind: None,
                                failure_disposition: None,
                            })
                        })
                        .collect()
                });
            crate::domain::workflow::NodeHistoryEntry {
                node_name: node.node_name.clone(),
                completed_at: node.completed_at.unwrap_or(node.started_at),
                result: node.result_summary.clone(),
                session_id: node.session_id.clone(),
                token_usage: node
                    .token_usage
                    .as_ref()
                    .map(resume_orchestration::runtime_token_usage),
                artifact: node
                    .artifact
                    .as_ref()
                    .map(|artifact| artifact.value.clone()),
                attempt: node.attempt,
                fanout_children,
                state: crate::domain::workflow::NODE_STATUS_COMPLETED.to_string(),
            }
        })
        .collect()
}

fn hydrate_fanout_runtime(
    checkpoint: &ActiveRestartProjection,
    parent_execution: &DomainNodeExecution,
) -> Result<Option<FanoutRuntimeState>, WorkflowRuntimeError> {
    if parent_execution.kind != crate::domain::workflow::NodeKindName::Fanout {
        return Ok(None);
    }
    let mut children = Vec::new();
    for child in checkpoint
        .projected_execution
        .node_executions
        .iter()
        .filter(|node| {
            node.fanout_parent.as_ref().is_some_and(|parent| {
                parent.parent_node == parent_execution.node_name
                    && parent.parent_attempt == parent_execution.attempt
            })
        })
    {
        let state = match child.status {
            DomainNodeExecutionStatus::Running
            | DomainNodeExecutionStatus::Paused
            | DomainNodeExecutionStatus::WaitingApproval => FanoutChildRuntimeState::Running,
            DomainNodeExecutionStatus::Succeeded => FanoutChildRuntimeState::Completed,
            DomainNodeExecutionStatus::Failed => FanoutChildRuntimeState::Failed,
            DomainNodeExecutionStatus::Aborted => FanoutChildRuntimeState::Interrupted,
        };
        let contract = checkpoint
            .workflow
            .nodes
            .iter()
            .find(|node| node.name == child.node_name)
            .and_then(|node| node.artifact.clone());
        let artifact = child
            .artifact
            .as_ref()
            .map(|artifact| artifact.value.clone());
        let result = match artifact.as_ref() {
            Some(value) => runtime_result_from_artifact(checkpoint, contract.as_deref(), value)?
                .or_else(|| child.result_summary.clone()),
            None => child.result_summary.clone(),
        };
        children.push(FanoutChildRuntime {
            node_execution_id: child.id.clone(),
            node_name: child.node_name.clone(),
            session_id: child.session_id.clone().unwrap_or_default(),
            state,
            result,
            artifact,
            contract,
            failure_kind: None,
            failure_disposition: None,
            token_usage: child
                .token_usage
                .as_ref()
                .map(resume_orchestration::runtime_token_usage)
                .unwrap_or_default(),
            attempt: child.attempt,
            completed_at: child.completed_at,
        });
    }
    Ok(Some(FanoutRuntimeState {
        parent_node_name: parent_execution.node_name.clone(),
        parent_node_execution_id: parent_execution.id.clone(),
        children,
    }))
}

pub(super) fn hydrate_restart_execution(
    checkpoint: &ActiveRestartProjection,
) -> Result<DomainWorkflowExecution, WorkflowRuntimeError> {
    let state = match checkpoint.projected_execution.status {
        ExecutionStatus::Running => RuntimeExecutionState::Running,
        other => {
            return Err(invalid(format!(
                "restart reconciliation cannot hydrate workflow status {}",
                other.as_str()
            )));
        }
    };
    let (parent_position, parent) = checkpoint
        .projected_execution
        .node_executions
        .iter()
        .enumerate()
        .rev()
        .find(|(_, node)| {
            node.fanout_parent.is_none()
                && matches!(
                    node.status,
                    DomainNodeExecutionStatus::Running
                        | DomainNodeExecutionStatus::Paused
                        | DomainNodeExecutionStatus::WaitingApproval
                )
        })
        .ok_or_else(|| invalid("restart reconciliation has no active top-level node attempt"))?;
    let current_node_index = checkpoint
        .workflow
        .nodes
        .iter()
        .position(|node| node.name == parent.node_name)
        .ok_or_else(|| {
            invalid(format!(
                "restart reconciliation node '{}' is absent from the workflow snapshot",
                parent.node_name
            ))
        })?;
    let fanout_runtime = hydrate_fanout_runtime(checkpoint, parent)?;
    Ok(
        crate::adaptor::gateway::workflow::workflow_host::execution_state::domain_workflow_execution! {
            id: checkpoint.execution_id.clone(),
            workflow: checkpoint.workflow.clone(),
            lifecycle: DomainWorkflowExecution::lifecycle_from_state(state),
            current_node_index,
            node_execution_counts: checkpoint.node_execution_counts.clone(),
            loop_guard_reset_baselines: checkpoint.loop_guard_reset_baselines.clone(),
            node_history: hydrate_node_history(checkpoint, parent_position),
            workflow_defaults: WorkflowDefaults,
            worktree_path: checkpoint.worktree_path.clone(),
            created_from: checkpoint.created_from,
            error_reason: None,
            started_at: checkpoint.started_at,
            updated_at: checkpoint.projected_execution.updated_at,
            current_session_id: (parent.kind == crate::domain::workflow::NodeKindName::Session)
                .then(|| parent.session_id.clone())
                .flatten(),
            current_node_token_usage: TokenUsage::default(),
            artifacts: hydrate_runtime_artifacts(checkpoint)?,
            node_executions: checkpoint
                .projected_execution
                .node_executions
                .iter()
                .map(resume_orchestration::runtime_node_execution)
                .collect(),
            request: Some(checkpoint.request.clone()),
            fanout_runtime,
            current_stall_observations: Vec::new(),
        },
    )
}
