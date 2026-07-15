use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::adaptor::gateway::workflow::event_projection::derive_workflow_execution_fields;
use crate::adaptor::protocol::workflow::{
    WorkflowExecutionChangedPayloadView, WorkflowExecutionView,
};
use crate::domain::workflow::{
    Artifact, ExecutionInterruptionReason, ExecutionStatus, NodeExecution, RuntimeExecutionState,
    WorkflowExecution, WorkflowRuntimeSnapshot,
};

fn optional_arc_state<R, T>(app: &tauri::AppHandle<R>) -> Option<Arc<T>>
where
    R: tauri::Runtime,
    T: Send + Sync + 'static,
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.state::<Arc<T>>().inner().clone()
    }))
    .ok()
}

fn emit_workflow_execution_view<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    view: WorkflowExecutionView,
) {
    let payload = WorkflowExecutionChangedPayloadView {
        worktree_path: view.worktree_path.clone(),
        workflow_execution: view,
    };
    let _ = app.emit("workflow-execution-changed", payload);
}

/// Maps the already-updated runtime snapshot to the public execution read model.
///
/// This path is incremental: it never replays the event log after a runtime commit.
/// Historical/file-direct queries use the event-log projection repository instead.
pub(crate) fn workflow_execution_from_runtime_snapshot(
    state: WorkflowRuntimeSnapshot,
) -> WorkflowExecution {
    let status = execution_status(&state.state);
    let mut node_executions = state.node_executions.clone();
    enrich_node_executions(&mut node_executions, &state);
    let derived = derive_workflow_execution_fields(
        &state.request,
        state.started_at,
        status,
        &node_executions,
    );
    let interruption_reason = if status == ExecutionStatus::Interrupted {
        Some(
            state
                .error_reason
                .as_deref()
                .and_then(ExecutionInterruptionReason::from_reason)
                .unwrap_or(ExecutionInterruptionReason::Crash),
        )
    } else {
        None
    };
    let resume_from_node =
        (status == ExecutionStatus::Interrupted).then(|| state.current_node_name.clone());
    let error_reason = if status == ExecutionStatus::Interrupted {
        None
    } else {
        state.error_reason.clone()
    };

    WorkflowExecution {
        id: state.execution_id,
        workflow_name: state.workflow_name,
        status: derived.status,
        current_node: derived.current_node,
        created_from: state.created_from,
        worktree_path: state.worktree_path,
        started_at: state.started_at,
        updated_at: state.updated_at,
        completed_at: derived.status.is_terminal().then_some(state.updated_at),
        error_reason,
        interruption_reason,
        resume_from_node,
        total_token_usage: state.total_token_usage,
        node_executions,
        artifacts: derived.artifacts,
        fanouts: derived.fanouts,
        approval_target: derived.approval_target,
    }
}

fn execution_status(state: &RuntimeExecutionState) -> ExecutionStatus {
    match state {
        RuntimeExecutionState::Running => ExecutionStatus::Running,
        RuntimeExecutionState::WaitingApproval => ExecutionStatus::WaitingApproval,
        RuntimeExecutionState::Completed => ExecutionStatus::Completed,
        RuntimeExecutionState::Failed { .. } => ExecutionStatus::Failed,
        RuntimeExecutionState::Aborted => ExecutionStatus::Aborted,
        RuntimeExecutionState::Interrupted => ExecutionStatus::Interrupted,
    }
}

fn enrich_node_executions(nodes: &mut [NodeExecution], state: &WorkflowRuntimeSnapshot) {
    for node in nodes {
        if node.result_summary.is_none() {
            node.result_summary = result_summary(state, node);
        }
        let runtime_artifact = state
            .artifacts
            .get(&node.node_name)
            .filter(|artifact| artifact.attempt == node.attempt);
        let definition_contract = state
            .workflow_definition
            .nodes
            .iter()
            .find(|definition| definition.name == node.node_name)
            .and_then(|definition| definition.artifact.clone());
        if let Some(artifact) = node.artifact.as_mut() {
            artifact.contract = runtime_artifact
                .and_then(|runtime| runtime.contract.clone())
                .or(definition_contract.clone());
            artifact.produced_at = runtime_artifact
                .map(|runtime| runtime.completed_at)
                .or(node.completed_at)
                .unwrap_or(node.started_at);
        } else if let Some(runtime_artifact) = runtime_artifact {
            if let Some(value) = runtime_artifact.artifact.clone() {
                node.artifact = Some(Artifact {
                    node_name: node.node_name.clone(),
                    contract: runtime_artifact
                        .contract
                        .clone()
                        .or(definition_contract.clone()),
                    value,
                    produced_at: runtime_artifact.completed_at,
                });
            }
        }
    }
}

fn result_summary(state: &WorkflowRuntimeSnapshot, node: &NodeExecution) -> Option<String> {
    if node.fanout_parent.is_none() {
        return state
            .node_history
            .iter()
            .rev()
            .find(|entry| entry.node_name == node.node_name && entry.attempt == node.attempt)
            .and_then(|entry| entry.result.clone());
    }
    let parent = node.fanout_parent.as_ref()?;
    state
        .node_history
        .iter()
        .rev()
        .find(|entry| {
            entry.node_name == parent.parent_node && entry.attempt == parent.parent_attempt
        })
        .and_then(|entry| entry.fanout_children.as_ref())
        .and_then(|children| {
            children
                .iter()
                .find(|child| child.node_name == node.node_name && child.attempt == node.attempt)
        })
        .and_then(|child| child.result.clone())
}

pub(crate) async fn build_workflow_execution_view_from_snapshot(
    state: WorkflowRuntimeSnapshot,
) -> WorkflowExecutionView {
    crate::adaptor::presenter::workflow::workflow_execution_to_view(
        workflow_execution_from_runtime_snapshot(state),
    )
}

pub(crate) async fn emit_workflow_execution_from_snapshot<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    _worktree_path: &str,
    state: WorkflowRuntimeSnapshot,
) {
    let execution_id = state.execution_id.clone();
    let execution_state = state.state.as_str().to_string();
    let node_session_projections =
        crate::domain::workflow::services::node_session_projection::collect_node_session_projections(
            &state,
        );
    let workflow_agent_state =
        crate::usecase::agent_session::status::AgentStatusCenter::workflow_execution_status_to_agent_state(
            &state.state,
        );
    let updated_at = state.updated_at;
    let worktree_path = state.worktree_path.clone();
    let view = build_workflow_execution_view_from_snapshot(state).await;
    emit_workflow_execution_view(app, view);
    sync_agent_status(
        app,
        &worktree_path,
        &execution_id,
        &execution_state,
        node_session_projections,
        workflow_agent_state,
        updated_at,
    );
}

fn sync_agent_status<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    worktree_path: &str,
    execution_id: &str,
    execution_state: &str,
    node_session_projections: Vec<
        crate::domain::workflow::services::node_session_projection::NodeSessionProjection,
    >,
    workflow_agent_state: Option<crate::usecase::agent_session::status::AgentState>,
    updated_at: f64,
) {
    let Some(center) =
        optional_arc_state::<_, crate::usecase::agent_session::status::AgentStatusCenter>(app)
    else {
        return;
    };
    for changes in center.sync_workflow_node_session_statuses(
        worktree_path,
        execution_id,
        execution_state,
        node_session_projections,
    ) {
        crate::adaptor::presenter::agent_status::emit_agent_status_changes(app, changes);
    }
    let changes = center.update_workflow_snapshot(
        worktree_path,
        execution_id,
        workflow_agent_state,
        updated_at,
    );
    crate::adaptor::presenter::agent_status::emit_agent_status_changes(app, changes);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::adaptor::gateway::workflow::event::{
        FanoutParentRef as EventFanoutParentRef, TokenUsage as EventTokenUsage, WorkflowEvent,
    };
    use crate::adaptor::gateway::workflow::event_projection::project_workflow_execution;
    use crate::adaptor::gateway::workflow::schema::{
        NodeDefinition as EventNodeDefinition, NodeKindName as EventNodeKindName,
        Workflow as EventWorkflow,
    };
    use crate::domain::workflow::{
        FanoutParentRef, NodeDefinition, NodeExecutionFailure, NodeExecutionFailureKind,
        NodeExecutionStatus, NodeHistoryEntry, NodeKindName, RuntimeArtifact, TokenUsage,
        WorkflowDefinition,
    };

    #[test]
    fn runtime_snapshot_mapper_matches_event_log_projection() {
        let execution_id = "00000000-0000-4000-8000-000000000001";
        let node_execution_id = "node-1";
        let value = serde_json::json!({"verdict": "approve"});
        let events = vec![
            WorkflowEvent::ExecutionStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: crate::domain::workflow::ExecutionOrigin::Cli,
                request: "ship it".to_string(),
                permission_mode: "ask".to_string(),
                definition: EventWorkflow {
                    name: "review".to_string(),
                    nodes: vec![EventNodeDefinition {
                        name: "review".to_string(),
                        artifact: Some("review_result".to_string()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                timestamp: 1.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: node_execution_id.to_string(),
                node_name: "review".to_string(),
                kind: EventNodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 2.0,
            },
            WorkflowEvent::ArtifactProduced {
                execution_id: execution_id.to_string(),
                node_execution_id: node_execution_id.to_string(),
                node_name: "review".to_string(),
                contract: Some("review_result".to_string()),
                value: value.clone(),
                request_id: None,
                submitted_at: None,
                timestamp: 3.0,
            },
            WorkflowEvent::NodeCompleted {
                execution_id: execution_id.to_string(),
                node_execution_id: node_execution_id.to_string(),
                node_name: "review".to_string(),
                attempt: 1,
                result_summary: Some("approve".to_string()),
                token_usage: Some(EventTokenUsage {
                    input_tokens: 3,
                    output_tokens: 2,
                }),
                timestamp: 4.0,
            },
            WorkflowEvent::ExecutionCompleted {
                execution_id: execution_id.to_string(),
                total_token_usage: EventTokenUsage {
                    input_tokens: 3,
                    output_tokens: 2,
                },
                timestamp: 5.0,
            },
        ];
        let event_projection = project_workflow_execution(execution_id, &events)
            .unwrap()
            .unwrap();
        let artifact = Artifact {
            node_name: "review".to_string(),
            contract: Some("review_result".to_string()),
            value: value.clone(),
            produced_at: 3.0,
        };
        let runtime_projection =
            workflow_execution_from_runtime_snapshot(WorkflowRuntimeSnapshot {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: crate::domain::workflow::ExecutionOrigin::Cli,
                request: "ship it".to_string(),
                error_reason: None,
                state: RuntimeExecutionState::Completed,
                current_node_index: 0,
                current_node_name: "review".to_string(),
                current_session_id: None,
                total_nodes: 1,
                node_history: vec![NodeHistoryEntry {
                    node_name: "review".to_string(),
                    completed_at: 4.0,
                    result: Some("approve".to_string()),
                    session_id: None,
                    token_usage: Some(TokenUsage {
                        input_tokens: 3,
                        output_tokens: 2,
                    }),
                    artifact: Some(value.clone()),
                    attempt: 1,
                    fanout_children: None,
                    state: crate::domain::workflow::NODE_STATUS_COMPLETED.to_string(),
                }],
                node_execution_counts: HashMap::from([("review".to_string(), 1)]),
                workflow_definition: WorkflowDefinition {
                    name: "review".to_string(),
                    nodes: vec![NodeDefinition {
                        name: "review".to_string(),
                        artifact: Some("review_result".to_string()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                total_token_usage: TokenUsage {
                    input_tokens: 3,
                    output_tokens: 2,
                },
                node_statuses: HashMap::new(),
                artifacts: HashMap::from([(
                    "review".to_string(),
                    RuntimeArtifact {
                        node_name: "review".to_string(),
                        attempt: 1,
                        session_id: None,
                        result: Some("approve".to_string()),
                        artifact: Some(value),
                        contract: Some("review_result".to_string()),
                        token_usage: Some(TokenUsage {
                            input_tokens: 3,
                            output_tokens: 2,
                        }),
                        completed_at: 3.0,
                    },
                )]),
                node_executions: vec![NodeExecution {
                    id: node_execution_id.to_string(),
                    execution_id: execution_id.to_string(),
                    node_name: "review".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    status: NodeExecutionStatus::Succeeded,
                    session_id: None,
                    result_summary: Some("approve".to_string()),
                    artifact: Some(artifact),
                    token_usage: Some(TokenUsage {
                        input_tokens: 3,
                        output_tokens: 2,
                    }),
                    failure: None,
                    fanout_parent: None,
                    started_at: 2.0,
                    completed_at: Some(4.0),
                }],
                approval_operations: None,
                stall_observations: Vec::new(),
                started_at: 1.0,
                updated_at: 5.0,
            });

        assert_eq!(runtime_projection, event_projection);
    }

    #[test]
    fn failed_fanout_runtime_mapper_matches_event_log_projection() {
        let execution_id = "00000000-0000-4000-8000-000000000002";
        let event_parent = |item_index| EventFanoutParentRef {
            parent_node: "reviews".to_string(),
            parent_attempt: 1,
            item_index: Some(item_index),
            child_index: 0,
        };
        let events = vec![
            WorkflowEvent::ExecutionStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: crate::domain::workflow::ExecutionOrigin::Cli,
                request: "ship it".to_string(),
                permission_mode: "ask".to_string(),
                definition: EventWorkflow {
                    name: "review".to_string(),
                    ..Default::default()
                },
                timestamp: 1.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "parent".to_string(),
                node_name: "reviews".to_string(),
                kind: EventNodeKindName::Fanout,
                attempt: 1,
                fanout_parent: None,
                timestamp: 2.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "child-1".to_string(),
                node_name: "review".to_string(),
                kind: EventNodeKindName::Session,
                attempt: 1,
                fanout_parent: Some(event_parent(0)),
                timestamp: 2.1,
            },
            WorkflowEvent::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "child-2".to_string(),
                node_name: "review".to_string(),
                kind: EventNodeKindName::Session,
                attempt: 1,
                fanout_parent: Some(event_parent(1)),
                timestamp: 2.2,
            },
            WorkflowEvent::NodeFailed {
                execution_id: execution_id.to_string(),
                node_execution_id: "child-1".to_string(),
                node_name: "review".to_string(),
                attempt: 1,
                reason: "review failed".to_string(),
                failure_kind: NodeExecutionFailureKind::ValidationFailure,
                retry_count: None,
                timestamp: 3.0,
            },
            WorkflowEvent::ExecutionFailed {
                execution_id: execution_id.to_string(),
                reason: "review failed".to_string(),
                failure_kind: NodeExecutionFailureKind::ValidationFailure,
                timestamp: 4.0,
            },
        ];
        let event_projection = project_workflow_execution(execution_id, &events)
            .unwrap()
            .unwrap();
        let failure = || NodeExecutionFailure {
            reason: "review failed".to_string(),
            kind: NodeExecutionFailureKind::ValidationFailure,
        };
        let domain_parent = |item_index| FanoutParentRef {
            parent_node: "reviews".to_string(),
            parent_attempt: 1,
            item_index: Some(item_index),
            child_index: 0,
        };
        let node = |id: &str,
                    node_name: &str,
                    kind: NodeKindName,
                    status: NodeExecutionStatus,
                    failure: Option<NodeExecutionFailure>,
                    fanout_parent: Option<FanoutParentRef>,
                    started_at: f64,
                    completed_at: f64| NodeExecution {
            id: id.to_string(),
            execution_id: execution_id.to_string(),
            node_name: node_name.to_string(),
            kind,
            attempt: 1,
            status,
            session_id: None,
            result_summary: None,
            artifact: None,
            token_usage: None,
            failure,
            fanout_parent,
            started_at,
            completed_at: Some(completed_at),
        };
        let runtime_projection =
            workflow_execution_from_runtime_snapshot(WorkflowRuntimeSnapshot {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: crate::domain::workflow::ExecutionOrigin::Cli,
                request: "ship it".to_string(),
                error_reason: Some("review failed".to_string()),
                state: RuntimeExecutionState::Failed {
                    reason: "review failed".to_string(),
                    kind: NodeExecutionFailureKind::ValidationFailure,
                    retry_count: None,
                },
                current_node_index: 0,
                current_node_name: "reviews".to_string(),
                current_session_id: None,
                total_nodes: 2,
                node_history: Vec::new(),
                node_execution_counts: HashMap::from([
                    ("reviews".to_string(), 1),
                    ("review".to_string(), 1),
                ]),
                workflow_definition: WorkflowDefinition {
                    name: "review".to_string(),
                    ..Default::default()
                },
                total_token_usage: TokenUsage::default(),
                node_statuses: HashMap::new(),
                artifacts: HashMap::new(),
                node_executions: vec![
                    node(
                        "parent",
                        "reviews",
                        NodeKindName::Fanout,
                        NodeExecutionStatus::Failed,
                        Some(failure()),
                        None,
                        2.0,
                        4.0,
                    ),
                    node(
                        "child-1",
                        "review",
                        NodeKindName::Session,
                        NodeExecutionStatus::Failed,
                        Some(failure()),
                        Some(domain_parent(0)),
                        2.1,
                        3.0,
                    ),
                    node(
                        "child-2",
                        "review",
                        NodeKindName::Session,
                        NodeExecutionStatus::Aborted,
                        None,
                        Some(domain_parent(1)),
                        2.2,
                        4.0,
                    ),
                ],
                approval_operations: None,
                stall_observations: Vec::new(),
                started_at: 1.0,
                updated_at: 4.0,
            });

        assert_eq!(runtime_projection, event_projection);
    }

    #[test]
    fn interrupted_runtime_snapshot_exposes_checkpoint_without_error_or_current_node() {
        let execution_id = "00000000-0000-4000-8000-000000000099";
        let projection = workflow_execution_from_runtime_snapshot(WorkflowRuntimeSnapshot {
            execution_id: execution_id.to_string(),
            workflow_name: "review".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: crate::domain::workflow::ExecutionOrigin::Cli,
            request: "ship it".to_string(),
            error_reason: Some("stop".to_string()),
            state: RuntimeExecutionState::Interrupted,
            current_node_index: 0,
            current_node_name: "review".to_string(),
            current_session_id: None,
            total_nodes: 1,
            node_history: Vec::new(),
            node_execution_counts: HashMap::from([("review".to_string(), 1)]),
            workflow_definition: WorkflowDefinition {
                name: "review".to_string(),
                ..Default::default()
            },
            total_token_usage: TokenUsage::default(),
            node_statuses: HashMap::new(),
            artifacts: HashMap::new(),
            node_executions: vec![NodeExecution {
                id: "review-1".to_string(),
                execution_id: execution_id.to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                status: NodeExecutionStatus::Aborted,
                session_id: Some("old-session".to_string()),
                result_summary: None,
                artifact: None,
                token_usage: None,
                failure: None,
                fanout_parent: None,
                started_at: 2.0,
                completed_at: Some(3.0),
            }],
            approval_operations: None,
            stall_observations: Vec::new(),
            started_at: 1.0,
            updated_at: 3.0,
        });

        assert_eq!(projection.status, ExecutionStatus::Interrupted);
        assert_eq!(projection.current_node, None);
        assert_eq!(projection.completed_at, None);
        assert_eq!(projection.error_reason, None);
        assert_eq!(
            projection.interruption_reason,
            Some(ExecutionInterruptionReason::Stop)
        );
        assert_eq!(projection.resume_from_node.as_deref(), Some("review"));
    }
}
