use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::HashMap;

use crate::adaptor::gateway::workflow::schema;
#[cfg(test)]
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
        entry: workflow.entry.clone(),
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

#[cfg(test)]
pub(crate) fn runtime_execution_state_to_domain(
    state: &runtime_state::RuntimeExecutionState,
) -> domain::RuntimeExecutionState {
    state.clone()
}

#[cfg(test)]
pub(crate) fn artifacts_to_domain(
    artifacts: &HashMap<String, runtime_state::RuntimeArtifact>,
) -> HashMap<String, domain::RuntimeArtifact> {
    artifacts
        .iter()
        .map(|(key, artifact)| (key.clone(), runtime_artifact_to_domain(artifact)))
        .collect()
}

#[cfg(test)]
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

#[cfg(test)]
pub(crate) fn node_history_entries_to_domain(
    entries: &[runtime_state::NodeHistoryEntry],
) -> Vec<domain::NodeHistoryEntry> {
    entries.iter().map(node_history_entry_to_domain).collect()
}

#[cfg(test)]
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

#[cfg(test)]
fn child_output_to_domain(
    output: &runtime_state::FanoutChildSnapshot,
) -> domain::value_objects::FanoutChildSnapshot {
    domain::value_objects::FanoutChildSnapshot {
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

#[cfg(test)]
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
        completion: node_completion_to_domain(node.completion),
        worktree: node.worktree.clone(),
    }
}

pub(crate) fn node_kind_to_domain(kind: &schema::NodeKind) -> domain::NodeKind {
    match kind {
        schema::NodeKind::Command(spec) => domain::NodeKind::Command(domain::CommandSpec {
            command: spec.command.clone(),
            env: spec.env.clone(),
        }),
        schema::NodeKind::Session(spec) => domain::NodeKind::Session(domain::SessionSpec {
            provider: spec.provider,
            model: spec.model.clone(),
            permission: spec.permission,
            facets: facet_refs_to_domain(&spec.facets),
        }),
        schema::NodeKind::Fanout(spec) => domain::NodeKind::Fanout(domain::FanoutSpec {
            children: spec.children.clone(),
            items: spec.items.as_ref().map(items_source_to_domain),
        }),
        schema::NodeKind::Sequence(spec) => domain::NodeKind::Sequence(domain::SequenceSpec {
            entry: spec.entry.clone(),
            output: spec.output.clone(),
            children: spec.children.clone(),
        }),
    }
}

fn node_completion_to_domain(completion: schema::NodeCompletion) -> domain::NodeCompletion {
    match completion {
        schema::NodeCompletion::Auto => domain::NodeCompletion::Auto,
        schema::NodeCompletion::Approval => domain::NodeCompletion::Approval,
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
            entry: "implement".to_string(),
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
                name: "main".to_string(),
                kind: schema::NodeKind::Fanout(schema::FanoutSpec {
                    children: vec![
                        domain::ChildEntry::reference("lint"),
                        domain::ChildEntry::reference("test"),
                    ],
                    items: Some(schema::ItemsSource::ArtifactField {
                        node: "plan".to_string(),
                        field: "targets".to_string(),
                    }),
                }),
                ..Default::default()
            }],
            entry: "main".to_string(),
        };

        let mapped = workflow_definition_to_domain(&workflow);
        let fanout = mapped.nodes[0].fanout().unwrap();

        assert_eq!(
            fanout.children,
            vec![
                domain::ChildEntry::reference("lint"),
                domain::ChildEntry::reference("test"),
            ]
        );
        assert_eq!(
            fanout.items,
            Some(domain::ItemsSource::ArtifactField {
                node: "plan".to_string(),
                field: "targets".to_string(),
            })
        );
    }
}
