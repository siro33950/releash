use std::collections::HashSet;

use crate::domain::workflow::status_aggregation::{NodeProgress, RepresentativeStatus};
use crate::domain::workflow::WorkflowRuntimeSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSessionProjection {
    pub node_execution_id: Option<String>,
    pub session_id: Option<String>,
    pub node_name: String,
    pub attempt: Option<u32>,
    pub group_node_name: String,
    pub group_attempt: Option<u32>,
    pub progress: NodeProgress,
    pub representative: RepresentativeStatus,
    pub order: usize,
}

pub fn current_attempt(state: &WorkflowRuntimeSnapshot) -> Option<u32> {
    state
        .node_execution_counts
        .get(&state.current_node_name)
        .copied()
        .or(Some(1))
}

struct ProjectionInput<'a> {
    node_execution_id: Option<String>,
    session_id: Option<String>,
    node_name: String,
    attempt: Option<u32>,
    group_node_name: String,
    group_attempt: Option<u32>,
    status: &'a str,
    order: usize,
}

fn projection(input: ProjectionInput<'_>) -> NodeSessionProjection {
    NodeSessionProjection {
        node_execution_id: input.node_execution_id,
        session_id: input.session_id,
        node_name: input.node_name,
        attempt: input.attempt,
        group_node_name: input.group_node_name,
        group_attempt: input.group_attempt,
        progress: NodeProgress::from_status_str(input.status),
        representative: RepresentativeStatus::from_status_str(input.status),
        order: input.order,
    }
}

pub fn collect_node_session_projections(
    state: &WorkflowRuntimeSnapshot,
) -> Vec<NodeSessionProjection> {
    let mut projections = Vec::new();
    for (order, execution) in state.node_executions.iter().enumerate() {
        let Some(session_id) = execution.session_id.clone() else {
            continue;
        };
        let (group_node_name, group_attempt) = execution
            .fanout_parent
            .as_ref()
            .map(|parent| (parent.parent_node.clone(), Some(parent.parent_attempt)))
            .unwrap_or_else(|| (execution.node_name.clone(), Some(execution.attempt)));
        projections.push(projection(ProjectionInput {
            node_execution_id: Some(execution.id.clone()),
            session_id: Some(session_id),
            node_name: execution.node_name.clone(),
            attempt: Some(execution.attempt),
            group_node_name,
            group_attempt,
            status: execution.status.as_str(),
            order,
        }));
    }

    let current_session_is_projected =
        state.current_session_id.as_ref().is_some_and(|session_id| {
            projections
                .iter()
                .any(|projection| projection.session_id.as_ref() == Some(session_id))
        });
    if !current_session_is_projected
        && (state.current_session_id.is_some() || state.state.is_active())
        && state.node_executions.is_empty()
    {
        let attempt = current_attempt(state);
        projections.push(projection(ProjectionInput {
            node_execution_id: None,
            session_id: state.current_session_id.clone(),
            node_name: state.current_node_name.clone(),
            attempt,
            group_node_name: state.current_node_name.clone(),
            group_attempt: attempt,
            status: state.state.as_str(),
            order: 0,
        }));
    }

    retain_unique_node_session_projections(&mut projections);
    projections
}

fn retain_unique_node_session_projections(projections: &mut Vec<NodeSessionProjection>) {
    let mut seen = HashSet::new();
    projections.retain(|projection| {
        seen.insert((
            projection.session_id.clone(),
            projection.node_execution_id.clone(),
            projection.node_name.clone(),
            projection.attempt,
            projection.group_node_name.clone(),
            projection.group_attempt,
        ))
    });
}

#[cfg(test)]
pub fn collect_node_session_ids(state: &WorkflowRuntimeSnapshot) -> HashSet<String> {
    collect_node_session_projections(state)
        .into_iter()
        .filter_map(|projection| projection.session_id)
        .collect::<HashSet<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        FanoutChildSnapshot, FanoutParentRef, NodeExecution, NodeExecutionStatus, NodeHistoryEntry,
        NodeKindName, RuntimeExecutionState, WorkflowDefinition, NODE_STATUS_ABORTED,
        NODE_STATUS_COMPLETED,
    };
    use std::collections::HashMap;

    fn state() -> WorkflowRuntimeSnapshot {
        WorkflowRuntimeSnapshot {
            execution_id: "exec-1".to_string(),
            workflow_name: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: crate::domain::workflow::ExecutionOrigin::Cli,
            request: "ship it".to_string(),
            error_reason: None,
            state: RuntimeExecutionState::Running,
            current_node_index: 1,
            current_node_name: "current".to_string(),
            current_session_id: Some("current-session".to_string()),
            node_history: vec![NodeHistoryEntry {
                node_name: "done".to_string(),
                completed_at: 1.0,
                result: Some("ok".to_string()),
                session_id: Some("done-session".to_string()),
                token_usage: None,
                artifact: None,
                attempt: 1,
                fanout_children: Some(vec![FanoutChildSnapshot {
                    node_name: "child".to_string(),
                    session_id: Some("child-session".to_string()),
                    result: Some("ok".to_string()),
                    attempt: 1,
                    completed_at: 2.0,
                    artifact: None,
                    contract: None,
                    state: NODE_STATUS_COMPLETED.to_string(),
                    failure_kind: None,
                    failure_disposition: None,
                }]),
                state: NODE_STATUS_COMPLETED.to_string(),
            }],
            node_execution_counts: HashMap::new(),
            workflow_definition: WorkflowDefinition {
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: Vec::new(),
            },
            total_token_usage: Default::default(),
            artifacts: HashMap::new(),
            node_executions: vec![NodeExecution {
                id: "ne-child".to_string(),
                execution_id: "exec-1".to_string(),
                node_name: "running-child".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                status: NodeExecutionStatus::Running,
                session_id: Some("fanout-session".to_string()),
                display_command: None,
                result_summary: None,
                artifact: None,
                token_usage: None,
                failure: None,
                fanout_parent: Some(FanoutParentRef {
                    parent_node: "current".to_string(),
                    parent_attempt: 1,
                    item_index: None,
                    child_index: 0,
                }),
                completion_signals: Default::default(),
                started_at: 1.5,
                completed_at: None,
            }],
            started_at: 0.0,
            updated_at: 2.0,
        }
    }

    #[test]
    fn collects_session_ids_only_from_node_executions_when_they_are_available() {
        let ids = collect_node_session_ids(&state());

        assert_eq!(ids, HashSet::from(["fanout-session".to_string()]));
    }

    #[test]
    fn projects_group_key_attempt_progress_and_order_from_node_executions() {
        let projections = collect_node_session_projections(&state());

        assert_eq!(projections.len(), 1);
        let fanout = &projections[0];
        assert_eq!(fanout.node_name, "running-child");
        assert_eq!(fanout.node_execution_id.as_deref(), Some("ne-child"));
        assert_eq!(fanout.attempt, Some(1));
        assert_eq!(fanout.group_node_name, "current");
        assert_eq!(fanout.group_attempt, Some(1));
        assert_eq!(fanout.progress, NodeProgress::Running);
        assert_eq!(fanout.representative, RepresentativeStatus::Running);
        assert_eq!(fanout.order, 0);
    }

    #[test]
    fn legacy_history_does_not_create_node_session_projections() {
        let aborted_state = WorkflowRuntimeSnapshot {
            execution_id: "exec-aborted".to_string(),
            workflow_name: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: crate::domain::workflow::ExecutionOrigin::Cli,
            request: "ship it".to_string(),
            error_reason: None,
            state: RuntimeExecutionState::Aborted,
            current_node_index: 0,
            current_node_name: "plan".to_string(),
            current_session_id: None,
            node_history: vec![
                NodeHistoryEntry {
                    node_name: "plan".to_string(),
                    completed_at: 1.0,
                    result: None,
                    session_id: Some("aborted-node-session".to_string()),
                    token_usage: None,
                    artifact: None,
                    attempt: 1,
                    fanout_children: None,
                    state: NODE_STATUS_ABORTED.to_string(),
                },
                NodeHistoryEntry {
                    node_name: "fanout-review".to_string(),
                    completed_at: 2.0,
                    result: None,
                    session_id: None,
                    token_usage: None,
                    artifact: None,
                    attempt: 1,
                    fanout_children: Some(vec![
                        FanoutChildSnapshot {
                            node_name: "child-a".to_string(),
                            session_id: Some("session-a".to_string()),
                            result: Some("LGTM".to_string()),
                            attempt: 1,
                            completed_at: 1.5,
                            artifact: None,
                            contract: None,
                            state: NODE_STATUS_COMPLETED.to_string(),
                            failure_kind: None,
                            failure_disposition: None,
                        },
                        FanoutChildSnapshot {
                            node_name: "child-b".to_string(),
                            session_id: Some("session-b".to_string()),
                            result: None,
                            attempt: 1,
                            completed_at: 2.0,
                            artifact: None,
                            contract: None,
                            state: NODE_STATUS_ABORTED.to_string(),
                            failure_kind: None,
                            failure_disposition: None,
                        },
                    ]),
                    state: NODE_STATUS_ABORTED.to_string(),
                },
            ],
            node_execution_counts: HashMap::new(),
            workflow_definition: WorkflowDefinition {
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: Vec::new(),
            },
            total_token_usage: Default::default(),
            artifacts: HashMap::new(),
            node_executions: vec![],
            started_at: 0.0,
            updated_at: 2.0,
        };

        let ids = collect_node_session_ids(&aborted_state);

        assert!(ids.is_empty());
    }

    #[test]
    fn duplicate_legacy_history_session_is_not_projected_twice() {
        let mut state = state();
        state.node_history[0].session_id = Some("fanout-session".to_string());
        state.node_history[0].fanout_children.as_mut().unwrap()[0].session_id =
            Some("fanout-session".to_string());

        let projections = collect_node_session_projections(&state);

        assert_eq!(projections.len(), 1);
        assert_eq!(
            projections[0].node_execution_id.as_deref(),
            Some("ne-child")
        );
    }
}
