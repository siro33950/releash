//! Fanout activation and commit preparation.

use std::collections::HashMap;

#[cfg(test)]
use super::node_settings::WorkflowDefaults;

use crate::adaptor::gateway::workflow::workflow_host::execution_state::{
    DomainWorkflowExecution, FanoutChildRuntime, FanoutChildRuntimeState, FanoutRuntimeState,
};
use crate::domain::workflow::entities::workflow_execution::TransitionOutcome;
use crate::domain::workflow::services::fanout as workflow_fanout;
use crate::domain::workflow::FanoutParentRef;
use crate::domain::workflow::NodeDefinition;
use crate::domain::workflow::{RuntimeArtifact, TokenUsage};
use crate::usecase::workflow::runtime_error::{
    workflow_error_to_runtime_error, WorkflowRuntimeError,
};
use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

#[derive(Debug, Clone)]
pub(crate) struct FanoutChildExpansion {
    pub(crate) node_execution_id: String,
    pub(crate) node: NodeDefinition,
    pub(crate) attempt: u32,
    pub(crate) item: Option<serde_json::Value>,
    pub(crate) item_index: Option<usize>,
    pub(crate) child_index: usize,
    pub(crate) reused: Option<ReusableFanoutChild>,
}

/// A child output confirmed by `NodeCompleted` before the parent fanout was interrupted.
#[derive(Debug, Clone)]
pub(crate) struct ReusableFanoutChild {
    pub(crate) result: Option<String>,
    pub(crate) display_command: Option<String>,
    pub(crate) artifact: Option<serde_json::Value>,
    pub(crate) contract: Option<String>,
    pub(crate) token_usage: Option<TokenUsage>,
    pub(crate) completed_at: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct FanoutStartContext {
    pub(crate) children: Vec<FanoutChildExpansion>,
    pub(crate) parent_node_name: String,
    pub(crate) parent_attempt: u32,
    pub(crate) execution_id: String,
    pub(crate) request: Option<String>,
}

impl FanoutStartContext {
    #[cfg(test)]
    pub(crate) fn child_node_names(&self) -> Vec<String> {
        self.children
            .iter()
            .map(|child| child.node.name.clone())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FanoutPromptInputs {
    pub(crate) artifacts: HashMap<String, RuntimeArtifact>,
}

pub(crate) struct FanoutChildSessionSetup {
    pub(crate) node_execution_id: String,
    pub(crate) session_id: String,
}

pub(crate) fn prepare_fanout_start_context(
    exec: &DomainWorkflowExecution,
) -> Result<FanoutStartContext, WorkflowRuntimeError> {
    let node = exec
        .workflow
        .nodes
        .get(exec.current_node_index)
        .ok_or_else(|| {
            WorkflowRuntimeError::InvalidState(format!(
                "current_node_index {} is out of bounds for workflow '{}'",
                exec.current_node_index, exec.workflow.name
            ))
        })?;
    let fanout = node.fanout().ok_or_else(|| {
        WorkflowRuntimeError::InvalidState(format!(
            "StartFanout requires fanout node '{}'",
            node.name
        ))
    })?;
    if fanout.child.is_empty() {
        return Err(WorkflowRuntimeError::InvalidState(format!(
            "StartFanout requires child references for node '{}'",
            node.name
        )));
    }
    let parent_attempt = exec
        .node_execution_counts
        .get(&node.name)
        .copied()
        .unwrap_or(1);
    let domain_workflow = exec.workflow.clone();
    let domain_artifacts = exec.artifacts.clone();
    let domain_node = domain_workflow
        .nodes
        .get(exec.current_node_index)
        .ok_or_else(|| {
            WorkflowRuntimeError::InvalidState(format!(
                "current_node_index {} is out of bounds for workflow '{}'",
                exec.current_node_index, exec.workflow.name
            ))
        })?;
    let domain_fanout = domain_node.fanout().ok_or_else(|| {
        WorkflowRuntimeError::InvalidState(format!(
            "StartFanout requires fanout node '{}'",
            node.name
        ))
    })?;
    let expansion_plan = workflow_fanout::plan_fanout_expansion(
        &domain_workflow,
        &domain_fanout.child,
        domain_fanout.items.as_ref(),
        &domain_artifacts,
        &exec.node_execution_counts,
    )
    .map_err(workflow_error_to_runtime_error)?;
    let children = expansion_plan
        .children
        .into_iter()
        .map(|child| {
            let node = exec
                .workflow
                .nodes
                .iter()
                .find(|node| node.name == child.node_name)
                .cloned()
                .ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "fanout child node '{}' is undefined",
                        child.node_name
                    ))
                })?;
            Ok(FanoutChildExpansion {
                node_execution_id: uuid::Uuid::new_v4().to_string(),
                node,
                attempt: child.attempt,
                item: child.item,
                item_index: child.item_index,
                child_index: child.child_index,
                reused: None,
            })
        })
        .collect::<Result<Vec<_>, WorkflowRuntimeError>>()?;
    Ok(FanoutStartContext {
        parent_node_name: node.name.clone(),
        parent_attempt,
        children,
        execution_id: exec.id.clone(),
        request: exec.request.clone(),
    })
}

pub(crate) fn fanout_prompt_inputs(exec: &DomainWorkflowExecution) -> FanoutPromptInputs {
    FanoutPromptInputs {
        artifacts: exec.artifacts.clone(),
    }
}

pub(crate) fn apply_fanout_runtime_state(
    exec: &mut DomainWorkflowExecution,
    fanout_start: &FanoutStartContext,
    session_setups: &[FanoutChildSessionSetup],
    timestamp: f64,
) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
    let parent_node_execution_id = exec
        .active_current_node_execution_id()
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            WorkflowRuntimeError::InvalidState(format!(
                "active fanout parent NodeExecution for '{}' is unavailable",
                fanout_start.parent_node_name
            ))
        })?;
    let children = fanout_start
        .children
        .iter()
        .map(|child| {
            exec.increase_node_attempt_count_to(child.node.name.clone(), child.attempt, timestamp);
            exec.start_node_execution(
                child.node.name.clone(),
                child.node.kind_name(),
                child.attempt,
                Some(FanoutParentRef {
                    parent_node: fanout_start.parent_node_name.clone(),
                    parent_attempt: fanout_start.parent_attempt,
                    item_index: child.item_index,
                    child_index: child.child_index,
                }),
                child.node_execution_id.clone(),
                timestamp,
            );
            let session_id = session_setups
                .iter()
                .find(|setup| setup.node_execution_id == child.node_execution_id)
                .map(|setup| setup.session_id.clone())
                .unwrap_or_default();
            if !session_id.is_empty() {
                let _ = exec.attach_child_node_session(
                    &child.node_execution_id,
                    session_id.clone(),
                    timestamp,
                );
            }
            if let Some(display_command) = child
                .reused
                .as_ref()
                .and_then(|reused| reused.display_command.clone())
            {
                let _ = exec.record_node_display_command(
                    &child.node_execution_id,
                    display_command,
                    timestamp,
                );
            }
            if let Some(reused) = child.reused.as_ref() {
                exec.record_reused_node_completion(
                    &child.node_execution_id,
                    reused.artifact.clone(),
                    reused.token_usage.clone(),
                    timestamp,
                );
            }
            FanoutChildRuntime {
                node_execution_id: child.node_execution_id.clone(),
                node_name: child.node.name.clone(),
                session_id,
                state: if child.reused.is_some() {
                    FanoutChildRuntimeState::Completed
                } else {
                    FanoutChildRuntimeState::Running
                },
                result: child
                    .reused
                    .as_ref()
                    .and_then(|reused| reused.result.clone()),
                artifact: child
                    .reused
                    .as_ref()
                    .and_then(|reused| reused.artifact.clone()),
                contract: child
                    .reused
                    .as_ref()
                    .and_then(|reused| reused.contract.clone())
                    .or_else(|| child.node.artifact.clone()),
                failure_kind: None,
                failure_disposition: None,
                token_usage: child
                    .reused
                    .as_ref()
                    .and_then(|reused| reused.token_usage.clone())
                    .unwrap_or_default(),
                attempt: child.attempt,
                completed_at: child.reused.as_ref().map(|reused| reused.completed_at),
            }
        })
        .collect();
    let outcome = exec.install_fanout(
        FanoutRuntimeState {
            parent_node_name: fanout_start.parent_node_name.clone(),
            parent_node_execution_id,
            children,
        },
        timestamp,
    );
    if !matches!(
        outcome,
        TransitionOutcome::Applied | TransitionOutcome::AlreadyApplied
    ) {
        return Err(WorkflowRuntimeError::InvalidState(format!(
            "fanout runtime installation was rejected by the aggregate: {outcome:?}"
        )));
    }
    RuntimeCommitSnapshot::from_execution(exec)
}

pub(crate) struct FanoutParentCompletionPlan {
    pub(crate) parent_artifact: RuntimeArtifact,
    pub(crate) history_entry: crate::domain::workflow::NodeHistoryEntry,
}

pub(crate) fn plan_fanout_parent_completion(
    parent_node_name: &str,
    parent_attempt: u32,
    children: &[FanoutChildRuntime],
    timestamp: f64,
) -> FanoutParentCompletionPlan {
    let children: Vec<workflow_fanout::FanoutChildCompletionInput> = children
        .iter()
        .map(|child| workflow_fanout::FanoutChildCompletionInput {
            node_name: child.node_name.clone(),
            session_id: (!child.session_id.is_empty()).then(|| child.session_id.clone()),
            result: child.result.clone(),
            artifact: child.artifact.clone().unwrap_or(serde_json::Value::Null),
            contract: child.contract.clone(),
            token_usage: child.token_usage.clone(),
            attempt: child.attempt,
            completed_at: child.completed_at.unwrap_or(timestamp),
            state: match child.state {
                FanoutChildRuntimeState::Running => crate::domain::workflow::NODE_STATUS_RUNNING,
                FanoutChildRuntimeState::Completed => {
                    crate::domain::workflow::NODE_STATUS_COMPLETED
                }
                FanoutChildRuntimeState::Failed => crate::domain::workflow::NODE_STATUS_FAILED,
                FanoutChildRuntimeState::Interrupted => {
                    crate::domain::workflow::NODE_STATUS_INTERRUPTED
                }
            }
            .to_string(),
            failure_kind: child.failure_kind,
            failure_disposition: child.failure_disposition,
        })
        .collect();
    let plan = workflow_fanout::plan_fanout_parent_completion(
        parent_node_name,
        parent_attempt,
        &children,
        timestamp,
    );
    FanoutParentCompletionPlan {
        parent_artifact: plan.parent_artifact,
        history_entry: plan.history_entry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::RuntimeExecutionState;
    use crate::domain::workflow::{
        CommandSpec, FanoutSpec, ItemsSource, NodeDefinition, NodeKind, SessionSpec,
        WorkflowDefinition,
    };

    fn workflow_execution_fixture(
        node: NodeDefinition,
    ) -> crate::adaptor::gateway::workflow::workflow_host::execution_state::DomainWorkflowExecution
    {
        workflow_execution_with_nodes(vec![node])
    }

    fn workflow_execution_with_nodes(
        nodes: Vec<NodeDefinition>,
    ) -> crate::adaptor::gateway::workflow::workflow_host::execution_state::DomainWorkflowExecution
    {
        let entry = nodes
            .first()
            .map(|node| node.name.clone())
            .unwrap_or_else(|| "main".to_string());
        crate::adaptor::gateway::workflow::workflow_host::execution_state::domain_workflow_execution! {
            id: "execution-1".to_string(),
            workflow: WorkflowDefinition {
                name: "test-workflow".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes,
                entry,
            },
            lifecycle: DomainWorkflowExecution::lifecycle_from_state(RuntimeExecutionState::Running),
            current_node_index: 0,
            node_execution_counts: HashMap::new(),
            loop_guard_reset_baselines: Default::default(),
            node_history: Vec::new(),
            workflow_defaults: WorkflowDefaults,
            worktree_path: "/tmp/repo".to_string(),
            created_from: crate::domain::workflow::ExecutionOrigin::Cli,
            error_reason: None,
            started_at: 1.0,
            updated_at: 1.0,
            current_session_id: None,
            current_node_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: Vec::new(),
            request: Some("ship it".to_string()),
            fanout_runtime: None,
            current_stall_observations: Vec::new(),
        }
    }

    fn session_node(name: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec::default()),
            ..Default::default()
        }
    }

    fn command_node(name: &str, command: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: command.to_string(),
            }),
            ..Default::default()
        }
    }

    fn fanout_node(children: &[&str], items: Option<ItemsSource>) -> NodeDefinition {
        NodeDefinition {
            name: "fanout-review".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: children.iter().map(|name| (*name).to_string()).collect(),
                items,
            }),
            ..Default::default()
        }
    }

    fn apply_fanout_and_assert_child_node_executions(
        exec: &mut DomainWorkflowExecution,
        context: &FanoutStartContext,
    ) {
        let parent_node_execution_id =
            exec.start_current_node_execution("fanout-parent".to_string(), 1.5);

        apply_fanout_runtime_state(exec, context, &[], 2.0).unwrap();

        let fanout = exec.fanout_runtime.as_ref().unwrap();
        assert_eq!(fanout.parent_node_execution_id, parent_node_execution_id);
        assert_eq!(fanout.children.len(), context.children.len());
        assert_eq!(exec.node_executions.len(), context.children.len() + 1);

        for expansion in &context.children {
            let execution = exec
                .node_executions
                .iter()
                .find(|execution| execution.id == expansion.node_execution_id)
                .unwrap();
            assert_eq!(execution.node_name, expansion.node.name);
            assert_eq!(execution.kind, expansion.node.kind_name());
            assert_eq!(execution.attempt, expansion.attempt);
            assert_eq!(
                execution.fanout_parent.as_ref().unwrap(),
                &FanoutParentRef {
                    parent_node: context.parent_node_name.clone(),
                    parent_attempt: context.parent_attempt,
                    item_index: expansion.item_index,
                    child_index: expansion.child_index,
                }
            );
        }
    }

    #[test]
    fn prepare_fanout_start_context_expands_multiple_children() {
        let mut exec = workflow_execution_with_nodes(vec![
            fanout_node(&["review-a", "review-b"], None),
            session_node("review-a"),
            NodeDefinition {
                name: "review-b".to_string(),
                kind: NodeKind::Command(CommandSpec {
                    command: "true".to_string(),
                }),
                ..Default::default()
            },
        ]);

        let context = prepare_fanout_start_context(&exec).unwrap();

        assert_eq!(context.execution_id, "execution-1");
        assert_eq!(context.parent_node_name, "fanout-review");
        assert_eq!(
            context.child_node_names(),
            vec!["review-a".to_string(), "review-b".to_string()]
        );
        assert_eq!(context.request.as_deref(), Some("ship it"));
        assert_eq!(context.children[0].child_index, 0);
        assert_eq!(context.children[1].child_index, 1);
        assert!(context.children.iter().all(|child| child.item.is_none()));
        assert_ne!(
            context.children[0].node_execution_id,
            context.children[1].node_execution_id
        );
        apply_fanout_and_assert_child_node_executions(&mut exec, &context);
    }

    #[test]
    fn prepare_fanout_start_context_expands_one_child_over_items() {
        let mut exec = workflow_execution_with_nodes(vec![
            fanout_node(
                &["review"],
                Some(ItemsSource::Literal(vec![
                    serde_json::json!({ "id": 1 }),
                    serde_json::json!({ "id": 2 }),
                ])),
            ),
            session_node("review"),
        ]);

        let context = prepare_fanout_start_context(&exec).unwrap();

        assert_eq!(context.child_node_names(), vec!["review", "review"]);
        assert_eq!(
            context
                .children
                .iter()
                .map(|child| child.item_index)
                .collect::<Vec<_>>(),
            vec![Some(0), Some(1)]
        );
        assert_eq!(
            context
                .children
                .iter()
                .map(|child| child.attempt)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        apply_fanout_and_assert_child_node_executions(&mut exec, &context);
    }

    #[test]
    fn apply_fanout_runtime_state_seeds_reused_child_without_a_session() {
        let mut exec = workflow_execution_with_nodes(vec![
            fanout_node(&["review"], None),
            command_node("review", "printf confirmed"),
        ]);
        exec.node_execution_counts
            .insert("fanout-review".to_string(), 2);
        let mut context = prepare_fanout_start_context(&exec).unwrap();
        context.children[0].reused = Some(ReusableFanoutChild {
            result: Some("confirmed".to_string()),
            display_command: Some("printf confirmed".to_string()),
            artifact: Some(serde_json::json!({"ok": true})),
            contract: Some("review".to_string()),
            token_usage: Some(TokenUsage {
                input_tokens: 3,
                output_tokens: 4,
            }),
            completed_at: 2.0,
        });
        exec.start_current_node_execution("fanout-parent".to_string(), 1.5);

        apply_fanout_runtime_state(&mut exec, &context, &[], 3.0).unwrap();

        let child = &exec.fanout_runtime.as_ref().unwrap().children[0];
        assert_eq!(child.state, FanoutChildRuntimeState::Completed);
        assert!(child.session_id.is_empty());
        assert_eq!(child.artifact, Some(serde_json::json!({"ok": true})));
        let projected = exec
            .node_executions
            .iter()
            .find(|node| node.id == child.node_execution_id)
            .unwrap();
        assert_eq!(
            projected.status,
            crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(
            projected.display_command.as_deref(),
            Some("printf confirmed")
        );
    }

    #[test]
    fn apply_fanout_runtime_state_preserves_reused_child_and_starts_only_pending_child() {
        let mut exec = workflow_execution_with_nodes(vec![
            fanout_node(&["review-reused", "review-pending"], None),
            session_node("review-reused"),
            session_node("review-pending"),
        ]);
        let mut context = prepare_fanout_start_context(&exec).unwrap();
        let reused_node_execution_id = context.children[0].node_execution_id.clone();
        let pending_node_execution_id = context.children[1].node_execution_id.clone();
        context.children[0].reused = Some(ReusableFanoutChild {
            result: Some("already confirmed".to_string()),
            display_command: None,
            artifact: Some(serde_json::json!({"verdict": "pass"})),
            contract: Some("review".to_string()),
            token_usage: Some(TokenUsage {
                input_tokens: 3,
                output_tokens: 4,
            }),
            completed_at: 2.0,
        });
        let pending_setup = FanoutChildSessionSetup {
            node_execution_id: pending_node_execution_id.clone(),
            session_id: "new-pending-session".to_string(),
        };
        exec.start_current_node_execution("fanout-parent".to_string(), 1.5);

        let snapshot =
            apply_fanout_runtime_state(&mut exec, &context, &[pending_setup], 3.0).unwrap();

        let fanout = exec.fanout_runtime.as_ref().unwrap();
        let reused = fanout
            .children
            .iter()
            .find(|child| child.node_execution_id == reused_node_execution_id)
            .unwrap();
        assert_eq!(reused.state, FanoutChildRuntimeState::Completed);
        assert!(reused.session_id.is_empty());
        assert_eq!(reused.result.as_deref(), Some("already confirmed"));
        assert_eq!(
            reused.artifact,
            Some(serde_json::json!({"verdict": "pass"}))
        );
        assert_eq!(reused.contract.as_deref(), Some("review"));
        assert_eq!(reused.token_usage.input_tokens, 3);
        assert_eq!(reused.token_usage.output_tokens, 4);
        assert_eq!(reused.completed_at, Some(2.0));

        let pending = fanout
            .children
            .iter()
            .find(|child| child.node_execution_id == pending_node_execution_id)
            .unwrap();
        assert_eq!(pending.state, FanoutChildRuntimeState::Running);
        assert_eq!(pending.session_id, "new-pending-session");
        assert!(pending.result.is_none());
        assert!(pending.artifact.is_none());
        assert!(pending.completed_at.is_none());

        let reused_projection = snapshot
            .node_executions
            .iter()
            .find(|node| node.id == reused_node_execution_id)
            .unwrap();
        assert_eq!(
            reused_projection.status,
            crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::Succeeded
        );
        assert!(reused_projection.session_id.is_none());
        assert_eq!(
            reused_projection.artifact,
            Some(serde_json::json!({"verdict": "pass"}))
        );
        assert_eq!(
            reused_projection.token_usage,
            Some(TokenUsage {
                input_tokens: 3,
                output_tokens: 4,
            })
        );

        let pending_projection = snapshot
            .node_executions
            .iter()
            .find(|node| node.id == pending_node_execution_id)
            .unwrap();
        assert_eq!(
            pending_projection.status,
            crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::Running
        );
        assert_eq!(
            pending_projection.session_id.as_deref(),
            Some("new-pending-session")
        );
        assert!(pending_projection.artifact.is_none());
    }

    #[test]
    fn prepare_fanout_start_context_expands_matrix_items_major() {
        let mut exec = workflow_execution_with_nodes(vec![
            fanout_node(
                &["a", "b"],
                Some(ItemsSource::Literal(vec![
                    serde_json::json!("first"),
                    serde_json::json!("second"),
                ])),
            ),
            session_node("a"),
            session_node("b"),
        ]);

        let context = prepare_fanout_start_context(&exec).unwrap();

        assert_eq!(context.child_node_names(), vec!["a", "b", "a", "b"]);
        assert_eq!(
            context
                .children
                .iter()
                .map(|child| (child.item_index, child.child_index, child.attempt))
                .collect::<Vec<_>>(),
            vec![
                (Some(0), 0, 1),
                (Some(0), 1, 1),
                (Some(1), 0, 2),
                (Some(1), 1, 2)
            ]
        );
        apply_fanout_and_assert_child_node_executions(&mut exec, &context);
    }

    #[test]
    fn prepare_fanout_start_context_expands_items_from_artifact_field() {
        let mut exec = workflow_execution_with_nodes(vec![
            fanout_node(
                &["a", "b"],
                Some(ItemsSource::ArtifactField {
                    node: "source".to_string(),
                    field: "targets".to_string(),
                }),
            ),
            session_node("a"),
            session_node("b"),
        ]);
        exec.artifacts.insert(
            "source".to_string(),
            RuntimeArtifact {
                node_name: "source".to_string(),
                attempt: 1,
                session_id: None,
                result: None,
                artifact: Some(serde_json::json!({
                    "targets": [
                        { "id": "first" },
                        { "id": "second" }
                    ]
                })),
                contract: None,
                token_usage: None,
                completed_at: 1000.0,
            },
        );

        let context = prepare_fanout_start_context(&exec).unwrap();

        assert_eq!(context.child_node_names(), vec!["a", "b", "a", "b"]);
        assert_eq!(
            context
                .children
                .iter()
                .map(|child| (
                    child.item.clone(),
                    child.item_index,
                    child.child_index,
                    child.attempt
                ))
                .collect::<Vec<_>>(),
            vec![
                (Some(serde_json::json!({ "id": "first" })), Some(0), 0, 1),
                (Some(serde_json::json!({ "id": "first" })), Some(0), 1, 1),
                (Some(serde_json::json!({ "id": "second" })), Some(1), 0, 2),
                (Some(serde_json::json!({ "id": "second" })), Some(1), 1, 2),
            ]
        );
    }

    #[test]
    fn prepare_fanout_start_context_rejects_unavailable_artifact_field_items() {
        let exec = workflow_execution_with_nodes(vec![
            fanout_node(
                &["review"],
                Some(ItemsSource::ArtifactField {
                    node: "source".to_string(),
                    field: "targets".to_string(),
                }),
            ),
            session_node("review"),
        ]);

        let err = prepare_fanout_start_context(&exec).unwrap_err();

        assert!(matches!(
            err,
            WorkflowRuntimeError::InvalidState(message)
                if message == "fanout items source 'source.targets' is unavailable"
        ));
    }

    #[test]
    fn prepare_fanout_start_context_rejects_non_array_artifact_field_items() {
        let mut exec = workflow_execution_with_nodes(vec![
            fanout_node(
                &["review"],
                Some(ItemsSource::ArtifactField {
                    node: "source".to_string(),
                    field: "targets".to_string(),
                }),
            ),
            session_node("review"),
        ]);
        exec.artifacts.insert(
            "source".to_string(),
            RuntimeArtifact {
                node_name: "source".to_string(),
                attempt: 1,
                session_id: None,
                result: None,
                artifact: Some(serde_json::json!({ "targets": "not-array" })),
                contract: None,
                token_usage: None,
                completed_at: 1000.0,
            },
        );

        let err = prepare_fanout_start_context(&exec).unwrap_err();

        assert!(matches!(
            err,
            WorkflowRuntimeError::InvalidState(message)
                if message == "fanout items source 'source.targets' is not an array"
        ));
    }

    #[test]
    fn prepare_fanout_start_context_empty_items_produces_no_children() {
        let exec = workflow_execution_with_nodes(vec![
            fanout_node(&["review"], Some(ItemsSource::Literal(Vec::new()))),
            session_node("review"),
        ]);

        let context = prepare_fanout_start_context(&exec).unwrap();

        assert!(context.children.is_empty());
    }

    #[test]
    fn prepare_fanout_start_context_rejects_node_without_children() {
        let exec = workflow_execution_fixture(fanout_node(&[], None));

        let err = prepare_fanout_start_context(&exec).unwrap_err();

        assert!(matches!(
            err,
            WorkflowRuntimeError::InvalidState(message)
                if message == "StartFanout requires child references for node 'fanout-review'"
        ));
    }

    #[test]
    fn fanout_prompt_inputs_clones_runtime_inputs() {
        let mut exec = workflow_execution_with_nodes(vec![
            fanout_node(&["review-a"], None),
            session_node("review-a"),
        ]);
        exec.artifacts.insert(
            "plan".to_string(),
            make_node_output("plan", "draft", Some("DONE")),
        );
        let inputs = fanout_prompt_inputs(&exec);

        assert_eq!(
            inputs.artifacts["plan"].artifact,
            Some(serde_json::json!({ "text": "draft" }))
        );
    }

    fn make_node_output(node_name: &str, text: &str, result: Option<&str>) -> RuntimeArtifact {
        RuntimeArtifact {
            node_name: node_name.to_string(),
            attempt: 0,
            session_id: Some(format!("session-{node_name}")),
            result: result.map(str::to_string),
            artifact: Some(serde_json::json!({ "text": text })),
            contract: None,
            token_usage: None,
            completed_at: 1000.0,
        }
    }
}
