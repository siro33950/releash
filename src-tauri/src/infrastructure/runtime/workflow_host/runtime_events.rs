//! Domain decision to durable event mapping.

use crate::domain::workflow::RuntimeExecutionState;
use crate::domain::workflow::WorkflowEvent;
use crate::domain::workflow::NODE_STATUS_ABORTED;
use crate::infrastructure::runtime::workflow_host::internal_node_command::InternalNodeCommand;
use crate::infrastructure::runtime::workflow_host::runtime_commit::NodeOutcome;
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

/// [05] internal dispatch path: driver 内部の node 完了 / 失敗 typed command の
/// 単一 commit 関数。`InternalNodeCommand::CompleteNode` / `FailNode` を受け取り、
/// 対応する state mutation を snapshot に適用したうえで
/// `WorkflowEvent::NodeCompleted` / `NodeFailed` を返す。
pub(crate) fn dispatch_internal_node_command(
    snapshot: &mut RuntimeCommitSnapshot,
    command: InternalNodeCommand,
) -> Result<WorkflowEvent, WorkflowRuntimeError> {
    apply_internal_node_command_state_mutation(snapshot, &command)?;
    map_internal_node_command_to_event(command)
}

fn apply_internal_node_command_state_mutation(
    snapshot: &mut RuntimeCommitSnapshot,
    command: &InternalNodeCommand,
) -> Result<(), WorkflowRuntimeError> {
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
                return Err(WorkflowRuntimeError::ValidationError(format!(
                    "CompleteNode execution_id mismatch: command='{execution_id}', snapshot='{}'",
                    snapshot.execution_id
                )));
            }
            if workflow_name != &snapshot.workflow_name {
                return Err(WorkflowRuntimeError::ValidationError(format!(
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
                return Err(WorkflowRuntimeError::ValidationError(format!(
                    "CompleteNode for node '{node_name}' but snapshot.node_history is empty"
                )));
            };
            if last_entry.node_name != *node_name {
                return Err(WorkflowRuntimeError::ValidationError(format!(
                    "CompleteNode node mismatch: command='{node_name}', snapshot last='{}'",
                    last_entry.node_name
                )));
            }
            if (last_entry.completed_at - *timestamp).abs() > f64::EPSILON {
                return Err(WorkflowRuntimeError::ValidationError(format!(
                    "CompleteNode timestamp mismatch for node '{node_name}': command={timestamp}, snapshot={}",
                    last_entry.completed_at
                )));
            }
            if last_entry.result != *result {
                return Err(WorkflowRuntimeError::ValidationError(format!(
                    "CompleteNode result mismatch for node '{node_name}'"
                )));
            }
            if last_entry.session_id != *session_id {
                return Err(WorkflowRuntimeError::ValidationError(format!(
                    "CompleteNode session_id mismatch for node '{node_name}'"
                )));
            }
            if last_entry.token_usage != *token_usage {
                return Err(WorkflowRuntimeError::ValidationError(format!(
                    "CompleteNode token_usage mismatch for node '{node_name}'"
                )));
            }
            if last_entry.artifact != *artifact {
                return Err(WorkflowRuntimeError::ValidationError(format!(
                    "CompleteNode artifact mismatch for node '{node_name}'"
                )));
            }
            if Some(last_entry.attempt) != *attempt {
                return Err(WorkflowRuntimeError::ValidationError(format!(
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
                return Err(WorkflowRuntimeError::ValidationError(format!(
                    "FailNode execution_id mismatch: command='{execution_id}', snapshot='{}'",
                    snapshot.execution_id
                )));
            }
            if workflow_name != &snapshot.workflow_name {
                return Err(WorkflowRuntimeError::ValidationError(format!(
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
                return Err(WorkflowRuntimeError::ValidationError(format!(
                    "FailNode node_name mismatch: command='{node_name}', snapshot='{}'",
                    snapshot.current_node_name
                )));
            }
            if !matches!(snapshot.state, RuntimeExecutionState::Failed { .. }) {
                snapshot.apply_lifecycle_projection(
                    RuntimeExecutionState::Failed {
                        reason: reason.clone(),
                        kind: *failure_kind,
                        retry_count: *retry_count,
                    },
                    *timestamp,
                );
            }
            Ok(())
        }
    }
}

fn validate_top_level_node_execution(
    snapshot: &RuntimeCommitSnapshot,
    node_execution_id: &str,
    node_name: &str,
    attempt: Option<u32>,
    command_name: &str,
) -> Result<(), WorkflowRuntimeError> {
    let execution = snapshot
        .node_executions
        .iter()
        .find(|execution| execution.id == node_execution_id)
        .ok_or_else(|| {
            WorkflowRuntimeError::ValidationError(format!(
                "{command_name} references unknown node_execution_id '{node_execution_id}'"
            ))
        })?;
    if execution.execution_id != snapshot.execution_id
        || execution.node_name != node_name
        || execution.fanout_parent.is_some()
        || attempt.is_some_and(|attempt| execution.attempt != attempt)
    {
        return Err(WorkflowRuntimeError::ValidationError(format!(
            "{command_name} node execution mismatch: id='{node_execution_id}', node='{node_name}', attempt={attempt:?}"
        )));
    }
    Ok(())
}

fn map_internal_node_command_to_event(
    command: InternalNodeCommand,
) -> Result<WorkflowEvent, WorkflowRuntimeError> {
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

pub(crate) fn pre_commit_required_events_for_outcome(
    outcome: &NodeOutcome,
) -> Result<Vec<WorkflowEvent>, WorkflowRuntimeError> {
    let mut events = Vec::new();
    let mut snapshot = outcome.snapshot().clone();
    match outcome {
        NodeOutcome::Persist(s) => {
            let is_terminal = matches!(
                s.state,
                RuntimeExecutionState::Completed | RuntimeExecutionState::Failed { .. }
            );
            if is_terminal {
                return terminal_required_events_for_snapshot(s);
            }
        }
        NodeOutcome::RetryCurrentNode { snapshot, .. } => {
            events.push(node_started_event_for_snapshot(snapshot)?);
        }
        NodeOutcome::TransitionAndStart(_) => {
            if let Some(ev) = last_node_completed_event_for_snapshot(&mut snapshot)? {
                events.push(ev);
            }
            events.push(node_started_event_for_snapshot(&snapshot)?);
        }
        NodeOutcome::StartFanout(_) => {
            if let Some(ev) = last_node_completed_event_for_snapshot(&mut snapshot)? {
                events.push(ev);
            }
            events.push(fanout_parent_started_event_for_snapshot(&snapshot)?);
        }
    }
    Ok(events)
}

pub(crate) fn terminal_required_events_for_snapshot(
    snapshot: &RuntimeCommitSnapshot,
) -> Result<Vec<WorkflowEvent>, WorkflowRuntimeError> {
    let mut local = snapshot.clone();
    let mut events = Vec::new();
    if matches!(snapshot.state, RuntimeExecutionState::Completed) {
        if let Some(event) = last_node_completed_event_for_snapshot(&mut local)? {
            events.push(event);
        }
    }
    events.extend(terminal_events_for_snapshot(&mut local)?);
    Ok(events)
}

#[cfg(test)]
pub(crate) fn terminal_events_for_append(
    snapshot: &RuntimeCommitSnapshot,
) -> Result<Vec<WorkflowEvent>, String> {
    let mut local = snapshot.clone();
    terminal_events_for_snapshot(&mut local).map_err(|e| format!("{e:?}"))
}

pub(crate) fn last_node_completed_event_for_snapshot(
    snapshot: &mut RuntimeCommitSnapshot,
) -> Result<Option<WorkflowEvent>, WorkflowRuntimeError> {
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

fn current_node_attempt(snapshot: &RuntimeCommitSnapshot) -> u32 {
    snapshot
        .node_execution_counts
        .get(&snapshot.current_node_name)
        .copied()
        .unwrap_or(1)
}

fn top_level_node_execution<'a>(
    snapshot: &'a RuntimeCommitSnapshot,
    node_name: &str,
    attempt: u32,
) -> Result<
    &'a crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution,
    WorkflowRuntimeError,
> {
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
            WorkflowRuntimeError::InvalidState(format!(
                "top-level NodeExecution for node '{node_name}' attempt {attempt} is unavailable"
            ))
        })
}

fn top_level_node_execution_id(
    snapshot: &RuntimeCommitSnapshot,
    node_name: &str,
    attempt: u32,
) -> Result<String, WorkflowRuntimeError> {
    Ok(top_level_node_execution(snapshot, node_name, attempt)?
        .id
        .clone())
}

pub(crate) fn node_started_event_for_snapshot(
    snapshot: &RuntimeCommitSnapshot,
) -> Result<WorkflowEvent, WorkflowRuntimeError> {
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
    snapshot: &RuntimeCommitSnapshot,
) -> Result<Option<WorkflowEvent>, WorkflowRuntimeError> {
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

pub(crate) fn fanout_parent_started_event_for_snapshot(
    snapshot: &RuntimeCommitSnapshot,
) -> Result<WorkflowEvent, WorkflowRuntimeError> {
    let event = node_started_event_for_snapshot(snapshot)?;
    if !matches!(
        event,
        WorkflowEvent::NodeStarted {
            kind: crate::domain::workflow::NodeKindName::Fanout,
            ..
        }
    ) {
        return Err(WorkflowRuntimeError::InvalidState(format!(
            "fanout parent '{}' does not reference a fanout NodeExecution",
            snapshot.current_node_name
        )));
    }
    Ok(event)
}

pub(crate) fn terminal_events_for_snapshot(
    snapshot: &mut RuntimeCommitSnapshot,
) -> Result<Vec<WorkflowEvent>, WorkflowRuntimeError> {
    match snapshot.state.clone() {
        RuntimeExecutionState::Completed => Ok(vec![WorkflowEvent::ExecutionCompleted {
            execution_id: snapshot.execution_id.clone(),
            total_token_usage: snapshot.total_token_usage.clone(),
            timestamp: snapshot.updated_at,
        }]),
        RuntimeExecutionState::Interrupted => Err(WorkflowRuntimeError::InvalidState(
            "Interrupted transitions require an explicit typed interruption reason".to_string(),
        )),
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
    outcome: &mut NodeOutcome,
) -> Result<Vec<WorkflowEvent>, WorkflowRuntimeError> {
    let mut events = vec![approval_event];
    match outcome {
        NodeOutcome::Persist(snapshot) => {
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
        NodeOutcome::RetryCurrentNode { snapshot, .. } => {
            events.push(node_started_event_for_snapshot(snapshot)?);
        }
        NodeOutcome::TransitionAndStart(snapshot) => {
            if let Some(event) = last_node_completed_event_for_snapshot(snapshot)? {
                events.push(event);
            }
            events.push(node_started_event_for_snapshot(snapshot)?);
        }
        NodeOutcome::StartFanout(snapshot) => {
            if let Some(event) = last_node_completed_event_for_snapshot(snapshot)? {
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
        | WorkflowEvent::CommandPrepared { timestamp, .. }
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
        | WorkflowEvent::ExecutionResumed { timestamp, .. }
        | WorkflowEvent::ContractViolated { timestamp, .. }
        | WorkflowEvent::ArtifactProduced { timestamp, .. } => *timestamp,
    }
}

pub(crate) fn set_workflow_event_timestamp(event: &mut WorkflowEvent, commit_timestamp: f64) {
    match event {
        WorkflowEvent::ExecutionStarted { timestamp, .. }
        | WorkflowEvent::NodeStarted { timestamp, .. }
        | WorkflowEvent::SessionAttached { timestamp, .. }
        | WorkflowEvent::CommandPrepared { timestamp, .. }
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
        | WorkflowEvent::ExecutionResumed { timestamp, .. }
        | WorkflowEvent::ContractViolated { timestamp, .. }
        | WorkflowEvent::ArtifactProduced { timestamp, .. } => *timestamp = commit_timestamp,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::domain::workflow::entities::workflow_execution::{
        RuntimeNodeExecution as NodeExecution, RuntimeNodeExecutionStatus as NodeExecutionStatus,
    };
    use crate::domain::workflow::{ExecutionOrigin, NODE_STATUS_COMPLETED};
    use crate::domain::workflow::{NodeHistoryEntry, TokenUsage};
    use crate::domain::workflow::{NodeKindName, WorkflowDefinition};

    fn commit_snapshot_fixture() -> RuntimeCommitSnapshot {
        RuntimeCommitSnapshot {
            execution_id: "execution-1".to_string(),
            workflow_name: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            request: "ship it".to_string(),
            error_reason: None,
            state: RuntimeExecutionState::Running,
            current_node_index: 0,
            current_node_name: "implement".to_string(),
            current_session_id: None,
            node_history: Vec::new(),
            node_execution_counts: HashMap::from([("implement".to_string(), 3)]),
            workflow_definition: WorkflowDefinition::default(),
            total_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: vec![NodeExecution {
                id: "node-execution-implement-3".to_string(),
                execution_id: "execution-1".to_string(),
                node_name: "implement".to_string(),
                kind: NodeKindName::Session,
                attempt: 3,
                status: NodeExecutionStatus::Running,
                session_id: None,
                display_command: None,
                artifact: None,
                token_usage: None,
                failure: None,
                fanout_parent: None,
                started_at: 40.0,
                completed_at: None,
            }],
            started_at: 1.0,
            updated_at: 42.0,
        }
    }

    fn fanout_commit_snapshot_fixture() -> RuntimeCommitSnapshot {
        let mut snapshot = commit_snapshot_fixture();
        snapshot.current_node_name = "reviews".to_string();
        snapshot.current_node_index = 1;
        snapshot
            .node_execution_counts
            .insert("reviews".to_string(), 1);
        snapshot.node_executions = vec![NodeExecution {
            id: "node-execution-reviews-1".to_string(),
            execution_id: "execution-1".to_string(),
            node_name: "reviews".to_string(),
            kind: NodeKindName::Fanout,
            attempt: 1,
            status: NodeExecutionStatus::Running,
            session_id: None,
            display_command: None,
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
    fn terminal_required_events_for_completed_snapshot_includes_last_node_then_execution_completed()
    {
        let mut snapshot = commit_snapshot_fixture();
        snapshot.apply_lifecycle_projection(RuntimeExecutionState::Completed, snapshot.updated_at);
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
            } if execution_id == "execution-1"
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
            } if execution_id == "execution-1"
                && (*timestamp - 42.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn terminal_required_events_for_failed_snapshot_preserves_retry_count() {
        let mut snapshot = commit_snapshot_fixture();
        snapshot.apply_lifecycle_projection(
            RuntimeExecutionState::Failed {
                reason: "startup exhausted".to_string(),
                kind: crate::domain::workflow::NodeExecutionFailureKind::StartupTimeout,
                retry_count: Some(2),
            },
            snapshot.updated_at,
        );

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
    fn pre_commit_required_events_for_retry_current_node_includes_node_started() {
        let snapshot = commit_snapshot_fixture();
        let events = pre_commit_required_events_for_outcome(&NodeOutcome::RetryCurrentNode {
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
            } if execution_id == "execution-1"
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
        let snapshot = fanout_commit_snapshot_fixture();

        let events =
            pre_commit_required_events_for_outcome(&NodeOutcome::StartFanout(snapshot)).unwrap();

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
        let mut snapshot = commit_snapshot_fixture();
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
        let mut snapshot = commit_snapshot_fixture();
        snapshot.node_executions.clear();

        let error = node_started_event_for_snapshot(&snapshot).unwrap_err();

        assert!(matches!(error, WorkflowRuntimeError::InvalidState(message)
            if message.contains("NodeExecution") && message.contains("implement")));
    }
}
