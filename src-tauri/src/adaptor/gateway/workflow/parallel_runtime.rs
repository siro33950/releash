use std::collections::HashMap;

use tokio::sync::Mutex;

#[cfg(test)]
use crate::adaptor::gateway::workflow::domain_mapping::step_output_to_domain;
use crate::adaptor::gateway::workflow::domain_mapping::{
    collect_config_to_domain, parallel_aggregate_to_domain, step_history_entry_from_domain,
    step_output_from_domain, step_outputs_to_domain, token_usage_to_domain,
};
use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::event::{CollectedOutputEntry, WorkflowEvent};
use crate::adaptor::gateway::workflow::execution_registry::find_by_worktree_mut;
use crate::adaptor::gateway::workflow::runtime_commit::StepOutcome;
use crate::adaptor::gateway::workflow::runtime_state::{
    ParallelChildRun, ParallelChildState, ParallelRunState, WorkflowExecution,
};
use crate::adaptor::gateway::workflow::schema::{
    ChildNodeDefinition, CollectConfig, ParallelAggregate,
};
use crate::adaptor::gateway::workflow::state::{StepOutput, TokenUsage, WorkflowState};
use crate::adaptor::gateway::workflow::step_settings::WorkflowDefaults;
use crate::adaptor::gateway::workflow::turn_completion;
use crate::domain::workflow::services::parallel as workflow_parallel;

#[derive(Debug, Clone)]
pub(crate) struct ParallelStartContext {
    pub(crate) parallel_steps: Vec<ChildNodeDefinition>,
    pub(crate) parent_step_name: String,
    pub(crate) parent_run_index: u32,
    pub(crate) order: u32,
    pub(crate) child_run_indices: Vec<u32>,
    pub(crate) aggregate: Option<ParallelAggregate>,
    pub(crate) execution_id: String,
    pub(crate) workflow_name: String,
    pub(crate) task: Option<String>,
    pub(crate) workflow_defaults: WorkflowDefaults,
}

impl ParallelStartContext {
    pub(crate) fn child_step_names(&self) -> Vec<String> {
        child_step_names(&self.parallel_steps)
    }

    pub(crate) fn started_event(&self, timestamp: f64) -> WorkflowEvent {
        WorkflowEvent::ParallelStarted {
            run_id: self.execution_id.clone(),
            workflow_name: self.workflow_name.clone(),
            parent_node_name: self.parent_step_name.clone(),
            child_node_names: self.child_step_names(),
            timestamp,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParallelPromptInputs {
    pub(crate) step_outputs: HashMap<String, StepOutput>,
    pub(crate) workflow_variables: HashMap<String, String>,
    pub(crate) workflow_declared_variables: HashMap<String, String>,
}

pub(crate) struct ParallelChildSessionSetup {
    pub(crate) step_name: String,
    pub(crate) session_id: String,
    pub(crate) system_prompt: Option<String>,
    pub(crate) user_message: String,
    pub(crate) output_contract: Option<String>,
    pub(crate) permission_mode: String,
    pub(crate) run_index: u32,
}

pub(crate) fn child_step_names(
    parallel_steps: &[crate::adaptor::gateway::workflow::schema::ChildNodeDefinition],
) -> Vec<String> {
    parallel_steps
        .iter()
        .map(|step| step.name.clone())
        .collect()
}

fn next_child_run_indices(
    counts: &HashMap<String, u32>,
    parallel_steps: &[ChildNodeDefinition],
) -> Vec<u32> {
    let mut counts = counts.clone();
    parallel_steps
        .iter()
        .map(|step| {
            let count = counts.entry(step.name.clone()).or_insert(0);
            *count += 1;
            *count
        })
        .collect()
}

pub(crate) fn prepare_parallel_start_context(
    exec: &WorkflowExecution,
) -> Result<ParallelStartContext, WorkflowEngineError> {
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
    let parallel_steps = step.parallel_children.clone().ok_or_else(|| {
        WorkflowEngineError::InvalidState(format!(
            "StartParallel requires parallel children for node '{}'",
            step.name
        ))
    })?;
    let parent_run_index = exec
        .step_execution_counts
        .get(&step.name)
        .copied()
        .unwrap_or(1);
    let child_run_indices = next_child_run_indices(&exec.step_execution_counts, &parallel_steps);
    Ok(ParallelStartContext {
        parent_step_name: step.name.clone(),
        parent_run_index,
        order: exec.step_history.len() as u32,
        child_run_indices,
        parallel_steps,
        aggregate: step.aggregate.clone(),
        execution_id: exec.id.clone(),
        workflow_name: exec.workflow.name.clone(),
        task: exec.task.clone(),
        workflow_defaults: exec.workflow_defaults.clone(),
    })
}

pub(crate) fn parallel_prompt_inputs(exec: &WorkflowExecution) -> ParallelPromptInputs {
    ParallelPromptInputs {
        step_outputs: exec.step_outputs.clone(),
        workflow_variables: exec.workflow_variables.clone(),
        workflow_declared_variables: exec.workflow.variables.clone(),
    }
}

pub(crate) fn apply_parallel_run_state(
    exec: &mut WorkflowExecution,
    parent_step_name: String,
    aggregate: Option<ParallelAggregate>,
    child_setups: &[ParallelChildSessionSetup],
) -> (Vec<u32>, WorkflowState) {
    let indices: Vec<u32> = child_setups.iter().map(|setup| setup.run_index).collect();
    for setup in child_setups {
        exec.step_execution_counts
            .insert(setup.step_name.clone(), setup.run_index);
    }

    let children: Vec<ParallelChildRun> = child_setups
        .iter()
        .map(|setup| ParallelChildRun {
            step_name: setup.step_name.clone(),
            session_id: setup.session_id.clone(),
            state: ParallelChildState::Running,
            result: None,
            structured_output: None,
            output_contract: setup.output_contract.clone(),
            failure_kind: None,
            failure_disposition: None,
            token_usage: TokenUsage::default(),
            run_index: setup.run_index,
        })
        .collect();

    exec.parallel_run = Some(ParallelRunState {
        parent_step_name,
        aggregate,
        children,
    });
    (indices, exec.to_workflow_state())
}

pub(crate) struct ReduceTransitionResult {
    pub(crate) next_outcome: StepOutcome,
    pub(crate) output_collected_event: WorkflowEvent,
    pub(crate) snapshot_before: WorkflowExecution,
}

pub(crate) enum ParallelParentCompletionTransition {
    Advance,
    TransitionTo {
        target_node_name: String,
        aggregate_result: String,
    },
}

pub(crate) struct ParallelParentCompletionPlan {
    pub(crate) child_step_names: Vec<String>,
    pub(crate) parent_step_output: StepOutput,
    pub(crate) history_entry: crate::adaptor::gateway::workflow::state::StepHistoryEntry,
    pub(crate) transition: ParallelParentCompletionTransition,
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
    let step_rules = step.transition_rules.clone();
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

    let next_outcome = if step_rules.is_empty() {
        exec.apply_advance()
    } else if let Some(ref result_str) = reduce_result.result {
        match turn_completion::evaluate_auto_rules(result_str, &step_rules) {
            Some((next_step, _)) => exec.apply_transition(&next_step)?,
            None => exec.apply_advance(),
        }
    } else {
        exec.apply_advance()
    };

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
pub(crate) fn evaluate_aggregate(
    aggregate: &ParallelAggregate,
    step_outputs: &HashMap<String, StepOutput>,
    child_step_names: &[String],
) -> bool {
    let aggregate = parallel_aggregate_to_domain(aggregate);
    let step_outputs = step_outputs_to_domain(step_outputs);
    workflow_parallel::evaluate_aggregate(&aggregate, &step_outputs, child_step_names)
}

pub(crate) fn plan_parallel_parent_completion(
    parent_step_name: &str,
    parent_run_index: u32,
    aggregate: Option<&ParallelAggregate>,
    children: &[ParallelChildRun],
    step_outputs: &HashMap<String, StepOutput>,
    timestamp: f64,
) -> ParallelParentCompletionPlan {
    let aggregate = aggregate.map(parallel_aggregate_to_domain);
    let children: Vec<workflow_parallel::ParallelChildCompletionInput> = children
        .iter()
        .map(|child| workflow_parallel::ParallelChildCompletionInput {
            step_name: child.step_name.clone(),
            session_id: child.session_id.clone(),
            result: child.result.clone(),
            token_usage: token_usage_to_domain(&child.token_usage),
            run_index: child.run_index,
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
    let step_outputs = step_outputs_to_domain(step_outputs);
    let plan = workflow_parallel::plan_parallel_parent_completion(
        parent_step_name,
        parent_run_index,
        aggregate.as_ref(),
        &children,
        &step_outputs,
        timestamp,
    );
    let transition = match plan.transition {
        workflow_parallel::ParallelParentTransitionPlan::Advance => {
            ParallelParentCompletionTransition::Advance
        }
        workflow_parallel::ParallelParentTransitionPlan::TransitionTo {
            target_node_name,
            aggregate_result,
        } => ParallelParentCompletionTransition::TransitionTo {
            target_node_name,
            aggregate_result,
        },
    };
    ParallelParentCompletionPlan {
        child_step_names: plan.child_step_names,
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
        ChildNodeDefinition, NodeDefinition, NodeType, ReduceStrategy, Workflow,
    };
    use crate::adaptor::gateway::workflow::state::WorkflowExecutionState;

    fn workflow_execution_fixture(node: NodeDefinition) -> WorkflowExecution {
        WorkflowExecution {
            id: "run-1".to_string(),
            workflow: Workflow {
                name: "test-workflow".to_string(),
                description: String::new(),
                builtin: false,
                variables: HashMap::from([("declared".to_string(), "yes".to_string())]),
                nodes: vec![node],
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
            task: Some("ship it".to_string()),
            parallel_run: None,
            workflow_variables: HashMap::new(),
        }
    }

    #[test]
    fn prepare_parallel_start_context_captures_current_parallel_node() {
        let child = ChildNodeDefinition {
            name: "review-a".to_string(),
            model: Some("model-a".to_string()),
            ..Default::default()
        };
        let aggregate = ParallelAggregate {
            all_match: Some("LGTM".to_string()),
            any_match: None,
            then: "merge".to_string(),
            r#else: "fix".to_string(),
        };
        let exec = workflow_execution_fixture(NodeDefinition {
            name: "parallel-review".to_string(),
            node_type: NodeType::Parallel,
            parallel_children: Some(vec![child]),
            aggregate: Some(aggregate.clone()),
            ..Default::default()
        });

        let context = prepare_parallel_start_context(&exec).unwrap();

        assert_eq!(context.execution_id, "run-1");
        assert_eq!(context.workflow_name, "test-workflow");
        assert_eq!(context.parent_step_name, "parallel-review");
        assert_eq!(context.child_step_names(), vec!["review-a".to_string()]);
        assert_eq!(context.aggregate, Some(aggregate));
        assert_eq!(context.task.as_deref(), Some("ship it"));
        assert_eq!(context.workflow_defaults.permission_mode, "ask");

        assert!(matches!(
            context.started_event(42.0),
            WorkflowEvent::ParallelStarted {
                run_id,
                workflow_name,
                parent_node_name,
                child_node_names,
                timestamp,
            } if run_id == "run-1"
                && workflow_name == "test-workflow"
                && parent_node_name == "parallel-review"
                && child_node_names == vec!["review-a".to_string()]
                && (timestamp - 42.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn prepare_parallel_start_context_rejects_node_without_children() {
        let exec = workflow_execution_fixture(NodeDefinition {
            name: "plan".to_string(),
            ..Default::default()
        });

        let err = prepare_parallel_start_context(&exec).unwrap_err();

        assert!(matches!(
            err,
            WorkflowEngineError::InvalidState(message)
                if message == "StartParallel requires parallel children for node 'plan'"
        ));
    }

    #[test]
    fn parallel_prompt_inputs_clones_runtime_inputs() {
        let mut exec = workflow_execution_fixture(NodeDefinition {
            name: "parallel-review".to_string(),
            node_type: NodeType::Parallel,
            parallel_children: Some(vec![ChildNodeDefinition {
                name: "review-a".to_string(),
                ..Default::default()
            }]),
            ..Default::default()
        });
        exec.step_outputs.insert(
            "plan".to_string(),
            make_step_output("plan", "draft", Some("DONE")),
        );
        exec.workflow_variables
            .insert("contract".to_string(), "ready".to_string());

        let inputs = parallel_prompt_inputs(&exec);

        assert_eq!(
            inputs.step_outputs["plan"].structured_output,
            Some(serde_json::json!({ "text": "draft" }))
        );
        assert_eq!(
            inputs
                .workflow_variables
                .get("contract")
                .map(String::as_str),
            Some("ready")
        );
        assert_eq!(
            inputs
                .workflow_declared_variables
                .get("declared")
                .map(String::as_str),
            Some("yes")
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
            output_contract: None,
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
    fn evaluate_aggregate_child_without_output_contract_has_no_step_output() {
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
            output_contract: None,
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
            output_contract: None,
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
            output_contract: None,
            token_usage: None,
            completed_at: 1000.0,
        };

        let result = resolve_step_result(&output);

        assert_eq!(result, Some("LGTM".to_string()));
    }
}
