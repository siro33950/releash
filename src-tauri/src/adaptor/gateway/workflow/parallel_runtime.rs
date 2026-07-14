use std::collections::HashMap;

use tokio::sync::Mutex;

#[cfg(test)]
use crate::adaptor::gateway::workflow::domain_mapping::step_output_to_domain;
use crate::adaptor::gateway::workflow::domain_mapping::{
    collect_config_to_domain, parallel_aggregate_to_domain, step_history_entry_from_domain,
    step_output_from_domain, step_outputs_to_domain, token_usage_to_domain,
    workflow_definition_to_domain,
};
use crate::adaptor::gateway::workflow::engine_error::{
    workflow_error_to_engine_error, WorkflowEngineError,
};
use crate::adaptor::gateway::workflow::event::{
    CollectedOutputEntry, FanoutParentRef, WorkflowEvent,
};
use crate::adaptor::gateway::workflow::execution_registry::find_by_worktree_mut;
use crate::adaptor::gateway::workflow::runtime_commit::StepOutcome;
use crate::adaptor::gateway::workflow::runtime_state::{
    ParallelChildRun, ParallelChildState, ParallelRunState, WorkflowExecution,
};
use crate::adaptor::gateway::workflow::schema::{CollectConfig, NodeDefinition, ParallelAggregate};
use crate::adaptor::gateway::workflow::state::{StepOutput, TokenUsage, WorkflowState};
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
    pub(crate) parent_step_name: String,
    pub(crate) parent_run_index: u32,
    pub(crate) order: u32,
    pub(crate) aggregate: Option<ParallelAggregate>,
    pub(crate) execution_id: String,
    pub(crate) workflow_name: String,
    pub(crate) task: Option<String>,
    pub(crate) workflow_defaults: WorkflowDefaults,
}

impl FanoutStartContext {
    #[cfg(test)]
    pub(crate) fn child_step_names(&self) -> Vec<String> {
        self.children
            .iter()
            .map(|child| child.node.name.clone())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FanoutPromptInputs {
    pub(crate) step_outputs: HashMap<String, StepOutput>,
}

pub(crate) struct FanoutChildSessionSetup {
    pub(crate) node_execution_id: String,
    pub(crate) step_name: String,
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
        .get(exec.current_step_index)
        .ok_or_else(|| {
            WorkflowEngineError::InvalidState(format!(
                "current_step_index {} is out of bounds for workflow '{}'",
                exec.current_step_index, exec.workflow.name
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
    let parent_run_index = exec
        .step_execution_counts
        .get(&step.name)
        .copied()
        .unwrap_or(1);
    let domain_workflow = workflow_definition_to_domain(&exec.workflow);
    let domain_step_outputs = step_outputs_to_domain(&exec.step_outputs);
    let domain_step = domain_workflow
        .nodes
        .get(exec.current_step_index)
        .ok_or_else(|| {
            WorkflowEngineError::InvalidState(format!(
                "current_step_index {} is out of bounds for workflow '{}'",
                exec.current_step_index, exec.workflow.name
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
        &domain_step_outputs,
        &exec.step_execution_counts,
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
    // Zero-item fanout is a normal successful parent completion with no children. Do not carry
    // the temporary aggregate compatibility route into that completion: the parent node's
    // ordinary rules own the next transition for this case.
    let aggregate = (!children.is_empty())
        .then(|| fanout.aggregate.clone())
        .flatten();
    Ok(FanoutStartContext {
        parent_step_name: step.name.clone(),
        parent_run_index,
        order: exec.step_history.len() as u32,
        children,
        aggregate,
        execution_id: exec.id.clone(),
        workflow_name: exec.workflow.name.clone(),
        task: exec.task.clone(),
        workflow_defaults: exec.workflow_defaults.clone(),
    })
}

pub(crate) fn fanout_prompt_inputs(exec: &WorkflowExecution) -> FanoutPromptInputs {
    FanoutPromptInputs {
        step_outputs: exec.step_outputs.clone(),
    }
}

pub(crate) fn apply_fanout_run_state(
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
                fanout_start.parent_step_name
            ))
        })?;
    let children = fanout_start
        .children
        .iter()
        .map(|child| {
            exec.step_execution_counts
                .entry(child.node.name.clone())
                .and_modify(|attempt| *attempt = (*attempt).max(child.attempt))
                .or_insert(child.attempt);
            exec.start_node_execution(
                child.node.name.clone(),
                child.node.kind_name(),
                child.attempt,
                Some(FanoutParentRef {
                    parent_node: fanout_start.parent_step_name.clone(),
                    parent_attempt: fanout_start.parent_run_index,
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
            ParallelChildRun {
                node_execution_id: child.node_execution_id.clone(),
                step_name: child.node.name.clone(),
                session_id,
                state: ParallelChildState::Running,
                result: None,
                structured_output: None,
                artifact_contract: child.node.artifact.clone(),
                failure_kind: None,
                failure_disposition: None,
                token_usage: TokenUsage::default(),
                run_index: child.attempt,
                completed_at: None,
            }
        })
        .collect();
    exec.parallel_run = Some(ParallelRunState {
        parent_step_name: fanout_start.parent_step_name.clone(),
        parent_node_execution_id,
        aggregate: fanout_start.aggregate.clone(),
        children,
    });
    exec.updated_at = timestamp;
    Ok(exec.to_workflow_state())
}

pub(crate) struct ReduceTransitionResult {
    pub(crate) next_outcome: StepOutcome,
    pub(crate) output_collected_event: WorkflowEvent,
    pub(crate) snapshot_before: WorkflowExecution,
}

pub(crate) enum FanoutParentCompletionTransition {
    Advance,
    TransitionTo { target_node_name: String },
}

pub(crate) struct FanoutParentCompletionPlan {
    pub(crate) parent_step_output: StepOutput,
    pub(crate) history_entry: crate::adaptor::gateway::workflow::state::StepHistoryEntry,
    pub(crate) transition: FanoutParentCompletionTransition,
}

/// reduce処理の結果。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReduceResult {
    pub result: Option<String>,
    pub structured_output: Option<serde_json::Value>,
}

/// Legacy schema/state を domain の parallel service に接続する境界。
pub(crate) fn apply_reduce(
    collect: &CollectConfig,
    step_outputs: &HashMap<String, StepOutput>,
) -> ReduceResult {
    let collect = collect_config_to_domain(collect);
    let step_outputs = step_outputs_to_domain(step_outputs);
    let result = workflow_parallel::apply_reduce(&collect, &step_outputs);
    ReduceResult {
        result: result.result,
        structured_output: result.structured_output,
    }
}

pub(crate) fn apply_reduce_transition(
    exec: &mut WorkflowExecution,
    snapshot: &WorkflowState,
) -> Result<ReduceTransitionResult, WorkflowEngineError> {
    let step = &exec.workflow.nodes[exec.current_step_index];
    let collect = step
        .collect
        .clone()
        .expect("ReduceAndTransition requires collect config");
    let reduce_result = apply_reduce(&collect, &exec.step_outputs);
    let snapshot_before = exec.clone();

    let entry = exec.make_step_history_entry(
        reduce_result.result.clone(),
        reduce_result.structured_output.clone(),
        None,
    );
    exec.step_history.push(entry);

    let step_name = exec.workflow.nodes[exec.current_step_index].name.clone();
    let execution_id = exec.id.clone();
    let workflow_name = exec.workflow.name.clone();

    log::info!(
        "OutputCollected: step='{}', strategy={:?}, from={:?}",
        step_name,
        collect.reduce,
        collect.from,
    );

    let next_outcome = exec.apply_advance();

    let node_outputs: Vec<CollectedOutputEntry> = collect
        .from
        .iter()
        .map(|name| {
            let output = snapshot.step_outputs.get(name);
            CollectedOutputEntry {
                node_name: name.clone(),
                result: output.and_then(|o| o.result.clone()),
                structured_output: output.and_then(|o| o.structured_output.clone()),
            }
        })
        .collect();
    let output_collected_event = WorkflowEvent::OutputCollected {
        run_id: execution_id,
        workflow_name,
        node_name: step_name,
        node_outputs,
        reduce_strategy: format!("{:?}", collect.reduce),
        reduce_result: reduce_result.result,
        reduce_structured_output: reduce_result.structured_output,
        timestamp: crate::usecase::agent_session::session::now_timestamp(),
    };

    Ok(ReduceTransitionResult {
        next_outcome,
        output_collected_event,
        snapshot_before,
    })
}

pub(crate) async fn apply_reduce_transition_by_worktree(
    executions: &Mutex<HashMap<String, WorkflowExecution>>,
    worktree_path: &str,
    snapshot: &WorkflowState,
) -> Result<ReduceTransitionResult, WorkflowEngineError> {
    let mut execs = executions.lock().await;
    let exec = find_by_worktree_mut(&mut execs, worktree_path)
        .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
    apply_reduce_transition(exec, snapshot)
}

/// aggregate条件を評価する。trueなら`then`、falseなら`else`。
#[cfg(test)]
pub(crate) fn evaluate_aggregate(
    aggregate: &ParallelAggregate,
    step_outputs: &HashMap<String, StepOutput>,
    child_step_names: &[String],
) -> bool {
    let aggregate = parallel_aggregate_to_domain(aggregate);
    let step_outputs = step_outputs_to_domain(step_outputs);
    workflow_parallel::evaluate_aggregate(&aggregate, &step_outputs, child_step_names)
}

pub(crate) fn plan_fanout_parent_completion(
    parent_step_name: &str,
    parent_run_index: u32,
    aggregate: Option<&ParallelAggregate>,
    children: &[ParallelChildRun],
    timestamp: f64,
) -> FanoutParentCompletionPlan {
    let aggregate = aggregate.map(parallel_aggregate_to_domain);
    let children: Vec<workflow_parallel::FanoutChildCompletionInput> = children
        .iter()
        .map(|child| workflow_parallel::FanoutChildCompletionInput {
            node_execution_id: child.node_execution_id.clone(),
            node_name: child.step_name.clone(),
            session_id: (!child.session_id.is_empty()).then(|| child.session_id.clone()),
            result: child.result.clone(),
            artifact: child
                .structured_output
                .clone()
                .unwrap_or(serde_json::Value::Null),
            artifact_contract: child.artifact_contract.clone(),
            token_usage: token_usage_to_domain(&child.token_usage),
            attempt: child.run_index,
            completed_at: child.completed_at.unwrap_or(timestamp),
            state: match child.state {
                ParallelChildState::Running => crate::domain::workflow::STEP_STATE_RUNNING,
                ParallelChildState::Completed => crate::domain::workflow::STEP_STATE_COMPLETED,
                ParallelChildState::Failed => crate::domain::workflow::STEP_STATE_FAILED,
                ParallelChildState::Interrupted => crate::domain::workflow::STEP_STATE_INTERRUPTED,
            }
            .to_string(),
            failure_kind: child.failure_kind,
            failure_disposition: child.failure_disposition,
        })
        .collect();
    let plan = workflow_parallel::plan_fanout_parent_completion(
        parent_step_name,
        parent_run_index,
        aggregate.as_ref(),
        &children,
        timestamp,
    );
    let transition = match plan.transition {
        workflow_parallel::FanoutParentTransitionPlan::Advance => {
            FanoutParentCompletionTransition::Advance
        }
        workflow_parallel::FanoutParentTransitionPlan::TransitionTo {
            target_node_name, ..
        } => FanoutParentCompletionTransition::TransitionTo { target_node_name },
    };
    FanoutParentCompletionPlan {
        parent_step_output: step_output_from_domain(plan.parent_step_output),
        history_entry: step_history_entry_from_domain(plan.history_entry),
        transition,
    }
}

#[cfg(test)]
pub(crate) fn collect_step_output_entries(
    from: &[String],
    step_outputs: &HashMap<String, StepOutput>,
) -> Vec<serde_json::Value> {
    let step_outputs = step_outputs_to_domain(step_outputs);
    workflow_parallel::collect_step_output_entries(from, &step_outputs)
}

#[cfg(test)]
pub(crate) fn resolve_step_result(output: &StepOutput) -> Option<String> {
    let output = step_output_to_domain(output);
    workflow_parallel::resolve_step_result(&output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        CommandSpec, FanoutSpec, ItemsSource, NodeDefinition, NodeKind, ReduceStrategy,
        SessionSpec, Workflow,
    };
    use crate::adaptor::gateway::workflow::state::WorkflowExecutionState;

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
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            workflow_defaults: WorkflowDefaults {
                backend_id: Some("backend-1".to_string()),
                permission_mode: "ask".to_string(),
            },
            worktree_path: "/tmp/repo".to_string(),
            started_at: 1.0,
            updated_at: 1.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            node_executions: Vec::new(),
            task: Some("ship it".to_string()),
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
                aggregate: None,
            }),
            ..Default::default()
        }
    }

    fn apply_fanout_and_assert_child_node_executions(
        exec: &mut WorkflowExecution,
        context: &FanoutStartContext,
    ) {
        let parent_node_execution_id = exec.start_current_node_execution(1.5);

        apply_fanout_run_state(exec, context, &[], 2.0).unwrap();

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
                    parent_node: context.parent_step_name.clone(),
                    parent_attempt: context.parent_run_index,
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
        assert_eq!(context.parent_step_name, "fanout-review");
        assert_eq!(
            context.child_step_names(),
            vec!["review-a".to_string(), "review-b".to_string()]
        );
        assert_eq!(context.task.as_deref(), Some("ship it"));
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

        assert_eq!(context.child_step_names(), vec!["review", "review"]);
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

        assert_eq!(context.child_step_names(), vec!["a", "b", "a", "b"]);
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
        exec.step_outputs.insert(
            "source".to_string(),
            StepOutput {
                step_name: "source".to_string(),
                run_index: 1,
                session_id: None,
                result: None,
                structured_output: Some(serde_json::json!({
                    "targets": [
                        { "id": "first" },
                        { "id": "second" }
                    ]
                })),
                artifact_contract: None,
                token_usage: None,
                completed_at: 1000.0,
            },
        );

        let context = prepare_fanout_start_context(&exec).unwrap();

        assert_eq!(context.child_step_names(), vec!["a", "b", "a", "b"]);
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
        exec.step_outputs.insert(
            "source".to_string(),
            StepOutput {
                step_name: "source".to_string(),
                run_index: 1,
                session_id: None,
                result: None,
                structured_output: Some(serde_json::json!({ "targets": "not-array" })),
                artifact_contract: None,
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
        assert!(context.aggregate.is_none());
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
        exec.step_outputs.insert(
            "plan".to_string(),
            make_step_output("plan", "draft", Some("DONE")),
        );
        let inputs = fanout_prompt_inputs(&exec);

        assert_eq!(
            inputs.step_outputs["plan"].structured_output,
            Some(serde_json::json!({ "text": "draft" }))
        );
    }

    #[tokio::test]
    async fn apply_reduce_transition_by_worktree_updates_active_execution() {
        let mut exec = workflow_execution_fixture(NodeDefinition {
            name: "collect-reviews".to_string(),
            collect: Some(make_collect(
                vec!["review-a", "review-b"],
                ReduceStrategy::Concat,
            )),
            ..Default::default()
        });
        exec.step_outputs = make_outputs(vec![
            ("review-a", "first", None),
            ("review-b", "second", None),
        ]);
        let snapshot = exec.to_workflow_state();
        let executions = Mutex::new(HashMap::from([("run-1".to_string(), exec)]));

        let result = apply_reduce_transition_by_worktree(&executions, "/tmp/repo", &snapshot)
            .await
            .unwrap();

        assert!(matches!(
            result.next_outcome,
            StepOutcome::Persist(state) if matches!(state.state, WorkflowExecutionState::Completed)
        ));
        assert!(matches!(
            result.output_collected_event,
            WorkflowEvent::OutputCollected {
                node_name,
                node_outputs,
                reduce_strategy,
                ..
            } if node_name == "collect-reviews"
                && node_outputs.len() == 2
                && node_outputs[0].node_name == "review-a"
                && node_outputs[1].node_name == "review-b"
                && reduce_strategy == "Concat"
        ));

        let execs = executions.lock().await;
        let exec = execs.get("run-1").unwrap();
        assert_eq!(exec.step_history.len(), 1);
        assert_eq!(exec.step_history[0].step_name, "collect-reviews");
    }

    fn make_step_output(step_name: &str, text: &str, result: Option<&str>) -> StepOutput {
        StepOutput {
            step_name: step_name.to_string(),
            run_index: 0,
            session_id: Some(format!("session-{step_name}")),
            result: result.map(str::to_string),
            structured_output: Some(serde_json::json!({ "text": text })),
            artifact_contract: None,
            token_usage: None,
            completed_at: 1000.0,
        }
    }

    fn make_collect(from: Vec<&str>, reduce: ReduceStrategy) -> CollectConfig {
        CollectConfig {
            from: from.iter().map(|s| s.to_string()).collect(),
            reduce,
        }
    }

    fn make_outputs(entries: Vec<(&str, &str, Option<&str>)>) -> HashMap<String, StepOutput> {
        let mut map = HashMap::new();
        for (name, text, result) in entries {
            map.insert(name.to_string(), make_step_output(name, text, result));
        }
        map
    }

    fn aggregate(all_match: Option<&str>, any_match: Option<&str>) -> ParallelAggregate {
        ParallelAggregate {
            all_match: all_match.map(str::to_string),
            any_match: any_match.map(str::to_string),
            then: "then-step".to_string(),
            r#else: "else-step".to_string(),
        }
    }

    #[test]
    fn evaluate_aggregate_all_match_all_children_match() {
        let aggregate = aggregate(Some("LGTM"), None);
        let mut outputs = make_outputs(vec![
            ("arch-review", "looks good", Some("LGTM")),
            ("security-review", "no issues", Some("LGTM")),
        ]);
        outputs.insert(
            "implement".to_string(),
            make_step_output("implement", "done", Some("DONE")),
        );
        let children = vec!["arch-review".to_string(), "security-review".to_string()];

        assert!(evaluate_aggregate(&aggregate, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_all_match_one_child_mismatch() {
        let aggregate = aggregate(Some("LGTM"), None);
        let outputs = make_outputs(vec![
            ("arch-review", "ok", Some("LGTM")),
            ("security-review", "problems", Some("NEEDS_FIX")),
        ]);
        let children = vec!["arch-review".to_string(), "security-review".to_string()];

        assert!(!evaluate_aggregate(&aggregate, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_any_match_one_child_matches() {
        let aggregate = aggregate(None, Some("NEEDS_FIX"));
        let outputs = make_outputs(vec![
            ("arch-review", "ok", Some("LGTM")),
            ("security-review", "problems", Some("NEEDS_FIX")),
        ]);
        let children = vec!["arch-review".to_string(), "security-review".to_string()];

        assert!(evaluate_aggregate(&aggregate, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_any_match_no_child_matches() {
        let aggregate = aggregate(None, Some("NEEDS_FIX"));
        let outputs = make_outputs(vec![
            ("arch-review", "ok", Some("LGTM")),
            ("security-review", "ok", Some("LGTM")),
        ]);
        let children = vec!["arch-review".to_string(), "security-review".to_string()];

        assert!(!evaluate_aggregate(&aggregate, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_no_condition_returns_true() {
        let aggregate = aggregate(None, None);
        let outputs = HashMap::new();
        let children: Vec<String> = vec![];

        assert!(evaluate_aggregate(&aggregate, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_result_none_does_not_match() {
        let aggregate = aggregate(Some("LGTM"), None);
        let outputs = make_outputs(vec![
            ("arch-review", "Review result: LGTM", None),
            ("security-review", "All good. LGTM", None),
        ]);
        let children = vec!["arch-review".to_string(), "security-review".to_string()];

        assert!(!evaluate_aggregate(&aggregate, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_filters_only_child_steps() {
        let aggregate = aggregate(Some("LGTM"), None);
        let mut outputs = make_outputs(vec![("arch-review", "", Some("LGTM"))]);
        outputs.insert(
            "implement".to_string(),
            make_step_output("implement", "done", Some("DONE")),
        );
        let children = vec!["arch-review".to_string()];

        assert!(evaluate_aggregate(&aggregate, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_all_match_missing_child_output_returns_false() {
        let aggregate = aggregate(Some("LGTM"), None);
        let outputs = make_outputs(vec![("arch-review", "ok", Some("LGTM"))]);
        let children = vec!["arch-review".to_string(), "security-review".to_string()];

        assert!(!evaluate_aggregate(&aggregate, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_invalid_regex_falls_back_to_contains() {
        let aggregate = aggregate(Some("[invalid(regex"), None);
        let outputs = make_outputs(vec![("arch-review", "text", Some("[invalid(regex"))]);
        let children = vec!["arch-review".to_string()];

        assert!(evaluate_aggregate(&aggregate, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_invalid_regex_contains_no_match() {
        let aggregate = aggregate(Some("[invalid(regex"), None);
        let outputs = make_outputs(vec![("arch-review", "LGTM text", Some("LGTM"))]);
        let children = vec!["arch-review".to_string()];

        assert!(!evaluate_aggregate(&aggregate, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_empty_children_all_match_returns_true() {
        let aggregate = aggregate(Some("LGTM"), None);
        let outputs = HashMap::new();
        let children: Vec<String> = vec![];

        assert!(evaluate_aggregate(&aggregate, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_child_without_artifact_contract_has_no_step_output() {
        let aggregate = aggregate(Some("LGTM"), None);
        let outputs = make_outputs(vec![("arch-review", "ok", Some("LGTM"))]);
        let children = vec!["arch-review".to_string(), "test-step".to_string()];

        assert!(!evaluate_aggregate(&aggregate, &outputs, &children));
    }

    #[test]
    fn reduce_last_returns_latest_completed_entry() {
        let collect = make_collect(vec!["a", "b", "c"], ReduceStrategy::Last);
        let mut outputs = HashMap::new();
        outputs.insert(
            "a".to_string(),
            StepOutput {
                completed_at: 1000.0,
                ..make_step_output("a", "text_a", Some("LGTM"))
            },
        );
        outputs.insert(
            "b".to_string(),
            StepOutput {
                completed_at: 3000.0,
                ..make_step_output("b", "text_b", Some("NEEDS_FIX"))
            },
        );
        outputs.insert(
            "c".to_string(),
            StepOutput {
                completed_at: 2000.0,
                ..make_step_output("c", "text_c", Some("LGTM"))
            },
        );

        let reduced = apply_reduce(&collect, &outputs);

        assert_eq!(reduced.result, Some("NEEDS_FIX".to_string()));
        assert_eq!(reduced.structured_output.unwrap()["text"], "text_b");
    }

    #[test]
    fn reduce_concat_joins_all() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::Concat);
        let outputs = make_outputs(vec![
            ("a", "output from a", None),
            ("b", "output from b", None),
        ]);

        let reduced = apply_reduce(&collect, &outputs);

        assert!(reduced.result.is_none());
        let structured_output = reduced.structured_output.unwrap();
        let entries = structured_output.as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["stepName"], "a");
        assert_eq!(entries[0]["output"]["text"], "output from a");
        assert_eq!(entries[1]["stepName"], "b");
        assert_eq!(entries[1]["output"]["text"], "output from b");
    }

    #[test]
    fn reduce_grouped_groups_by_result() {
        let collect = make_collect(vec!["a", "b", "c"], ReduceStrategy::Grouped);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("LGTM")),
            ("b", "text_b", Some("NEEDS_FIX")),
            ("c", "text_c", Some("LGTM")),
        ]);

        let reduced = apply_reduce(&collect, &outputs);

        assert!(reduced.result.is_none());
        let structured_output = reduced.structured_output.unwrap();
        let lgtm = structured_output["LGTM"].as_array().unwrap();
        assert!(lgtm.contains(&serde_json::Value::String("a".to_string())));
        assert!(lgtm.contains(&serde_json::Value::String("c".to_string())));
        let needs_fix = structured_output["NEEDS_FIX"].as_array().unwrap();
        assert!(needs_fix.contains(&serde_json::Value::String("b".to_string())));
    }

    #[test]
    fn reduce_any_needs_fix_one_needs_fix() {
        let collect = make_collect(vec!["a", "b", "c"], ReduceStrategy::AnyNeedsFix);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("LGTM")),
            ("b", "text_b", Some("NEEDS_FIX")),
            ("c", "text_c", Some("LGTM")),
        ]);

        let reduced = apply_reduce(&collect, &outputs);

        assert_eq!(reduced.result, Some("NEEDS_FIX".to_string()));
    }

    #[test]
    fn reduce_any_needs_fix_all_lgtm() {
        let collect = make_collect(vec!["a", "b", "c"], ReduceStrategy::AnyNeedsFix);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("LGTM")),
            ("b", "text_b", Some("LGTM")),
            ("c", "text_c", Some("LGTM")),
        ]);

        let reduced = apply_reduce(&collect, &outputs);

        assert_eq!(reduced.result, Some("LGTM".to_string()));
    }

    #[test]
    fn reduce_any_needs_fix_no_result_treated_as_lgtm() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AnyNeedsFix);
        let outputs = make_outputs(vec![
            ("a", "Everything looks good", None),
            ("b", "Found issues text", None),
        ]);

        let reduced = apply_reduce(&collect, &outputs);

        assert_eq!(reduced.result, Some("LGTM".to_string()));
    }

    #[test]
    fn reduce_all_passed_all_pass() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AllPassed);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("PASSED")),
            ("b", "text_b", Some("PASSED")),
        ]);

        let reduced = apply_reduce(&collect, &outputs);

        assert_eq!(reduced.result, Some("PASSED".to_string()));
    }

    #[test]
    fn reduce_all_passed_one_failed() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AllPassed);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("PASSED")),
            ("b", "text_b", Some("FAILED")),
        ]);

        let reduced = apply_reduce(&collect, &outputs);

        assert_eq!(reduced.result, Some("FAILED".to_string()));
    }

    #[test]
    fn reduce_all_passed_no_result_treated_as_failed() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AllPassed);
        let outputs = make_outputs(vec![
            ("a", "All tests ran", None),
            ("b", "Some tests ran", None),
        ]);

        let reduced = apply_reduce(&collect, &outputs);

        assert_eq!(reduced.result, Some("FAILED".to_string()));
    }

    #[test]
    fn reduce_any_needs_fix_structured_output_is_array() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AnyNeedsFix);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("LGTM")),
            ("b", "text_b", Some("NEEDS_FIX")),
        ]);

        let reduced = apply_reduce(&collect, &outputs);

        assert_eq!(reduced.result, Some("NEEDS_FIX".to_string()));
        let structured_output = reduced.structured_output.unwrap();
        let entries = structured_output.as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["stepName"], "a");
        assert_eq!(entries[0]["output"]["text"], "text_a");
        assert_eq!(entries[1]["stepName"], "b");
        assert_eq!(entries[1]["output"]["text"], "text_b");
    }

    #[test]
    fn reduce_all_passed_structured_output_is_array() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AllPassed);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("PASSED")),
            ("b", "text_b", Some("PASSED")),
        ]);

        let reduced = apply_reduce(&collect, &outputs);

        assert_eq!(reduced.result, Some("PASSED".to_string()));
        let structured_output = reduced.structured_output.unwrap();
        let entries = structured_output.as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["stepName"], "a");
        assert_eq!(entries[1]["stepName"], "b");
    }

    #[test]
    fn collect_step_output_entries_returns_array_with_step_name_and_output() {
        let outputs = make_outputs(vec![
            ("s1", "out1", Some("LGTM")),
            ("s2", "out2", Some("NEEDS_FIX")),
        ]);
        let from = vec!["s1".to_string(), "s2".to_string()];

        let entries = collect_step_output_entries(&from, &outputs);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["stepName"], "s1");
        assert_eq!(entries[0]["output"]["text"], "out1");
        assert_eq!(entries[1]["stepName"], "s2");
        assert_eq!(entries[1]["output"]["text"], "out2");
    }

    #[test]
    fn collect_step_output_entries_skips_missing_outputs() {
        let outputs = make_outputs(vec![("s1", "out1", None)]);
        let from = vec!["s1".to_string(), "s2".to_string()];

        let entries = collect_step_output_entries(&from, &outputs);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["stepName"], "s1");
    }

    #[test]
    fn collect_step_output_entries_skips_none_structured_output() {
        let mut outputs = HashMap::new();
        outputs.insert(
            "s1".to_string(),
            StepOutput {
                structured_output: None,
                ..make_step_output("s1", "text", None)
            },
        );
        let from = vec!["s1".to_string()];

        let entries = collect_step_output_entries(&from, &outputs);

        assert!(entries.is_empty());
    }

    #[test]
    fn resolve_step_result_returns_result_field() {
        let output = make_step_output("s", "output with NEEDS_FIX text", Some("LGTM"));

        let result = resolve_step_result(&output);

        assert_eq!(result, Some("LGTM".to_string()));
    }

    #[test]
    fn resolve_step_result_none_when_no_result() {
        let output = make_step_output("s", "found NEEDS_FIX issue", None);

        let result = resolve_step_result(&output);

        assert!(result.is_none());
    }

    #[test]
    fn resolve_step_result_no_match_returns_none() {
        let output = make_step_output("s", "everything is fine", None);

        let result = resolve_step_result(&output);

        assert!(result.is_none());
    }

    #[test]
    fn resolve_step_result_prefers_structured_verdict() {
        let output = StepOutput {
            step_name: "s".to_string(),
            run_index: 0,
            session_id: None,
            result: Some("LGTM".to_string()),
            structured_output: Some(serde_json::json!({
                "verdict": "NEEDS_FIX",
                "findings": [{ "severity": "error", "message": "bug" }],
            })),
            artifact_contract: None,
            token_usage: None,
            completed_at: 1000.0,
        };

        let result = resolve_step_result(&output);

        assert_eq!(result, Some("NEEDS_FIX".to_string()));
    }

    #[test]
    fn resolve_step_result_prefers_structured_status() {
        let output = StepOutput {
            step_name: "s".to_string(),
            run_index: 0,
            session_id: None,
            result: None,
            structured_output: Some(serde_json::json!({"status": "FIXED"})),
            artifact_contract: None,
            token_usage: None,
            completed_at: 1000.0,
        };

        let result = resolve_step_result(&output);

        assert_eq!(result, Some("FIXED".to_string()));
    }

    #[test]
    fn resolve_step_result_verdict_over_status() {
        let output = StepOutput {
            step_name: "s".to_string(),
            run_index: 0,
            session_id: None,
            result: None,
            structured_output: Some(serde_json::json!({
                "verdict": "LGTM",
                "status": "FIXED",
            })),
            artifact_contract: None,
            token_usage: None,
            completed_at: 1000.0,
        };

        let result = resolve_step_result(&output);

        assert_eq!(result, Some("LGTM".to_string()));
    }
}
