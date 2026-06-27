use std::collections::HashMap;

use crate::adaptor::gateway::workflow::schema;
use crate::adaptor::gateway::workflow::state as legacy_state;
use crate::domain::workflow as domain;
use crate::domain::workflow::value_objects::ResolvedFacets;

pub(crate) fn workflow_definition_to_domain(
    workflow: &schema::Workflow,
) -> domain::WorkflowDefinition {
    domain::WorkflowDefinition {
        name: workflow.name.clone(),
        description: workflow.description.clone(),
        builtin: workflow.builtin,
        variables: workflow.variables.clone(),
        nodes: workflow
            .nodes
            .iter()
            .map(node_definition_to_domain)
            .collect(),
    }
}

pub(crate) fn workflow_execution_state_to_domain(
    state: &legacy_state::WorkflowExecutionState,
) -> domain::WorkflowExecutionState {
    match state {
        legacy_state::WorkflowExecutionState::Running => domain::WorkflowExecutionState::Running,
        legacy_state::WorkflowExecutionState::WaitingApproval => {
            domain::WorkflowExecutionState::WaitingApproval
        }
        legacy_state::WorkflowExecutionState::Completed => {
            domain::WorkflowExecutionState::Completed
        }
        legacy_state::WorkflowExecutionState::Failed {
            reason,
            kind,
            retry_count,
        } => domain::WorkflowExecutionState::Failed {
            reason: reason.clone(),
            kind: *kind,
            retry_count: *retry_count,
        },
        legacy_state::WorkflowExecutionState::Aborted => domain::WorkflowExecutionState::Aborted,
    }
}

pub(crate) fn step_outputs_to_domain(
    step_outputs: &HashMap<String, legacy_state::StepOutput>,
) -> HashMap<String, domain::StepOutput> {
    step_outputs
        .iter()
        .map(|(key, output)| (key.clone(), step_output_to_domain(output)))
        .collect()
}

pub(crate) fn step_output_to_domain(output: &legacy_state::StepOutput) -> domain::StepOutput {
    domain::StepOutput {
        step_name: output.step_name.clone(),
        run_index: output.run_index,
        session_id: output.session_id.clone(),
        result: output.result.clone(),
        structured_output: output.structured_output.clone(),
        output_contract: output.output_contract.clone(),
        token_usage: output.token_usage.as_ref().map(token_usage_to_domain),
        completed_at: output.completed_at,
    }
}

pub(crate) fn step_output_from_domain(output: domain::StepOutput) -> legacy_state::StepOutput {
    let token_usage = output.token_usage.as_ref().map(token_usage_from_domain);
    legacy_state::StepOutput {
        step_name: output.step_name,
        run_index: output.run_index,
        session_id: output.session_id,
        result: output.result,
        structured_output: output.structured_output,
        output_contract: output.output_contract,
        token_usage,
        completed_at: output.completed_at,
    }
}

pub(crate) fn step_history_entries_to_domain(
    entries: &[legacy_state::StepHistoryEntry],
) -> Vec<domain::StepHistoryEntry> {
    entries.iter().map(step_history_entry_to_domain).collect()
}

fn step_history_entry_to_domain(
    entry: &legacy_state::StepHistoryEntry,
) -> domain::StepHistoryEntry {
    domain::StepHistoryEntry {
        step_name: entry.step_name.clone(),
        completed_at: entry.completed_at,
        result: entry.result.clone(),
        session_id: entry.session_id.clone(),
        token_usage: entry.token_usage.as_ref().map(token_usage_to_domain),
        structured_output: entry.structured_output.clone(),
        run_index: entry.run_index,
        child_outputs: entry
            .child_outputs
            .as_ref()
            .map(|children| children.iter().map(child_output_to_domain).collect()),
        state: entry.state.clone(),
    }
}

pub(crate) fn step_history_entry_from_domain(
    entry: domain::StepHistoryEntry,
) -> legacy_state::StepHistoryEntry {
    legacy_state::StepHistoryEntry {
        step_name: entry.step_name,
        completed_at: entry.completed_at,
        result: entry.result,
        session_id: entry.session_id,
        token_usage: entry.token_usage.as_ref().map(token_usage_from_domain),
        structured_output: entry.structured_output,
        run_index: entry.run_index,
        child_outputs: entry
            .child_outputs
            .map(|children| children.into_iter().map(child_output_from_domain).collect()),
        state: entry.state,
    }
}

fn child_output_from_domain(
    output: domain::ChildOutputSnapshot,
) -> legacy_state::ChildOutputSnapshot {
    legacy_state::ChildOutputSnapshot {
        step_name: output.step_name,
        session_id: output.session_id,
        result: output.result,
        run_index: output.run_index,
        completed_at: output.completed_at,
        structured_output: output.structured_output,
        output_contract: output.output_contract,
        state: output.state,
        failure_kind: output.failure_kind,
        failure_disposition: output.failure_disposition,
    }
}

fn child_output_to_domain(
    output: &legacy_state::ChildOutputSnapshot,
) -> domain::ChildOutputSnapshot {
    domain::ChildOutputSnapshot {
        step_name: output.step_name.clone(),
        session_id: output.session_id.clone(),
        result: output.result.clone(),
        run_index: output.run_index,
        completed_at: output.completed_at,
        structured_output: output.structured_output.clone(),
        output_contract: output.output_contract.clone(),
        state: output.state.clone(),
        failure_kind: output.failure_kind,
        failure_disposition: output.failure_disposition,
    }
}

pub(crate) fn token_usage_to_domain(usage: &legacy_state::TokenUsage) -> domain::TokenUsage {
    domain::TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

pub(crate) fn token_usage_from_domain(usage: &domain::TokenUsage) -> legacy_state::TokenUsage {
    legacy_state::TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

pub(crate) fn parallel_step_state_from_domain(
    state: domain::ParallelStepState,
) -> legacy_state::ParallelStepState {
    legacy_state::ParallelStepState {
        step_name: state.step_name,
        state: state.state,
        session_id: state.session_id,
        result: state.result,
        run_index: state.run_index,
        completed_at: state.completed_at,
        structured_output: state.structured_output,
        output_contract: state.output_contract,
        failure_kind: state.failure_kind,
        failure_disposition: state.failure_disposition,
    }
}

pub(crate) fn node_definition_to_domain(node: &schema::NodeDefinition) -> domain::NodeDefinition {
    domain::NodeDefinition {
        name: node.name.clone(),
        node_type: node_type_to_domain(node.node_type),
        policy: node.policy.clone(),
        knowledge: node.knowledge.clone(),
        instruction: node.instruction.clone(),
        output_contract: node.output_contract.clone(),
        input_contracts: node.input_contracts.clone(),
        pass_previous_response: node.pass_previous_response,
        pass_output_from: node.pass_output_from.clone(),
        inline_prompt: node.inline_prompt.clone(),
        collect: node.collect.as_ref().map(collect_config_to_domain),
        command: node.command.clone(),
        parallel_children: node
            .parallel_children
            .as_ref()
            .map(|children| children.iter().map(child_node_to_domain).collect()),
        aggregate: node.aggregate.as_ref().map(parallel_aggregate_to_domain),
        transition_rules: node
            .transition_rules
            .iter()
            .map(transition_rule_to_domain)
            .collect(),
        cycle_guard: node.cycle_guard.as_ref().map(cycle_guard_to_domain),
        resets_cycle_for: node.resets_cycle_for.clone(),
        model: node.model.clone(),
        permission: node.permission.clone(),
        resolved_facets: resolved_facets_to_domain(&node.resolved_facets),
    }
}

fn child_node_to_domain(child: &schema::ChildNodeDefinition) -> domain::ChildNodeDefinition {
    domain::ChildNodeDefinition {
        name: child.name.clone(),
        node_type: node_type_to_domain(child.node_type),
        policy: child.policy.clone(),
        knowledge: child.knowledge.clone(),
        instruction: child.instruction.clone(),
        output_contract: child.output_contract.clone(),
        input_contracts: child.input_contracts.clone(),
        pass_previous_response: child.pass_previous_response,
        pass_output_from: child.pass_output_from.clone(),
        model: child.model.clone(),
        permission: child.permission.clone(),
        resolved_facets: resolved_facets_to_domain(&child.resolved_facets),
    }
}

pub(crate) fn node_type_to_domain(node_type: schema::NodeType) -> domain::NodeType {
    match node_type {
        schema::NodeType::Agent => domain::NodeType::Agent,
        schema::NodeType::Bash => domain::NodeType::Bash,
        schema::NodeType::Approval => domain::NodeType::Approval,
        schema::NodeType::Parallel => domain::NodeType::Parallel,
    }
}

pub(crate) fn node_type_from_domain(node_type: domain::NodeType) -> schema::NodeType {
    match node_type {
        domain::NodeType::Agent => schema::NodeType::Agent,
        domain::NodeType::Bash => schema::NodeType::Bash,
        domain::NodeType::Approval => schema::NodeType::Approval,
        domain::NodeType::Parallel => schema::NodeType::Parallel,
    }
}

pub(crate) fn collect_config_to_domain(collect: &schema::CollectConfig) -> domain::CollectConfig {
    domain::CollectConfig {
        from: collect.from.clone(),
        reduce: reduce_strategy_to_domain(&collect.reduce),
    }
}

fn reduce_strategy_to_domain(reduce: &schema::ReduceStrategy) -> domain::ReduceStrategy {
    match reduce {
        schema::ReduceStrategy::Last => domain::ReduceStrategy::Last,
        schema::ReduceStrategy::Concat => domain::ReduceStrategy::Concat,
        schema::ReduceStrategy::Grouped => domain::ReduceStrategy::Grouped,
        schema::ReduceStrategy::AnyNeedsFix => domain::ReduceStrategy::AnyNeedsFix,
        schema::ReduceStrategy::AllPassed => domain::ReduceStrategy::AllPassed,
    }
}

pub(crate) fn parallel_aggregate_to_domain(
    aggregate: &schema::ParallelAggregate,
) -> domain::ParallelAggregate {
    domain::ParallelAggregate {
        all_match: aggregate.all_match.clone(),
        any_match: aggregate.any_match.clone(),
        then: aggregate.then.clone(),
        r#else: aggregate.r#else.clone(),
    }
}

pub(crate) fn transition_rule_to_domain(rule: &schema::TransitionRule) -> domain::TransitionRule {
    domain::TransitionRule {
        r#match: rule.r#match.clone(),
        next: rule.next.clone(),
    }
}

pub(crate) fn transition_rule_from_domain(rule: domain::TransitionRule) -> schema::TransitionRule {
    schema::TransitionRule {
        r#match: rule.r#match,
        next: rule.next,
    }
}

fn cycle_guard_to_domain(guard: &schema::CycleGuard) -> domain::CycleGuard {
    domain::CycleGuard {
        max_iterations: guard.max_iterations,
        on_exhausted: guard.on_exhausted.clone(),
    }
}

fn resolved_facets_to_domain(resolved: &schema::ResolvedFacets) -> ResolvedFacets {
    ResolvedFacets {
        policy: resolved.policy.clone(),
        knowledge: resolved.knowledge.clone(),
        instruction: resolved.instruction.clone(),
        output_contract: resolved.output_contract.clone(),
        input_contracts: resolved.input_contracts.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_definition_to_domain_preserves_runtime_only_resolved_facets() {
        let workflow = schema::Workflow {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            variables: Default::default(),
            nodes: vec![schema::NodeDefinition {
                name: "implement".to_string(),
                node_type: schema::NodeType::Agent,
                instruction: Some("inst".to_string()),
                resolved_facets: schema::ResolvedFacets {
                    instruction: Some("resolved instruction".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }],
        };

        let mapped = workflow_definition_to_domain(&workflow);

        assert_eq!(mapped.nodes[0].instruction.as_deref(), Some("inst"));
        assert_eq!(
            mapped.nodes[0].resolved_facets.instruction.as_deref(),
            Some("resolved instruction")
        );
    }
}
