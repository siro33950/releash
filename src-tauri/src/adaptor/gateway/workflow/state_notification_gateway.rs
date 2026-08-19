use tauri::Emitter;

use crate::adaptor::protocol::workflow::{
    WorkflowExecutionChangedPayloadView, WorkflowExecutionView,
};
use crate::domain::workflow::services::event_replay::derive_workflow_execution_fields;
use crate::domain::workflow::{
    Artifact, ExecutionStatus, NodeExecution, RuntimeExecutionState, WorkflowExecution,
    WorkflowRuntimeSnapshot,
};

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
        error_reason: state.error_reason.clone(),
        interruption_reason: None,
        resume_from_node: None,
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
        #[cfg(test)]
        RuntimeExecutionState::WaitingApproval => ExecutionStatus::WaitingApproval,
        RuntimeExecutionState::Completed => ExecutionStatus::Completed,
        RuntimeExecutionState::Aborted => ExecutionStatus::Aborted,
        #[cfg(test)]
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
    state
        .node_history
        .iter()
        .rev()
        .find(|entry| entry.node_name == node.node_name && entry.attempt == node.attempt)
        .and_then(|entry| entry.result.clone())
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
    let view = build_workflow_execution_view_from_snapshot(state).await;
    emit_workflow_execution_view(app, view);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::adaptor::gateway::workflow::event::WorkflowEvent;
    use crate::adaptor::gateway::workflow::schema::{
        NodeDefinition as EventNodeDefinition, NodeKindName as EventNodeKindName,
        WorkflowDefinitionYaml as EventWorkflowDefinitionYaml,
    };
    use crate::domain::workflow::TokenUsage as EventTokenUsage;
    use crate::domain::workflow::{
        ExecutionParentRef, NodeDefinition, NodeExecutionFailure, NodeExecutionFailureKind,
        NodeExecutionStatus, NodeHistoryEntry, NodeKindName, RuntimeArtifact, TokenUsage,
        WorkflowDefinition,
    };

    /// event 列を事実ログへ写像し、fold で読み model を導出する
    /// （canonical な読み経路とruntime snapshot mapper の parity を固定する）。
    fn fold_projection(
        execution_id: &str,
        events: &[WorkflowEvent],
    ) -> crate::domain::workflow::WorkflowExecution {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                tmp.path().to_path_buf(),
            ),
        )
        .unwrap();
        crate::adaptor::gateway::workflow::test_support::append_canonical_events(&store, events)
            .unwrap();
        let backend = crate::adaptor::gateway::workflow::fact_log::FactLogReadBackend::Live(store);
        let folded =
            crate::adaptor::gateway::workflow::fact_log::fold_tree_from(&backend, execution_id)
                .unwrap()
                .unwrap();
        crate::domain::workflow::services::fact_replay::derive_read_model(&folded)
    }

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
                definition: EventWorkflowDefinitionYaml {
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
                parent: None,
                timestamp: 1.0,
            },
            WorkflowEvent::NodeSubmitReceived {
                execution_id: execution_id.to_string(),
                node_execution_id: node_execution_id.to_string(),
                timestamp: 3.0,
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
            WorkflowEvent::NodeStopReceived {
                execution_id: execution_id.to_string(),
                node_execution_id: node_execution_id.to_string(),
                timestamp: 4.0,
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
                timestamp: 4.0,
            },
        ];
        let event_projection = fold_projection(execution_id, &events);
        // 成果の produced_at は settle（stop 受理決着）時刻。
        let artifact = Artifact {
            node_name: "review".to_string(),
            contract: Some("review_result".to_string()),
            value: value.clone(),
            produced_at: 4.0,
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
                current_node_name: Some("review".to_string()),
                current_session_id: None,
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
                        completed_at: 4.0,
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
                    display_command: None,
                    result_summary: Some("approve".to_string()),
                    artifact: Some(artifact),
                    token_usage: Some(TokenUsage {
                        input_tokens: 3,
                        output_tokens: 2,
                    }),
                    failure: None,
                    parent: None,
                    completion_signals: crate::domain::workflow::NodeCompletionSignalState::Ready,
                    started_at: 1.0,
                    completed_at: Some(4.0),
                }],
                started_at: 1.0,
                updated_at: 4.0,
            });

        assert_eq!(runtime_projection, event_projection);
    }

    #[test]
    fn node_failure_does_not_create_a_workflow_terminal_projection() {
        let execution_id = "00000000-0000-4000-8000-000000000002";
        let event_parent =
            |item_index| ExecutionParentRef::fanout_child("parent", Some(item_index), 0);
        let events = vec![
            WorkflowEvent::ExecutionStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: crate::domain::workflow::ExecutionOrigin::Cli,
                request: "ship it".to_string(),
                definition: EventWorkflowDefinitionYaml {
                    name: "review".to_string(),
                    nodes: vec![
                        EventNodeDefinition {
                            name: "review".to_string(),
                            ..Default::default()
                        },
                        EventNodeDefinition {
                            name: "reviews".to_string(),
                            kind: crate::domain::workflow::NodeKind::Fanout(
                                crate::domain::workflow::FanoutSpec {
                                    children: vec![crate::domain::workflow::ChildEntry::reference(
                                        "review",
                                    )],
                                    items: None,
                                },
                            ),
                            ..Default::default()
                        },
                    ],
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
                parent: None,
                timestamp: 2.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "child-1".to_string(),
                node_name: "review".to_string(),
                kind: EventNodeKindName::Session,
                attempt: 1,
                parent: Some(event_parent(0)),
                timestamp: 2.1,
            },
            WorkflowEvent::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "child-2".to_string(),
                node_name: "review".to_string(),
                kind: EventNodeKindName::Session,
                attempt: 1,
                parent: Some(event_parent(1)),
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
        ];
        let event_projection = fold_projection(execution_id, &events);
        let failure = || NodeExecutionFailure {
            reason: "review failed".to_string(),
            kind: NodeExecutionFailureKind::ValidationFailure,
        };
        let domain_parent =
            |item_index| ExecutionParentRef::fanout_child("parent", Some(item_index), 0);
        let node = |id: &str,
                    node_name: &str,
                    kind: NodeKindName,
                    status: NodeExecutionStatus,
                    failure: Option<NodeExecutionFailure>,
                    parent: Option<ExecutionParentRef>,
                    started_at: f64,
                    completed_at: Option<f64>| NodeExecution {
            id: id.to_string(),
            execution_id: execution_id.to_string(),
            node_name: node_name.to_string(),
            kind,
            attempt: 1,
            status,
            session_id: None,
            display_command: None,
            result_summary: None,
            artifact: None,
            token_usage: None,
            failure,
            parent,
            completion_signals: Default::default(),
            started_at,
            completed_at,
        };
        let runtime_projection =
            workflow_execution_from_runtime_snapshot(WorkflowRuntimeSnapshot {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: crate::domain::workflow::ExecutionOrigin::Cli,
                request: "ship it".to_string(),
                error_reason: None,
                state: RuntimeExecutionState::Running,
                current_node_name: Some("reviews".to_string()),
                current_session_id: None,
                node_history: Vec::new(),
                workflow_definition: WorkflowDefinition {
                    name: "review".to_string(),
                    ..Default::default()
                },
                total_token_usage: TokenUsage::default(),
                artifacts: HashMap::new(),
                node_executions: vec![
                    node(
                        "parent",
                        "reviews",
                        NodeKindName::Fanout,
                        NodeExecutionStatus::Running,
                        None,
                        None,
                        2.0,
                        None,
                    ),
                    node(
                        "child-1",
                        "review",
                        NodeKindName::Session,
                        NodeExecutionStatus::Failed,
                        Some(failure()),
                        Some(domain_parent(0)),
                        2.1,
                        Some(3.0),
                    ),
                    node(
                        "child-2",
                        "review",
                        NodeKindName::Session,
                        NodeExecutionStatus::Running,
                        None,
                        Some(domain_parent(1)),
                        2.2,
                        None,
                    ),
                ],
                started_at: 1.0,
                updated_at: 3.0,
            });

        // 主張: node の失敗は workflow を terminal にしない。
        // live（runtime snapshot）は engine の観測どおり Failed を示す。
        assert_eq!(runtime_projection.status, ExecutionStatus::Running);
        assert_eq!(
            runtime_projection.node_executions[1].status,
            NodeExecutionStatus::Failed
        );
        // fold（事実ログ）は D1a により process_exited を再開可能な中断として
        // 導出する（Failed は live の観測・Paused は永続からの導出）。
        assert_eq!(event_projection.status, ExecutionStatus::Running);
        assert_eq!(
            event_projection.node_executions[1].status,
            NodeExecutionStatus::Paused
        );
        assert_eq!(event_projection.completed_at, None);
        assert_eq!(runtime_projection.completed_at, None);
    }
}
