use std::collections::{BTreeMap, HashMap};

use crate::adaptor::gateway::workflow::schema;
use crate::adaptor::gateway::workflow::state as legacy_state;
use crate::domain::workflow as domain;

pub(crate) fn workflow_definition_to_domain(
    workflow: &schema::Workflow,
) -> domain::WorkflowDefinition {
    domain::WorkflowDefinition {
        name: workflow.name.clone(),
        description: workflow.description.clone(),
        builtin: workflow.builtin,
        schemas: workflow_schemas_to_domain(&workflow.schemas),
        nodes: workflow
            .nodes
            .iter()
            .map(node_definition_to_domain)
            .collect(),
    }
}

pub(crate) fn workflow_schemas_to_domain(
    schemas: &BTreeMap<String, schema::SchemaDef>,
) -> BTreeMap<String, domain::SchemaDef> {
    schemas
        .iter()
        .map(|(name, schema)| (name.clone(), schema_def_to_domain(schema)))
        .collect()
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
        artifact_contract: output.artifact_contract.clone(),
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
        artifact_contract: output.artifact_contract,
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
        artifact_contract: output.artifact_contract,
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
        artifact_contract: output.artifact_contract.clone(),
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
        artifact_contract: state.artifact_contract,
        failure_kind: state.failure_kind,
        failure_disposition: state.failure_disposition,
    }
}

pub(crate) fn node_definition_to_domain(node: &schema::NodeDefinition) -> domain::NodeDefinition {
    domain::NodeDefinition {
        name: node.name.clone(),
        kind: node_kind_to_domain(&node.kind),
        artifact: node.artifact.clone(),
        input: node.input.clone(),
        inputs: node.inputs.clone(),
        collect: node.collect.as_ref().map(collect_config_to_domain),
        transition_rules: node
            .transition_rules
            .iter()
            .map(transition_rule_to_domain)
            .collect(),
        cycle_guard: node.cycle_guard.as_ref().map(cycle_guard_to_domain),
        resets_cycle_for: node.resets_cycle_for.clone(),
    }
}

pub(crate) fn node_kind_to_domain(kind: &schema::NodeKind) -> domain::NodeKind {
    match kind {
        schema::NodeKind::Command(spec) => domain::NodeKind::Command(domain::CommandSpec {
            command: spec.command.clone(),
        }),
        schema::NodeKind::Session(spec) => domain::NodeKind::Session(domain::SessionSpec {
            model: spec.model.clone(),
            permission: spec.permission.clone(),
            gate: session_gate_to_domain(spec.gate),
            facets: facet_refs_to_domain(&spec.facets),
        }),
        schema::NodeKind::Fanout(spec) => domain::NodeKind::Fanout(domain::FanoutSpec {
            parallel_children: spec
                .parallel_children
                .iter()
                .map(interim_child_to_domain)
                .collect(),
            aggregate: spec.aggregate.as_ref().map(parallel_aggregate_to_domain),
        }),
    }
}

fn session_gate_to_domain(gate: schema::SessionGate) -> domain::SessionGate {
    match gate {
        schema::SessionGate::Auto => domain::SessionGate::Auto,
        schema::SessionGate::Approval => domain::SessionGate::Approval,
    }
}

fn facet_refs_to_domain(facets: &schema::FacetRefs) -> domain::FacetRefs {
    domain::FacetRefs {
        policy: facets.policy.clone(),
        knowledge: facets.knowledge.clone(),
        instruction: facets.instruction.clone(),
    }
}

fn interim_child_to_domain(child: &schema::InterimChild) -> domain::InterimChild {
    domain::InterimChild {
        name: child.name.clone(),
        model: child.model.clone(),
        permission: child.permission.clone(),
        facets: facet_refs_to_domain(&child.facets),
        artifact: child.artifact.clone(),
        input: child.input.clone(),
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

pub(crate) fn schema_def_to_domain(schema: &schema::SchemaDef) -> domain::SchemaDef {
    match schema {
        schema::SchemaDef::Object {
            properties,
            required,
            additional_properties,
        } => domain::SchemaDef::Object {
            properties: properties
                .iter()
                .map(|(name, schema)| (name.clone(), schema_def_to_domain(schema)))
                .collect(),
            required: required.clone(),
            additional_properties: *additional_properties,
        },
        schema::SchemaDef::Array { items } => domain::SchemaDef::Array {
            items: items.clone(),
        },
        schema::SchemaDef::String { r#enum } => domain::SchemaDef::String {
            r#enum: r#enum.clone(),
        },
        schema::SchemaDef::Boolean => domain::SchemaDef::Boolean,
        schema::SchemaDef::Integer => domain::SchemaDef::Integer,
        schema::SchemaDef::Number => domain::SchemaDef::Number,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_definition_to_domain_preserves_facet_refs_without_runtime_contents() {
        let workflow = schema::Workflow {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![schema::NodeDefinition {
                name: "implement".to_string(),
                kind: schema::NodeKind::Session(schema::SessionSpec {
                    facets: schema::FacetRefs {
                        instruction: Some("inst".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
        };

        let mapped = workflow_definition_to_domain(&workflow);

        assert_eq!(
            mapped.nodes[0]
                .session()
                .unwrap()
                .facets
                .instruction
                .as_deref(),
            Some("inst")
        );
    }
}
