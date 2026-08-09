use std::collections::{BTreeMap, HashMap};

use crate::adaptor::gateway::workflow::schema;
use crate::adaptor::gateway::workflow::state as runtime_state;
use crate::domain::workflow as domain;

pub(crate) fn workflow_definition_to_domain(
    workflow: &schema::WorkflowDefinitionYaml,
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

pub(crate) fn runtime_execution_state_to_domain(
    state: &runtime_state::RuntimeExecutionState,
) -> domain::RuntimeExecutionState {
    state.clone()
}

pub(crate) fn artifacts_to_domain(
    artifacts: &HashMap<String, runtime_state::RuntimeArtifact>,
) -> HashMap<String, domain::RuntimeArtifact> {
    artifacts
        .iter()
        .map(|(key, artifact)| (key.clone(), runtime_artifact_to_domain(artifact)))
        .collect()
}

pub(crate) fn runtime_artifact_to_domain(
    output: &runtime_state::RuntimeArtifact,
) -> domain::RuntimeArtifact {
    domain::RuntimeArtifact {
        node_name: output.node_name.clone(),
        attempt: output.attempt,
        session_id: output.session_id.clone(),
        result: output.result.clone(),
        artifact: output.artifact.clone(),
        contract: output.contract.clone(),
        token_usage: output.token_usage.as_ref().map(token_usage_to_domain),
        completed_at: output.completed_at,
    }
}

pub(crate) fn node_history_entries_to_domain(
    entries: &[runtime_state::NodeHistoryEntry],
) -> Vec<domain::NodeHistoryEntry> {
    entries.iter().map(node_history_entry_to_domain).collect()
}

fn node_history_entry_to_domain(
    entry: &runtime_state::NodeHistoryEntry,
) -> domain::NodeHistoryEntry {
    domain::NodeHistoryEntry {
        node_name: entry.node_name.clone(),
        completed_at: entry.completed_at,
        result: entry.result.clone(),
        session_id: entry.session_id.clone(),
        token_usage: entry.token_usage.as_ref().map(token_usage_to_domain),
        artifact: entry.artifact.clone(),
        attempt: entry.attempt,
        fanout_children: entry
            .fanout_children
            .as_ref()
            .map(|children| children.iter().map(child_output_to_domain).collect()),
        state: entry.state.clone(),
    }
}

fn child_output_to_domain(
    output: &runtime_state::FanoutChildSnapshot,
) -> domain::FanoutChildSnapshot {
    domain::FanoutChildSnapshot {
        node_name: output.node_name.clone(),
        session_id: output.session_id.clone(),
        result: output.result.clone(),
        attempt: output.attempt,
        completed_at: output.completed_at,
        artifact: output.artifact.clone(),
        contract: output.contract.clone(),
        state: output.state.clone(),
        failure_kind: output.failure_kind,
        failure_disposition: output.failure_disposition,
    }
}

pub(crate) fn token_usage_to_domain(usage: &runtime_state::TokenUsage) -> domain::TokenUsage {
    domain::TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

pub(crate) fn node_definition_to_domain(node: &schema::NodeDefinition) -> domain::NodeDefinition {
    domain::NodeDefinition {
        name: node.name.clone(),
        kind: node_kind_to_domain(&node.kind),
        artifact: node.artifact.clone(),
        input: node.input.clone(),
        inputs: node.inputs.clone(),
        rules: node.rules.iter().map(rule_to_domain).collect(),
    }
}

pub(crate) fn node_kind_to_domain(kind: &schema::NodeKind) -> domain::NodeKind {
    match kind {
        schema::NodeKind::Command(spec) => domain::NodeKind::Command(domain::CommandSpec {
            command: spec.command.clone(),
        }),
        schema::NodeKind::Session(spec) => domain::NodeKind::Session(domain::SessionSpec {
            provider: spec.provider,
            gate: session_gate_to_domain(spec.gate),
            facets: facet_refs_to_domain(&spec.facets),
        }),
        schema::NodeKind::Fanout(spec) => domain::NodeKind::Fanout(domain::FanoutSpec {
            child: spec.child.clone(),
            items: spec.items.as_ref().map(items_source_to_domain),
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

fn items_source_to_domain(items: &schema::ItemsSource) -> domain::ItemsSource {
    match items {
        schema::ItemsSource::Literal(values) => domain::ItemsSource::Literal(values.clone()),
        schema::ItemsSource::ArtifactField { node, field } => domain::ItemsSource::ArtifactField {
            node: node.clone(),
            field: field.clone(),
        },
    }
}

pub(crate) fn rule_to_domain(rule: &schema::Rule) -> domain::Rule {
    match rule {
        schema::Rule::When { on, then, next } => domain::Rule::When {
            on: on.clone(),
            then: then.clone(),
            next: next.clone(),
        },
        schema::Rule::Switch { on, cases, next } => domain::Rule::Switch {
            on: on.clone(),
            cases: cases.clone(),
            next: next.clone(),
        },
        schema::Rule::LoopGuard {
            max_iterations,
            on_exhausted,
            reset_on,
        } => domain::Rule::LoopGuard {
            max_iterations: *max_iterations,
            on_exhausted: on_exhausted.clone(),
            reset_on: reset_on.clone(),
        },
        schema::Rule::Next(next) => domain::Rule::Next(next.clone()),
    }
}

pub(crate) fn schema_def_to_domain(schema: &schema::SchemaDef) -> domain::SchemaDef {
    match schema {
        schema::SchemaDef::Object {
            properties,
            required,
        } => domain::SchemaDef::Object {
            properties: properties
                .iter()
                .map(|(name, schema)| (name.clone(), schema_def_to_domain(schema)))
                .collect(),
            required: required.clone(),
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
        let workflow = schema::WorkflowDefinitionYaml {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![schema::NodeDefinition {
                name: "implement".to_string(),
                kind: schema::NodeKind::Session(schema::SessionSpec {
                    facets: schema::FacetRefs {
                        knowledge: vec!["knowledge-a".to_string(), "knowledge-b".to_string()],
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
            mapped.nodes[0].session().unwrap().facets.knowledge,
            vec!["knowledge-a", "knowledge-b"]
        );
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

    #[test]
    fn workflow_definition_to_domain_preserves_fanout_child_and_items() {
        let workflow = schema::WorkflowDefinitionYaml {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![schema::NodeDefinition {
                name: "fanout".to_string(),
                kind: schema::NodeKind::Fanout(schema::FanoutSpec {
                    child: vec!["lint".to_string(), "test".to_string()],
                    items: Some(schema::ItemsSource::ArtifactField {
                        node: "plan".to_string(),
                        field: "targets".to_string(),
                    }),
                }),
                ..Default::default()
            }],
        };

        let mapped = workflow_definition_to_domain(&workflow);
        let fanout = mapped.nodes[0].fanout().unwrap();

        assert_eq!(fanout.child, vec!["lint".to_string(), "test".to_string()]);
        assert_eq!(
            fanout.items,
            Some(domain::ItemsSource::ArtifactField {
                node: "plan".to_string(),
                field: "targets".to_string(),
            })
        );
    }
}
