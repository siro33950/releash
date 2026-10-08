#[cfg(any(test, feature = "test-support"))]
use crate::adaptor::gateway::workflow::event as workflow_event;
use crate::adaptor::gateway::workflow::facet as gateway_facet;
use crate::domain::workflow as domain;
#[cfg(any(test, feature = "test-support"))]
use crate::usecase::workflow::ports::WorkflowEventDraft;

pub(crate) fn domain_workflow_to_schema(
    definition: &domain::WorkflowDefinition,
) -> Result<crate::adaptor::gateway::workflow::schema::WorkflowDefinitionYaml, domain::WorkflowError>
{
    Ok(
        crate::adaptor::gateway::workflow::schema::WorkflowDefinitionYaml {
            name: definition.name.clone(),
            description: definition.description.clone(),
            builtin: definition.builtin,
            schemas: definition
                .schemas
                .iter()
                .map(|(name, schema)| (name.clone(), domain_schema_to_schema(schema)))
                .collect(),
            nodes: definition.nodes.iter().map(domain_node_to_schema).collect(),
            entry: definition.entry.clone(),
        },
    )
}

pub fn schema_workflow_to_domain(
    workflow: crate::adaptor::gateway::workflow::schema::WorkflowDefinitionYaml,
) -> Result<domain::WorkflowDefinition, domain::WorkflowError> {
    Ok(crate::adaptor::gateway::workflow::domain_mapping::workflow_definition_to_domain(&workflow))
}

fn domain_node_to_schema(
    node: &domain::NodeDefinition,
) -> crate::adaptor::gateway::workflow::schema::NodeDefinition {
    crate::adaptor::gateway::workflow::schema::NodeDefinition {
        name: node.name.clone(),
        kind: domain_kind_to_schema(&node.kind),
        artifact: node.artifact.clone(),
        input: node.input.clone(),
        completion: node.completion.clone(),
        worktree: node.worktree,
    }
}

fn domain_kind_to_schema(
    kind: &domain::NodeKind,
) -> crate::adaptor::gateway::workflow::schema::NodeKind {
    match kind {
        domain::NodeKind::Command(spec) => {
            crate::adaptor::gateway::workflow::schema::NodeKind::Command(
                crate::adaptor::gateway::workflow::schema::CommandSpec {
                    command: spec.command.clone(),
                    env: spec.env.clone(),
                },
            )
        }
        domain::NodeKind::Session(spec) => {
            crate::adaptor::gateway::workflow::schema::NodeKind::Session(
                crate::adaptor::gateway::workflow::schema::SessionSpec {
                    provider: spec.provider,
                    model: spec.model.clone(),
                    permission: spec.permission,
                    facets: domain_facets_to_schema(&spec.facets),
                },
            )
        }
        domain::NodeKind::Fanout(spec) => {
            crate::adaptor::gateway::workflow::schema::NodeKind::Fanout(
                crate::adaptor::gateway::workflow::schema::FanoutSpec {
                    children: spec.children.clone(),
                    items: spec.items.as_ref().map(domain_items_source_to_schema),
                },
            )
        }
        domain::NodeKind::Sequence(spec) => {
            crate::adaptor::gateway::workflow::schema::NodeKind::Sequence(
                crate::adaptor::gateway::workflow::schema::SequenceSpec {
                    entry: spec.entry.clone(),
                    children: spec.children.clone(),
                },
            )
        }
    }
}

fn domain_facets_to_schema(
    facets: &domain::FacetRefs,
) -> crate::adaptor::gateway::workflow::schema::FacetRefs {
    crate::adaptor::gateway::workflow::schema::FacetRefs {
        policy: facets.policy.clone(),
        knowledge: facets.knowledge.clone(),
        instruction: facets.instruction.clone(),
    }
}

fn domain_items_source_to_schema(
    items: &domain::ItemsSource,
) -> crate::adaptor::gateway::workflow::schema::ItemsSource {
    match items {
        domain::ItemsSource::Literal(values) => {
            crate::adaptor::gateway::workflow::schema::ItemsSource::Literal(values.clone())
        }
        domain::ItemsSource::ArtifactField { node, field_path } => {
            crate::adaptor::gateway::workflow::schema::ItemsSource::ArtifactField {
                node: node.clone(),
                field_path: field_path.clone(),
            }
        }
    }
}

pub(crate) fn schema_workflow_summary_to_domain(
    summary: crate::adaptor::gateway::workflow::schema::Summary,
) -> domain::WorkflowSummary {
    domain::WorkflowSummary {
        failure: summary.failure,
        name: summary.name,
        description: summary.description,
        builtin: summary.builtin,
        is_running: summary.is_running,
        source_format: summary.source_format,
    }
}

#[cfg(test)]
pub(crate) fn domain_workflow_summary_to_schema(
    summary: domain::WorkflowSummary,
) -> crate::adaptor::gateway::workflow::schema::Summary {
    crate::adaptor::gateway::workflow::schema::Summary {
        failure: summary.failure,
        name: summary.name,
        description: summary.description,
        builtin: summary.builtin,
        is_running: summary.is_running,
        source_format: summary.source_format,
    }
}

pub(crate) fn domain_facet_kind_to_gateway(kind: domain::FacetKind) -> gateway_facet::FacetKind {
    match kind {
        domain::FacetKind::Policy => gateway_facet::FacetKind::Policy,
        domain::FacetKind::Knowledge => gateway_facet::FacetKind::Knowledge,
        domain::FacetKind::Instruction => gateway_facet::FacetKind::Instruction,
    }
}

pub(crate) fn gateway_facet_summary_to_domain(
    summary: crate::adaptor::gateway::workflow::schema::FacetSummary,
) -> domain::FacetSummary {
    domain::FacetSummary {
        key: summary.key,
        kind: summary.kind,
        description: summary.description,
        builtin: summary.builtin,
    }
}

#[cfg(test)]
pub(crate) fn domain_facet_summary_to_gateway(
    summary: domain::FacetSummary,
) -> crate::adaptor::gateway::workflow::schema::FacetSummary {
    crate::adaptor::gateway::workflow::schema::FacetSummary {
        key: summary.key,
        kind: summary.kind,
        description: summary.description,
        builtin: summary.builtin,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn event_draft_to_event(
    event: &WorkflowEventDraft,
) -> Result<workflow_event::WorkflowEvent, domain::WorkflowError> {
    let mut object = event.payload.as_object().cloned().ok_or_else(|| {
        domain::WorkflowError::validation(format!(
            "invalid payload for {} event: expected object",
            event.event_kind
        ))
    })?;
    object.insert(
        "event".to_string(),
        serde_json::Value::String(event.event_kind.clone()),
    );
    object.insert(
        "execution_id".to_string(),
        serde_json::Value::String(event.execution_id.clone()),
    );
    object.insert("timestamp".to_string(), serde_json::json!(event.timestamp));
    serde_json::from_value(serde_json::Value::Object(object)).map_err(|error| {
        domain::WorkflowError::validation(format!(
            "invalid payload for {} event: {error}",
            event.event_kind
        ))
    })
}

fn domain_schema_to_schema(
    schema: &domain::SchemaDef,
) -> crate::adaptor::gateway::workflow::schema::SchemaDef {
    match schema {
        domain::SchemaDef::Object {
            properties,
            required,
        } => crate::adaptor::gateway::workflow::schema::SchemaDef::Object {
            properties: properties
                .iter()
                .map(|(name, schema)| (name.clone(), domain_schema_to_schema(schema)))
                .collect(),
            required: required.clone(),
        },
        domain::SchemaDef::Array { items } => {
            crate::adaptor::gateway::workflow::schema::SchemaDef::Array {
                items: items.clone(),
            }
        }
        domain::SchemaDef::String { r#enum } => {
            crate::adaptor::gateway::workflow::schema::SchemaDef::String {
                r#enum: r#enum.clone(),
            }
        }
        domain::SchemaDef::Boolean => crate::adaptor::gateway::workflow::schema::SchemaDef::Boolean,
        domain::SchemaDef::Integer => crate::adaptor::gateway::workflow::schema::SchemaDef::Integer,
        domain::SchemaDef::Number => crate::adaptor::gateway::workflow::schema::SchemaDef::Number,
    }
}

#[cfg(test)]
#[path = "mapper_test.rs"]
mod mapper_tests;
