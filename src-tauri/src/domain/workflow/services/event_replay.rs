//! On-demand projection from the append-only workflow execution event log.

use std::collections::HashMap;

use crate::domain::workflow::entities::workflow_execution::{
    CanonicalNodeFact, NodeStallObservation as AggregateStallObservation, ReplayOutcome,
    WorkflowDefaults, WorkflowExecution as WorkflowExecutionAggregate, WorkflowExecutionRestore,
};
use crate::domain::workflow::services::routing::LoopGuardResetBaselines;
#[cfg(test)]
use crate::domain::workflow::services::routing::{self, RouteDecision};
use crate::domain::workflow::{
    ApprovalTarget, Artifact, ExecutionStatus, Fanout, FanoutParentRef, NodeCompletionSignal,
    NodeCompletionSignalState, NodeExecution, NodeExecutionFailure, NodeExecutionStatus,
    NodeKindName, TokenUsage, WorkflowExecution,
};
use crate::domain::workflow::{
    FanoutParentRef as EventFanoutParentRef, NodeKindName as EventNodeKindName,
    TokenUsage as EventTokenUsage, WorkflowEvent,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DerivedWorkflowExecutionFields {
    pub(crate) status: ExecutionStatus,
    pub(crate) current_node: Option<String>,
    pub(crate) approval_target: Option<ApprovalTarget>,
    pub(crate) artifacts: Vec<Artifact>,
    pub(crate) fanouts: Vec<Fanout>,
}

#[derive(Debug)]
pub(crate) struct RetainedWorkflowExecutionProjection {
    pub(crate) execution: WorkflowExecution,
    pub(crate) node_execution_counts: HashMap<String, u32>,
    pub(crate) loop_guard_reset_baselines: LoopGuardResetBaselines,
}

struct ProjectedWorkflowExecution {
    execution: WorkflowExecution,
    aggregate: WorkflowExecutionAggregate,
}

/// Projects one public workflow execution read model from its event stream.
///
/// An empty stream (or an audit-only stream without `ExecutionStarted`) means the
/// execution does not exist. Existing NDJSON shapes are not interpreted here.
pub fn project_workflow_execution(
    execution_id: &str,
    events: &[WorkflowEvent],
) -> Result<Option<WorkflowExecution>, String> {
    project_retained_workflow_execution(execution_id, events)
        .map(|projection| projection.map(|projection| projection.execution))
}

pub(crate) fn project_retained_workflow_execution(
    execution_id: &str,
    events: &[WorkflowEvent],
) -> Result<Option<RetainedWorkflowExecutionProjection>, String> {
    project_workflow_execution_retained(execution_id, events).map(|projection| {
        projection.map(|projection| RetainedWorkflowExecutionProjection {
            execution: projection.execution,
            node_execution_counts: projection.aggregate.node_execution_counts.clone(),
            loop_guard_reset_baselines: projection.aggregate.loop_guard_reset_baselines.clone(),
        })
    })
}

fn project_workflow_execution_retained(
    execution_id: &str,
    events: &[WorkflowEvent],
) -> Result<Option<ProjectedWorkflowExecution>, String> {
    for event in events {
        if event.execution_id() != execution_id {
            return Err(format!(
                "workflow event belongs to execution {} instead of {execution_id}",
                event.execution_id()
            ));
        }
    }

    let starts = events
        .iter()
        .filter_map(|event| match event {
            WorkflowEvent::ExecutionStarted {
                workflow_name,
                worktree_path,
                created_from,
                request,
                definition,
                timestamp,
                ..
            } => Some((
                workflow_name,
                worktree_path,
                *created_from,
                request,
                definition,
                *timestamp,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    let Some((workflow_name, worktree_path, created_from, request, definition, started_at)) =
        starts.first().copied()
    else {
        return Ok(None);
    };
    if starts.len() != 1 {
        return Err(format!(
            "execution {execution_id} contains more than one execution_started event"
        ));
    }

    let mut execution = WorkflowExecution {
        id: execution_id.to_string(),
        workflow_name: workflow_name.clone(),
        status: ExecutionStatus::Running,
        current_node: None,
        created_from,
        worktree_path: worktree_path.clone(),
        started_at,
        updated_at: started_at,
        completed_at: None,
        error_reason: None,
        interruption_reason: None,
        resume_from_node: None,
        total_token_usage: TokenUsage::default(),
        node_executions: Vec::new(),
        artifacts: vec![Artifact {
            node_name: "request".to_string(),
            contract: None,
            value: serde_json::Value::String(request.clone()),
            produced_at: started_at,
        }],
        fanouts: Vec::new(),
        approval_target: None,
    };
    let mut aggregate = restore_workflow_execution_aggregate(
        execution_id,
        definition,
        worktree_path,
        created_from,
        request,
        started_at,
    );
    let mut authoritative_total_usage = None;

    for event in events {
        apply_event_to_aggregate(execution_id, &mut aggregate, event)?;
        execution.updated_at = execution.updated_at.max(event.timestamp());

        match event {
            WorkflowEvent::ExecutionStarted { .. } => {}
            WorkflowEvent::NodeStarted {
                node_execution_id,
                node_name,
                kind,
                attempt,
                fanout_parent,
                timestamp,
                ..
            } => {
                if execution
                    .node_executions
                    .iter()
                    .any(|node| node.id == *node_execution_id)
                {
                    return Err(format!(
                        "execution {execution_id} contains duplicate node_execution_id {node_execution_id}"
                    ));
                }
                execution.node_executions.push(NodeExecution {
                    id: node_execution_id.clone(),
                    execution_id: execution_id.to_string(),
                    node_name: node_name.clone(),
                    kind: node_kind_to_domain(*kind),
                    attempt: *attempt,
                    status: NodeExecutionStatus::Running,
                    session_id: None,
                    display_command: None,
                    result_summary: None,
                    artifact: None,
                    token_usage: None,
                    failure: None,
                    fanout_parent: fanout_parent.as_ref().map(fanout_parent_to_domain),
                    completion_signals: NodeCompletionSignalState::Pending,
                    started_at: *timestamp,
                    completed_at: None,
                });
            }
            WorkflowEvent::SessionAttached {
                node_execution_id,
                session_id,
                ..
            } => {
                node_mut(&mut execution, node_execution_id, "session_attached")?.session_id =
                    Some(session_id.clone());
            }
            WorkflowEvent::NodeSubmitReceived {
                node_execution_id, ..
            }
            | WorkflowEvent::NodeStopReceived {
                node_execution_id, ..
            } => {
                let state = aggregate
                    .node_executions
                    .iter()
                    .find(|node| node.id == *node_execution_id)
                    .ok_or_else(|| {
                        format!(
                            "execution {execution_id} completion signal references unknown node_execution_id {node_execution_id}"
                        )
                    })?
                    .completion_signals;
                node_mut(&mut execution, node_execution_id, "node_completion_signal")?
                    .completion_signals = state;
            }
            WorkflowEvent::NodeRetryRequested {
                node_execution_id,
                timestamp,
                ..
            } => {
                let aggregate_node = aggregate
                    .node_executions
                    .iter()
                    .find(|node| node.id == *node_execution_id)
                    .ok_or_else(|| {
                        format!(
                            "execution {execution_id} node_retry_requested references unknown node_execution_id {node_execution_id}"
                        )
                    })?;
                let node = node_mut(&mut execution, node_execution_id, "node_retry_requested")?;
                node.status = match aggregate_node.status {
                    crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::Running => NodeExecutionStatus::Running,
                    crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::Paused => NodeExecutionStatus::Paused,
                    crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::WaitingApproval => NodeExecutionStatus::WaitingApproval,
                    crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::Succeeded => NodeExecutionStatus::Succeeded,
                    crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::Failed => NodeExecutionStatus::Failed,
                    crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::Aborted => NodeExecutionStatus::Aborted,
                };
                node.completed_at = aggregate_node.completed_at.or(Some(*timestamp));
            }
            WorkflowEvent::NodePaused {
                node_execution_id, ..
            } => {
                node_mut(&mut execution, node_execution_id, "node_paused")?.replay_paused();
            }
            WorkflowEvent::NodeResumed {
                node_execution_id, ..
            } => {
                node_mut(&mut execution, node_execution_id, "node_resumed")?.replay_resumed();
            }
            WorkflowEvent::CommandPrepared {
                node_execution_id,
                display_command,
                ..
            } => {
                let node = node_mut(&mut execution, node_execution_id, "command_prepared")?;
                if node.kind != NodeKindName::Command {
                    return Err(format!(
                        "execution {execution_id} command_prepared targets non-command node_execution_id {node_execution_id}"
                    ));
                }
                node.display_command = Some(display_command.clone());
            }
            WorkflowEvent::ArtifactProduced {
                node_execution_id,
                node_name,
                contract,
                value,
                timestamp,
                ..
            } => {
                let artifact = Artifact {
                    node_name: node_name.clone(),
                    contract: contract.clone(),
                    value: value.clone(),
                    produced_at: *timestamp,
                };
                let node = node_mut(&mut execution, node_execution_id, "artifact_produced")?;
                require_node_name(node, node_name, "artifact_produced")?;
                node.record_artifact(artifact);
            }
            WorkflowEvent::NodeCompleted {
                node_execution_id,
                node_name,
                attempt,
                result_summary,
                token_usage,
                timestamp,
                ..
            } => {
                let node = node_mut(&mut execution, node_execution_id, "node_completed")?;
                require_node_identity(node, node_name, *attempt, "node_completed")?;
                node.replay_completed(
                    result_summary.clone(),
                    token_usage.as_ref().map(token_usage_to_domain),
                    *timestamp,
                );
            }
            WorkflowEvent::NodeFailed {
                node_execution_id,
                node_name,
                attempt,
                reason,
                failure_kind,
                timestamp,
                ..
            } => {
                let node = node_mut(&mut execution, node_execution_id, "node_failed")?;
                require_node_identity(node, node_name, *attempt, "node_failed")?;
                node.replay_failed(
                    NodeExecutionFailure {
                        reason: reason.clone(),
                        kind: *failure_kind,
                    },
                    *timestamp,
                );
            }
            WorkflowEvent::ApprovalRequested {
                node_execution_id,
                node_name,
                ..
            } => {
                let node = node_mut(&mut execution, node_execution_id, "approval_requested")?;
                require_node_name(node, node_name, "approval_requested")?;
                node.replay_approval_requested();
            }
            WorkflowEvent::ApprovalResolved {
                node_execution_id,
                node_name,
                ..
            } => {
                let node = node_mut(&mut execution, node_execution_id, "approval_resolved")?;
                require_node_name(node, node_name, "approval_resolved")?;
                node.replay_approval_resolved();
            }
            WorkflowEvent::ExecutionCompleted {
                total_token_usage,
                timestamp,
                ..
            } => {
                execution.status = ExecutionStatus::Completed;
                execution.completed_at = Some(*timestamp);
                execution.error_reason = None;
                execution.interruption_reason = None;
                execution.resume_from_node = None;
                authoritative_total_usage = Some(token_usage_to_domain(total_token_usage));
                close_active_nodes(
                    &mut execution.node_executions,
                    NodeExecutionStatus::Succeeded,
                    *timestamp,
                    None,
                );
            }
            WorkflowEvent::ExecutionAborted { timestamp, .. } => {
                execution.status = ExecutionStatus::Aborted;
                execution.completed_at = Some(*timestamp);
                execution.error_reason = None;
                execution.interruption_reason = None;
                execution.resume_from_node = None;
                close_active_nodes(
                    &mut execution.node_executions,
                    NodeExecutionStatus::Aborted,
                    *timestamp,
                    None,
                );
            }
            WorkflowEvent::ExecutionInterrupted { .. } | WorkflowEvent::ExecutionResumed { .. } => {
                return Err("workflow-level interruption events are unsupported".to_string());
            }
            WorkflowEvent::ContractViolated { .. }
            | WorkflowEvent::StallObserved { .. }
            | WorkflowEvent::StallCleared { .. } => {}
        }
    }

    execution.total_token_usage = authoritative_total_usage
        .unwrap_or_else(|| derive_total_token_usage(&execution.node_executions));
    let derived = derive_workflow_execution_fields(
        request,
        started_at,
        execution.status,
        &execution.node_executions,
    );
    execution.status = derived.status;
    execution.current_node = derived.current_node;
    execution.approval_target = derived.approval_target;
    execution.artifacts = derived.artifacts;
    execution.fanouts = derived.fanouts;
    Ok(Some(ProjectedWorkflowExecution {
        execution,
        aggregate,
    }))
}

#[allow(clippy::too_many_arguments)]
fn restore_workflow_execution_aggregate(
    execution_id: &str,
    definition: &crate::domain::workflow::WorkflowDefinition,
    worktree_path: &str,
    created_from: crate::domain::workflow::ExecutionOrigin,
    request: &str,
    started_at: f64,
) -> WorkflowExecutionAggregate {
    WorkflowExecutionAggregate::restore_runtime(WorkflowExecutionRestore {
        id: execution_id.to_string(),
        workflow: definition.clone(),
        workflow_defaults: WorkflowDefaults,
        worktree_path: worktree_path.to_string(),
        created_from,
        started_at,
        updated_at: started_at,
        request: (!request.is_empty()).then(|| request.to_string()),
        ..WorkflowExecutionRestore::default()
    })
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn replay_workflow_execution_aggregate(
    execution_id: &str,
    definition: &crate::domain::workflow::WorkflowDefinition,
    worktree_path: &str,
    created_from: crate::domain::workflow::ExecutionOrigin,
    request: &str,
    started_at: f64,
    events: &[WorkflowEvent],
) -> Result<WorkflowExecutionAggregate, String> {
    let mut aggregate = restore_workflow_execution_aggregate(
        execution_id,
        definition,
        worktree_path,
        created_from,
        request,
        started_at,
    );
    for event in events {
        apply_event_to_aggregate(execution_id, &mut aggregate, event)?;
    }
    Ok(aggregate)
}

fn apply_event_to_aggregate(
    execution_id: &str,
    aggregate: &mut WorkflowExecutionAggregate,
    event: &WorkflowEvent,
) -> Result<(), String> {
    match event {
        WorkflowEvent::ExecutionStarted { .. } => require_replay(
            execution_id,
            "execution_started",
            aggregate.replay_started(),
            true,
        )?,
        WorkflowEvent::NodeStarted {
            node_execution_id,
            node_name,
            kind,
            attempt,
            fanout_parent,
            timestamp,
            ..
        } => {
            if aggregate
                .node_executions
                .iter()
                .any(|node| node.id == *node_execution_id)
            {
                return Err(format!(
                        "execution {execution_id} contains duplicate node_execution_id {node_execution_id}"
                    ));
            }
            if let Some(index) = aggregate
                .workflow
                .nodes
                .iter()
                .position(|node| node.name == *node_name)
            {
                let _ = aggregate.set_current_node(index, *timestamp);
            }
            if let Some(parent) = fanout_parent {
                let parent_node_execution_id = aggregate
                        .node_executions
                        .iter()
                        .rev()
                        .find(|node| {
                            node.node_name == parent.parent_node
                                && node.attempt == parent.parent_attempt
                                && node.fanout_parent.is_none()
                        })
                        .map(|node| node.id.clone())
                        .ok_or_else(|| {
                            format!(
                                "execution {execution_id} fanout child {node_execution_id} has no parent attempt"
                            )
                        })?;
                aggregate
                        .start_fanout_child_execution(
                            parent.parent_node.clone(),
                            parent_node_execution_id,
                            node_execution_id.clone(),
                            node_name.clone(),
                            node_kind_to_domain(*kind),
                            *attempt,
                            fanout_parent_to_domain(parent),
                            *timestamp,
                        )
                        .map_err(|reason| {
                            format!(
                                "execution {execution_id} rejected node_started for {node_execution_id}: {reason:?}"
                            )
                        })?;
            } else {
                aggregate
                        .begin_node_attempt(
                            node_name.clone(),
                            node_kind_to_domain(*kind),
                            *attempt,
                            None,
                            node_execution_id.clone(),
                            *timestamp,
                        )
                        .map_err(|reason| {
                            format!(
                                "execution {execution_id} rejected node_started for {node_execution_id}: {reason:?}"
                            )
                        })?;
            }
        }
        WorkflowEvent::SessionAttached {
            node_execution_id,
            session_id,
            timestamp,
            ..
        } => {
            let fanout_child = aggregate
                .node_executions
                .iter()
                .find(|node| node.id == *node_execution_id)
                .is_some_and(|node| node.fanout_parent.is_some());
            let outcome = if fanout_child {
                aggregate.attach_child_node_session(
                    node_execution_id,
                    session_id.clone(),
                    *timestamp,
                )
            } else {
                aggregate.attach_node_session(node_execution_id, session_id.clone(), *timestamp)
            };
            require_transition(execution_id, "session_attached", outcome)?;
        }
        WorkflowEvent::NodeSubmitReceived {
            node_execution_id,
            timestamp,
            ..
        } => require_transition(
            execution_id,
            "node_submit_received",
            aggregate.record_node_completion_signal(
                node_execution_id,
                NodeCompletionSignal::Submit,
                *timestamp,
            ),
        )?,
        WorkflowEvent::NodeStopReceived {
            node_execution_id,
            timestamp,
            ..
        } => require_transition(
            execution_id,
            "node_stop_received",
            aggregate.record_node_completion_signal(
                node_execution_id,
                NodeCompletionSignal::Stop,
                *timestamp,
            ),
        )?,
        WorkflowEvent::NodeRetryRequested {
            node_execution_id,
            timestamp,
            ..
        } => require_transition(
            execution_id,
            "node_retry_requested",
            aggregate.request_node_retry(node_execution_id, *timestamp),
        )?,
        WorkflowEvent::NodePaused {
            node_execution_id,
            timestamp,
            ..
        } => require_transition(
            execution_id,
            "node_paused",
            aggregate.pause_node_execution(node_execution_id, *timestamp),
        )?,
        WorkflowEvent::NodeResumed {
            node_execution_id,
            timestamp,
            ..
        } => require_transition(
            execution_id,
            "node_resumed",
            aggregate.resume_node_execution(node_execution_id, *timestamp),
        )?,
        WorkflowEvent::CommandPrepared {
            node_execution_id,
            display_command,
            timestamp,
            ..
        } => {
            if !aggregate
                .node_executions
                .iter()
                .any(|node| node.id == *node_execution_id)
            {
                return Err(format!(
                        "execution {execution_id} command_prepared references unknown node_execution_id {node_execution_id}"
                    ));
            }
            require_transition(
                execution_id,
                "command_prepared",
                aggregate.record_node_display_command(
                    node_execution_id,
                    display_command.clone(),
                    *timestamp,
                ),
            )?;
        }
        WorkflowEvent::ArtifactProduced {
            node_execution_id,
            node_name,
            contract,
            value,
            timestamp,
            ..
        } => {
            require_transition(
                execution_id,
                "artifact_produced",
                aggregate.replay_artifact_produced(
                    node_execution_id,
                    node_name,
                    contract.clone(),
                    value.clone(),
                    *timestamp,
                ),
            )?;
        }
        WorkflowEvent::NodeCompleted {
            node_execution_id,
            node_name,
            token_usage,
            timestamp,
            ..
        } => {
            let artifact = aggregate
                .node_executions
                .iter()
                .find(|node| node.id == *node_execution_id)
                .and_then(|node| node.artifact.clone());
            let decision = aggregate.apply_observed_turn(
                node_execution_id,
                CanonicalNodeFact::Completed,
                artifact,
                token_usage.as_ref().map(token_usage_to_domain),
                *timestamp,
            );
            if decision.application
                    != crate::domain::workflow::entities::workflow_execution::TurnCompletionApplication::Superseded
                {
                    aggregate.record_successful_node_completion(node_name, *timestamp);
                }
            let parent_completed = aggregate
                .fanout_runtime
                .as_ref()
                .is_some_and(|fanout| fanout.parent_node_execution_id == *node_execution_id);
            if parent_completed {
                let _ = aggregate.clear_fanout(*timestamp);
            }
        }
        WorkflowEvent::NodeFailed {
            node_execution_id,
            reason,
            failure_kind,
            timestamp,
            ..
        } => {
            aggregate.apply_observed_turn(
                node_execution_id,
                CanonicalNodeFact::Failed {
                    reason: reason.clone(),
                    kind: *failure_kind,
                },
                None,
                None,
                *timestamp,
            );
        }
        WorkflowEvent::ApprovalRequested {
            node_execution_id,
            timestamp,
            ..
        } => {
            require_transition(
                execution_id,
                "approval_requested_node",
                aggregate.mark_node_waiting_approval(node_execution_id, *timestamp),
            )?;
        }
        WorkflowEvent::ApprovalResolved {
            node_execution_id,
            timestamp,
            ..
        } => {
            require_transition(
                execution_id,
                "approval_resolved_node",
                aggregate.mark_node_running(node_execution_id, *timestamp),
            )?;
        }
        WorkflowEvent::StallObserved {
            node_name,
            attempt,
            session_id,
            turn_phase,
            idle_secs,
            signal_count,
            cap_reached,
            timestamp,
            ..
        } => {
            let _ = aggregate.observe_node_stall(AggregateStallObservation {
                session_id: session_id.clone(),
                node_name: node_name.clone(),
                attempt: *attempt,
                turn_phase: turn_phase.clone(),
                idle_secs: *idle_secs,
                signal_count: *signal_count,
                cap_reached: *cap_reached,
                observed_at: *timestamp,
            });
        }
        WorkflowEvent::StallCleared {
            session_id,
            timestamp,
            ..
        } => {
            aggregate.clear_stalls_for_session(session_id, *timestamp);
        }
        WorkflowEvent::ExecutionCompleted { timestamp, .. } => {
            require_replay(
                execution_id,
                "execution_completed",
                aggregate.replay_completed_at(*timestamp),
                true,
            )?;
        }
        WorkflowEvent::ExecutionAborted { timestamp, .. } => {
            require_replay(
                execution_id,
                "execution_aborted",
                aggregate.replay_aborted_at(*timestamp),
                true,
            )?;
        }
        WorkflowEvent::ExecutionInterrupted { .. } | WorkflowEvent::ExecutionResumed { .. } => {
            return Err("workflow-level interruption events are unsupported".to_string());
        }
        WorkflowEvent::ContractViolated { .. } => {}
    }
    Ok(())
}

fn require_transition(
    execution_id: &str,
    event_name: &str,
    outcome: crate::domain::workflow::entities::workflow_execution::TransitionOutcome,
) -> Result<(), String> {
    use crate::domain::workflow::entities::workflow_execution::TransitionOutcome;
    match outcome {
        TransitionOutcome::Applied | TransitionOutcome::AlreadyApplied => Ok(()),
        TransitionOutcome::NotApplicable => Err(format!(
            "execution {execution_id} cannot apply {event_name} to its aggregate"
        )),
        TransitionOutcome::Rejected(reason) => Err(format!(
            "execution {execution_id} rejected {event_name}: {reason:?}"
        )),
    }
}

fn require_replay(
    execution_id: &str,
    event_name: &str,
    outcome: ReplayOutcome,
    allow_not_applicable: bool,
) -> Result<(), String> {
    match outcome {
        ReplayOutcome::Applied | ReplayOutcome::AlreadyApplied => Ok(()),
        ReplayOutcome::NotApplicable if allow_not_applicable => Ok(()),
        ReplayOutcome::NotApplicable => Err(format!(
            "execution {execution_id} cannot apply {event_name} from its current lifecycle state"
        )),
        ReplayOutcome::Rejected(reason) => Err(format!(
            "execution {execution_id} rejected {event_name}: {reason:?}"
        )),
    }
}

fn node_mut<'a>(
    execution: &'a mut WorkflowExecution,
    node_execution_id: &str,
    event_name: &str,
) -> Result<&'a mut NodeExecution, String> {
    let projected_execution_id = execution.id.clone();
    execution
        .node_executions
        .iter_mut()
        .find(|node| node.id == node_execution_id)
        .ok_or_else(|| {
            format!(
                "{event_name} refers to unknown node_execution_id {node_execution_id} in execution {}",
                projected_execution_id
            )
        })
}

fn require_node_name(
    node: &NodeExecution,
    node_name: &str,
    event_name: &str,
) -> Result<(), String> {
    if node.node_name == node_name {
        Ok(())
    } else {
        Err(format!(
            "{event_name} identifies node_execution_id {} as {node_name}, expected {}",
            node.id, node.node_name
        ))
    }
}

fn require_node_identity(
    node: &NodeExecution,
    node_name: &str,
    attempt: u32,
    event_name: &str,
) -> Result<(), String> {
    require_node_name(node, node_name, event_name)?;
    if node.attempt == attempt {
        Ok(())
    } else {
        Err(format!(
            "{event_name} identifies node_execution_id {} as attempt {attempt}, expected {}",
            node.id, node.attempt
        ))
    }
}

fn node_kind_to_domain(kind: EventNodeKindName) -> NodeKindName {
    match kind {
        EventNodeKindName::Command => NodeKindName::Command,
        EventNodeKindName::Session => NodeKindName::Session,
        EventNodeKindName::Fanout => NodeKindName::Fanout,
        EventNodeKindName::Sequence => NodeKindName::Sequence,
    }
}

fn fanout_parent_to_domain(parent: &EventFanoutParentRef) -> FanoutParentRef {
    FanoutParentRef {
        parent_node: parent.parent_node.clone(),
        parent_attempt: parent.parent_attempt,
        item_index: parent.item_index,
        child_index: parent.child_index,
    }
}

fn token_usage_to_domain(usage: &EventTokenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

fn upsert_artifact(artifacts: &mut Vec<Artifact>, artifact: Artifact) {
    if let Some(current) = artifacts
        .iter_mut()
        .find(|current| current.node_name == artifact.node_name)
    {
        *current = artifact;
    } else {
        artifacts.push(artifact);
    }
}

fn close_active_nodes(
    nodes: &mut [NodeExecution],
    status: NodeExecutionStatus,
    completed_at: f64,
    failure: Option<NodeExecutionFailure>,
) {
    for node in nodes.iter_mut().filter(|node| node.status.is_active()) {
        node.status = status;
        node.completed_at = Some(completed_at);
        if failure.is_some() {
            node.failure = failure.clone();
        }
    }
}

fn derive_total_token_usage(nodes: &[NodeExecution]) -> TokenUsage {
    let mut usage = TokenUsage::default();
    for node in nodes {
        let include = match &node.fanout_parent {
            None => true,
            Some(parent) => {
                let own_parent_has_usage = nodes.iter().any(|candidate| {
                    candidate.node_name == parent.parent_node
                        && candidate.attempt == parent.parent_attempt
                        && candidate.token_usage.is_some()
                });
                let usage_was_carried_into_later_parent = nodes.iter().any(|candidate| {
                    let Some(candidate_parent) = candidate.fanout_parent.as_ref() else {
                        return false;
                    };
                    candidate.node_name == node.node_name
                        && candidate_parent.parent_node == parent.parent_node
                        && candidate_parent.item_index == parent.item_index
                        && candidate_parent.child_index == parent.child_index
                        && candidate_parent.parent_attempt > parent.parent_attempt
                        && candidate.status == NodeExecutionStatus::Succeeded
                        && candidate.session_id.is_none()
                        && candidate.token_usage.is_none()
                        && nodes.iter().any(|later_parent| {
                            later_parent.node_name == candidate_parent.parent_node
                                && later_parent.attempt == candidate_parent.parent_attempt
                                && later_parent.token_usage.is_some()
                        })
                });
                !own_parent_has_usage && !usage_was_carried_into_later_parent
            }
        };
        if include {
            if let Some(node_usage) = &node.token_usage {
                usage.input_tokens = usage.input_tokens.saturating_add(node_usage.input_tokens);
                usage.output_tokens = usage.output_tokens.saturating_add(node_usage.output_tokens);
            }
        }
    }
    usage
}

pub(crate) fn derive_workflow_execution_fields(
    request: &str,
    started_at: f64,
    status: ExecutionStatus,
    nodes: &[NodeExecution],
) -> DerivedWorkflowExecutionFields {
    let (status, current_node, approval_target) = derive_active_fields(status, nodes);
    DerivedWorkflowExecutionFields {
        status,
        current_node,
        approval_target,
        artifacts: derive_top_level_artifacts(request, started_at, nodes),
        fanouts: derive_fanouts(nodes),
    }
}

pub(crate) fn derive_top_level_artifacts(
    request: &str,
    started_at: f64,
    nodes: &[NodeExecution],
) -> Vec<Artifact> {
    let mut artifacts = vec![Artifact {
        node_name: "request".to_string(),
        contract: None,
        value: serde_json::Value::String(request.to_string()),
        produced_at: started_at,
    }];
    let successful = nodes
        .iter()
        .filter(|node| {
            node.fanout_parent.is_none() && node.status == NodeExecutionStatus::Succeeded
        })
        .filter_map(|node| node.artifact.clone())
        .collect::<Vec<_>>();
    for artifact in successful {
        upsert_artifact(&mut artifacts, artifact);
    }
    artifacts
}

pub(crate) fn derive_fanouts(nodes: &[NodeExecution]) -> Vec<Fanout> {
    nodes
        .iter()
        .filter(|parent| parent.kind == NodeKindName::Fanout && parent.fanout_parent.is_none())
        .cloned()
        .map(|parent| {
            let mut children = nodes
                .iter()
                .filter(|child| {
                    child.fanout_parent.as_ref().is_some_and(|reference| {
                        reference.parent_node == parent.node_name
                            && reference.parent_attempt == parent.attempt
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            children.sort_by(|left, right| {
                let left_ref = left
                    .fanout_parent
                    .as_ref()
                    .expect("fanout children have a parent reference");
                let right_ref = right
                    .fanout_parent
                    .as_ref()
                    .expect("fanout children have a parent reference");
                (left_ref.item_index.unwrap_or(0), left_ref.child_index)
                    .cmp(&(right_ref.item_index.unwrap_or(0), right_ref.child_index))
                    .then_with(|| left.started_at.total_cmp(&right.started_at))
                    .then_with(|| left.id.cmp(&right.id))
            });
            Fanout {
                artifact: (parent.status == NodeExecutionStatus::Succeeded)
                    .then(|| parent.artifact.clone())
                    .flatten(),
                parent,
                children,
            }
        })
        .collect()
}

fn derive_active_fields(
    status: ExecutionStatus,
    nodes: &[NodeExecution],
) -> (ExecutionStatus, Option<String>, Option<ApprovalTarget>) {
    if status.is_finished() {
        return (status, None, None);
    }

    let waiting = nodes
        .iter()
        .rev()
        .filter(|node| node.status == NodeExecutionStatus::WaitingApproval)
        .collect::<Vec<_>>();
    if let Some(node) = waiting.first() {
        let current_node = Some(node.fanout_parent.as_ref().map_or_else(
            || node.node_name.clone(),
            |parent| parent.parent_node.clone(),
        ));
        let approval_target = (waiting.len() == 1).then(|| ApprovalTarget {
            node_execution_id: node.id.clone(),
            node_name: node.node_name.clone(),
            session_id: node.session_id.clone(),
        });
        return (ExecutionStatus::Running, current_node, approval_target);
    }

    let current_node = nodes
        .iter()
        .rev()
        .find(|node| node.fanout_parent.is_none() && node.status.is_active())
        .map(|node| node.node_name.clone());
    (ExecutionStatus::Running, current_node, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::FanoutParentRef as EventFanoutParentRef;
    use crate::domain::workflow::{
        ExecutionInterruptionReason, ExecutionOrigin, NodeExecutionFailureKind,
    };
    use crate::domain::workflow::{NodeDefinition, NodeKind, WorkflowDefinition};

    const EXECUTION_ID: &str = "00000000-0000-4000-8000-000000000001";

    fn definition() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "review".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                kind: NodeKind::default(),
                ..NodeDefinition::default()
            }],
            entry: "review".to_string(),
        }
    }

    fn started() -> WorkflowEvent {
        WorkflowEvent::ExecutionStarted {
            execution_id: EXECUTION_ID.to_string(),
            workflow_name: "review".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            request: "please review".to_string(),
            definition: definition(),
            timestamp: 1.0,
        }
    }

    fn root_sequence_node(
        children: Vec<(&str, Option<Vec<crate::domain::workflow::Rule>>)>,
    ) -> NodeDefinition {
        NodeDefinition {
            name: "main".to_string(),
            kind: NodeKind::Sequence(crate::domain::workflow::SequenceSpec {
                entry: None,
                output: None,
                children: children
                    .into_iter()
                    .map(|(name, rules)| crate::domain::workflow::ChildEntry {
                        name: name.to_string(),
                        inputs: Vec::new(),
                        rules,
                    })
                    .collect(),
            }),
            ..Default::default()
        }
    }

    fn node_started(id: &str, name: &str, kind: EventNodeKindName) -> WorkflowEvent {
        WorkflowEvent::NodeStarted {
            execution_id: EXECUTION_ID.to_string(),
            node_execution_id: id.to_string(),
            node_name: name.to_string(),
            kind,
            attempt: 1,
            fanout_parent: None,
            timestamp: 2.0,
        }
    }

    fn child_node_started(id: &str, name: &str, item_index: usize) -> WorkflowEvent {
        let mut event = node_started(id, name, EventNodeKindName::Session);
        if let WorkflowEvent::NodeStarted { fanout_parent, .. } = &mut event {
            *fanout_parent = Some(EventFanoutParentRef {
                parent_node: "fanout".to_string(),
                parent_attempt: 1,
                item_index: Some(item_index),
                child_index: 0,
            });
        }
        event
    }

    #[test]
    fn node_executions_preserve_node_started_append_order_when_timestamps_tie() {
        let events = vec![
            started(),
            node_started("a-1", "A", EventNodeKindName::Session),
            node_started("b-1", "B", EventNodeKindName::Command),
            node_started("a-2", "A", EventNodeKindName::Session),
            node_started("c-1", "C", EventNodeKindName::Command),
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(
            execution
                .node_executions
                .iter()
                .map(|node| (node.node_name.as_str(), node.id.as_str()))
                .collect::<Vec<_>>(),
            vec![("A", "a-1"), ("B", "b-1"), ("A", "a-2"), ("C", "c-1")]
        );
    }

    #[test]
    fn command_prepared_restores_only_the_masked_display_command() {
        let raw_secret = "RAW_COMMAND_SECRET_12345";
        let events = vec![
            started(),
            node_started("command-1", "run", EventNodeKindName::Command),
            WorkflowEvent::CommandPrepared {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "command-1".to_string(),
                display_command: "printf '[REDACTED]'".to_string(),
                timestamp: 2.5,
            },
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        let display_command = execution.node_executions[0]
            .display_command
            .as_deref()
            .unwrap();
        assert_eq!(display_command, "printf '[REDACTED]'");
        assert!(!display_command.contains(raw_secret));
    }

    #[test]
    fn command_prepared_rejects_a_non_command_node() {
        let events = vec![
            started(),
            node_started("session-1", "review", EventNodeKindName::Session),
            WorkflowEvent::CommandPrepared {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "session-1".to_string(),
                display_command: "true".to_string(),
                timestamp: 2.5,
            },
        ];

        let error = project_workflow_execution(EXECUTION_ID, &events).unwrap_err();
        assert!(error.contains("command_prepared targets non-command"));
    }

    #[test]
    fn command_prepared_rejects_an_unknown_node_execution() {
        let events = vec![
            started(),
            WorkflowEvent::CommandPrepared {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "missing-command".to_string(),
                display_command: "true".to_string(),
                timestamp: 2.5,
            },
        ];

        let error = project_workflow_execution(EXECUTION_ID, &events).unwrap_err();
        assert!(error.contains("unknown node_execution_id missing-command"));
    }

    #[test]
    fn replay_restores_node_completion_signal_state_for_the_same_attempt() {
        let mut events = vec![
            started(),
            node_started("session-1", "review", EventNodeKindName::Session),
            WorkflowEvent::NodeSubmitReceived {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "session-1".to_string(),
                timestamp: 2.5,
            },
        ];

        let submit_only = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(
            submit_only.node_executions[0].completion_signals,
            NodeCompletionSignalState::SubmitReceived
        );

        events.push(WorkflowEvent::NodeStopReceived {
            execution_id: EXECUTION_ID.to_string(),
            node_execution_id: "session-1".to_string(),
            timestamp: 3.0,
        });
        let ready = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(
            ready.node_executions[0].completion_signals,
            NodeCompletionSignalState::Ready
        );
    }

    #[test]
    fn replay_retry_preserves_old_attempt_but_clears_it_from_current_output() {
        let events = vec![
            started(),
            node_started("session-1", "review", EventNodeKindName::Session),
            WorkflowEvent::ArtifactProduced {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "session-1".to_string(),
                node_name: "review".to_string(),
                contract: Some("review-result".to_string()),
                value: serde_json::json!({"attempt": 1}),
                request_id: None,
                submitted_at: None,
                timestamp: 2.25,
            },
            WorkflowEvent::NodeSubmitReceived {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "session-1".to_string(),
                timestamp: 2.5,
            },
            WorkflowEvent::NodeRetryRequested {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "session-1".to_string(),
                timestamp: 3.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "session-2".to_string(),
                node_name: "review".to_string(),
                kind: EventNodeKindName::Session,
                attempt: 2,
                fanout_parent: None,
                timestamp: 3.0,
            },
        ];

        let projected = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(projected.node_executions.len(), 2);
        assert_eq!(
            projected.node_executions[0].status,
            NodeExecutionStatus::Aborted
        );
        assert_eq!(
            projected.node_executions[0].completion_signals,
            NodeCompletionSignalState::SubmitReceived
        );
        assert!(projected.node_executions[0].artifact.is_some());
        assert_eq!(
            projected.node_executions[1].completion_signals,
            NodeCompletionSignalState::Pending
        );

        let aggregate = replay_workflow_execution_aggregate(
            EXECUTION_ID,
            &definition(),
            "/repo",
            ExecutionOrigin::Cli,
            "review",
            1.0,
            &events,
        )
        .unwrap();
        assert!(!aggregate.artifacts.contains_key("review"));
    }

    #[test]
    fn replay_pause_and_resume_preserve_attempt_and_partial_signal() {
        let mut events = vec![
            started(),
            node_started("session-1", "review", EventNodeKindName::Session),
            WorkflowEvent::NodeSubmitReceived {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "session-1".to_string(),
                timestamp: 2.5,
            },
            WorkflowEvent::NodePaused {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "session-1".to_string(),
                timestamp: 3.0,
            },
        ];

        let paused = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(paused.status, ExecutionStatus::Running);
        assert_eq!(paused.node_executions.len(), 1);
        assert_eq!(paused.node_executions[0].attempt, 1);
        assert_eq!(
            paused.node_executions[0].status,
            NodeExecutionStatus::Paused
        );
        assert_eq!(
            paused.node_executions[0].completion_signals,
            NodeCompletionSignalState::SubmitReceived
        );

        events.push(WorkflowEvent::NodeResumed {
            execution_id: EXECUTION_ID.to_string(),
            node_execution_id: "session-1".to_string(),
            timestamp: 4.0,
        });
        let resumed = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(resumed.status, ExecutionStatus::Running);
        assert_eq!(resumed.node_executions[0].attempt, 1);
        assert_eq!(
            resumed.node_executions[0].status,
            NodeExecutionStatus::Running
        );
        assert_eq!(
            resumed.node_executions[0].completion_signals,
            NodeCompletionSignalState::SubmitReceived
        );
    }

    #[test]
    fn replay_and_live_execution_use_the_same_aggregate_transitions() {
        let definition = definition();
        let events = vec![
            started(),
            node_started("node-1", "review", EventNodeKindName::Session),
            WorkflowEvent::SessionAttached {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-1".to_string(),
                session_id: "session-1".to_string(),
                timestamp: 2.5,
            },
            WorkflowEvent::ArtifactProduced {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-1".to_string(),
                node_name: "review".to_string(),
                contract: Some("review-result".to_string()),
                value: serde_json::json!({"approved": true}),
                request_id: None,
                submitted_at: None,
                timestamp: 3.0,
            },
            WorkflowEvent::NodeSubmitReceived {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-1".to_string(),
                timestamp: 3.25,
            },
            WorkflowEvent::NodeStopReceived {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-1".to_string(),
                timestamp: 3.5,
            },
            node_completed(
                "node-1",
                "review",
                Some(EventTokenUsage {
                    input_tokens: 2,
                    output_tokens: 3,
                }),
                4.0,
            ),
            WorkflowEvent::ExecutionCompleted {
                execution_id: EXECUTION_ID.to_string(),
                total_token_usage: EventTokenUsage {
                    input_tokens: 2,
                    output_tokens: 3,
                },
                timestamp: 5.0,
            },
        ];
        let replayed = replay_workflow_execution_aggregate(
            EXECUTION_ID,
            &definition,
            "/repo",
            ExecutionOrigin::Cli,
            "please review",
            1.0,
            &events,
        )
        .unwrap();

        let mut live = WorkflowExecutionAggregate::restore_runtime(WorkflowExecutionRestore {
            id: EXECUTION_ID.to_string(),
            workflow: definition,
            workflow_defaults: WorkflowDefaults,
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            started_at: 1.0,
            updated_at: 1.0,
            request: Some("please review".to_string()),
            ..WorkflowExecutionRestore::default()
        });
        live.begin_node_attempt(
            "review".to_string(),
            NodeKindName::Session,
            1,
            None,
            "node-1".to_string(),
            2.0,
        )
        .unwrap();
        assert_eq!(
            live.attach_node_session("node-1", "session-1".to_string(), 2.5),
            crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
        );
        assert_eq!(
            live.apply_submitted_output(
                "review".to_string(),
                "node-1",
                1,
                Some("session-1".to_string()),
                "review-result".to_string(),
                serde_json::json!({"approved": true}),
                None,
                3.0,
            ),
            crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
        );
        assert_eq!(
            live.record_node_completion_signal("node-1", NodeCompletionSignal::Submit, 3.25,),
            crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
        );
        assert_eq!(
            live.record_node_completion_signal("node-1", NodeCompletionSignal::Stop, 3.5),
            crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
        );
        live.apply_observed_turn(
            "node-1",
            CanonicalNodeFact::Completed,
            Some(serde_json::json!({"approved": true})),
            Some(TokenUsage {
                input_tokens: 2,
                output_tokens: 3,
            }),
            4.0,
        );
        live.record_successful_node_completion("review", 4.0);
        assert_eq!(
            live.transition_completed(),
            crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
        );
        live.touch(5.0);

        assert_eq!(replayed.state(), live.state());
        assert_eq!(replayed.node_executions, live.node_executions);
        assert_eq!(replayed.node_execution_counts, live.node_execution_counts);
        assert_eq!(replayed.artifacts, live.artifacts);
        assert_eq!(
            replayed.loop_guard_reset_baselines,
            live.loop_guard_reset_baselines
        );
    }

    fn node_completed(
        id: &str,
        name: &str,
        token_usage: Option<EventTokenUsage>,
        timestamp: f64,
    ) -> WorkflowEvent {
        WorkflowEvent::NodeCompleted {
            execution_id: EXECUTION_ID.to_string(),
            node_execution_id: id.to_string(),
            node_name: name.to_string(),
            attempt: 1,
            result_summary: None,
            token_usage,
            timestamp,
        }
    }

    #[test]
    fn projects_completed_execution_with_artifact_and_usage() {
        let events = vec![
            started(),
            node_started("node-1", "review", EventNodeKindName::Session),
            WorkflowEvent::SessionAttached {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-1".to_string(),
                session_id: "session-1".to_string(),
                timestamp: 2.5,
            },
            WorkflowEvent::ArtifactProduced {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-1".to_string(),
                node_name: "review".to_string(),
                contract: Some("review-result".to_string()),
                value: serde_json::json!({"approved": true}),
                request_id: None,
                submitted_at: None,
                timestamp: 3.0,
            },
            WorkflowEvent::NodeCompleted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-1".to_string(),
                node_name: "review".to_string(),
                attempt: 1,
                result_summary: Some("approved".to_string()),
                token_usage: Some(EventTokenUsage {
                    input_tokens: 10,
                    output_tokens: 4,
                }),
                timestamp: 4.0,
            },
            WorkflowEvent::ExecutionCompleted {
                execution_id: EXECUTION_ID.to_string(),
                total_token_usage: EventTokenUsage {
                    input_tokens: 10,
                    output_tokens: 4,
                },
                timestamp: 5.0,
            },
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Completed);
        assert_eq!(execution.current_node, None);
        assert_eq!(execution.total_token_usage.input_tokens, 10);
        assert_eq!(
            execution.node_executions[0].session_id.as_deref(),
            Some("session-1")
        );
        assert_eq!(
            execution.node_executions[0].status,
            NodeExecutionStatus::Succeeded
        );
        assert!(execution
            .artifacts
            .iter()
            .any(|artifact| artifact.node_name == "review"));
        assert_eq!(execution.artifacts[0].node_name, "request");
        assert_eq!(
            execution.artifacts[0].value,
            serde_json::Value::String("please review".to_string())
        );
    }

    #[test]
    fn projects_node_failure_without_workflow_terminal_failure() {
        let events = vec![
            started(),
            node_started("node-1", "review", EventNodeKindName::Session),
            WorkflowEvent::NodeFailed {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-1".to_string(),
                node_name: "review".to_string(),
                attempt: 1,
                reason: "invalid result".to_string(),
                failure_kind: NodeExecutionFailureKind::ValidationFailure,
                retry_count: Some(0),
                timestamp: 3.0,
            },
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Running);
        assert!(execution.error_reason.is_none());
        assert_eq!(
            execution.node_executions[0].status,
            NodeExecutionStatus::Failed
        );
        assert_eq!(
            execution.node_executions[0]
                .failure
                .as_ref()
                .map(|failure| failure.kind),
            Some(NodeExecutionFailureKind::ValidationFailure)
        );
    }

    #[test]
    fn artifact_map_keeps_latest_successful_attempt() {
        let mut second_attempt = node_started("node-2", "review", EventNodeKindName::Session);
        if let WorkflowEvent::NodeStarted { attempt, .. } = &mut second_attempt {
            *attempt = 2;
        }
        let artifact_event = |node_execution_id: &str, value: &str, timestamp: f64| {
            WorkflowEvent::ArtifactProduced {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: node_execution_id.to_string(),
                node_name: "review".to_string(),
                contract: Some("review-result".to_string()),
                value: serde_json::Value::String(value.to_string()),
                request_id: None,
                submitted_at: None,
                timestamp,
            }
        };
        let events = vec![
            started(),
            node_started("node-1", "review", EventNodeKindName::Session),
            artifact_event("node-1", "accepted", 2.5),
            WorkflowEvent::NodeCompleted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-1".to_string(),
                node_name: "review".to_string(),
                attempt: 1,
                result_summary: None,
                token_usage: None,
                timestamp: 3.0,
            },
            second_attempt,
            artifact_event("node-2", "rejected", 3.5),
            WorkflowEvent::NodeFailed {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-2".to_string(),
                node_name: "review".to_string(),
                attempt: 2,
                reason: "failed".to_string(),
                failure_kind: NodeExecutionFailureKind::ValidationFailure,
                retry_count: Some(1),
                timestamp: 4.0,
            },
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        let artifact = execution
            .artifacts
            .iter()
            .find(|artifact| artifact.node_name == "review")
            .unwrap();
        assert_eq!(artifact.value, serde_json::Value::String("accepted".into()));
        assert_eq!(
            execution.node_executions[1]
                .artifact
                .as_ref()
                .map(|artifact| &artifact.value),
            Some(&serde_json::Value::String("rejected".into()))
        );
    }

    #[test]
    fn projects_aborted_execution_and_active_node() {
        let events = vec![
            started(),
            node_started("node-1", "review", EventNodeKindName::Session),
            WorkflowEvent::ExecutionAborted {
                execution_id: EXECUTION_ID.to_string(),
                aborted_node: Some("review".to_string()),
                timestamp: 3.0,
            },
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Aborted);
        assert_eq!(
            execution.node_executions[0].status,
            NodeExecutionStatus::Aborted
        );
        assert_eq!(execution.node_executions[0].completed_at, Some(3.0));
        assert_eq!(execution.error_reason, None);
    }

    #[test]
    fn rejects_workflow_level_interruption_events() {
        let events = vec![
            started(),
            node_started("node-1", "review", EventNodeKindName::Session),
            WorkflowEvent::ExecutionInterrupted {
                execution_id: EXECUTION_ID.to_string(),
                reason: ExecutionInterruptionReason::Stop,
                timestamp: 3.0,
            },
        ];

        assert_eq!(
            project_workflow_execution(EXECUTION_ID, &events).unwrap_err(),
            "workflow-level interruption events are unsupported"
        );
    }

    #[test]
    fn rejects_workflow_level_interruption_after_node_completion() {
        let mut workflow = definition();
        workflow.nodes[0].name = "prepare".to_string();
        workflow
            .nodes
            .push(crate::domain::workflow::NodeDefinition {
                name: "review".to_string(),
                kind: crate::domain::workflow::NodeKind::default(),
                ..Default::default()
            });
        let events = vec![
            WorkflowEvent::ExecutionStarted {
                execution_id: EXECUTION_ID.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::Cli,
                request: "please review".to_string(),
                definition: workflow,
                timestamp: 1.0,
            },
            node_started("node-1", "prepare", EventNodeKindName::Session),
            WorkflowEvent::NodeCompleted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-1".to_string(),
                node_name: "prepare".to_string(),
                attempt: 1,
                result_summary: None,
                token_usage: None,
                timestamp: 3.0,
            },
            WorkflowEvent::ExecutionInterrupted {
                execution_id: EXECUTION_ID.to_string(),
                reason: ExecutionInterruptionReason::Crash,
                timestamp: 4.0,
            },
        ];

        assert_eq!(
            project_workflow_execution(EXECUTION_ID, &events).unwrap_err(),
            "workflow-level interruption events are unsupported"
        );
    }

    #[test]
    fn rejects_workflow_level_interruption_after_loop_progress() {
        use crate::domain::workflow::Rule;

        let workflow = WorkflowDefinition {
            name: "reset-replay".to_string(),
            nodes: vec![
                root_sequence_node(vec![
                    ("round", Some(vec![Rule::Next("fix".to_string())])),
                    (
                        "fix",
                        Some(vec![Rule::LoopGuard {
                            max_iterations: 2,
                            on_exhausted: "done".to_string(),
                            reset_on: Some("round".to_string()),
                        }]),
                    ),
                    ("done", None),
                ]),
                NodeDefinition {
                    name: "round".to_string(),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "fix".to_string(),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "done".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut fix_second_start = node_started("fix-2", "fix", EventNodeKindName::Session);
        if let WorkflowEvent::NodeStarted {
            attempt, timestamp, ..
        } = &mut fix_second_start
        {
            *attempt = 2;
            *timestamp = 4.0;
        }
        let events = vec![
            WorkflowEvent::ExecutionStarted {
                execution_id: EXECUTION_ID.to_string(),
                workflow_name: "reset-replay".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::Cli,
                request: "review".to_string(),
                definition: workflow,
                timestamp: 1.0,
            },
            node_started("fix-1", "fix", EventNodeKindName::Session),
            WorkflowEvent::NodeCompleted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "fix-1".to_string(),
                node_name: "fix".to_string(),
                attempt: 1,
                result_summary: None,
                token_usage: None,
                timestamp: 3.0,
            },
            fix_second_start,
            WorkflowEvent::NodeCompleted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "fix-2".to_string(),
                node_name: "fix".to_string(),
                attempt: 2,
                result_summary: None,
                token_usage: None,
                timestamp: 5.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "round-1".to_string(),
                node_name: "round".to_string(),
                kind: EventNodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 6.0,
            },
            WorkflowEvent::NodeCompleted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "round-1".to_string(),
                node_name: "round".to_string(),
                attempt: 1,
                result_summary: None,
                token_usage: None,
                timestamp: 7.0,
            },
            WorkflowEvent::ExecutionInterrupted {
                execution_id: EXECUTION_ID.to_string(),
                reason: ExecutionInterruptionReason::Crash,
                timestamp: 8.0,
            },
        ];

        assert_eq!(
            project_workflow_execution_retained(EXECUTION_ID, &events)
                .err()
                .as_deref(),
            Some("workflow-level interruption events are unsupported")
        );
    }

    #[test]
    fn rejects_repeated_workflow_level_interruption_history() {
        use crate::domain::workflow::Rule;

        let workflow = WorkflowDefinition {
            name: "repeated-reset-replay".to_string(),
            nodes: vec![
                root_sequence_node(vec![
                    ("round", Some(vec![Rule::Next("fix".to_string())])),
                    (
                        "fix",
                        Some(vec![Rule::LoopGuard {
                            max_iterations: 2,
                            on_exhausted: "done".to_string(),
                            reset_on: Some("round".to_string()),
                        }]),
                    ),
                    ("done", None),
                ]),
                NodeDefinition {
                    name: "round".to_string(),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "fix".to_string(),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "done".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let start_node =
            |id: &str, name: &str, attempt: u32, timestamp: f64| WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: id.to_string(),
                node_name: name.to_string(),
                kind: EventNodeKindName::Session,
                attempt,
                fanout_parent: None,
                timestamp,
            };
        let complete_node =
            |id: &str, name: &str, attempt: u32, timestamp: f64| WorkflowEvent::NodeCompleted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: id.to_string(),
                node_name: name.to_string(),
                attempt,
                result_summary: None,
                token_usage: None,
                timestamp,
            };
        let interrupt = |timestamp| WorkflowEvent::ExecutionInterrupted {
            execution_id: EXECUTION_ID.to_string(),
            reason: ExecutionInterruptionReason::Crash,
            timestamp,
        };
        let resume = |timestamp| WorkflowEvent::ExecutionResumed {
            execution_id: EXECUTION_ID.to_string(),
            resume_from_node: "fix".to_string(),
            timestamp,
        };
        let events = vec![
            WorkflowEvent::ExecutionStarted {
                execution_id: EXECUTION_ID.to_string(),
                workflow_name: "repeated-reset-replay".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::Cli,
                request: "review".to_string(),
                definition: workflow,
                timestamp: 1.0,
            },
            start_node("fix-1", "fix", 1, 2.0),
            complete_node("fix-1", "fix", 1, 3.0),
            start_node("fix-2", "fix", 2, 4.0),
            complete_node("fix-2", "fix", 2, 5.0),
            start_node("round-1", "round", 1, 6.0),
            complete_node("round-1", "round", 1, 7.0),
            interrupt(8.0),
            resume(9.0),
            start_node("fix-3", "fix", 3, 10.0),
            interrupt(11.0),
            resume(12.0),
            start_node("fix-4", "fix", 4, 13.0),
            complete_node("fix-4", "fix", 4, 14.0),
            interrupt(15.0),
        ];

        assert_eq!(
            project_workflow_execution(EXECUTION_ID, &events).unwrap_err(),
            "workflow-level interruption events are unsupported"
        );
    }

    #[test]
    fn fanout_child_completion_does_not_reset_a_guard_bound_to_the_parent() {
        use crate::domain::workflow::{FanoutSpec, Rule};

        let workflow = WorkflowDefinition {
            name: "fanout-parent-reset-replay".to_string(),
            nodes: vec![
                root_sequence_node(vec![
                    ("round", Some(vec![Rule::Next("fix".to_string())])),
                    (
                        "fix",
                        Some(vec![Rule::LoopGuard {
                            max_iterations: 2,
                            on_exhausted: "done".to_string(),
                            reset_on: Some("round".to_string()),
                        }]),
                    ),
                    ("done", None),
                ]),
                NodeDefinition {
                    name: "round".to_string(),
                    kind: NodeKind::Fanout(FanoutSpec {
                        children: vec![crate::domain::workflow::ChildEntry::reference("worker")],
                        items: None,
                    }),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "worker".to_string(),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "fix".to_string(),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "done".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let start = WorkflowEvent::ExecutionStarted {
            execution_id: EXECUTION_ID.to_string(),
            workflow_name: workflow.name.clone(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            request: "review".to_string(),
            definition: workflow,
            timestamp: 1.0,
        };
        let mut events_before_child_completion = vec![start];
        for (id, attempt, started_at, completed_at) in
            [("fix-1", 1, 2.0, 3.0), ("fix-2", 2, 4.0, 5.0)]
        {
            events_before_child_completion.extend([
                WorkflowEvent::NodeStarted {
                    execution_id: EXECUTION_ID.to_string(),
                    node_execution_id: id.to_string(),
                    node_name: "fix".to_string(),
                    kind: EventNodeKindName::Session,
                    attempt,
                    fanout_parent: None,
                    timestamp: started_at,
                },
                WorkflowEvent::NodeCompleted {
                    execution_id: EXECUTION_ID.to_string(),
                    node_execution_id: id.to_string(),
                    node_name: "fix".to_string(),
                    attempt,
                    result_summary: None,
                    token_usage: None,
                    timestamp: completed_at,
                },
            ]);
        }
        events_before_child_completion.extend([
            WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "round-1".to_string(),
                node_name: "round".to_string(),
                kind: EventNodeKindName::Fanout,
                attempt: 1,
                fanout_parent: None,
                timestamp: 6.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "worker-1".to_string(),
                node_name: "worker".to_string(),
                kind: EventNodeKindName::Session,
                attempt: 1,
                fanout_parent: Some(EventFanoutParentRef {
                    parent_node: "round".to_string(),
                    parent_attempt: 1,
                    item_index: Some(0),
                    child_index: 0,
                }),
                timestamp: 7.0,
            },
        ]);
        let mut events_after_child_completion = events_before_child_completion.clone();
        events_after_child_completion.push(WorkflowEvent::NodeCompleted {
            execution_id: EXECUTION_ID.to_string(),
            node_execution_id: "worker-1".to_string(),
            node_name: "worker".to_string(),
            attempt: 1,
            result_summary: None,
            token_usage: None,
            timestamp: 8.0,
        });
        let mut events_after_parent_completion = events_after_child_completion.clone();
        events_after_parent_completion.push(WorkflowEvent::NodeCompleted {
            execution_id: EXECUTION_ID.to_string(),
            node_execution_id: "round-1".to_string(),
            node_name: "round".to_string(),
            attempt: 1,
            result_summary: None,
            token_usage: None,
            timestamp: 9.0,
        });

        let replay = |events: &[WorkflowEvent]| {
            project_retained_workflow_execution(EXECUTION_ID, events)
                .unwrap()
                .unwrap()
        };
        let before_child = replay(&events_before_child_completion);
        let after_child = replay(&events_after_child_completion);
        let after_parent = replay(&events_after_parent_completion);
        let WorkflowEvent::ExecutionStarted {
            definition: domain_workflow,
            ..
        } = &events_before_child_completion[0]
        else {
            unreachable!("the first event is ExecutionStarted");
        };
        let round_index = domain_workflow
            .nodes
            .iter()
            .position(|node| node.name == "round")
            .expect("round node exists");
        let route = |projection: &RetainedWorkflowExecutionProjection| {
            routing::route_with_reset_baselines(
                domain_workflow,
                round_index,
                None,
                &projection.node_execution_counts,
                &projection.loop_guard_reset_baselines,
            )
            .unwrap()
        };
        let in_range_count = |projection: &RetainedWorkflowExecutionProjection| {
            projection.loop_guard_reset_baselines.execution_count(
                "fix",
                projection.node_execution_counts["fix"],
                Some("round"),
            )
        };

        assert_eq!(before_child.node_execution_counts["fix"], 2);
        assert_eq!(after_child.node_execution_counts["fix"], 2);
        assert_eq!(in_range_count(&before_child), 2);
        assert_eq!(in_range_count(&after_child), 2);
        assert_eq!(
            route(&before_child),
            RouteDecision::TransitionTo("done".into())
        );
        assert_eq!(
            route(&after_child),
            RouteDecision::TransitionTo("done".into())
        );

        assert_eq!(after_parent.node_execution_counts["fix"], 2);
        assert_eq!(in_range_count(&after_parent), 0);
        assert_eq!(
            route(&after_parent),
            RouteDecision::TransitionTo("fix".into())
        );
    }

    #[test]
    fn rejects_workflow_level_resume_history() {
        let mut second_attempt = node_started("node-2", "review", EventNodeKindName::Session);
        if let WorkflowEvent::NodeStarted {
            attempt, timestamp, ..
        } = &mut second_attempt
        {
            *attempt = 2;
            *timestamp = 4.0;
        }
        let events = vec![
            started(),
            node_started("node-1", "review", EventNodeKindName::Session),
            WorkflowEvent::SessionAttached {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-1".to_string(),
                session_id: "old-session".to_string(),
                timestamp: 2.5,
            },
            WorkflowEvent::ExecutionInterrupted {
                execution_id: EXECUTION_ID.to_string(),
                reason: ExecutionInterruptionReason::Stop,
                timestamp: 3.0,
            },
            WorkflowEvent::ExecutionResumed {
                execution_id: EXECUTION_ID.to_string(),
                resume_from_node: "review".to_string(),
                timestamp: 3.5,
            },
            second_attempt,
        ];

        assert_eq!(
            project_workflow_execution(EXECUTION_ID, &events).unwrap_err(),
            "workflow-level interruption events are unsupported"
        );
    }

    #[test]
    fn fallback_usage_excludes_fanout_children_when_parent_has_usage() {
        let events = vec![
            started(),
            node_started("parent", "fanout", EventNodeKindName::Fanout),
            child_node_started("child-1", "review", 0),
            child_node_started("child-2", "review", 1),
            node_completed(
                "child-1",
                "review",
                Some(EventTokenUsage {
                    input_tokens: 3,
                    output_tokens: 4,
                }),
                3.0,
            ),
            node_completed(
                "child-2",
                "review",
                Some(EventTokenUsage {
                    input_tokens: 5,
                    output_tokens: 6,
                }),
                3.1,
            ),
            node_completed(
                "parent",
                "fanout",
                Some(EventTokenUsage {
                    input_tokens: 100,
                    output_tokens: 10,
                }),
                4.0,
            ),
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(execution.total_token_usage.input_tokens, 100);
        assert_eq!(execution.total_token_usage.output_tokens, 10);
    }

    #[test]
    fn fallback_usage_includes_fanout_children_when_parent_has_no_usage() {
        let events = vec![
            started(),
            node_started("parent", "fanout", EventNodeKindName::Fanout),
            child_node_started("child-1", "review", 0),
            child_node_started("child-2", "review", 1),
            node_completed(
                "child-1",
                "review",
                Some(EventTokenUsage {
                    input_tokens: 3,
                    output_tokens: 4,
                }),
                3.0,
            ),
            node_completed(
                "child-2",
                "review",
                Some(EventTokenUsage {
                    input_tokens: 5,
                    output_tokens: 6,
                }),
                3.1,
            ),
            node_completed("parent", "fanout", None, 4.0),
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(execution.total_token_usage.input_tokens, 8);
        assert_eq!(execution.total_token_usage.output_tokens, 10);
    }

    #[test]
    fn fallback_usage_does_not_double_count_a_child_reused_by_a_later_parent_attempt() {
        let node = |id: &str,
                    name: &str,
                    kind: NodeKindName,
                    attempt: u32,
                    status: NodeExecutionStatus,
                    session_id: Option<&str>,
                    token_usage: Option<TokenUsage>,
                    fanout_parent: Option<FanoutParentRef>| NodeExecution {
            id: id.to_string(),
            execution_id: EXECUTION_ID.to_string(),
            node_name: name.to_string(),
            kind,
            attempt,
            status,
            session_id: session_id.map(str::to_string),
            display_command: None,
            result_summary: None,
            artifact: None,
            token_usage,
            failure: None,
            fanout_parent,
            completion_signals: Default::default(),
            started_at: f64::from(attempt),
            completed_at: Some(f64::from(attempt) + 0.5),
        };
        let coordinate = |parent_attempt| FanoutParentRef {
            parent_node: "fanout".to_string(),
            parent_attempt,
            item_index: Some(0),
            child_index: 0,
        };
        let nodes = vec![
            node(
                "parent-1",
                "fanout",
                NodeKindName::Fanout,
                1,
                NodeExecutionStatus::Aborted,
                None,
                None,
                None,
            ),
            node(
                "child-1",
                "review",
                NodeKindName::Session,
                1,
                NodeExecutionStatus::Succeeded,
                Some("old-session"),
                Some(TokenUsage {
                    input_tokens: 3,
                    output_tokens: 4,
                }),
                Some(coordinate(1)),
            ),
            node(
                "parent-2",
                "fanout",
                NodeKindName::Fanout,
                2,
                NodeExecutionStatus::Succeeded,
                None,
                Some(TokenUsage {
                    input_tokens: 8,
                    output_tokens: 10,
                }),
                None,
            ),
            // Synthetic confirmation copied from child-1 during resume.
            node(
                "child-2-copy",
                "review",
                NodeKindName::Session,
                2,
                NodeExecutionStatus::Succeeded,
                None,
                None,
                Some(coordinate(2)),
            ),
        ];

        assert_eq!(
            derive_total_token_usage(&nodes),
            TokenUsage {
                input_tokens: 8,
                output_tokens: 10,
            }
        );
    }

    #[test]
    fn projects_fanout_parent_children_and_artifact() {
        let mut child_two = node_started("child-2", "check", EventNodeKindName::Session);
        let mut child_one = node_started("child-1", "check", EventNodeKindName::Session);
        if let WorkflowEvent::NodeStarted {
            fanout_parent,
            timestamp,
            ..
        } = &mut child_two
        {
            *fanout_parent = Some(EventFanoutParentRef {
                parent_node: "fanout".to_string(),
                parent_attempt: 1,
                item_index: Some(1),
                child_index: 0,
            });
            *timestamp = 3.0;
        }
        if let WorkflowEvent::NodeStarted { fanout_parent, .. } = &mut child_one {
            *fanout_parent = Some(EventFanoutParentRef {
                parent_node: "fanout".to_string(),
                parent_attempt: 1,
                item_index: Some(0),
                child_index: 0,
            });
        }

        let events = vec![
            started(),
            node_started("parent", "fanout", EventNodeKindName::Fanout),
            child_two,
            child_one,
            WorkflowEvent::ArtifactProduced {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "child-1".to_string(),
                node_name: "check".to_string(),
                contract: Some("check-result".to_string()),
                value: serde_json::json!({"ok": true}),
                request_id: None,
                submitted_at: None,
                timestamp: 3.5,
            },
            WorkflowEvent::ArtifactProduced {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "parent".to_string(),
                node_name: "fanout".to_string(),
                contract: None,
                value: serde_json::json!(["a", "b"]),
                request_id: None,
                submitted_at: None,
                timestamp: 4.0,
            },
            WorkflowEvent::ExecutionCompleted {
                execution_id: EXECUTION_ID.to_string(),
                total_token_usage: EventTokenUsage::default(),
                timestamp: 5.0,
            },
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(execution.fanouts.len(), 1);
        assert_eq!(execution.fanouts[0].parent.id, "parent");
        assert_eq!(execution.fanouts[0].children[0].id, "child-1");
        assert_eq!(execution.fanouts[0].children[1].id, "child-2");
        assert_eq!(
            execution.fanouts[0]
                .artifact
                .as_ref()
                .map(|artifact| artifact.node_name.as_str()),
            Some("fanout")
        );
        assert!(execution.fanouts[0].children[0].artifact.is_some());
        assert!(execution
            .artifacts
            .iter()
            .all(|artifact| artifact.node_name != "check"));
    }

    #[test]
    fn derives_waiting_approval_target_without_changing_workflow_status() {
        let events = vec![
            started(),
            node_started("node-1", "review", EventNodeKindName::Session),
            WorkflowEvent::SessionAttached {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-1".to_string(),
                session_id: "session-1".to_string(),
                timestamp: 2.5,
            },
            WorkflowEvent::ApprovalRequested {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "node-1".to_string(),
                node_name: "review".to_string(),
                timestamp: 3.0,
            },
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Running);
        assert_eq!(execution.current_node.as_deref(), Some("review"));
        let target = execution.approval_target.unwrap();
        assert_eq!(target.node_execution_id, "node-1");
        assert_eq!(target.session_id.as_deref(), Some("session-1"));
    }

    #[test]
    fn ambiguous_fanout_child_approvals_have_no_single_target() {
        let mut first = node_started("child-1", "review", EventNodeKindName::Session);
        let mut second = node_started("child-2", "review", EventNodeKindName::Session);
        for (event, item_index) in [(&mut first, 0), (&mut second, 1)] {
            if let WorkflowEvent::NodeStarted { fanout_parent, .. } = event {
                *fanout_parent = Some(EventFanoutParentRef {
                    parent_node: "reviews".to_string(),
                    parent_attempt: 1,
                    item_index: Some(item_index),
                    child_index: 0,
                });
            }
        }
        let events = vec![
            started(),
            node_started("parent", "reviews", EventNodeKindName::Fanout),
            first,
            second,
            WorkflowEvent::ApprovalRequested {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "child-1".to_string(),
                node_name: "review".to_string(),
                timestamp: 3.0,
            },
            WorkflowEvent::ApprovalRequested {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "child-2".to_string(),
                node_name: "review".to_string(),
                timestamp: 3.1,
            },
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Running);
        assert_eq!(execution.current_node.as_deref(), Some("reviews"));
        assert_eq!(execution.approval_target, None);
    }

    #[test]
    fn failed_fanout_child_keeps_parent_and_sibling_running() {
        let mut failed_child = node_started("child-1", "review", EventNodeKindName::Session);
        let mut sibling = node_started("child-2", "review", EventNodeKindName::Session);
        for (event, item_index) in [(&mut failed_child, 0), (&mut sibling, 1)] {
            if let WorkflowEvent::NodeStarted { fanout_parent, .. } = event {
                *fanout_parent = Some(EventFanoutParentRef {
                    parent_node: "reviews".to_string(),
                    parent_attempt: 1,
                    item_index: Some(item_index),
                    child_index: 0,
                });
            }
        }
        let events = vec![
            started(),
            node_started("parent", "reviews", EventNodeKindName::Fanout),
            failed_child,
            sibling,
            WorkflowEvent::NodeFailed {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "child-1".to_string(),
                node_name: "review".to_string(),
                attempt: 1,
                reason: "review failed".to_string(),
                failure_kind: NodeExecutionFailureKind::ValidationFailure,
                retry_count: None,
                timestamp: 3.0,
            },
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        let status = |id: &str| {
            execution
                .node_executions
                .iter()
                .find(|node| node.id == id)
                .map(|node| node.status)
                .unwrap()
        };
        assert_eq!(execution.status, ExecutionStatus::Running);
        assert_eq!(status("parent"), NodeExecutionStatus::Running);
        assert_eq!(status("child-1"), NodeExecutionStatus::Failed);
        assert_eq!(status("child-2"), NodeExecutionStatus::Running);
    }
}
