//! Pure event-log projection used by `ResumeExecution`.
//!
//! `NodeCompleted` is the only confirmation boundary. An artifact emitted by an unfinished node
//! is intentionally ignored, while completed fanout children are retained by their stable
//! parent/item/child coordinates.

use std::collections::HashMap;

use crate::adaptor::gateway::workflow::event::WorkflowEvent;
use crate::adaptor::gateway::workflow::event_projection::project_workflow_execution;
use crate::adaptor::gateway::workflow::schema::WorkflowDefinitionYaml;
use crate::domain::workflow::{
    ExecutionOrigin, ExecutionStatus, NodeExecution, NodeExecutionStatus, TokenUsage,
};

#[derive(Debug, Clone)]
pub(crate) struct ConfirmedFanoutChild {
    pub(crate) node_name: String,
    pub(crate) item_index: Option<usize>,
    pub(crate) child_index: usize,
    pub(crate) result_summary: Option<String>,
    pub(crate) artifact: Option<serde_json::Value>,
    pub(crate) contract: Option<String>,
    pub(crate) token_usage: Option<TokenUsage>,
    pub(crate) completed_at: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct ResumeProjection {
    pub(crate) execution_id: String,
    pub(crate) workflow: WorkflowDefinitionYaml,
    pub(crate) worktree_path: String,
    pub(crate) request: String,
    pub(crate) permission_mode: String,
    pub(crate) created_from: ExecutionOrigin,
    pub(crate) started_at: f64,
    pub(crate) resume_from_node: String,
    pub(crate) node_execution_counts: HashMap<String, u32>,
    pub(crate) projected_node_executions: Vec<NodeExecution>,
    pub(crate) confirmed_top_level_nodes: Vec<NodeExecution>,
    pub(crate) confirmed_fanout_children: Vec<ConfirmedFanoutChild>,
}

pub(crate) fn project_resume_checkpoint(
    execution_id: &str,
    events: &[WorkflowEvent],
) -> Result<ResumeProjection, String> {
    let execution = project_workflow_execution(execution_id, events)?
        .ok_or_else(|| format!("execution {execution_id} has no execution_started event"))?;
    if execution.status != ExecutionStatus::Interrupted {
        return Err(format!(
            "execution {execution_id} cannot resume from status {}",
            execution.status.as_str()
        ));
    }
    let resume_from_node = execution.resume_from_node.clone().ok_or_else(|| {
        format!("execution {execution_id} has no resumable NodeExecution checkpoint")
    })?;

    let starts = events
        .iter()
        .filter_map(|event| match event {
            WorkflowEvent::ExecutionStarted {
                definition,
                worktree_path,
                request,
                permission_mode,
                created_from,
                timestamp,
                ..
            } => Some((
                definition,
                worktree_path,
                request,
                permission_mode,
                *created_from,
                *timestamp,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [(workflow, worktree_path, request, permission_mode, created_from, started_at)] =
        starts.as_slice()
    else {
        return Err(format!(
            "execution {execution_id} must contain exactly one execution_started event"
        ));
    };

    let mut node_execution_counts = HashMap::new();
    for node in &execution.node_executions {
        node_execution_counts
            .entry(node.node_name.clone())
            .and_modify(|attempt: &mut u32| *attempt = (*attempt).max(node.attempt))
            .or_insert(node.attempt);
    }

    let mut confirmed_top_level_nodes = execution
        .node_executions
        .iter()
        .filter(|node| {
            node.fanout_parent.is_none() && node.status == NodeExecutionStatus::Succeeded
        })
        .cloned()
        .collect::<Vec<_>>();
    confirmed_top_level_nodes.sort_by(|left, right| {
        left.completed_at
            .unwrap_or(left.started_at)
            .partial_cmp(&right.completed_at.unwrap_or(right.started_at))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let interrupted_parent_attempt = execution
        .node_executions
        .iter()
        .filter(|node| {
            node.fanout_parent.is_none()
                && node.node_name == resume_from_node
                && node.status != NodeExecutionStatus::Succeeded
        })
        .map(|node| node.attempt)
        .max();
    // A resumed fanout can crash again before it copies reusable child facts into the new parent
    // attempt. Fold every unfinished parent attempt since the last successful parent by stable
    // child coordinate, preferring the newest confirmed copy. This keeps replay sufficient even
    // after a process restart, where the runtime-local checkpoint map no longer exists.
    let last_successful_parent_attempt = execution
        .node_executions
        .iter()
        .filter(|node| {
            node.fanout_parent.is_none()
                && node.node_name == resume_from_node
                && node.status == NodeExecutionStatus::Succeeded
        })
        .map(|node| node.attempt)
        .max()
        .unwrap_or(0);
    let mut confirmed_by_coordinate: HashMap<
        (String, Option<usize>, usize),
        (u32, ConfirmedFanoutChild),
    > = HashMap::new();
    if let Some(current_parent_attempt) = interrupted_parent_attempt {
        for node in &execution.node_executions {
            let Some(parent) = node.fanout_parent.as_ref() else {
                continue;
            };
            if parent.parent_node != resume_from_node
                || parent.parent_attempt <= last_successful_parent_attempt
                || parent.parent_attempt > current_parent_attempt
                || node.status != NodeExecutionStatus::Succeeded
            {
                continue;
            }
            let artifact = node.artifact.as_ref();
            let coordinate = (
                node.node_name.clone(),
                parent.item_index,
                parent.child_index,
            );
            let mut confirmed = ConfirmedFanoutChild {
                node_name: node.node_name.clone(),
                item_index: parent.item_index,
                child_index: parent.child_index,
                result_summary: node.result_summary.clone(),
                artifact: artifact.map(|artifact| artifact.value.clone()),
                contract: artifact.and_then(|artifact| artifact.contract.clone()),
                token_usage: node.token_usage.clone(),
                completed_at: node.completed_at.unwrap_or(node.started_at),
            };
            match confirmed_by_coordinate.get(&coordinate) {
                Some((attempt, _)) if *attempt >= parent.parent_attempt => {}
                _ => {
                    // Synthetic copies intentionally omit NodeCompleted.token_usage to avoid
                    // charging the same work twice in the event stream. Preserve the original
                    // confirmed usage as provenance when the newest copy has none.
                    if confirmed.token_usage.is_none() {
                        confirmed.token_usage = confirmed_by_coordinate
                            .get(&coordinate)
                            .and_then(|(_, previous)| previous.token_usage.clone());
                    }
                    confirmed_by_coordinate.insert(coordinate, (parent.parent_attempt, confirmed));
                }
            }
        }
    }
    let mut confirmed_fanout_children = confirmed_by_coordinate
        .into_values()
        .map(|(_, child)| child)
        .collect::<Vec<_>>();
    confirmed_fanout_children.sort_by(|left, right| {
        (
            left.item_index.unwrap_or(0),
            left.child_index,
            &left.node_name,
        )
            .cmp(&(
                right.item_index.unwrap_or(0),
                right.child_index,
                &right.node_name,
            ))
    });

    Ok(ResumeProjection {
        execution_id: execution_id.to_string(),
        workflow: (*workflow).clone(),
        worktree_path: (*worktree_path).clone(),
        request: (*request).clone(),
        permission_mode: (*permission_mode).clone(),
        created_from: *created_from,
        started_at: *started_at,
        resume_from_node,
        node_execution_counts,
        projected_node_executions: execution.node_executions,
        confirmed_top_level_nodes,
        confirmed_fanout_children,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::event::{
        FanoutParentRef, TokenUsage as EventTokenUsage,
    };
    use crate::adaptor::gateway::workflow::schema::{
        FanoutSpec, NodeDefinition, NodeKind, NodeKindName, SessionSpec,
    };
    use crate::domain::workflow::ExecutionInterruptionReason;

    const EXECUTION_ID: &str = "00000000-0000-4000-8000-000000000133";

    fn workflow() -> WorkflowDefinitionYaml {
        WorkflowDefinitionYaml {
            name: "resume-test".to_string(),
            nodes: vec![
                NodeDefinition {
                    name: "prepare".to_string(),
                    kind: NodeKind::Session(SessionSpec::default()),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "fanout".to_string(),
                    kind: NodeKind::Fanout(FanoutSpec {
                        child: vec!["review".to_string()],
                        items: None,
                    }),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "review".to_string(),
                    kind: NodeKind::Session(SessionSpec::default()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    fn base_events() -> Vec<WorkflowEvent> {
        vec![WorkflowEvent::ExecutionStarted {
            execution_id: EXECUTION_ID.to_string(),
            workflow_name: "resume-test".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            request: "review".to_string(),
            permission_mode: crate::domain::agent_session::PermissionMode::ASK.to_string(),
            definition: workflow(),
            timestamp: 1.0,
        }]
    }

    #[test]
    fn projects_execution_start_permission_mode_into_resume_checkpoint() {
        for expected in [
            crate::domain::agent_session::PermissionMode::EDIT,
            crate::domain::agent_session::PermissionMode::FULL,
        ] {
            let mut events = base_events();
            let WorkflowEvent::ExecutionStarted {
                permission_mode, ..
            } = &mut events[0]
            else {
                unreachable!("base event must be ExecutionStarted");
            };
            *permission_mode = expected.to_string();
            events.extend([
                WorkflowEvent::NodeStarted {
                    execution_id: EXECUTION_ID.to_string(),
                    node_execution_id: "prepare-1".to_string(),
                    node_name: "prepare".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
                WorkflowEvent::ExecutionInterrupted {
                    execution_id: EXECUTION_ID.to_string(),
                    reason: ExecutionInterruptionReason::Stop,
                    timestamp: 3.0,
                },
            ]);

            let checkpoint = project_resume_checkpoint(EXECUTION_ID, &events).unwrap();
            assert_eq!(checkpoint.permission_mode, expected);
        }
    }

    #[test]
    fn resumes_unconfirmed_node_after_last_completed_node() {
        let mut events = base_events();
        events.extend([
            WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "prepare-1".to_string(),
                node_name: "prepare".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: 2.0,
            },
            WorkflowEvent::NodeCompleted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "prepare-1".to_string(),
                node_name: "prepare".to_string(),
                attempt: 1,
                result_summary: Some("done".to_string()),
                token_usage: None,
                timestamp: 3.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "fanout-1".to_string(),
                node_name: "fanout".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                fanout_parent: None,
                timestamp: 4.0,
            },
            WorkflowEvent::ExecutionInterrupted {
                execution_id: EXECUTION_ID.to_string(),
                reason: ExecutionInterruptionReason::Crash,
                timestamp: 5.0,
            },
        ]);

        let checkpoint = project_resume_checkpoint(EXECUTION_ID, &events).unwrap();

        assert_eq!(checkpoint.resume_from_node, "fanout");
        assert_eq!(checkpoint.confirmed_top_level_nodes.len(), 1);
        assert_eq!(checkpoint.confirmed_top_level_nodes[0].node_name, "prepare");
        assert_eq!(checkpoint.node_execution_counts["fanout"], 1);
    }

    #[test]
    fn reuses_only_fanout_children_confirmed_by_node_completed() {
        let mut events = base_events();
        events.extend([
            WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "fanout-1".to_string(),
                node_name: "fanout".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                fanout_parent: None,
                timestamp: 2.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: Some(FanoutParentRef {
                    parent_node: "fanout".to_string(),
                    parent_attempt: 1,
                    item_index: Some(0),
                    child_index: 0,
                }),
                timestamp: 3.0,
            },
            WorkflowEvent::ArtifactProduced {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "review-1".to_string(),
                node_name: "review".to_string(),
                contract: Some("review".to_string()),
                value: serde_json::json!({"ok": true}),
                request_id: None,
                submitted_at: None,
                timestamp: 4.0,
            },
            WorkflowEvent::NodeCompleted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "review-1".to_string(),
                node_name: "review".to_string(),
                attempt: 1,
                result_summary: Some("done".to_string()),
                token_usage: Some(EventTokenUsage {
                    input_tokens: 1,
                    output_tokens: 2,
                }),
                timestamp: 4.5,
            },
            WorkflowEvent::ExecutionInterrupted {
                execution_id: EXECUTION_ID.to_string(),
                reason: ExecutionInterruptionReason::Stop,
                timestamp: 5.0,
            },
        ]);

        let checkpoint = project_resume_checkpoint(EXECUTION_ID, &events).unwrap();

        assert_eq!(checkpoint.confirmed_fanout_children.len(), 1);
        assert_eq!(checkpoint.confirmed_fanout_children[0].item_index, Some(0));
        assert_eq!(
            checkpoint.confirmed_fanout_children[0].artifact,
            Some(serde_json::json!({"ok": true}))
        );
    }

    #[test]
    fn reuses_a_confirmed_fanout_child_without_an_artifact() {
        let mut events = base_events();
        events.extend([
            WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "fanout-1".to_string(),
                node_name: "fanout".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                fanout_parent: None,
                timestamp: 2.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: Some(FanoutParentRef {
                    parent_node: "fanout".to_string(),
                    parent_attempt: 1,
                    item_index: Some(0),
                    child_index: 0,
                }),
                timestamp: 3.0,
            },
            WorkflowEvent::NodeCompleted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "review-1".to_string(),
                node_name: "review".to_string(),
                attempt: 1,
                result_summary: Some("done without artifact".to_string()),
                token_usage: None,
                timestamp: 4.0,
            },
            WorkflowEvent::ExecutionInterrupted {
                execution_id: EXECUTION_ID.to_string(),
                reason: ExecutionInterruptionReason::Crash,
                timestamp: 5.0,
            },
        ]);

        let checkpoint = project_resume_checkpoint(EXECUTION_ID, &events).unwrap();

        assert_eq!(checkpoint.confirmed_fanout_children.len(), 1);
        assert_eq!(checkpoint.confirmed_fanout_children[0].artifact, None);
        assert_eq!(
            checkpoint.confirmed_fanout_children[0]
                .result_summary
                .as_deref(),
            Some("done without artifact")
        );
    }

    #[test]
    fn carries_confirmed_child_across_a_second_interrupted_parent_attempt() {
        let mut events = base_events();
        events.extend([
            WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "fanout-1".to_string(),
                node_name: "fanout".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                fanout_parent: None,
                timestamp: 2.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "review-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: Some(FanoutParentRef {
                    parent_node: "fanout".to_string(),
                    parent_attempt: 1,
                    item_index: Some(0),
                    child_index: 0,
                }),
                timestamp: 3.0,
            },
            WorkflowEvent::ArtifactProduced {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "review-1".to_string(),
                node_name: "review".to_string(),
                contract: Some("review".to_string()),
                value: serde_json::json!({"ok": true}),
                request_id: None,
                submitted_at: None,
                timestamp: 4.0,
            },
            WorkflowEvent::NodeCompleted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "review-1".to_string(),
                node_name: "review".to_string(),
                attempt: 1,
                result_summary: Some("done".to_string()),
                token_usage: None,
                timestamp: 4.5,
            },
            WorkflowEvent::ExecutionInterrupted {
                execution_id: EXECUTION_ID.to_string(),
                reason: ExecutionInterruptionReason::Crash,
                timestamp: 5.0,
            },
            WorkflowEvent::ExecutionResumed {
                execution_id: EXECUTION_ID.to_string(),
                resume_from_node: "fanout".to_string(),
                timestamp: 6.0,
            },
            // The process crashes before attempt 2 can copy the completed child facts.
            WorkflowEvent::NodeStarted {
                execution_id: EXECUTION_ID.to_string(),
                node_execution_id: "fanout-2".to_string(),
                node_name: "fanout".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 2,
                fanout_parent: None,
                timestamp: 6.0,
            },
            WorkflowEvent::ExecutionInterrupted {
                execution_id: EXECUTION_ID.to_string(),
                reason: ExecutionInterruptionReason::Crash,
                timestamp: 7.0,
            },
        ]);

        let checkpoint = project_resume_checkpoint(EXECUTION_ID, &events).unwrap();

        assert_eq!(checkpoint.resume_from_node, "fanout");
        assert_eq!(checkpoint.node_execution_counts["fanout"], 2);
        assert_eq!(checkpoint.confirmed_fanout_children.len(), 1);
        assert_eq!(
            checkpoint.confirmed_fanout_children[0].artifact,
            Some(serde_json::json!({"ok": true}))
        );
    }
}
