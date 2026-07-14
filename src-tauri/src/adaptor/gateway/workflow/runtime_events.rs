use crate::adaptor::gateway::workflow::engine_error::{
    classify_cli_mutation_rejection_reason, WorkflowEngineError,
};
use crate::adaptor::gateway::workflow::event::{CliMutationRequestRecord, WorkflowEvent};
use crate::adaptor::gateway::workflow::internal_node_command::InternalNodeCommand;
use crate::adaptor::gateway::workflow::route_context::CommandCommitContext;
use crate::adaptor::gateway::workflow::runtime_commit::StepOutcome;
use crate::adaptor::gateway::workflow::state::{RuntimeExecutionState, WorkflowState};
use crate::domain::workflow::NODE_STATUS_ABORTED;

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
            execution_id,
            workflow_name,
            node_execution_id,
            node_name,
            result,
            session_id,
            token_usage,
            artifact,
            attempt,
            timestamp,
        } => {
            if execution_id != &snapshot.execution_id {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode execution_id mismatch: command='{execution_id}', snapshot='{}'",
                    snapshot.execution_id
                )));
            }
            if workflow_name != &snapshot.workflow_name {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode workflow_name mismatch: command='{workflow_name}', snapshot='{}'",
                    snapshot.workflow_name
                )));
            }
            validate_top_level_node_execution(
                snapshot,
                node_execution_id,
                node_name,
                *attempt,
                "CompleteNode",
            )?;
            let Some(last_entry) = snapshot.node_history.last() else {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode for node '{node_name}' but snapshot.node_history is empty"
                )));
            };
            if last_entry.node_name != *node_name {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode node mismatch: command='{node_name}', snapshot last='{}'",
                    last_entry.node_name
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
            if last_entry.artifact != *artifact {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode artifact mismatch for node '{node_name}'"
                )));
            }
            if Some(last_entry.attempt) != *attempt {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "CompleteNode attempt mismatch for node '{node_name}': command={attempt:?}, snapshot={}",
                    last_entry.attempt
                )));
            }
            Ok(())
        }
        InternalNodeCommand::FailNode {
            execution_id,
            workflow_name,
            node_execution_id,
            node_name,
            attempt,
            reason,
            failure_kind,
            retry_count,
            timestamp,
        } => {
            if execution_id != &snapshot.execution_id {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "FailNode execution_id mismatch: command='{execution_id}', snapshot='{}'",
                    snapshot.execution_id
                )));
            }
            if workflow_name != &snapshot.workflow_name {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "FailNode workflow_name mismatch: command='{workflow_name}', snapshot='{}'",
                    snapshot.workflow_name
                )));
            }
            validate_top_level_node_execution(
                snapshot,
                node_execution_id,
                node_name,
                Some(*attempt),
                "FailNode",
            )?;
            if *node_name != snapshot.current_node_name {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "FailNode node_name mismatch: command='{node_name}', snapshot='{}'",
                    snapshot.current_node_name
                )));
            }
            if !matches!(snapshot.state, RuntimeExecutionState::Failed { .. }) {
                snapshot.state = RuntimeExecutionState::Failed {
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

fn validate_top_level_node_execution(
    snapshot: &WorkflowState,
    node_execution_id: &str,
    node_name: &str,
    attempt: Option<u32>,
    command_name: &str,
) -> Result<(), WorkflowEngineError> {
    let execution = snapshot
        .node_executions
        .iter()
        .find(|execution| execution.id == node_execution_id)
        .ok_or_else(|| {
            WorkflowEngineError::ValidationError(format!(
                "{command_name} references unknown node_execution_id '{node_execution_id}'"
            ))
        })?;
    if execution.execution_id != snapshot.execution_id
        || execution.node_name != node_name
        || execution.fanout_parent.is_some()
        || attempt.is_some_and(|attempt| execution.attempt != attempt)
    {
        return Err(WorkflowEngineError::ValidationError(format!(
            "{command_name} node execution mismatch: id='{node_execution_id}', node='{node_name}', attempt={attempt:?}"
        )));
    }
    Ok(())
}

fn map_internal_node_command_to_event(
    command: InternalNodeCommand,
) -> Result<WorkflowEvent, WorkflowEngineError> {
    match command {
        InternalNodeCommand::CompleteNode {
            execution_id,
            workflow_name: _,
            node_execution_id,
            node_name,
            result,
            session_id: _,
            token_usage,
            artifact: _,
            attempt,
            timestamp,
        } => {
            let attempt = attempt.unwrap_or(1);
            Ok(WorkflowEvent::NodeCompleted {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                result_summary: result,
                token_usage,
                timestamp,
            })
        }
        InternalNodeCommand::FailNode {
            execution_id,
            workflow_name: _,
            node_execution_id,
            node_name,
            attempt,
            reason,
            failure_kind,
            retry_count,
            timestamp,
        } => Ok(WorkflowEvent::NodeFailed {
            execution_id,
            node_execution_id,
            node_name,
            attempt,
            reason,
            failure_kind,
            retry_count,
            timestamp,
        }),
    }
}

pub(crate) fn cli_mutation_requested_event(
    _workflow_name: &str,
    context: CommandCommitContext,
    timestamp: f64,
) -> Option<WorkflowEvent> {
    let mutation = context.into_cli_pending_mutation()?;
    let (execution_id, request, requested_at, request_id) = mutation.into_event_parts();
    Some(WorkflowEvent::CliMutationRequested {
        execution_id,
        request_id,
        request,
        requested_at,
        timestamp,
    })
}

pub(crate) fn cli_mutation_rejected_event(
    _workflow_name: String,
    context: &CommandCommitContext,
    error: &WorkflowEngineError,
    timestamp: f64,
) -> Result<WorkflowEvent, WorkflowEngineError> {
    let CommandCommitContext::CliPending { mutation } = context else {
        return Err(WorkflowEngineError::InvalidState(
            "append_cli_mutation_rejected requires CliPending context".to_string(),
        ));
    };
    let (execution_id, request, requested_at, request_id) = mutation.clone().into_event_parts();
    Ok(WorkflowEvent::CliMutationRejected {
        execution_id,
        request_id,
        request,
        reason: classify_cli_mutation_rejection_reason(error),
        message: error.to_string(),
        requested_at,
        timestamp,
    })
}

pub(crate) fn submit_output_cli_mutation_rejected_event(
    _workflow_name: String,
    execution_id: &str,
    context: &CommandCommitContext,
    error: &WorkflowEngineError,
    timestamp: f64,
) -> Result<WorkflowEvent, WorkflowEngineError> {
    let (request_id, requested_at, node_name, contract) =
        context.submit_output_rejection_parts().ok_or_else(|| {
            WorkflowEngineError::InvalidState(
                "append_cli_mutation_rejected_for_submit_output requires SubmitOutput context"
                    .to_string(),
            )
        })?;
    Ok(WorkflowEvent::CliMutationRejected {
        execution_id: execution_id.to_string(),
        request_id,
        request: CliMutationRequestRecord::SubmitOutput {
            node_name,
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
                RuntimeExecutionState::Completed | RuntimeExecutionState::Failed { .. }
            );
            if is_terminal {
                return terminal_required_events_for_snapshot(s);
            }
        }
        StepOutcome::RetryCurrentStep { snapshot, .. } => {
            events.push(node_started_event_for_snapshot(snapshot)?);
        }
        StepOutcome::TransitionAndStart(_) => {
            if let Some(ev) = last_step_completed_event_for_snapshot(&mut snapshot)? {
                events.push(ev);
            }
            events.push(node_started_event_for_snapshot(&snapshot)?);
        }
        StepOutcome::StartParallel(_) => {
            if let Some(ev) = last_step_completed_event_for_snapshot(&mut snapshot)? {
                events.push(ev);
            }
            events.push(fanout_parent_started_event_for_snapshot(&snapshot)?);
        }
    }
    Ok(events)
}

pub(crate) fn terminal_required_events_for_snapshot(
    snapshot: &WorkflowState,
) -> Result<Vec<WorkflowEvent>, WorkflowEngineError> {
    let mut local = snapshot.clone();
    let mut events = Vec::new();
    if matches!(snapshot.state, RuntimeExecutionState::Completed) {
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
    let Some(last_entry) = snapshot.node_history.last().cloned() else {
        return Ok(None);
    };
    let command = InternalNodeCommand::CompleteNode {
        execution_id: snapshot.execution_id.clone(),
        workflow_name: snapshot.workflow_name.clone(),
        node_execution_id: top_level_node_execution_id(
            snapshot,
            &last_entry.node_name,
            last_entry.attempt,
        )?,
        node_name: last_entry.node_name,
        result: last_entry.result,
        session_id: last_entry.session_id,
        token_usage: last_entry.token_usage,
        artifact: last_entry.artifact,
        attempt: Some(last_entry.attempt),
        timestamp: last_entry.completed_at,
    };
    dispatch_internal_node_command(snapshot, command).map(Some)
}

fn current_node_attempt(snapshot: &WorkflowState) -> u32 {
    snapshot
        .node_execution_counts
        .get(&snapshot.current_node_name)
        .copied()
        .unwrap_or(1)
}

fn top_level_node_execution<'a>(
    snapshot: &'a WorkflowState,
    node_name: &str,
    attempt: u32,
) -> Result<&'a crate::adaptor::gateway::workflow::state::NodeExecution, WorkflowEngineError> {
    snapshot
        .node_executions
        .iter()
        .rev()
        .find(|execution| {
            execution.node_name == node_name
                && execution.attempt == attempt
                && execution.fanout_parent.is_none()
        })
        .ok_or_else(|| {
            WorkflowEngineError::InvalidState(format!(
                "top-level NodeExecution for node '{node_name}' attempt {attempt} is unavailable"
            ))
        })
}

fn top_level_node_execution_id(
    snapshot: &WorkflowState,
    node_name: &str,
    attempt: u32,
) -> Result<String, WorkflowEngineError> {
    Ok(top_level_node_execution(snapshot, node_name, attempt)?
        .id
        .clone())
}

pub(crate) fn node_started_event_for_snapshot(
    snapshot: &WorkflowState,
) -> Result<WorkflowEvent, WorkflowEngineError> {
    let attempt = current_node_attempt(snapshot);
    let execution = top_level_node_execution(snapshot, &snapshot.current_node_name, attempt)?;
    Ok(WorkflowEvent::NodeStarted {
        execution_id: snapshot.execution_id.clone(),
        node_execution_id: execution.id.clone(),
        node_name: snapshot.current_node_name.clone(),
        kind: execution.kind,
        attempt,
        fanout_parent: None,
        timestamp: snapshot.updated_at,
    })
}

pub(crate) fn node_session_started_event_for_snapshot(
    snapshot: &WorkflowState,
) -> Result<Option<WorkflowEvent>, WorkflowEngineError> {
    let Some(session_id) = snapshot.current_session_id.clone() else {
        return Ok(None);
    };
    let attempt = current_node_attempt(snapshot);
    let execution = top_level_node_execution(snapshot, &snapshot.current_node_name, attempt)?;
    Ok(Some(WorkflowEvent::SessionAttached {
        execution_id: snapshot.execution_id.clone(),
        node_execution_id: execution.id.clone(),
        session_id,
        timestamp: snapshot.updated_at,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostCommitProgressEventPlan {
    TransitionAndStart,
    StartParallel,
}

impl PostCommitProgressEventPlan {
    fn outcome_label(self) -> &'static str {
        match self {
            Self::TransitionAndStart => "TransitionAndStart",
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

    pub(crate) fn followup_event(
        self,
        snapshot: &WorkflowState,
    ) -> Result<Option<WorkflowEvent>, WorkflowEngineError> {
        match self {
            Self::TransitionAndStart => node_started_event_for_snapshot(snapshot).map(Some),
            Self::StartParallel => fanout_parent_started_event_for_snapshot(snapshot).map(Some),
        }
    }
}

pub(crate) fn fanout_parent_started_event_for_snapshot(
    snapshot: &WorkflowState,
) -> Result<WorkflowEvent, WorkflowEngineError> {
    let event = node_started_event_for_snapshot(snapshot)?;
    if !matches!(
        event,
        WorkflowEvent::NodeStarted {
            kind: crate::adaptor::gateway::workflow::schema::NodeKindName::Fanout,
            ..
        }
    ) {
        return Err(WorkflowEngineError::InvalidState(format!(
            "fanout parent '{}' does not reference a fanout NodeExecution",
            snapshot.current_node_name
        )));
    }
    Ok(event)
}

pub(crate) fn terminal_events_for_snapshot(
    snapshot: &mut WorkflowState,
) -> Result<Vec<WorkflowEvent>, WorkflowEngineError> {
    match snapshot.state.clone() {
        RuntimeExecutionState::Completed => Ok(vec![WorkflowEvent::ExecutionCompleted {
            execution_id: snapshot.execution_id.clone(),
            total_token_usage: snapshot.total_token_usage.clone(),
            timestamp: snapshot.updated_at,
        }]),
        RuntimeExecutionState::Interrupted => Ok(vec![WorkflowEvent::ExecutionInterrupted {
            execution_id: snapshot.execution_id.clone(),
            reason: "interrupted".to_string(),
            timestamp: snapshot.updated_at,
        }]),
        RuntimeExecutionState::Failed {
            reason,
            kind,
            retry_count,
        } => {
            let execution_id = snapshot.execution_id.clone();
            let workflow_name = snapshot.workflow_name.clone();
            let node_name = snapshot.current_node_name.clone();
            let failure_kind = kind;
            let timestamp = snapshot.updated_at;
            let fail_command = InternalNodeCommand::FailNode {
                execution_id: execution_id.clone(),
                workflow_name: workflow_name.clone(),
                node_execution_id: top_level_node_execution_id(
                    snapshot,
                    &node_name,
                    snapshot
                        .node_execution_counts
                        .get(&node_name)
                        .copied()
                        .unwrap_or(1),
                )?,
                node_name,
                attempt: snapshot
                    .node_execution_counts
                    .get(&snapshot.current_node_name)
                    .copied()
                    .unwrap_or(1),
                reason: reason.clone(),
                failure_kind,
                retry_count,
                timestamp,
            };
            let node_failed = dispatch_internal_node_command(snapshot, fail_command)?;
            Ok(vec![
                node_failed,
                WorkflowEvent::ExecutionFailed {
                    execution_id,
                    reason,
                    failure_kind,
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
                RuntimeExecutionState::Completed | RuntimeExecutionState::Failed { .. }
            );
            if is_terminal {
                events.extend(terminal_required_events_for_snapshot(snapshot)?);
            } else if matches!(snapshot.state, RuntimeExecutionState::Aborted) {
                events.push(WorkflowEvent::ExecutionAborted {
                    execution_id: snapshot.execution_id.clone(),
                    aborted_node: snapshot
                        .node_history
                        .last()
                        .filter(|entry| entry.state == NODE_STATUS_ABORTED)
                        .map(|entry| entry.node_name.clone()),
                    timestamp: snapshot.updated_at,
                });
            }
        }
        StepOutcome::RetryCurrentStep { snapshot, .. } => {
            events.push(node_started_event_for_snapshot(snapshot)?);
        }
        StepOutcome::TransitionAndStart(snapshot) => {
            if let Some(event) = last_step_completed_event_for_snapshot(snapshot)? {
                events.push(event);
            }
            events.push(node_started_event_for_snapshot(snapshot)?);
        }
        StepOutcome::StartParallel(snapshot) => {
            if let Some(event) = last_step_completed_event_for_snapshot(snapshot)? {
                events.push(event);
            }
            events.push(fanout_parent_started_event_for_snapshot(snapshot)?);
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

pub(crate) fn workflow_event_timestamp(event: &WorkflowEvent) -> f64 {
    match event {
        WorkflowEvent::ExecutionStarted { timestamp, .. }
        | WorkflowEvent::NodeStarted { timestamp, .. }
        | WorkflowEvent::SessionAttached { timestamp, .. }
        | WorkflowEvent::StallObserved { timestamp, .. }
        | WorkflowEvent::StallCleared { timestamp, .. }
        | WorkflowEvent::NodeCompleted { timestamp, .. }
        | WorkflowEvent::NodeFailed { timestamp, .. }
        | WorkflowEvent::ApprovalRequested { timestamp, .. }
        | WorkflowEvent::ApprovalResolved { timestamp, .. }
        | WorkflowEvent::ExecutionCompleted { timestamp, .. }
        | WorkflowEvent::ExecutionFailed { timestamp, .. }
        | WorkflowEvent::ExecutionAborted { timestamp, .. }
        | WorkflowEvent::ExecutionInterrupted { timestamp, .. }
        | WorkflowEvent::ContractViolated { timestamp, .. }
        | WorkflowEvent::CliMutationRequested { timestamp, .. }
        | WorkflowEvent::ArtifactProduced { timestamp, .. }
        | WorkflowEvent::CliMutationRejected { timestamp, .. } => *timestamp,
    }
}

pub(crate) fn set_workflow_event_timestamp(event: &mut WorkflowEvent, commit_timestamp: f64) {
    match event {
        WorkflowEvent::ExecutionStarted { timestamp, .. }
        | WorkflowEvent::NodeStarted { timestamp, .. }
        | WorkflowEvent::SessionAttached { timestamp, .. }
        | WorkflowEvent::StallObserved { timestamp, .. }
        | WorkflowEvent::StallCleared { timestamp, .. }
        | WorkflowEvent::NodeCompleted { timestamp, .. }
        | WorkflowEvent::NodeFailed { timestamp, .. }
        | WorkflowEvent::ApprovalRequested { timestamp, .. }
        | WorkflowEvent::ApprovalResolved { timestamp, .. }
        | WorkflowEvent::ExecutionCompleted { timestamp, .. }
        | WorkflowEvent::ExecutionFailed { timestamp, .. }
        | WorkflowEvent::ExecutionAborted { timestamp, .. }
        | WorkflowEvent::ExecutionInterrupted { timestamp, .. }
        | WorkflowEvent::ContractViolated { timestamp, .. }
        | WorkflowEvent::CliMutationRequested { timestamp, .. }
        | WorkflowEvent::ArtifactProduced { timestamp, .. }
        | WorkflowEvent::CliMutationRejected { timestamp, .. } => *timestamp = commit_timestamp,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::adaptor::gateway::workflow::schema::{NodeKindName, Workflow};
    use crate::adaptor::gateway::workflow::state::{
        NodeExecution, NodeExecutionStatus, NodeHistoryEntry, TokenUsage,
    };
    use crate::domain::workflow::{ExecutionOrigin, NODE_STATUS_COMPLETED};
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
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            request: "ship it".to_string(),
            error_reason: None,
            state: RuntimeExecutionState::Running,
            current_node_index: 0,
            current_node_name: "implement".to_string(),
            current_session_id: None,
            total_nodes: 1,
            node_history: Vec::new(),
            node_execution_counts: HashMap::from([("implement".to_string(), 3)]),
            workflow_definition: Workflow::default(),
            total_token_usage: TokenUsage::default(),
            node_statuses: HashMap::new(),
            artifacts: HashMap::new(),
            node_executions: vec![NodeExecution {
                id: "node-execution-implement-3".to_string(),
                execution_id: "run-1".to_string(),
                node_name: "implement".to_string(),
                kind: NodeKindName::Session,
                attempt: 3,
                status: NodeExecutionStatus::Running,
                session_id: None,
                artifact: None,
                token_usage: None,
                failure: None,
                fanout_parent: None,
                started_at: 40.0,
                completed_at: None,
            }],
            stall_observations: Vec::new(),
            approval_operations: None,
            started_at: 1.0,
            updated_at: 42.0,
        }
    }

    fn fanout_workflow_state_fixture() -> WorkflowState {
        let mut snapshot = workflow_state_fixture();
        snapshot.current_node_name = "reviews".to_string();
        snapshot.current_node_index = 1;
        snapshot
            .node_execution_counts
            .insert("reviews".to_string(), 1);
        snapshot.node_executions = vec![NodeExecution {
            id: "node-execution-reviews-1".to_string(),
            execution_id: "run-1".to_string(),
            node_name: "reviews".to_string(),
            kind: NodeKindName::Fanout,
            attempt: 1,
            status: NodeExecutionStatus::Running,
            session_id: None,
            artifact: None,
            token_usage: None,
            failure: None,
            fanout_parent: None,
            started_at: 42.0,
            completed_at: None,
        }];
        snapshot
    }

    #[test]
    fn terminal_required_events_for_completed_snapshot_includes_last_node_then_run_completed() {
        let mut snapshot = workflow_state_fixture();
        snapshot.state = RuntimeExecutionState::Completed;
        snapshot.node_history.push(NodeHistoryEntry {
            node_name: "implement".to_string(),
            completed_at: 41.0,
            result: Some("done".to_string()),
            session_id: Some("session-1".to_string()),
            token_usage: None,
            artifact: None,
            attempt: 3,
            fanout_children: None,
            state: NODE_STATUS_COMPLETED.to_string(),
        });

        let events = terminal_required_events_for_snapshot(&snapshot).unwrap();

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            WorkflowEvent::NodeCompleted {
                execution_id,
                node_name,
                result_summary,
                attempt,
                timestamp,
                ..
            } if execution_id == "run-1"
                && node_name == "implement"
                && result_summary.as_deref() == Some("done")
                && *attempt == 3
                && (*timestamp - 41.0).abs() < f64::EPSILON
        ));
        assert!(matches!(
            &events[1],
            WorkflowEvent::ExecutionCompleted {
                execution_id,
                timestamp,
                ..
            } if execution_id == "run-1"
                && (*timestamp - 42.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn terminal_required_events_for_failed_snapshot_preserves_retry_count() {
        let mut snapshot = workflow_state_fixture();
        snapshot.state = RuntimeExecutionState::Failed {
            reason: "startup exhausted".to_string(),
            kind: crate::domain::workflow::NodeExecutionFailureKind::StartupTimeout,
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
            } if *failure_kind == crate::domain::workflow::NodeExecutionFailureKind::StartupTimeout
                && *retry_count == Some(2)
        ));
        assert!(matches!(
            &events[1],
            WorkflowEvent::ExecutionFailed {
                failure_kind,
                ..
            } if *failure_kind == crate::domain::workflow::NodeExecutionFailureKind::StartupTimeout
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
                execution_id,
                node_execution_id,
                node_name,
                kind,
                attempt,
                fanout_parent,
                timestamp,
            } if execution_id == "run-1"
                && node_execution_id == "node-execution-implement-3"
                && node_name == "implement"
                && *kind == NodeKindName::Session
                && *attempt == 3
                && fanout_parent.is_none()
                && (*timestamp - 42.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn pre_commit_required_events_for_fanout_includes_parent_node_started() {
        let snapshot = fanout_workflow_state_fixture();

        let events =
            pre_commit_required_events_for_outcome(&StepOutcome::StartParallel(snapshot)).unwrap();

        assert!(matches!(
            events.as_slice(),
            [WorkflowEvent::NodeStarted {
                node_execution_id,
                node_name,
                kind: NodeKindName::Fanout,
                fanout_parent: None,
                ..
            }] if node_execution_id == "node-execution-reviews-1" && node_name == "reviews"
        ));
    }

    #[test]
    fn session_attached_event_uses_current_node_execution_id() {
        let mut snapshot = workflow_state_fixture();
        snapshot.current_session_id = Some("session-3".to_string());

        let event = node_session_started_event_for_snapshot(&snapshot)
            .unwrap()
            .expect("current session should produce an event");

        assert!(matches!(
            event,
            WorkflowEvent::SessionAttached {
                node_execution_id,
                session_id,
                ..
            } if node_execution_id == "node-execution-implement-3"
                && session_id == "session-3"
        ));
    }

    #[test]
    fn node_started_event_rejects_missing_node_execution() {
        let mut snapshot = workflow_state_fixture();
        snapshot.node_executions.clear();

        let error = node_started_event_for_snapshot(&snapshot).unwrap_err();

        assert!(matches!(error, WorkflowEngineError::InvalidState(message)
            if message.contains("NodeExecution") && message.contains("implement")));
    }

    #[test]
    fn post_commit_progress_event_plan_emits_only_next_node_start() {
        let snapshot = workflow_state_fixture();
        let fanout_snapshot = fanout_workflow_state_fixture();

        assert!(matches!(
            PostCommitProgressEventPlan::TransitionAndStart
                .followup_event(&snapshot)
                .unwrap(),
            Some(WorkflowEvent::NodeStarted {
                execution_id,
                node_execution_id,
                node_name,
                kind,
                attempt,
                fanout_parent,
                timestamp,
            }) if execution_id == "run-1"
                && node_execution_id == "node-execution-implement-3"
                && node_name == "implement"
                && kind == NodeKindName::Session
                && attempt == 3
                && fanout_parent.is_none()
                && (timestamp - 42.0).abs() < f64::EPSILON
        ));
        assert!(matches!(
            PostCommitProgressEventPlan::StartParallel
                .followup_event(&fanout_snapshot)
                .unwrap(),
            Some(WorkflowEvent::NodeStarted {
                node_execution_id,
                node_name,
                kind: NodeKindName::Fanout,
                fanout_parent: None,
                ..
            }) if node_execution_id == "node-execution-reviews-1" && node_name == "reviews"
        ));
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
                execution_id,
                request_id,
                request: CliMutationRequestRecord::Abort { node_name: None },
                requested_at,
                timestamp,
            }) if execution_id == "run-1"
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
            CliMutationRequestRecord::Approve {
                node_name: "approval".to_string(),
                comment: None,
            },
            50.0,
        ));

        let event = cli_mutation_rejected_event(
            "wf".to_string(),
            &context,
            &WorkflowEngineError::InvalidState("execution is already terminal".to_string()),
            60.0,
        )
        .unwrap();

        assert!(matches!(
            event,
            WorkflowEvent::CliMutationRejected {
                execution_id,
                request_id,
                request: CliMutationRequestRecord::Approve { .. },
                reason: CliMutationRejectionReason::ExecutionNotActive,
                requested_at,
                timestamp,
                ..
            } if execution_id == "run-2"
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
                execution_id,
                request_id,
                request: CliMutationRequestRecord::SubmitOutput {
                    node_name,
                    contract,
                },
                reason: CliMutationRejectionReason::ContractMismatch,
                requested_at,
                timestamp,
                ..
            } if execution_id == "run-3"
                && request_id == "request-3"
                && node_name == "review"
                && contract == "review-verdict"
                && (requested_at - 70.0).abs() < f64::EPSILON
                && (timestamp - 80.0).abs() < f64::EPSILON
        ));
    }
}
