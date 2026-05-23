use std::collections::{HashMap, HashSet};

use crate::protocol;
use crate::workflow::schema;
use crate::workflow::state::WorkflowState;

#[derive(Debug, Clone, Default)]
pub struct WorkflowStepRuntimeProjection {
    pub runtime_active: bool,
    pub tab_open: bool,
}

#[derive(Debug, Clone)]
pub struct WorkflowStateProjection {
    pub state: WorkflowState,
    pub runtime_states: HashMap<String, WorkflowStepRuntimeProjection>,
}

pub fn build_workflow_state_projection_from_sets(
    state: WorkflowState,
    active_sessions: &HashSet<String>,
    open_sessions: &HashSet<String>,
) -> WorkflowStateProjection {
    let runtime_states: HashMap<String, WorkflowStepRuntimeProjection> =
        crate::workflow::runtime_view::collect_step_session_ids(&state)
            .into_iter()
            .map(|session_id| {
                (
                    session_id.clone(),
                    WorkflowStepRuntimeProjection {
                        runtime_active: active_sessions.contains(&session_id),
                        tab_open: open_sessions.contains(&session_id),
                    },
                )
            })
            .collect();
    WorkflowStateProjection {
        state,
        runtime_states,
    }
}

pub fn workflow_state_to_view(state: WorkflowState) -> protocol::WorkflowStateFieldsView {
    protocol::WorkflowStateFieldsView {
        execution_id: state.execution_id,
        workflow_name: state.workflow_name,
        state: workflow_execution_state_to_view(state.state),
        current_step_index: state.current_step_index,
        current_step_name: state.current_step_name,
        current_session_id: state.current_session_id,
        total_steps: state.total_steps,
        step_history: state
            .step_history
            .into_iter()
            .map(step_history_entry_to_view)
            .collect(),
        step_execution_counts: state.step_execution_counts,
        workflow_definition: workflow_definition_to_view(state.workflow_definition),
        total_token_usage: token_usage_to_view(state.total_token_usage),
        step_states: state.step_states,
        step_outputs: state
            .step_outputs
            .into_iter()
            .map(|(key, output)| (key, step_output_to_view(output)))
            .collect(),
        active_parallel_steps: state
            .active_parallel_steps
            .into_iter()
            .map(parallel_step_state_to_view)
            .collect(),
        workflow_variables: state.workflow_variables,
        approval_operations: state.approval_operations.map(|operations| {
            protocol::ApprovalOperationsView {
                can_reject: operations.can_reject,
            }
        }),
        started_at: state.started_at,
        updated_at: state.updated_at,
    }
}

fn workflow_execution_state_to_view(
    state: crate::workflow::state::WorkflowExecutionState,
) -> protocol::WorkflowExecutionStateView {
    match state {
        crate::workflow::state::WorkflowExecutionState::Running => {
            protocol::WorkflowExecutionStateView::Running
        }
        crate::workflow::state::WorkflowExecutionState::WaitingApproval => {
            protocol::WorkflowExecutionStateView::WaitingApproval
        }
        crate::workflow::state::WorkflowExecutionState::Completed => {
            protocol::WorkflowExecutionStateView::Completed
        }
        crate::workflow::state::WorkflowExecutionState::Failed { reason } => {
            protocol::WorkflowExecutionStateView::Failed { reason }
        }
        crate::workflow::state::WorkflowExecutionState::Aborted => {
            protocol::WorkflowExecutionStateView::Aborted
        }
    }
}

fn token_usage_to_view(usage: crate::workflow::state::TokenUsage) -> protocol::TokenUsageView {
    protocol::TokenUsageView {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

fn workflow_definition_to_view(workflow: schema::Workflow) -> protocol::WorkflowDefinitionView {
    protocol::WorkflowDefinitionView {
        name: workflow.name,
        description: workflow.description,
        builtin: workflow.builtin,
        nodes: workflow
            .nodes
            .into_iter()
            .map(workflow_node_to_view)
            .collect(),
    }
}

fn workflow_node_to_view(node: schema::NodeDefinition) -> protocol::WorkflowNodeDefinitionView {
    protocol::WorkflowNodeDefinitionView {
        name: node.name,
        node_type: node_type_to_view(node.node_type),
        policy: node.policy,
        knowledge: node.knowledge,
        instruction: node.instruction,
        output_contract: node.output_contract,
        pass_previous_response: node.pass_previous_response,
        pass_output_from: node.pass_output_from,
        inline_prompt: node.inline_prompt,
        collect: node.collect.map(collect_config_to_view),
        command: node.command,
        parallel_children: node
            .parallel_children
            .map(|children| children.into_iter().map(child_node_to_view).collect()),
        aggregate: node.aggregate.map(aggregate_config_to_view),
        rules: node
            .transition_rules
            .into_iter()
            .map(transition_rule_to_view)
            .collect(),
        cycle_guard: node.cycle_guard.map(cycle_guard_to_view),
        resets_cycle_for: node.resets_cycle_for,
        model: node.model,
        permission: node.permission,
    }
}

fn child_node_to_view(
    child: schema::ChildNodeDefinition,
) -> protocol::WorkflowChildNodeDefinitionView {
    protocol::WorkflowChildNodeDefinitionView {
        name: child.name,
        node_type: node_type_to_view(child.node_type),
        policy: child.policy,
        knowledge: child.knowledge,
        instruction: child.instruction,
        output_contract: child.output_contract,
        pass_previous_response: child.pass_previous_response,
        pass_output_from: child.pass_output_from,
        model: child.model,
        permission: child.permission,
    }
}

fn node_type_to_view(node_type: schema::NodeType) -> protocol::WorkflowNodeTypeView {
    match node_type {
        schema::NodeType::Agent => protocol::WorkflowNodeTypeView::Agent,
        schema::NodeType::Bash => protocol::WorkflowNodeTypeView::Bash,
        schema::NodeType::Approval => protocol::WorkflowNodeTypeView::Approval,
        schema::NodeType::Parallel => protocol::WorkflowNodeTypeView::Parallel,
    }
}

fn transition_rule_to_view(rule: schema::TransitionRule) -> protocol::WorkflowTransitionRuleView {
    protocol::WorkflowTransitionRuleView {
        r#match: rule.r#match,
        next: rule.next,
    }
}

fn cycle_guard_to_view(guard: schema::CycleGuard) -> protocol::WorkflowCycleGuardView {
    protocol::WorkflowCycleGuardView {
        max_iterations: guard.max_iterations,
        on_exhausted: guard.on_exhausted,
    }
}

fn collect_config_to_view(collect: schema::CollectConfig) -> protocol::WorkflowCollectConfigView {
    protocol::WorkflowCollectConfigView {
        from: collect.from,
        reduce: reduce_strategy_to_view(collect.reduce),
    }
}

fn reduce_strategy_to_view(reduce: schema::ReduceStrategy) -> protocol::WorkflowReduceStrategyView {
    match reduce {
        schema::ReduceStrategy::Last => protocol::WorkflowReduceStrategyView::Last,
        schema::ReduceStrategy::Concat => protocol::WorkflowReduceStrategyView::Concat,
        schema::ReduceStrategy::Grouped => protocol::WorkflowReduceStrategyView::Grouped,
        schema::ReduceStrategy::AnyNeedsFix => protocol::WorkflowReduceStrategyView::AnyNeedsFix,
        schema::ReduceStrategy::AllPassed => protocol::WorkflowReduceStrategyView::AllPassed,
    }
}

fn aggregate_config_to_view(
    aggregate: schema::ParallelAggregate,
) -> protocol::WorkflowAggregateConfigView {
    protocol::WorkflowAggregateConfigView {
        all_match: aggregate.all_match,
        any_match: aggregate.any_match,
        then: aggregate.then,
        r#else: aggregate.r#else,
    }
}

fn step_history_entry_to_view(
    entry: crate::workflow::state::StepHistoryEntry,
) -> protocol::StepHistoryEntryView {
    protocol::StepHistoryEntryView {
        step_name: entry.step_name,
        completed_at: entry.completed_at,
        result: entry.result,
        session_id: entry.session_id,
        token_usage: entry.token_usage.map(token_usage_to_view),
        structured_output: entry.structured_output,
        run_index: entry.run_index,
        child_outputs: entry
            .child_outputs
            .map(|children| children.into_iter().map(child_output_to_view).collect()),
        state: entry.state,
    }
}

fn child_output_to_view(
    output: crate::workflow::state::ChildOutputSnapshot,
) -> protocol::ChildOutputSnapshotView {
    protocol::ChildOutputSnapshotView {
        step_name: output.step_name,
        session_id: output.session_id,
        result: output.result,
        run_index: output.run_index,
        completed_at: output.completed_at,
        structured_output: output.structured_output,
        output_contract: output.output_contract,
        state: output.state,
    }
}

fn parallel_step_state_to_view(
    state: crate::workflow::state::ParallelStepState,
) -> protocol::ParallelStepStateView {
    protocol::ParallelStepStateView {
        step_name: state.step_name,
        state: state.state,
        session_id: state.session_id,
        result: state.result,
        run_index: state.run_index,
        completed_at: state.completed_at,
        structured_output: state.structured_output,
        output_contract: state.output_contract,
    }
}

fn step_output_to_view(output: crate::workflow::state::StepOutput) -> protocol::StepOutputView {
    protocol::StepOutputView {
        step_name: output.step_name,
        run_index: output.run_index,
        session_id: output.session_id,
        result: output.result,
        structured_output: output.structured_output,
        output_contract: output.output_contract,
        token_usage: output.token_usage.map(token_usage_to_view),
        completed_at: output.completed_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::schema::Workflow;
    use crate::workflow::state::{
        ChildOutputSnapshot, ParallelStepState, StepHistoryEntry, TokenUsage,
        WorkflowExecutionState,
    };

    fn state() -> WorkflowState {
        WorkflowState {
            execution_id: "exec-1".to_string(),
            workflow_name: "wf".to_string(),
            state: WorkflowExecutionState::Running,
            current_step_index: 1,
            current_step_name: "current".to_string(),
            current_session_id: Some("current-session".to_string()),
            total_steps: 2,
            step_history: vec![StepHistoryEntry {
                step_name: "done".to_string(),
                completed_at: 1.0,
                result: Some("ok".to_string()),
                session_id: Some("done-session".to_string()),
                token_usage: None,
                structured_output: None,
                run_index: 1,
                child_outputs: Some(vec![ChildOutputSnapshot {
                    step_name: "child".to_string(),
                    session_id: Some("child-session".to_string()),
                    result: Some("ok".to_string()),
                    run_index: 1,
                    completed_at: 2.0,
                    structured_output: None,
                    output_contract: None,
                    state: crate::workflow::state::default_step_entry_state(),
                }]),
                state: crate::workflow::state::default_step_entry_state(),
            }],
            step_execution_counts: HashMap::new(),
            workflow_definition: Workflow {
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                nodes: vec![],
            },
            total_token_usage: TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![ParallelStepState {
                step_name: "running-child".to_string(),
                state: "running".to_string(),
                session_id: Some("parallel-session".to_string()),
                result: None,
                run_index: 1,
                completed_at: None,
                structured_output: None,
                output_contract: None,
            }],
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 0.0,
            updated_at: 2.0,
        }
    }

    #[test]
    fn presenter_adds_runtime_state_for_current_history_child_and_parallel_sessions() {
        let open_sessions =
            HashSet::from(["done-session".to_string(), "parallel-session".to_string()]);
        let active_sessions = HashSet::from([
            "current-session".to_string(),
            "child-session".to_string(),
            "parallel-session".to_string(),
        ]);

        let view =
            build_workflow_state_projection_from_sets(state(), &active_sessions, &open_sessions);

        assert!(view.runtime_states["current-session"].runtime_active);
        assert!(!view.runtime_states["current-session"].tab_open);
        assert!(view.runtime_states["child-session"].runtime_active);
        assert!(!view.runtime_states["child-session"].tab_open);
        assert!(view.runtime_states["done-session"].tab_open);
        assert!(!view.runtime_states["done-session"].runtime_active);
        assert!(view.runtime_states["parallel-session"].runtime_active);
        assert!(view.runtime_states["parallel-session"].tab_open);
    }
}
