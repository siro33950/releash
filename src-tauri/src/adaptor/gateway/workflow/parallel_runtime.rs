use std::collections::HashMap;

use crate::adaptor::gateway::workflow::domain_mapping::{
    artifacts_to_domain, node_history_entry_from_domain, runtime_artifact_from_domain,
    token_usage_to_domain, workflow_definition_to_domain,
};
use crate::adaptor::gateway::workflow::engine_error::{
    workflow_error_to_engine_error, WorkflowEngineError,
};
use crate::adaptor::gateway::workflow::event::FanoutParentRef;
use crate::adaptor::gateway::workflow::runtime_state::{
    FanoutChildRuntime, FanoutChildRuntimeState, FanoutRuntimeState, WorkflowExecution,
};
use crate::adaptor::gateway::workflow::schema::NodeDefinition;
use crate::adaptor::gateway::workflow::state::{RuntimeArtifact, TokenUsage, WorkflowState};
use crate::adaptor::gateway::workflow::step_settings::WorkflowDefaults;
use crate::domain::workflow::services::parallel as workflow_parallel;

#[derive(Debug, Clone)]
pub(crate) struct FanoutChildExpansion {
    pub(crate) node_execution_id: String,
    pub(crate) node: NodeDefinition,
    pub(crate) attempt: u32,
    pub(crate) item: Option<serde_json::Value>,
    pub(crate) item_index: Option<usize>,
    pub(crate) child_index: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct FanoutStartContext {
    pub(crate) children: Vec<FanoutChildExpansion>,
    pub(crate) parent_node_name: String,
    pub(crate) parent_attempt: u32,
    pub(crate) order: u32,
    pub(crate) execution_id: String,
    pub(crate) workflow_name: String,
    pub(crate) request: Option<String>,
    pub(crate) workflow_defaults: WorkflowDefaults,
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
    pub(crate) node_name: String,
    pub(crate) session_id: String,
    pub(crate) system_prompt: Option<String>,
    pub(crate) workflow_instruction: Option<String>,
    pub(crate) user_message: String,
    pub(crate) permission_mode: String,
}

pub(crate) fn prepare_fanout_start_context(
    exec: &WorkflowExecution,
) -> Result<FanoutStartContext, WorkflowEngineError> {
    let step = exec
        .workflow
        .nodes
        .get(exec.current_node_index)
        .ok_or_else(|| {
            WorkflowEngineError::InvalidState(format!(
                "current_node_index {} is out of bounds for workflow '{}'",
                exec.current_node_index, exec.workflow.name
            ))
        })?;
    let fanout = step.fanout().ok_or_else(|| {
        WorkflowEngineError::InvalidState(format!(
            "StartFanout requires fanout node '{}'",
            step.name
        ))
    })?;
    if fanout.child.is_empty() {
        return Err(WorkflowEngineError::InvalidState(format!(
            "StartFanout requires child references for node '{}'",
            step.name
        )));
    }
    let parent_attempt = exec
        .node_execution_counts
        .get(&step.name)
        .copied()
        .unwrap_or(1);
    let domain_workflow = workflow_definition_to_domain(&exec.workflow);
    let domain_artifacts = artifacts_to_domain(&exec.artifacts);
    let domain_step = domain_workflow
        .nodes
        .get(exec.current_node_index)
        .ok_or_else(|| {
            WorkflowEngineError::InvalidState(format!(
                "current_node_index {} is out of bounds for workflow '{}'",
                exec.current_node_index, exec.workflow.name
            ))
        })?;
    let domain_fanout = domain_step.fanout().ok_or_else(|| {
        WorkflowEngineError::InvalidState(format!(
            "StartFanout requires fanout node '{}'",
            step.name
        ))
    })?;
    let expansion_plan = workflow_parallel::plan_fanout_expansion(
        &domain_workflow,
        &domain_fanout.child,
        domain_fanout.items.as_ref(),
        &domain_artifacts,
        &exec.node_execution_counts,
    )
    .map_err(workflow_error_to_engine_error)?;
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
                    WorkflowEngineError::InvalidState(format!(
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
            })
        })
        .collect::<Result<Vec<_>, WorkflowEngineError>>()?;
    Ok(FanoutStartContext {
        parent_node_name: step.name.clone(),
        parent_attempt,
        order: exec.node_history.len() as u32,
        children,
        execution_id: exec.id.clone(),
        workflow_name: exec.workflow.name.clone(),
        request: exec.request.clone(),
        workflow_defaults: exec.workflow_defaults.clone(),
    })
}

pub(crate) fn fanout_prompt_inputs(exec: &WorkflowExecution) -> FanoutPromptInputs {
    FanoutPromptInputs {
        artifacts: exec.artifacts.clone(),
    }
}

pub(crate) fn apply_fanout_runtime_state(
    exec: &mut WorkflowExecution,
    fanout_start: &FanoutStartContext,
    session_setups: &[FanoutChildSessionSetup],
    timestamp: f64,
) -> Result<WorkflowState, WorkflowEngineError> {
    let parent_node_execution_id = exec
        .active_current_node_execution_id()
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            WorkflowEngineError::InvalidState(format!(
                "active fanout parent NodeExecution for '{}' is unavailable",
                fanout_start.parent_node_name
            ))
        })?;
    let children = fanout_start
        .children
        .iter()
        .map(|child| {
            exec.node_execution_counts
                .entry(child.node.name.clone())
                .and_modify(|attempt| *attempt = (*attempt).max(child.attempt))
                .or_insert(child.attempt);
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
                Some(child.node_execution_id.clone()),
                timestamp,
            );
            let session_id = session_setups
                .iter()
                .find(|setup| setup.node_execution_id == child.node_execution_id)
                .map(|setup| setup.session_id.clone())
                .unwrap_or_default();
            if !session_id.is_empty() {
                if let Some(execution) = exec
                    .node_executions
                    .iter_mut()
                    .find(|execution| execution.id == child.node_execution_id)
                {
                    execution.session_id = Some(session_id.clone());
                }
            }
            FanoutChildRuntime {
                node_execution_id: child.node_execution_id.clone(),
                node_name: child.node.name.clone(),
                session_id,
                state: FanoutChildRuntimeState::Running,
                result: None,
                artifact: None,
                contract: child.node.artifact.clone(),
                failure_kind: None,
                failure_disposition: None,
                token_usage: TokenUsage::default(),
                attempt: child.attempt,
                completed_at: None,
            }
        })
        .collect();
    exec.parallel_run = Some(FanoutRuntimeState {
        parent_node_name: fanout_start.parent_node_name.clone(),
        parent_node_execution_id,
        children,
    });
    exec.updated_at = timestamp;
    Ok(exec.to_workflow_state())
}

pub(crate) struct FanoutParentCompletionPlan {
    pub(crate) parent_step_output: RuntimeArtifact,
    pub(crate) history_entry: crate::adaptor::gateway::workflow::state::NodeHistoryEntry,
}

pub(crate) fn plan_fanout_parent_completion(
    parent_node_name: &str,
    parent_attempt: u32,
    children: &[FanoutChildRuntime],
    timestamp: f64,
) -> FanoutParentCompletionPlan {
    let children: Vec<workflow_parallel::FanoutChildCompletionInput> = children
        .iter()
        .map(|child| workflow_parallel::FanoutChildCompletionInput {
            node_name: child.node_name.clone(),
            session_id: (!child.session_id.is_empty()).then(|| child.session_id.clone()),
            result: child.result.clone(),
            artifact: child.artifact.clone().unwrap_or(serde_json::Value::Null),
            contract: child.contract.clone(),
            token_usage: token_usage_to_domain(&child.token_usage),
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
    let plan = workflow_parallel::plan_fanout_parent_completion(
        parent_node_name,
        parent_attempt,
        &children,
        timestamp,
    );
    FanoutParentCompletionPlan {
        parent_step_output: runtime_artifact_from_domain(plan.parent_step_output),
        history_entry: node_history_entry_from_domain(plan.history_entry),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        CommandSpec, FanoutSpec, ItemsSource, NodeDefinition, NodeKind, SessionSpec, Workflow,
    };
    use crate::adaptor::gateway::workflow::state::RuntimeExecutionState;

    fn workflow_execution_fixture(node: NodeDefinition) -> WorkflowExecution {
        workflow_execution_with_nodes(vec![node])
    }

    fn workflow_execution_with_nodes(nodes: Vec<NodeDefinition>) -> WorkflowExecution {
        WorkflowExecution {
            id: "run-1".to_string(),
            workflow: Workflow {
                name: "test-workflow".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes,
            },
            state: RuntimeExecutionState::Running,
            current_node_index: 0,
            node_execution_counts: HashMap::new(),
            node_history: Vec::new(),
            workflow_defaults: WorkflowDefaults {
                backend_id: Some("backend-1".to_string()),
                permission_mode: "ask".to_string(),
            },
            worktree_path: "/tmp/repo".to_string(),
            created_from: crate::domain::workflow::ExecutionOrigin::Cli,
            error_reason: None,
            started_at: 1.0,
            updated_at: 1.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: Vec::new(),
            request: Some("ship it".to_string()),
            parallel_run: None,
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
        exec: &mut WorkflowExecution,
        context: &FanoutStartContext,
    ) {
        let parent_node_execution_id = exec.start_current_node_execution(1.5);

        apply_fanout_runtime_state(exec, context, &[], 2.0).unwrap();

        let fanout_run = exec.parallel_run.as_ref().unwrap();
        assert_eq!(
            fanout_run.parent_node_execution_id,
            parent_node_execution_id
        );
        assert_eq!(fanout_run.children.len(), context.children.len());
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

        assert_eq!(context.execution_id, "run-1");
        assert_eq!(context.workflow_name, "test-workflow");
        assert_eq!(context.parent_node_name, "fanout-review");
        assert_eq!(
            context.child_node_names(),
            vec!["review-a".to_string(), "review-b".to_string()]
        );
        assert_eq!(context.request.as_deref(), Some("ship it"));
        assert_eq!(context.workflow_defaults.permission_mode, "ask");
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
            WorkflowEngineError::InvalidState(message)
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
            WorkflowEngineError::InvalidState(message)
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
            WorkflowEngineError::InvalidState(message)
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
            make_step_output("plan", "draft", Some("DONE")),
        );
        let inputs = fanout_prompt_inputs(&exec);

        assert_eq!(
            inputs.artifacts["plan"].artifact,
            Some(serde_json::json!({ "text": "draft" }))
        );
    }

    fn make_step_output(node_name: &str, text: &str, result: Option<&str>) -> RuntimeArtifact {
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
