use std::collections::HashSet;
use std::sync::Arc;

use tauri::Manager;

use crate::adaptor::gateway::workflow::domain_mapping::{
    node_history_entries_to_domain, runtime_execution_state_to_domain,
};
use crate::adaptor::gateway::workflow::engine_error::{
    workflow_error_to_engine_error, WorkflowEngineError,
};
use crate::adaptor::gateway::workflow::runtime_state::WorkflowExecution;
use crate::adaptor::gateway::workflow::state::WorkflowState;
use crate::domain::workflow::approval_rules as workflow_approval;
use crate::domain::workflow::WorkflowError;
use crate::usecase::agent_session::status::TurnPhase;

#[cfg(test)]
pub(crate) const MAX_APPROVAL_COMMENT_CHARS: usize = workflow_approval::MAX_APPROVAL_COMMENT_CHARS;

pub(crate) fn validate_approve_comment(comment: Option<&str>) -> Result<(), WorkflowEngineError> {
    workflow_approval::validate_optional_comment_text(comment, "Approve comment")
        .map_err(|err| WorkflowEngineError::ValidationError(err.to_string()))
}

pub(crate) fn resolve_chat_session_for_approval(
    exec: &WorkflowExecution,
) -> Result<String, WorkflowEngineError> {
    let current_node = exec
        .workflow
        .nodes
        .get(exec.current_node_index)
        .ok_or_else(|| {
            WorkflowEngineError::InvalidState("current node index is out of range".to_string())
        })?;
    let state = runtime_execution_state_to_domain(&exec.state);
    workflow_approval::resolve_chat_session_for_approval(
        workflow_approval::ApprovalChatSessionSnapshot {
            is_active: exec.is_active(),
            state: &state,
            is_current_approval_session: current_node.is_approval_session(),
            current_session_id: exec.current_session_id.as_deref(),
        },
    )
    .map(str::to_string)
    .map_err(workflow_error_to_engine_error)
}

pub(crate) fn validate_approval_chat_instruction(
    exec: &WorkflowExecution,
    session_id: &str,
    content: &str,
) -> Result<(), WorkflowEngineError> {
    let current_step = &exec.workflow.nodes[exec.current_node_index];
    let is_current_approval_session = current_step.is_approval_session()
        && exec.current_session_id.as_deref() == Some(session_id);
    let is_prior_approval_gate_session =
        !is_current_approval_session && is_approval_gate_session(exec, session_id);
    let state = runtime_execution_state_to_domain(&exec.state);
    workflow_approval::validate_approval_chat_instruction(
        workflow_approval::ApprovalChatInstructionContext {
            is_current_approval_session,
            is_prior_approval_gate_session,
            state: &state,
        },
        content,
    )
    .map_err(|err| match err {
        WorkflowError::Validation(message) => WorkflowEngineError::ValidationError(message),
        other => workflow_error_to_engine_error(other),
    })
}

fn is_approval_gate_session(exec: &WorkflowExecution, session_id: &str) -> bool {
    let approval_gate_session_names: HashSet<String> = exec
        .workflow
        .nodes
        .iter()
        .filter(|step| step.is_approval_session())
        .map(|step| step.name.clone())
        .collect();
    let history = node_history_entries_to_domain(&exec.node_history);
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
    exec: &WorkflowExecution,
    expected_execution_id: Option<&str>,
    expected_node_name: Option<&str>,
) -> Result<(), WorkflowEngineError> {
    let state = runtime_execution_state_to_domain(&exec.state);
    let current_step = &exec.workflow.nodes[exec.current_node_index];
    workflow_approval::validate_approval_target(
        workflow_approval::ApprovalTargetSnapshot {
            execution_id: &exec.id,
            state: &state,
            current_node_name: &current_step.name,
            is_approval_gate_session: current_step.is_approval_session(),
        },
        expected_execution_id,
        expected_node_name,
    )
    .map_err(workflow_error_to_engine_error)
}

pub(crate) fn resolve_approval_target_snapshot(
    exec: &WorkflowExecution,
    expected_execution_id: Option<&str>,
    expected_node_name: Option<&str>,
) -> Result<String, WorkflowEngineError> {
    let state = runtime_execution_state_to_domain(&exec.state);
    let current_step = &exec.workflow.nodes[exec.current_node_index];
    workflow_approval::resolve_approval_target(
        workflow_approval::ApprovalTargetSnapshot {
            execution_id: &exec.id,
            state: &state,
            current_node_name: &current_step.name,
            is_approval_gate_session: current_step.is_approval_session(),
        },
        expected_execution_id,
        expected_node_name,
    )
    .map(str::to_string)
    .map_err(workflow_error_to_engine_error)
}

pub(crate) fn validate_approval_turn_phase(
    turn_phase: Option<TurnPhase>,
) -> Result<(), WorkflowEngineError> {
    match turn_phase {
        Some(TurnPhase::Streaming) | Some(TurnPhase::WaitingPermission) => Err(
            WorkflowEngineError::ValidationError("approval output is not complete".to_string()),
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

pub(crate) fn should_auto_approve_workflow_approval(
    snapshot: &WorkflowState,
    approval_auto_approve_enabled: bool,
) -> bool {
    let state = runtime_execution_state_to_domain(&snapshot.state);
    workflow_approval::should_auto_approve_workflow_approval(&state, approval_auto_approve_enabled)
}

pub(crate) fn auto_approve_target_for_persisted_snapshot(
    snapshot: &WorkflowState,
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
    use crate::adaptor::gateway::workflow::schema::Workflow;
    use crate::adaptor::gateway::workflow::state::{
        RuntimeExecutionState, TokenUsage, WorkflowState,
    };

    fn workflow_state_fixture(state: RuntimeExecutionState) -> WorkflowState {
        WorkflowState {
            execution_id: "run-1".to_string(),
            workflow_name: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: crate::domain::workflow::ExecutionOrigin::Cli,
            request: "ship it".to_string(),
            error_reason: None,
            state,
            current_node_index: 0,
            current_node_name: "approval".to_string(),
            current_session_id: None,
            total_nodes: 1,
            node_history: Vec::new(),
            node_execution_counts: HashMap::new(),
            workflow_definition: Workflow::default(),
            total_token_usage: TokenUsage::default(),
            node_statuses: HashMap::new(),
            artifacts: HashMap::new(),
            node_executions: Vec::new(),
            stall_observations: Vec::new(),
            approval_operations: None,
            started_at: 1.0,
            updated_at: 2.0,
        }
    }

    #[test]
    fn auto_approve_target_for_persisted_snapshot_builds_target() {
        let snapshot = workflow_state_fixture(RuntimeExecutionState::WaitingApproval);

        let target = auto_approve_target_for_persisted_snapshot(&snapshot, true);

        assert_eq!(target, Some(("run-1".to_string(), "approval".to_string())));
    }

    #[test]
    fn auto_approve_target_for_persisted_snapshot_requires_waiting_and_enabled() {
        let waiting = workflow_state_fixture(RuntimeExecutionState::WaitingApproval);
        let running = workflow_state_fixture(RuntimeExecutionState::Running);

        assert!(auto_approve_target_for_persisted_snapshot(&waiting, false).is_none());
        assert!(auto_approve_target_for_persisted_snapshot(&running, true).is_none());
    }

    #[test]
    fn validate_approve_comment_delegates_length_rule_to_domain() {
        let approve = validate_approve_comment(Some(&"x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1)))
            .unwrap_err();
        assert!(matches!(approve, WorkflowEngineError::ValidationError(_)));
    }
}
