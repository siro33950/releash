//! On-demand projection from the append-only workflow execution event log.

use std::collections::HashMap;

use crate::adaptor::gateway::workflow::event::{
    FanoutParentRef as EventFanoutParentRef, TokenUsage as EventTokenUsage, WorkflowEvent,
};
use crate::adaptor::gateway::workflow::schema::NodeKindName as EventNodeKindName;
use crate::domain::workflow::services::routing::{self, RouteDecision};
use crate::domain::workflow::{
    ApprovalTarget, Artifact, ExecutionStatus, Fanout, FanoutParentRef, NodeExecution,
    NodeExecutionFailure, NodeExecutionStatus, NodeKindName, TokenUsage, WorkflowExecution,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DerivedWorkflowExecutionFields {
    pub(crate) status: ExecutionStatus,
    pub(crate) current_node: Option<String>,
    pub(crate) approval_target: Option<ApprovalTarget>,
    pub(crate) artifacts: Vec<Artifact>,
    pub(crate) fanouts: Vec<Fanout>,
}

/// Projects one public workflow execution read model from its event stream.
///
/// An empty stream (or an audit-only stream without `ExecutionStarted`) means the
/// execution does not exist. Existing NDJSON shapes are not interpreted here.
pub fn project_workflow_execution(
    execution_id: &str,
    events: &[WorkflowEvent],
) -> Result<Option<WorkflowExecution>, String> {
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
    let mut authoritative_total_usage = None;

    for event in events {
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
                    result_summary: None,
                    artifact: None,
                    token_usage: None,
                    failure: None,
                    fanout_parent: fanout_parent.as_ref().map(fanout_parent_to_domain),
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
                node.artifact = Some(artifact.clone());
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
                node.status = NodeExecutionStatus::Succeeded;
                node.result_summary = result_summary.clone();
                node.token_usage = token_usage.as_ref().map(token_usage_to_domain);
                node.failure = None;
                node.completed_at = Some(*timestamp);
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
                node.status = NodeExecutionStatus::Failed;
                node.failure = Some(NodeExecutionFailure {
                    reason: reason.clone(),
                    kind: *failure_kind,
                });
                node.completed_at = Some(*timestamp);
            }
            WorkflowEvent::ApprovalRequested {
                node_execution_id,
                node_name,
                ..
            } => {
                let node = node_mut(&mut execution, node_execution_id, "approval_requested")?;
                require_node_name(node, node_name, "approval_requested")?;
                node.status = NodeExecutionStatus::WaitingApproval;
            }
            WorkflowEvent::ApprovalResolved {
                node_execution_id,
                node_name,
                ..
            } => {
                let node = node_mut(&mut execution, node_execution_id, "approval_resolved")?;
                require_node_name(node, node_name, "approval_resolved")?;
                node.status = NodeExecutionStatus::Running;
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
            WorkflowEvent::ExecutionFailed {
                reason,
                failure_kind,
                timestamp,
                ..
            } => {
                execution.status = ExecutionStatus::Failed;
                execution.completed_at = Some(*timestamp);
                execution.error_reason = Some(reason.clone());
                execution.interruption_reason = None;
                execution.resume_from_node = None;
                close_failed_execution_nodes(
                    &mut execution.node_executions,
                    *timestamp,
                    NodeExecutionFailure {
                        reason: reason.clone(),
                        kind: *failure_kind,
                    },
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
            WorkflowEvent::ExecutionInterrupted {
                reason, timestamp, ..
            } => {
                if execution.status.is_finished() {
                    return Err(format!(
                        "execution {execution_id} cannot be interrupted from status {}",
                        execution.status.as_str()
                    ));
                }
                let resume_from_node =
                    derive_resume_from_node(definition, &execution.node_executions)?;
                execution.status = ExecutionStatus::Interrupted;
                execution.completed_at = None;
                execution.error_reason = None;
                execution.interruption_reason = Some(*reason);
                execution.resume_from_node = resume_from_node;
                close_active_nodes(
                    &mut execution.node_executions,
                    NodeExecutionStatus::Aborted,
                    *timestamp,
                    None,
                );
            }
            WorkflowEvent::ExecutionResumed {
                resume_from_node, ..
            } => {
                if execution.status != ExecutionStatus::Interrupted {
                    return Err(format!(
                        "execution {execution_id} cannot resume from status {}",
                        execution.status.as_str()
                    ));
                }
                if execution
                    .resume_from_node
                    .as_deref()
                    .is_some_and(|expected| expected != resume_from_node)
                {
                    return Err(format!(
                        "execution_resumed resume_from_node {resume_from_node} does not match projected checkpoint {}",
                        execution.resume_from_node.as_deref().unwrap_or_default()
                    ));
                }
                execution.status = ExecutionStatus::Running;
                execution.completed_at = None;
                execution.error_reason = None;
                execution.interruption_reason = None;
                execution.resume_from_node = None;
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
    Ok(Some(execution))
}

fn derive_resume_from_node(
    definition: &crate::adaptor::gateway::workflow::schema::WorkflowDefinitionYaml,
    nodes: &[NodeExecution],
) -> Result<Option<String>, String> {
    let Some(latest) = nodes.iter().rev().find(|node| node.fanout_parent.is_none()) else {
        return Ok(definition.nodes.first().map(|node| node.name.clone()));
    };
    if latest.status != NodeExecutionStatus::Succeeded {
        return Ok(Some(latest.node_name.clone()));
    }

    let workflow = crate::adaptor::gateway::workflow::domain_mapping::workflow_definition_to_domain(
        definition,
    );
    let current_index = workflow
        .nodes
        .iter()
        .position(|node| node.name == latest.node_name)
        .ok_or_else(|| {
            format!(
                "completed node '{}' is absent from execution workflow snapshot",
                latest.node_name
            )
        })?;
    let mut attempts = HashMap::new();
    for node in nodes {
        attempts
            .entry(node.node_name.clone())
            .and_modify(|attempt: &mut u32| *attempt = (*attempt).max(node.attempt))
            .or_insert(node.attempt);
    }
    let artifact = latest.artifact.as_ref().map(|artifact| &artifact.value);
    routing::route(&workflow, current_index, artifact, &attempts)
        .map_err(|error| error.to_string())
        .map(|decision| match decision {
            RouteDecision::TransitionTo(node) => Some(node),
            RouteDecision::Completed => None,
        })
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

fn close_failed_execution_nodes(
    nodes: &mut [NodeExecution],
    completed_at: f64,
    terminal_failure: NodeExecutionFailure,
) {
    let failed_fanout_parents = nodes
        .iter()
        .filter(|node| node.status == NodeExecutionStatus::Failed)
        .filter_map(|node| {
            let parent = node.fanout_parent.as_ref()?;
            Some((
                parent.parent_node.clone(),
                parent.parent_attempt,
                node.failure
                    .clone()
                    .unwrap_or_else(|| terminal_failure.clone()),
            ))
        })
        .collect::<Vec<_>>();

    for (parent_node, parent_attempt, failure) in failed_fanout_parents {
        if let Some(parent) = nodes.iter_mut().find(|node| {
            node.fanout_parent.is_none()
                && node.node_name == parent_node
                && node.attempt == parent_attempt
                && node.status.is_active()
        }) {
            parent.status = NodeExecutionStatus::Failed;
            parent.failure = Some(failure);
            parent.completed_at = Some(completed_at);
        }
    }

    if !nodes
        .iter()
        .any(|node| node.status == NodeExecutionStatus::Failed)
    {
        let fallback = nodes
            .iter()
            .rposition(|node| node.fanout_parent.is_none() && node.status.is_active())
            .or_else(|| nodes.iter().rposition(|node| node.status.is_active()));
        if let Some(index) = fallback {
            nodes[index].status = NodeExecutionStatus::Failed;
            nodes[index].failure = Some(terminal_failure);
            nodes[index].completed_at = Some(completed_at);
        }
    }

    close_active_nodes(nodes, NodeExecutionStatus::Aborted, completed_at, None);
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
    if status.is_finished() || status == ExecutionStatus::Interrupted {
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
        return (
            ExecutionStatus::WaitingApproval,
            current_node,
            approval_target,
        );
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
    use crate::adaptor::gateway::workflow::event::FanoutParentRef as EventFanoutParentRef;
    use crate::adaptor::gateway::workflow::schema::{
        NodeDefinition, NodeKind, WorkflowDefinitionYaml,
    };
    use crate::domain::workflow::{
        ExecutionInterruptionReason, ExecutionOrigin, NodeExecutionFailureKind,
    };

    const EXECUTION_ID: &str = "00000000-0000-4000-8000-000000000001";

    fn definition() -> WorkflowDefinitionYaml {
        WorkflowDefinitionYaml {
            name: "review".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                kind: NodeKind::default(),
                ..NodeDefinition::default()
            }],
        }
    }

    fn started() -> WorkflowEvent {
        WorkflowEvent::ExecutionStarted {
            execution_id: EXECUTION_ID.to_string(),
            workflow_name: "review".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            request: "please review".to_string(),
            permission_mode: "ask".to_string(),
            definition: definition(),
            timestamp: 1.0,
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
    fn projects_failed_execution_and_node_failure() {
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
            WorkflowEvent::ExecutionFailed {
                execution_id: EXECUTION_ID.to_string(),
                reason: "invalid result".to_string(),
                failure_kind: NodeExecutionFailureKind::ValidationFailure,
                timestamp: 4.0,
            },
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Failed);
        assert_eq!(execution.error_reason.as_deref(), Some("invalid result"));
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
            WorkflowEvent::ExecutionFailed {
                execution_id: EXECUTION_ID.to_string(),
                reason: "failed".to_string(),
                failure_kind: NodeExecutionFailureKind::ValidationFailure,
                timestamp: 5.0,
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
    fn projects_interrupted_execution_and_active_node_with_reason() {
        let events = vec![
            started(),
            node_started("node-1", "review", EventNodeKindName::Session),
            WorkflowEvent::ExecutionInterrupted {
                execution_id: EXECUTION_ID.to_string(),
                reason: ExecutionInterruptionReason::Stop,
                timestamp: 3.0,
            },
        ];

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Interrupted);
        assert_eq!(execution.completed_at, None);
        assert_eq!(execution.error_reason, None);
        assert_eq!(
            execution.interruption_reason,
            Some(ExecutionInterruptionReason::Stop)
        );
        assert_eq!(execution.resume_from_node.as_deref(), Some("review"));
        assert_eq!(
            execution.node_executions[0].status,
            NodeExecutionStatus::Aborted
        );
        assert_eq!(execution.node_executions[0].completed_at, Some(3.0));
    }

    #[test]
    fn interrupted_projection_routes_from_last_confirmed_node_when_no_node_is_active() {
        let mut workflow = definition();
        workflow.nodes[0].name = "prepare".to_string();
        workflow.nodes[0].rules = vec![crate::adaptor::gateway::workflow::schema::Rule::Next(
            "review".to_string(),
        )];
        workflow
            .nodes
            .push(crate::adaptor::gateway::workflow::schema::NodeDefinition {
                name: "review".to_string(),
                kind: crate::adaptor::gateway::workflow::schema::NodeKind::default(),
                ..Default::default()
            });
        let events = vec![
            WorkflowEvent::ExecutionStarted {
                execution_id: EXECUTION_ID.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::Cli,
                request: "please review".to_string(),
                permission_mode: "ask".to_string(),
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

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Interrupted);
        assert_eq!(execution.resume_from_node.as_deref(), Some("review"));
        assert_eq!(
            execution.node_executions[0].status,
            NodeExecutionStatus::Succeeded
        );
    }

    #[test]
    fn execution_resumed_opens_a_new_attempt_without_reopening_old_session() {
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

        let execution = project_workflow_execution(EXECUTION_ID, &events)
            .unwrap()
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Running);
        assert_eq!(execution.current_node.as_deref(), Some("review"));
        assert_eq!(execution.interruption_reason, None);
        assert_eq!(execution.resume_from_node, None);
        assert_eq!(
            execution.node_executions[0].status,
            NodeExecutionStatus::Aborted
        );
        assert_eq!(
            execution.node_executions[0].session_id.as_deref(),
            Some("old-session")
        );
        assert_eq!(
            execution.node_executions[1].status,
            NodeExecutionStatus::Running
        );
        assert_eq!(execution.node_executions[1].session_id, None);
        assert_eq!(execution.node_executions[1].attempt, 2);
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
            result_summary: None,
            artifact: None,
            token_usage,
            failure: None,
            fanout_parent,
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
    fn derives_waiting_approval_target_from_node_events() {
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
        assert_eq!(execution.status, ExecutionStatus::WaitingApproval);
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
        assert_eq!(execution.status, ExecutionStatus::WaitingApproval);
        assert_eq!(execution.current_node.as_deref(), Some("reviews"));
        assert_eq!(execution.approval_target, None);
    }

    #[test]
    fn failed_fanout_marks_parent_and_failed_child_failed_and_aborts_sibling() {
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
            WorkflowEvent::ExecutionFailed {
                execution_id: EXECUTION_ID.to_string(),
                reason: "review failed".to_string(),
                failure_kind: NodeExecutionFailureKind::ValidationFailure,
                timestamp: 4.0,
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
        assert_eq!(status("parent"), NodeExecutionStatus::Failed);
        assert_eq!(status("child-1"), NodeExecutionStatus::Failed);
        assert_eq!(status("child-2"), NodeExecutionStatus::Aborted);
    }
}
