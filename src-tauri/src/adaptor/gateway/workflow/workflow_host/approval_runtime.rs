//! Approval admission and session coordination.

use std::collections::HashSet;
use std::sync::Arc;

use tauri::Manager;

use crate::adaptor::gateway::workflow::workflow_host::execution_state::DomainWorkflowExecution;
use crate::domain::workflow::approval_rules as workflow_approval;
use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus;
use crate::domain::workflow::WorkflowError;
#[cfg(test)]
use crate::usecase::agent_session::status::TurnPhase;
use crate::usecase::workflow::runtime_error::{
    workflow_error_to_runtime_error, WorkflowRuntimeError,
};
#[cfg(test)]
use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

#[cfg(test)]
pub(crate) const MAX_APPROVAL_COMMENT_CHARS: usize = workflow_approval::MAX_APPROVAL_COMMENT_CHARS;

#[cfg(test)]
pub(crate) fn validate_approve_comment(comment: Option<&str>) -> Result<(), WorkflowRuntimeError> {
    workflow_approval::validate_optional_comment_text(comment, "Approve comment")
        .map_err(|err| WorkflowRuntimeError::ValidationError(err.to_string()))
}

fn current_approval_node_is_waiting(exec: &DomainWorkflowExecution) -> bool {
    let Some(current_node) = exec.workflow.nodes.get(exec.current_node_index) else {
        return false;
    };
    exec.node_executions.iter().rev().any(|execution| {
        execution.fanout_parent.is_none()
            && execution.node_name == current_node.name
            && execution.status == RuntimeNodeExecutionStatus::WaitingApproval
    })
}

pub(crate) fn resolve_chat_session_for_approval(
    exec: &DomainWorkflowExecution,
) -> Result<String, WorkflowRuntimeError> {
    let current_node = exec
        .workflow
        .nodes
        .get(exec.current_node_index)
        .ok_or_else(|| {
            WorkflowRuntimeError::InvalidState("current node index is out of range".to_string())
        })?;
    workflow_approval::resolve_chat_session_for_approval(
        workflow_approval::ApprovalChatSessionSnapshot {
            is_active: exec.is_active(),
            node_is_waiting_approval: current_approval_node_is_waiting(exec),
            is_current_approval_session: current_node.is_approval_session(),
            current_session_id: exec.current_session_id.as_deref(),
        },
    )
    .map(str::to_string)
    .map_err(workflow_error_to_runtime_error)
}

pub(crate) fn validate_approval_chat_instruction(
    exec: &DomainWorkflowExecution,
    session_id: &str,
    content: &str,
) -> Result<(), WorkflowRuntimeError> {
    let current_node = &exec.workflow.nodes[exec.current_node_index];
    let is_current_approval_session = current_node.is_approval_session()
        && exec.current_session_id.as_deref() == Some(session_id);
    let is_prior_approval_gate_session =
        !is_current_approval_session && is_approval_gate_session(exec, session_id);
    workflow_approval::validate_approval_chat_instruction(
        workflow_approval::ApprovalChatInstructionContext {
            is_current_approval_session,
            is_prior_approval_gate_session,
            node_is_waiting_approval: current_approval_node_is_waiting(exec),
        },
        content,
    )
    .map_err(|err| match err {
        WorkflowError::Validation(message) => WorkflowRuntimeError::ValidationError(message),
        other => workflow_error_to_runtime_error(other),
    })
}

fn is_approval_gate_session(exec: &DomainWorkflowExecution, session_id: &str) -> bool {
    let approval_gate_session_names: HashSet<String> = exec
        .workflow
        .nodes
        .iter()
        .filter(|node| node.is_approval_session())
        .map(|node| node.name.clone())
        .collect();
    let history = exec.node_history.clone();
    workflow_approval::is_approval_gate_session(
        session_id,
        exec.current_session_id.as_deref(),
        &exec.workflow.nodes[exec.current_node_index].name,
        &approval_gate_session_names,
        &history,
    )
}

#[cfg(test)]
pub(crate) fn validate_approval_target_snapshot(
    exec: &DomainWorkflowExecution,
    expected_execution_id: Option<&str>,
    expected_node_name: Option<&str>,
) -> Result<(), WorkflowRuntimeError> {
    let current_node = &exec.workflow.nodes[exec.current_node_index];
    workflow_approval::validate_approval_target(
        workflow_approval::ApprovalTargetSnapshot {
            execution_id: &exec.id,
            node_is_waiting_approval: current_approval_node_is_waiting(exec),
            current_node_name: &current_node.name,
            is_approval_gate_session: current_node.is_approval_session(),
        },
        expected_execution_id,
        expected_node_name,
    )
    .map_err(workflow_error_to_runtime_error)
}

#[cfg(test)]
pub(crate) fn resolve_approval_target_snapshot(
    exec: &DomainWorkflowExecution,
    expected_execution_id: Option<&str>,
    expected_node_name: Option<&str>,
) -> Result<String, WorkflowRuntimeError> {
    let current_node = &exec.workflow.nodes[exec.current_node_index];
    workflow_approval::resolve_approval_target(
        workflow_approval::ApprovalTargetSnapshot {
            execution_id: &exec.id,
            node_is_waiting_approval: current_approval_node_is_waiting(exec),
            current_node_name: &current_node.name,
            is_approval_gate_session: current_node.is_approval_session(),
        },
        expected_execution_id,
        expected_node_name,
    )
    .map(str::to_string)
    .map_err(workflow_error_to_runtime_error)
}

#[cfg(test)]
pub(crate) fn validate_approval_turn_phase(
    turn_phase: Option<TurnPhase>,
) -> Result<(), WorkflowRuntimeError> {
    match turn_phase {
        Some(TurnPhase::Streaming) | Some(TurnPhase::WaitingPermission) => Err(
            WorkflowRuntimeError::ValidationError("approval output is not complete".to_string()),
        ),
        Some(TurnPhase::Idle) | None => Ok(()),
    }
}

pub(crate) fn workflow_approval_auto_approve_enabled<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> bool {
    app.try_state::<Arc<dyn crate::domain::app_config::ConfigRepository>>()
        .and_then(|config| config.load().ok())
        .is_some_and(|cfg| cfg.workflow.approval_auto_approve)
}

#[cfg(test)]
pub(crate) fn should_auto_approve_workflow_approval(
    snapshot: &RuntimeCommitSnapshot,
    approval_auto_approve_enabled: bool,
) -> bool {
    workflow_approval::should_auto_approve_workflow_approval(
        snapshot
            .node_executions
            .iter()
            .any(|execution| execution.status == RuntimeNodeExecutionStatus::WaitingApproval),
        approval_auto_approve_enabled,
    )
}

#[cfg(test)]
pub(crate) fn auto_approve_target_for_persisted_snapshot(
    snapshot: &RuntimeCommitSnapshot,
    approval_auto_approve_enabled: bool,
) -> Option<(String, String)> {
    if should_auto_approve_workflow_approval(snapshot, approval_auto_approve_enabled) {
        Some((
            snapshot.execution_id.clone(),
            snapshot.current_node_name.clone(),
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution;
    use crate::domain::workflow::WorkflowDefinition;
    use crate::domain::workflow::{NodeKindName, RuntimeExecutionState, TokenUsage};
    use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

    fn commit_snapshot_fixture(node_is_waiting_approval: bool) -> RuntimeCommitSnapshot {
        RuntimeCommitSnapshot {
            execution_id: "execution-1".to_string(),
            workflow_name: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: crate::domain::workflow::ExecutionOrigin::Cli,
            request: "ship it".to_string(),
            error_reason: None,
            state: RuntimeExecutionState::Running,
            current_node_index: 0,
            current_node_name: "approval".to_string(),
            current_session_id: None,
            node_history: Vec::new(),
            node_execution_counts: HashMap::new(),
            workflow_definition: WorkflowDefinition::default(),
            total_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: node_is_waiting_approval
                .then(|| RuntimeNodeExecution {
                    id: "node-execution-1".to_string(),
                    execution_id: "execution-1".to_string(),
                    node_name: "approval".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    status: RuntimeNodeExecutionStatus::WaitingApproval,
                    session_id: None,
                    display_command: None,
                    artifact: None,
                    token_usage: None,
                    failure: None,
                    fanout_parent: None,
                    completion_signals: Default::default(),
                    started_at: 1.0,
                    completed_at: None,
                })
                .into_iter()
                .collect(),
            started_at: 1.0,
            updated_at: 2.0,
        }
    }

    #[test]
    fn auto_approve_target_for_persisted_snapshot_builds_target() {
        let snapshot = commit_snapshot_fixture(true);

        let target = auto_approve_target_for_persisted_snapshot(&snapshot, true);

        assert_eq!(
            target,
            Some(("execution-1".to_string(), "approval".to_string()))
        );
    }

    #[test]
    fn auto_approve_target_for_persisted_snapshot_requires_waiting_and_enabled() {
        let waiting = commit_snapshot_fixture(true);
        let running = commit_snapshot_fixture(false);

        assert!(auto_approve_target_for_persisted_snapshot(&waiting, false).is_none());
        assert!(auto_approve_target_for_persisted_snapshot(&running, true).is_none());
    }

    #[test]
    fn validate_approve_comment_delegates_length_rule_to_domain() {
        let approve = validate_approve_comment(Some(&"x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1)))
            .unwrap_err();
        assert!(matches!(approve, WorkflowRuntimeError::ValidationError(_)));
    }
}
