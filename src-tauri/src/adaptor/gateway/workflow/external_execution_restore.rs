use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::run::WorkflowRun;
use crate::adaptor::gateway::workflow::runtime_state::WorkflowExecution;
#[cfg(test)]
use crate::adaptor::gateway::workflow::state::WorkflowStallObservation;
use crate::adaptor::gateway::workflow::state::{TokenUsage, WorkflowExecutionState, WorkflowState};
use crate::adaptor::gateway::workflow::step_settings::WorkflowDefaults;

pub(crate) struct RestoredExternalExecution {
    pub(crate) execution: WorkflowExecution,
    pub(crate) current_session_id: Option<String>,
}

pub(crate) fn validate_run_record_for_external_restore(
    run_id: &str,
    run: &WorkflowRun,
) -> Result<(), WorkflowEngineError> {
    if run.status.is_terminal() {
        return Err(WorkflowEngineError::InvalidState(format!(
            "run {run_id} is already terminal"
        )));
    }
    Ok(())
}

pub(crate) fn restore_execution_from_projected_state(
    run_id: &str,
    run: WorkflowRun,
    state: WorkflowState,
) -> Result<RestoredExternalExecution, WorkflowEngineError> {
    if !matches!(
        state.state,
        WorkflowExecutionState::Running | WorkflowExecutionState::WaitingApproval
    ) {
        return Err(WorkflowEngineError::InvalidState(format!(
            "run {run_id} is already terminal"
        )));
    }
    if state.current_step_index >= state.workflow_definition.nodes.len() {
        return Err(WorkflowEngineError::InvalidState(format!(
            "run {run_id} has invalid current step"
        )));
    }

    // workflow_defaults is in-memory runtime state and cannot be recovered from
    // the event log. Later step startup settles model/permission from node definitions.
    let restored_workflow_defaults = WorkflowDefaults {
        backend_id: None,
        permission_mode: crate::domain::agent_session::PermissionMode::EDIT.to_string(),
    };
    let current_session_id = state.current_session_id.clone();
    let execution = WorkflowExecution {
        id: run_id.to_string(),
        workflow: state.workflow_definition,
        state: state.state,
        current_step_index: state.current_step_index,
        step_execution_counts: state.step_execution_counts,
        step_history: state.step_history,
        workflow_defaults: restored_workflow_defaults,
        worktree_path: run.worktree_path,
        started_at: state.started_at,
        updated_at: state.updated_at,
        current_session_id: current_session_id.clone(),
        current_step_token_usage: TokenUsage::default(),
        step_outputs: state.step_outputs,
        node_executions: state.node_executions,
        task: run.task,
        parallel_run: None,
        current_stall_observations: state.stall_observations,
    };

    Ok(RestoredExternalExecution {
        execution,
        current_session_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::run::{RunStatus, TriggerSource};
    use crate::adaptor::gateway::workflow::schema::{NodeDefinition, NodeKindName, Workflow};
    use crate::adaptor::gateway::workflow::state::{NodeExecution, NodeExecutionStatus};

    fn workflow_run(status: RunStatus) -> WorkflowRun {
        WorkflowRun {
            run_id: "run-1".to_string(),
            workflow_name: "wf".to_string(),
            task: Some("task".to_string()),
            status,
            worktree_path: "/tmp/wt".to_string(),
            current_node_name: Some("step-1".to_string()),
            trigger_source: TriggerSource::Cli,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
        }
    }

    fn workflow_state(state: WorkflowExecutionState) -> WorkflowState {
        WorkflowState {
            execution_id: "run-1".to_string(),
            workflow_name: "wf".to_string(),
            state,
            current_step_index: 0,
            current_step_name: "step-1".to_string(),
            current_session_id: Some("session-1".to_string()),
            total_steps: 1,
            step_history: Vec::new(),
            step_execution_counts: Default::default(),
            workflow_definition: Workflow {
                name: "wf".to_string(),
                nodes: vec![NodeDefinition {
                    name: "step-1".to_string(),
                    ..NodeDefinition::default()
                }],
                ..Workflow::default()
            },
            total_token_usage: TokenUsage::default(),
            step_states: Default::default(),
            step_outputs: Default::default(),
            node_executions: Vec::new(),
            stall_observations: Vec::new(),
            approval_operations: None,
            started_at: 10.0,
            updated_at: 20.0,
        }
    }

    #[test]
    fn validate_run_record_for_external_restore_rejects_terminal_metadata() {
        let err =
            validate_run_record_for_external_restore("run-1", &workflow_run(RunStatus::Completed))
                .unwrap_err();

        assert!(matches!(
            err,
            WorkflowEngineError::InvalidState(message) if message == "run run-1 is already terminal"
        ));
    }

    #[test]
    fn restore_execution_from_projected_state_rejects_terminal_projection() {
        let result = restore_execution_from_projected_state(
            "run-1",
            workflow_run(RunStatus::Running),
            workflow_state(WorkflowExecutionState::Completed),
        );

        assert!(matches!(
            result,
            Err(WorkflowEngineError::InvalidState(message))
                if message == "run run-1 is already terminal"
        ));
    }

    #[test]
    fn restore_execution_from_projected_state_rejects_invalid_current_step() {
        let mut state = workflow_state(WorkflowExecutionState::Running);
        state.current_step_index = 1;
        let result = restore_execution_from_projected_state(
            "run-1",
            workflow_run(RunStatus::Running),
            state,
        );

        assert!(matches!(
            result,
            Err(WorkflowEngineError::InvalidState(message))
                if message == "run run-1 has invalid current step"
        ));
    }

    #[test]
    fn restore_execution_from_projected_state_rebuilds_runtime_execution() {
        let mut state = workflow_state(WorkflowExecutionState::WaitingApproval);
        state.node_executions.push(NodeExecution {
            id: "node-execution-1".to_string(),
            execution_id: "run-1".to_string(),
            node_name: "step-1".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            status: NodeExecutionStatus::WaitingApproval,
            session_id: Some("session-1".to_string()),
            artifact: None,
            token_usage: None,
            failure: None,
            fanout_parent: None,
            started_at: 10.0,
            completed_at: None,
        });
        state.stall_observations.push(WorkflowStallObservation {
            session_id: "session-1".to_string(),
            step_name: "step-1".to_string(),
            run_index: 1,
            turn_phase: "streaming".to_string(),
            idle_secs: 181,
            signal_count: 1,
            cap_reached: false,
            observed_at: 30.0,
        });
        let restored = restore_execution_from_projected_state(
            "run-1",
            workflow_run(RunStatus::Running),
            state,
        )
        .unwrap();

        assert_eq!(restored.current_session_id.as_deref(), Some("session-1"));
        assert_eq!(restored.execution.id, "run-1");
        assert_eq!(restored.execution.workflow.name, "wf");
        assert!(matches!(
            restored.execution.state,
            WorkflowExecutionState::WaitingApproval
        ));
        assert_eq!(restored.execution.worktree_path, "/tmp/wt");
        assert_eq!(restored.execution.task.as_deref(), Some("task"));
        assert_eq!(restored.execution.current_step_index, 0);
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
        assert_eq!(restored.execution.current_stall_observations.len(), 1);
        assert_eq!(
            restored.execution.current_stall_observations[0].session_id,
            "session-1"
        );
    }
}
