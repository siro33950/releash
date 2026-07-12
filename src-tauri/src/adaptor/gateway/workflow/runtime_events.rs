use crate::adaptor::gateway::workflow::engine_error::{
    classify_cli_mutation_rejection_reason, WorkflowEngineError,
};
use crate::adaptor::gateway::workflow::event::{
    CliMutationRequestRecord, RunAbortedChildOutcome, RunAbortedChildOutputSnapshot,
    RunAbortedStepSnapshot, WorkflowEvent,
};
use crate::adaptor::gateway::workflow::internal_node_command::InternalNodeCommand;
use crate::adaptor::gateway::workflow::route_context::CommandCommitContext;
use crate::adaptor::gateway::workflow::runtime_commit::StepOutcome;
use crate::adaptor::gateway::workflow::state::{
    ChildOutputSnapshot, StepHistoryEntry, WorkflowExecutionState, WorkflowState,
};
use crate::domain::workflow::{STEP_STATE_ABORTED, STEP_STATE_COMPLETED};

/// [05] internal dispatch path: engine 内部の node 完了 / 失敗 typed command の
/// 単一 commit 関数。`InternalNodeCommand::CompleteNode` / `FailNode` を受け取り、
/// 対応する state mutation を snapshot に適用したうえで
/// `WorkflowEvent::NodeCompleted` / `NodeFailed` を返す。
pub(crate) fn dispatch_internal_node_command(
    snapshot: &mut WorkflowState,
    command: InternalNodeCommand,
) -> Result<WorkflowEvent, WorkflowEngineError> {
    apply_internal_node_command_state_mutation(snapshot, &command)?;
    map_internal_node_command_to_event(command)
}

fn apply_internal_node_command_state_mutation(
    snapshot: &mut WorkflowState,
    command: &InternalNodeCommand,
) -> Result<(), WorkflowEngineError> {
    match command {
        InternalNodeCommand::CompleteNode {
            run_id,
            workflow_name,
            node_name,
            result,
            session_id,
            token_usage,
            structured_output,
            run_index,
            timestamp,
        } => {
            if run_id != &snapshot.execution_id {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode run_id mismatch: command='{run_id}', snapshot='{}'",
                    snapshot.execution_id
                )));
            }
            if workflow_name != &snapshot.workflow_name {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode workflow_name mismatch: command='{workflow_name}', snapshot='{}'",
                    snapshot.workflow_name
                )));
            }
            let Some(last_entry) = snapshot.step_history.last() else {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode for node '{node_name}' but snapshot.step_history is empty"
                )));
            };
            if last_entry.step_name != *node_name {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode node mismatch: command='{node_name}', snapshot last='{}'",
                    last_entry.step_name
                )));
            }
            if (last_entry.completed_at - *timestamp).abs() > f64::EPSILON {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode timestamp mismatch for node '{node_name}': command={timestamp}, snapshot={}",
                    last_entry.completed_at
                )));
            }
            if last_entry.result != *result {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode result mismatch for node '{node_name}'"
                )));
            }
            if last_entry.session_id != *session_id {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode session_id mismatch for node '{node_name}'"
                )));
            }
            if last_entry.token_usage != *token_usage {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode token_usage mismatch for node '{node_name}'"
                )));
            }
            if last_entry.structured_output != *structured_output {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode structured_output mismatch for node '{node_name}'"
                )));
            }
            if Some(last_entry.run_index) != *run_index {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode run_index mismatch for node '{node_name}': command={run_index:?}, snapshot={}",
                    last_entry.run_index
                )));
            }
            Ok(())
        }
        InternalNodeCommand::FailNode {
            run_id,
            workflow_name,
            node_name,
            reason,
            failure_kind,
            retry_count,
            timestamp,
        } => {
            if run_id != &snapshot.execution_id {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "FailNode run_id mismatch: command='{run_id}', snapshot='{}'",
                    snapshot.execution_id
                )));
            }
            if workflow_name != &snapshot.workflow_name {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "FailNode workflow_name mismatch: command='{workflow_name}', snapshot='{}'",
                    snapshot.workflow_name
                )));
            }
            if *node_name != snapshot.current_step_name {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "FailNode node_name mismatch: command='{node_name}', snapshot='{}'",
                    snapshot.current_step_name
                )));
            }
            if !matches!(snapshot.state, WorkflowExecutionState::Failed { .. }) {
                snapshot.state = WorkflowExecutionState::Failed {
                    reason: reason.clone(),
                    kind: *failure_kind,
                    retry_count: *retry_count,
                };
                snapshot.updated_at = *timestamp;
            }
            Ok(())
        }
    }
}

fn map_internal_node_command_to_event(
    command: InternalNodeCommand,
) -> Result<WorkflowEvent, WorkflowEngineError> {
    match command {
        InternalNodeCommand::CompleteNode {
            run_id,
            workflow_name,
            node_name,
            result,
            session_id,
            token_usage,
            structured_output,
            run_index,
            timestamp,
        } => Ok(WorkflowEvent::NodeCompleted {
            run_id,
            workflow_name,
            node_name,
            result,
            session_id,
            token_usage,
            structured_output,
            run_index,
            timestamp,
        }),
        InternalNodeCommand::FailNode {
            run_id,
            workflow_name,
            node_name,
            reason,
            failure_kind,
            retry_count,
            timestamp,
        } => Ok(WorkflowEvent::NodeFailed {
            run_id,
            workflow_name,
            node_name,
            reason,
            failure_kind,
            retry_count,
            timestamp,
        }),
    }
}

pub(crate) fn cli_mutation_requested_event(
    workflow_name: &str,
    context: CommandCommitContext,
    timestamp: f64,
) -> Option<WorkflowEvent> {
    let mutation = context.into_cli_pending_mutation()?;
    let (run_id, request, requested_at, request_id) = mutation.into_event_parts();
    Some(WorkflowEvent::CliMutationRequested {
        run_id,
        workflow_name: workflow_name.to_string(),
        request_id,
        request,
        requested_at,
        timestamp,
    })
}

pub(crate) fn cli_mutation_rejected_event(
    workflow_name: String,
    context: &CommandCommitContext,
    error: &WorkflowEngineError,
    timestamp: f64,
) -> Result<WorkflowEvent, WorkflowEngineError> {
    let CommandCommitContext::CliPending { mutation } = context else {
        return Err(WorkflowEngineError::InvalidState(
            "append_cli_mutation_rejected requires CliPending context".to_string(),
        ));
    };
    let (run_id, request, requested_at, request_id) = mutation.clone().into_event_parts();
    Ok(WorkflowEvent::CliMutationRejected {
        run_id,
        workflow_name,
        request_id,
        request,
        reason: classify_cli_mutation_rejection_reason(error),
        message: error.to_string(),
        requested_at,
        timestamp,
    })
}

pub(crate) fn submit_output_cli_mutation_rejected_event(
    workflow_name: String,
    run_id: &str,
    context: &CommandCommitContext,
    error: &WorkflowEngineError,
    timestamp: f64,
) -> Result<WorkflowEvent, WorkflowEngineError> {
    let (request_id, requested_at, step_name, contract) =
        context.submit_output_rejection_parts().ok_or_else(|| {
            WorkflowEngineError::InvalidState(
                "append_cli_mutation_rejected_for_submit_output requires SubmitOutput context"
                    .to_string(),
            )
        })?;
    Ok(WorkflowEvent::CliMutationRejected {
        run_id: run_id.to_string(),
        workflow_name,
        request_id,
        request: CliMutationRequestRecord::SubmitOutput {
            step_name,
            contract,
        },
        reason: classify_cli_mutation_rejection_reason(error),
        message: error.to_string(),
        requested_at,
        timestamp,
    })
}

pub(crate) fn pre_commit_required_events_for_outcome(
    outcome: &StepOutcome,
) -> Result<Vec<WorkflowEvent>, WorkflowEngineError> {
    let mut events = Vec::new();
    let mut snapshot = outcome.snapshot().clone();
    match outcome {
        StepOutcome::Persist(s) => {
            let is_terminal = matches!(
                s.state,
                WorkflowExecutionState::Completed | WorkflowExecutionState::Failed { .. }
            );
            if is_terminal {
                return terminal_required_events_for_snapshot(s);
            }
        }
        StepOutcome::RetryCurrentStep { snapshot, .. } => {
            events.push(node_started_event_for_snapshot(snapshot));
        }
        StepOutcome::TransitionAndStart(_)
        | StepOutcome::ReduceAndTransition(_)
        | StepOutcome::StartParallel(_) => {
            if let Some(ev) = last_step_completed_event_for_snapshot(&mut snapshot)? {
                events.push(ev);
            }
        }
    }
    Ok(events)
}

pub(crate) fn terminal_required_events_for_snapshot(
    snapshot: &WorkflowState,
) -> Result<Vec<WorkflowEvent>, WorkflowEngineError> {
    let mut local = snapshot.clone();
    let mut events = Vec::new();
    if matches!(snapshot.state, WorkflowExecutionState::Completed) {
        if let Some(event) = last_step_completed_event_for_snapshot(&mut local)? {
            events.push(event);
        }
    }
    events.extend(terminal_events_for_snapshot(&mut local)?);
    Ok(events)
}

pub(crate) fn last_step_completed_event_for_append(
    snapshot: &WorkflowState,
) -> Result<Option<WorkflowEvent>, String> {
    let mut local = snapshot.clone();
    last_step_completed_event_for_snapshot(&mut local).map_err(|e| format!("{e:?}"))
}

pub(crate) fn terminal_events_for_append(
    snapshot: &WorkflowState,
) -> Result<Vec<WorkflowEvent>, String> {
    let mut local = snapshot.clone();
    terminal_events_for_snapshot(&mut local).map_err(|e| format!("{e:?}"))
}

pub(crate) fn last_step_completed_event_for_snapshot(
    snapshot: &mut WorkflowState,
) -> Result<Option<WorkflowEvent>, WorkflowEngineError> {
    let Some(last_entry) = snapshot.step_history.last().cloned() else {
        return Ok(None);
    };
    let command = InternalNodeCommand::CompleteNode {
        run_id: snapshot.execution_id.clone(),
        workflow_name: snapshot.workflow_name.clone(),
        node_name: last_entry.step_name,
        result: last_entry.result,
        session_id: last_entry.session_id,
        token_usage: last_entry.token_usage,
        structured_output: last_entry.structured_output,
        run_index: Some(last_entry.run_index),
        timestamp: last_entry.completed_at,
    };
    dispatch_internal_node_command(snapshot, command).map(Some)
}

pub(crate) fn node_started_event_for_snapshot(snapshot: &WorkflowState) -> WorkflowEvent {
    let exec_count = snapshot
        .step_execution_counts
        .get(&snapshot.current_step_name)
        .copied()
        .unwrap_or(1);
    WorkflowEvent::NodeStarted {
        run_id: snapshot.execution_id.clone(),
        workflow_name: snapshot.workflow_name.clone(),
        node_name: snapshot.current_step_name.clone(),
        execution_count: exec_count,
        timestamp: snapshot.updated_at,
    }
}

pub(crate) fn node_session_started_event_for_snapshot(
    snapshot: &WorkflowState,
) -> Option<WorkflowEvent> {
    let session_id = snapshot.current_session_id.clone()?;
    let exec_count = snapshot
        .step_execution_counts
        .get(&snapshot.current_step_name)
        .copied()
        .unwrap_or(1);
    Some(WorkflowEvent::StepSessionStarted {
        run_id: snapshot.execution_id.clone(),
        workflow_name: snapshot.workflow_name.clone(),
        node_name: snapshot.current_step_name.clone(),
        execution_count: exec_count,
        session_id,
        timestamp: snapshot.updated_at,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostCommitProgressEventPlan {
    TransitionAndStart,
    ReduceAndTransition,
    StartParallel,
}

impl PostCommitProgressEventPlan {
    fn outcome_label(self) -> &'static str {
        match self {
            Self::TransitionAndStart => "TransitionAndStart",
            Self::ReduceAndTransition => "ReduceAndTransition",
            Self::StartParallel => "StartParallel",
        }
    }

    pub(crate) fn node_completed_append_error(
        self,
        error: impl std::fmt::Display,
    ) -> WorkflowEngineError {
        WorkflowEngineError::SessionStore(format!(
            "{} pre-commit NodeCompleted append failed: {error}",
            self.outcome_label()
        ))
    }

    pub(crate) fn followup_event(self, snapshot: &WorkflowState) -> Option<WorkflowEvent> {
        match self {
            Self::TransitionAndStart => Some(node_started_event_for_snapshot(snapshot)),
            Self::ReduceAndTransition | Self::StartParallel => None,
        }
    }
}

pub(crate) fn parallel_started_event_for_snapshot(
    snapshot: &WorkflowState,
) -> Option<WorkflowEvent> {
    let node = snapshot
        .workflow_definition
        .nodes
        .get(snapshot.current_step_index)?;
    let child_node_names: Vec<String> = node
        .fanout()?
        .parallel_children
        .iter()
        .map(|child| child.name.clone())
        .collect();
    Some(WorkflowEvent::ParallelStarted {
        run_id: snapshot.execution_id.clone(),
        workflow_name: snapshot.workflow_name.clone(),
        parent_node_name: snapshot.current_step_name.clone(),
        child_node_names,
        timestamp: snapshot.updated_at,
    })
}

pub(crate) fn terminal_events_for_snapshot(
    snapshot: &mut WorkflowState,
) -> Result<Vec<WorkflowEvent>, WorkflowEngineError> {
    match snapshot.state.clone() {
        WorkflowExecutionState::Completed => Ok(vec![WorkflowEvent::RunCompleted {
            run_id: snapshot.execution_id.clone(),
            workflow_name: snapshot.workflow_name.clone(),
            total_token_usage: snapshot.total_token_usage.clone(),
            timestamp: snapshot.updated_at,
        }]),
        WorkflowExecutionState::Interrupted => Ok(vec![WorkflowEvent::RunInterrupted {
            run_id: snapshot.execution_id.clone(),
            workflow_name: snapshot.workflow_name.clone(),
            reason: "interrupted".to_string(),
            timestamp: snapshot.updated_at,
        }]),
        WorkflowExecutionState::Failed {
            reason,
            kind,
            retry_count,
        } => {
            let run_id = snapshot.execution_id.clone();
            let workflow_name = snapshot.workflow_name.clone();
            let node_name = snapshot.current_step_name.clone();
            let failure_kind = kind;
            let timestamp = snapshot.updated_at;
            let fail_command = InternalNodeCommand::FailNode {
                run_id: run_id.clone(),
                workflow_name: workflow_name.clone(),
                node_name,
                reason: reason.clone(),
                failure_kind,
                retry_count,
                timestamp,
            };
            let node_failed = dispatch_internal_node_command(snapshot, fail_command)?;
            Ok(vec![
                node_failed,
                WorkflowEvent::RunFailed {
                    run_id,
                    workflow_name,
                    reason,
                    failure_kind,
                    retry_count,
                    timestamp,
                },
            ])
        }
        _ => Ok(Vec::new()),
    }
}

pub(crate) fn required_events_for_approval_commit(
    approval_event: WorkflowEvent,
    outcome: &mut StepOutcome,
) -> Result<Vec<WorkflowEvent>, WorkflowEngineError> {
    let mut events = vec![approval_event];
    match outcome {
        StepOutcome::Persist(snapshot) => {
            let is_terminal = matches!(
                snapshot.state,
                WorkflowExecutionState::Completed | WorkflowExecutionState::Failed { .. }
            );
            if is_terminal {
                events.extend(terminal_required_events_for_snapshot(snapshot)?);
            } else if matches!(snapshot.state, WorkflowExecutionState::Aborted) {
                events.push(WorkflowEvent::RunAborted {
                    run_id: snapshot.execution_id.clone(),
                    workflow_name: snapshot.workflow_name.clone(),
                    aborted_step: snapshot
                        .step_history
                        .last()
                        .filter(|entry| entry.state == STEP_STATE_ABORTED)
                        .map(run_aborted_step_snapshot_from_history_entry),
                    timestamp: snapshot.updated_at,
                });
            }
        }
        StepOutcome::RetryCurrentStep { snapshot, .. } => {
            events.push(node_started_event_for_snapshot(snapshot));
        }
        StepOutcome::TransitionAndStart(snapshot) => {
            if let Some(event) = last_step_completed_event_for_snapshot(snapshot)? {
                events.push(event);
            }
            events.push(node_started_event_for_snapshot(snapshot));
        }
        StepOutcome::ReduceAndTransition(snapshot) => {
            if let Some(event) = last_step_completed_event_for_snapshot(snapshot)? {
                events.push(event);
            }
            events.push(node_started_event_for_snapshot(snapshot));
        }
        StepOutcome::StartParallel(snapshot) => {
            if let Some(event) = last_step_completed_event_for_snapshot(snapshot)? {
                events.push(event);
            }
            if let Some(event) = parallel_started_event_for_snapshot(snapshot) {
                events.push(event);
            }
        }
    }
    let commit_timestamp = events
        .iter()
        .map(workflow_event_timestamp)
        .fold(0.0_f64, f64::max);
    for event in &mut events {
        set_workflow_event_timestamp(event, commit_timestamp);
    }
    Ok(events)
}

pub(crate) fn run_aborted_step_snapshot_from_history_entry(
    entry: &StepHistoryEntry,
) -> RunAbortedStepSnapshot {
    RunAbortedStepSnapshot {
        step_name: entry.step_name.clone(),
        completed_at: entry.completed_at,
        result: entry.result.clone(),
        session_id: entry.session_id.clone(),
        token_usage: entry.token_usage.clone(),
        structured_output: entry.structured_output.clone(),
        run_index: entry.run_index,
        child_outputs: entry.child_outputs.as_ref().map(|children| {
            children
                .iter()
                .map(run_aborted_child_snapshot_from_child_output)
                .collect()
        }),
    }
}

fn run_aborted_child_snapshot_from_child_output(
    child: &ChildOutputSnapshot,
) -> RunAbortedChildOutputSnapshot {
    RunAbortedChildOutputSnapshot {
        step_name: child.step_name.clone(),
        session_id: child.session_id.clone(),
        result: child.result.clone(),
        run_index: child.run_index,
        completed_at: child.completed_at,
        structured_output: child.structured_output.clone(),
        artifact_contract: child.artifact_contract.clone(),
        outcome: run_aborted_child_outcome_from_state(&child.state),
    }
}

fn run_aborted_child_outcome_from_state(state: &str) -> RunAbortedChildOutcome {
    if state == STEP_STATE_COMPLETED {
        RunAbortedChildOutcome::Completed
    } else {
        RunAbortedChildOutcome::Aborted
    }
}

pub(crate) fn workflow_event_timestamp(event: &WorkflowEvent) -> f64 {
    match event {
        WorkflowEvent::RunStarted { timestamp, .. }
        | WorkflowEvent::NodeStarted { timestamp, .. }
        | WorkflowEvent::StepSessionStarted { timestamp, .. }
        | WorkflowEvent::WorkflowStallObserved { timestamp, .. }
        | WorkflowEvent::WorkflowStallCleared { timestamp, .. }
        | WorkflowEvent::NodeCompleted { timestamp, .. }
        | WorkflowEvent::NodeFailed { timestamp, .. }
        | WorkflowEvent::ApprovalRequested { timestamp, .. }
        | WorkflowEvent::ApprovalResolved { timestamp, .. }
        | WorkflowEvent::RunCompleted { timestamp, .. }
        | WorkflowEvent::RunFailed { timestamp, .. }
        | WorkflowEvent::RunAborted { timestamp, .. }
        | WorkflowEvent::RunInterrupted { timestamp, .. }
        | WorkflowEvent::OutputCollected { timestamp, .. }
        | WorkflowEvent::ParallelStarted { timestamp, .. }
        | WorkflowEvent::ParallelChildStarted { timestamp, .. }
        | WorkflowEvent::ParallelChildCompleted { timestamp, .. }
        | WorkflowEvent::ParallelCompleted { timestamp, .. }
        | WorkflowEvent::ContractRepairRequested { timestamp, .. }
        | WorkflowEvent::CliMutationRequested { timestamp, .. }
        | WorkflowEvent::ArtifactProduced { timestamp, .. }
        | WorkflowEvent::CliMutationRejected { timestamp, .. } => *timestamp,
    }
}

pub(crate) fn set_workflow_event_timestamp(event: &mut WorkflowEvent, commit_timestamp: f64) {
    match event {
        WorkflowEvent::RunStarted { timestamp, .. }
        | WorkflowEvent::NodeStarted { timestamp, .. }
        | WorkflowEvent::StepSessionStarted { timestamp, .. }
        | WorkflowEvent::WorkflowStallObserved { timestamp, .. }
        | WorkflowEvent::WorkflowStallCleared { timestamp, .. }
        | WorkflowEvent::NodeCompleted { timestamp, .. }
        | WorkflowEvent::NodeFailed { timestamp, .. }
        | WorkflowEvent::ApprovalRequested { timestamp, .. }
        | WorkflowEvent::ApprovalResolved { timestamp, .. }
        | WorkflowEvent::RunCompleted { timestamp, .. }
        | WorkflowEvent::RunFailed { timestamp, .. }
        | WorkflowEvent::RunAborted { timestamp, .. }
        | WorkflowEvent::RunInterrupted { timestamp, .. }
        | WorkflowEvent::OutputCollected { timestamp, .. }
        | WorkflowEvent::ParallelStarted { timestamp, .. }
        | WorkflowEvent::ParallelChildStarted { timestamp, .. }
        | WorkflowEvent::ParallelChildCompleted { timestamp, .. }
        | WorkflowEvent::ParallelCompleted { timestamp, .. }
        | WorkflowEvent::ContractRepairRequested { timestamp, .. }
        | WorkflowEvent::CliMutationRequested { timestamp, .. }
        | WorkflowEvent::ArtifactProduced { timestamp, .. }
        | WorkflowEvent::CliMutationRejected { timestamp, .. } => *timestamp = commit_timestamp,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::adaptor::gateway::workflow::schema::Workflow;
    use crate::adaptor::gateway::workflow::state::{
        default_step_entry_state, ChildOutputSnapshot, StepHistoryEntry, TokenUsage,
    };
    use crate::{
        adaptor::gateway::workflow::event::CliMutationRejectionReason,
        adaptor::gateway::workflow::route_context::{
            WorkflowMutationContext, WorkflowMutationSource,
        },
    };

    fn workflow_state_fixture() -> WorkflowState {
        WorkflowState {
            execution_id: "run-1".to_string(),
            workflow_name: "wf".to_string(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            current_step_name: "implement".to_string(),
            current_session_id: None,
            total_steps: 1,
            step_history: Vec::new(),
            step_execution_counts: HashMap::from([("implement".to_string(), 3)]),
            workflow_definition: Workflow::default(),
            total_token_usage: TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: Vec::new(),
            stall_observations: Vec::new(),
            approval_operations: None,
            started_at: 1.0,
            updated_at: 42.0,
        }
    }

    #[test]
    fn terminal_required_events_for_completed_snapshot_includes_last_node_then_run_completed() {
        let mut snapshot = workflow_state_fixture();
        snapshot.state = WorkflowExecutionState::Completed;
        snapshot.step_history.push(StepHistoryEntry {
            step_name: "implement".to_string(),
            completed_at: 41.0,
            result: Some("done".to_string()),
            session_id: Some("session-1".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: 3,
            child_outputs: None,
            state: default_step_entry_state(),
        });

        let events = terminal_required_events_for_snapshot(&snapshot).unwrap();

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            WorkflowEvent::NodeCompleted {
                run_id,
                workflow_name,
                node_name,
                result,
                session_id,
                run_index,
                timestamp,
                ..
            } if run_id == "run-1"
                && workflow_name == "wf"
                && node_name == "implement"
                && result.as_deref() == Some("done")
                && session_id.as_deref() == Some("session-1")
                && *run_index == Some(3)
                && (*timestamp - 41.0).abs() < f64::EPSILON
        ));
        assert!(matches!(
            &events[1],
            WorkflowEvent::RunCompleted {
                run_id,
                workflow_name,
                timestamp,
                ..
            } if run_id == "run-1"
                && workflow_name == "wf"
                && (*timestamp - 42.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn terminal_required_events_for_failed_snapshot_preserves_retry_count() {
        let mut snapshot = workflow_state_fixture();
        snapshot.state = WorkflowExecutionState::Failed {
            reason: "startup exhausted".to_string(),
            kind: crate::domain::workflow::WorkflowStepFailureKind::StartupTimeout,
            retry_count: Some(2),
        };

        let events = terminal_required_events_for_snapshot(&snapshot).unwrap();

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            WorkflowEvent::NodeFailed {
                failure_kind,
                retry_count,
                ..
            } if *failure_kind == crate::domain::workflow::WorkflowStepFailureKind::StartupTimeout
                && *retry_count == Some(2)
        ));
        assert!(matches!(
            &events[1],
            WorkflowEvent::RunFailed {
                failure_kind,
                retry_count,
                ..
            } if *failure_kind == crate::domain::workflow::WorkflowStepFailureKind::StartupTimeout
                && *retry_count == Some(2)
        ));
    }

    #[test]
    fn pre_commit_required_events_for_retry_current_step_includes_node_started() {
        let snapshot = workflow_state_fixture();
        let events = pre_commit_required_events_for_outcome(&StepOutcome::RetryCurrentStep {
            snapshot,
            completed_session_id: Some("previous-session".to_string()),
        })
        .unwrap();

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            WorkflowEvent::NodeStarted {
                run_id,
                workflow_name,
                node_name,
                execution_count,
                timestamp,
            } if run_id == "run-1"
                && workflow_name == "wf"
                && node_name == "implement"
                && *execution_count == 3
                && (*timestamp - 42.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn run_aborted_snapshot_maps_child_display_state_to_typed_outcome() {
        let entry = StepHistoryEntry {
            step_name: "parallel-review".to_string(),
            completed_at: 42.0,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: Some(vec![
                ChildOutputSnapshot {
                    step_name: "child-a".to_string(),
                    session_id: Some("session-a".to_string()),
                    result: Some("ok".to_string()),
                    run_index: 1,
                    completed_at: 40.0,
                    structured_output: None,
                    artifact_contract: None,
                    state: STEP_STATE_COMPLETED.to_string(),
                    failure_kind: None,
                    failure_disposition: None,
                },
                ChildOutputSnapshot {
                    step_name: "child-b".to_string(),
                    session_id: Some("session-b".to_string()),
                    result: None,
                    run_index: 1,
                    completed_at: 42.0,
                    structured_output: None,
                    artifact_contract: None,
                    state: STEP_STATE_ABORTED.to_string(),
                    failure_kind: None,
                    failure_disposition: None,
                },
            ]),
            state: STEP_STATE_ABORTED.to_string(),
        };

        let snapshot = run_aborted_step_snapshot_from_history_entry(&entry);
        let children = snapshot.child_outputs.expect("child outputs are preserved");

        assert_eq!(children[0].outcome, RunAbortedChildOutcome::Completed);
        assert_eq!(children[1].outcome, RunAbortedChildOutcome::Aborted);
    }

    #[test]
    fn post_commit_progress_event_plan_emits_only_next_node_start() {
        let snapshot = workflow_state_fixture();

        assert!(matches!(
            PostCommitProgressEventPlan::TransitionAndStart.followup_event(&snapshot),
            Some(WorkflowEvent::NodeStarted {
                run_id,
                workflow_name,
                node_name,
                execution_count,
                timestamp,
            }) if run_id == "run-1"
                && workflow_name == "wf"
                && node_name == "implement"
                && execution_count == 3
                && (timestamp - 42.0).abs() < f64::EPSILON
        ));
        assert!(PostCommitProgressEventPlan::ReduceAndTransition
            .followup_event(&snapshot)
            .is_none());
        assert!(PostCommitProgressEventPlan::StartParallel
            .followup_event(&snapshot)
            .is_none());
    }

    #[test]
    fn post_commit_progress_event_plan_formats_node_completed_error() {
        let error = PostCommitProgressEventPlan::StartParallel
            .node_completed_append_error("append failed")
            .to_string();

        assert_eq!(
            error,
            "StartParallel pre-commit NodeCompleted append failed: append failed"
        );
    }

    #[test]
    fn cli_mutation_requested_event_builds_cli_pending_event_only() {
        let context = CommandCommitContext::cli_pending(WorkflowMutationContext::new(
            "run-1".to_string(),
            WorkflowMutationSource::CliPendingCommand {
                request_id: "request-1".to_string(),
            },
            CliMutationRequestRecord::Abort { node_name: None },
            10.0,
        ));

        let event = cli_mutation_requested_event("wf", context, 20.0);

        assert!(matches!(
            event,
            Some(WorkflowEvent::CliMutationRequested {
                run_id,
                workflow_name,
                request_id,
                request: CliMutationRequestRecord::Abort { node_name: None },
                requested_at,
                timestamp,
            }) if run_id == "run-1"
                && workflow_name == "wf"
                && request_id == "request-1"
                && (requested_at - 10.0).abs() < f64::EPSILON
                && (timestamp - 20.0).abs() < f64::EPSILON
        ));

        assert!(cli_mutation_requested_event(
            "wf",
            CommandCommitContext::submit_output(
                "request-2".to_string(),
                30.0,
                "review".to_string(),
                "review-verdict".to_string(),
            ),
            40.0,
        )
        .is_none());
    }

    #[test]
    fn cli_mutation_rejected_event_classifies_cli_pending_error() {
        let context = CommandCommitContext::cli_pending(WorkflowMutationContext::new(
            "run-2".to_string(),
            WorkflowMutationSource::CliPendingCommand {
                request_id: "request-2".to_string(),
            },
            CliMutationRequestRecord::Reject {
                node_name: Some("approval".to_string()),
                reason: "no".to_string(),
            },
            50.0,
        ));

        let event = cli_mutation_rejected_event(
            "wf".to_string(),
            &context,
            &WorkflowEngineError::InvalidState("run is already terminal".to_string()),
            60.0,
        )
        .unwrap();

        assert!(matches!(
            event,
            WorkflowEvent::CliMutationRejected {
                run_id,
                workflow_name,
                request_id,
                request: CliMutationRequestRecord::Reject { .. },
                reason: CliMutationRejectionReason::RunNotActive,
                requested_at,
                timestamp,
                ..
            } if run_id == "run-2"
                && workflow_name == "wf"
                && request_id == "request-2"
                && (requested_at - 50.0).abs() < f64::EPSILON
                && (timestamp - 60.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn submit_output_cli_mutation_rejected_event_uses_compact_request_record() {
        let context = CommandCommitContext::submit_output(
            "request-3".to_string(),
            70.0,
            "review".to_string(),
            "review-verdict".to_string(),
        );

        let event = submit_output_cli_mutation_rejected_event(
            "wf".to_string(),
            "run-3",
            &context,
            &WorkflowEngineError::ValidationError("contract mismatch".to_string()),
            80.0,
        )
        .unwrap();

        assert!(matches!(
            event,
            WorkflowEvent::CliMutationRejected {
                run_id,
                workflow_name,
                request_id,
                request: CliMutationRequestRecord::SubmitOutput {
                    step_name,
                    contract,
                },
                reason: CliMutationRejectionReason::ContractMismatch,
                requested_at,
                timestamp,
                ..
            } if run_id == "run-3"
                && workflow_name == "wf"
                && request_id == "request-3"
                && step_name == "review"
                && contract == "review-verdict"
                && (requested_at - 70.0).abs() < f64::EPSILON
                && (timestamp - 80.0).abs() < f64::EPSILON
        ));
    }
}
