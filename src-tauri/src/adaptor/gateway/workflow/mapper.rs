#[cfg(test)]
use crate::adaptor::gateway::workflow::event as workflow_event;
#[cfg(test)]
use crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata;
use crate::adaptor::gateway::workflow::facet as gateway_facet;
use crate::domain::workflow as domain;
#[cfg(test)]
use crate::usecase::workflow::ports::WorkflowEventDraft;

#[cfg(test)]
pub(crate) fn workflow_execution_record_to_metadata(
    execution: &domain::WorkflowExecutionRecord,
) -> WorkflowExecutionMetadata {
    WorkflowExecutionMetadata {
        execution_id: execution.execution_id.clone(),
        workflow_name: execution.workflow_name.clone(),
        status: execution.status,
        worktree_path: execution.worktree_path.clone(),
        current_node: execution.current_node.clone(),
        created_from: execution.created_from,
        started_at: execution.started_at,
        updated_at: execution.updated_at,
        completed_at: execution.completed_at,
        error_reason: execution.error_reason.clone(),
        interruption_reason: execution.interruption_reason,
        resume_from_node: execution.resume_from_node.clone(),
        total_token_usage: execution.total_token_usage.clone(),
    }
}

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

pub(crate) fn schema_workflow_to_domain(
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
        completion: domain_completion_to_schema(node.completion),
        worktree: node.worktree.clone(),
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
                    output: spec.output.clone(),
                    children: spec.children.clone(),
                },
            )
        }
    }
}

fn domain_completion_to_schema(
    completion: domain::NodeCompletion,
) -> crate::adaptor::gateway::workflow::schema::NodeCompletion {
    match completion {
        domain::NodeCompletion::Auto => {
            crate::adaptor::gateway::workflow::schema::NodeCompletion::Auto
        }
        domain::NodeCompletion::Approval => {
            crate::adaptor::gateway::workflow::schema::NodeCompletion::Approval
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

#[cfg(test)]
pub(crate) fn event_draft_to_event(
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
mod tests {
    use super::*;
    use crate::domain::workflow::{
        ExecutionOrigin, ExecutionStatus, FacetRefs, FanoutSpec, InputParam, ItemsSource,
        NodeDefinition, NodeKind, SessionSpec, TokenUsage,
    };

    #[test]
    fn execution_metadata_serializes_with_canonical_vocabulary() {
        let execution = domain::WorkflowExecutionRecord {
            execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
            workflow_name: "wf".to_string(),
            status: ExecutionStatus::Running,
            worktree_path: "/repo".to_string(),
            current_node: None,
            created_from: ExecutionOrigin::DesktopUi,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: TokenUsage {
                input_tokens: 3,
                output_tokens: 5,
            },
        };
        let metadata = workflow_execution_record_to_metadata(&execution);
        let value = serde_json::to_value(metadata).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "executionId": "00000000-0000-4000-8000-000000000001",
                "workflowName": "wf",
                "status": "running",
                "worktreePath": "/repo",
                "createdFrom": "desktop_ui",
                "startedAt": 1.0,
                "updatedAt": 2.0,
                "totalTokenUsage": {
                    "inputTokens": 3,
                    "outputTokens": 5
                }
            })
        );
        assert!(value.get("task").is_none());
    }

    #[test]
    fn workflow_summary_serializes_like_existing_wire_shape() {
        let domain = domain::WorkflowSummary {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            is_running: true,
            source_format: domain::WorkflowSourceFormat::Yaml,
        };
        let mapped = domain_workflow_summary_to_schema(domain.clone());

        assert_eq!(
            serde_json::to_value(mapped).unwrap(),
            serde_json::json!({
                "name": "wf",
                "description": "desc",
                "builtin": false,
                "is_running": true,
                "source_format": "yaml"
            })
        );
    }

    #[test]
    fn facet_summary_serializes_like_existing_wire_shape() {
        let domain = domain::FacetSummary {
            key: "coding".to_string(),
            kind: "policy".to_string(),
            description: "desc".to_string(),
            builtin: false,
        };
        let mapped = domain_facet_summary_to_gateway(domain.clone());

        assert_eq!(
            serde_json::to_value(mapped).unwrap(),
            serde_json::json!({
                "key": "coding",
                "kind": "policy",
                "description": "desc",
                "builtin": false
            })
        );
    }

    #[test]
    fn workflow_mapping_preserves_facet_refs_without_runtime_contents() {
        let definition = domain::WorkflowDefinition {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "node".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        knowledge: vec![
                            "knowledge-a".to_string(),
                            "knowledge-b".to_string(),
                            "knowledge-a".to_string(),
                        ],
                        instruction: Some("inst".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
            entry: "node".to_string(),
        };

        let schema = domain_workflow_to_schema(&definition).unwrap();
        assert_eq!(
            schema.nodes[0].session().unwrap().facets.knowledge,
            vec!["knowledge-a", "knowledge-b", "knowledge-a"]
        );
        assert_eq!(
            schema.nodes[0]
                .session()
                .unwrap()
                .facets
                .instruction
                .as_deref(),
            Some("inst")
        );

        let mapped = schema_workflow_to_domain(schema).unwrap();
        assert_eq!(mapped, definition);
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
    fn workflow_mapping_round_trips_loop_guard() {
        let definition = domain::WorkflowDefinition {
            name: "wf".to_string(),
            entry: "main".to_string(),
            nodes: vec![
                NodeDefinition {
                    name: "main".to_string(),
                    kind: NodeKind::Sequence(domain::SequenceSpec {
                        entry: None,
                        output: None,
                        children: vec![domain::ChildEntry {
                            on_failure: None,
                            name: "fix".to_string(),
                            inputs: Vec::new(),
                            rules: Some(vec![domain::Rule::LoopGuard {
                                max_iterations: 2,
                                on_exhausted: "done".to_string(),
                            }]),
                        }],
                    }),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "fix".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let schema = domain_workflow_to_schema(&definition).unwrap();
        let mapped = schema_workflow_to_domain(schema).unwrap();

        assert_eq!(mapped, definition);
    }

    #[test]
    fn workflow_mapping_round_trips_fanout_child_and_literal_items() {
        let definition = domain::WorkflowDefinition {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![domain::ChildEntry::reference("worker")],
                    items: Some(ItemsSource::Literal(vec![serde_json::json!({
                        "path": "src/lib.rs"
                    })])),
                }),
                ..Default::default()
            }],
            entry: "main".to_string(),
        };

        let schema = domain_workflow_to_schema(&definition).unwrap();
        assert_eq!(
            schema.nodes[0].fanout().unwrap().items,
            Some(
                crate::adaptor::gateway::workflow::schema::ItemsSource::Literal(vec![
                    serde_json::json!({"path": "src/lib.rs"}),
                ])
            )
        );

        let mapped = schema_workflow_to_domain(schema).unwrap();
        assert_eq!(mapped, definition);
    }

    #[test]
    fn workflow_definition_serializes_like_existing_wire_shape() {
        let definition = domain::WorkflowDefinition {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: [(
                "plan".to_string(),
                domain::SchemaDef::Object {
                    properties: Default::default(),
                    required: Default::default(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![NodeDefinition {
                name: "implement".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        instruction: Some("inst".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                input: vec![InputParam {
                    name: "item".to_string(),
                    contract: Some("plan".to_string()),
                }],
                artifact: Some("plan".to_string()),
                ..Default::default()
            }],
            entry: "implement".to_string(),
        };
        let schema = domain_workflow_to_schema(&definition).unwrap();

        assert_eq!(
            serde_json::to_value(schema).unwrap(),
            serde_json::json!({
                "name": "wf",
                "description": "desc",
                "builtin": false,
                "schemas": {
                    "plan": {
                        "type": "object"
                    }
                },
                "nodes": {
                    "implement": {
                        "session": {
                            "provider": "claude",
                            "facets": {
                                "instruction": "inst"
                            }
                        },
                        "artifact": "plan",
                        "input": [{"item": "plan"}]
                    }
                }
            })
        );
    }

    #[test]
    fn execution_started_draft_maps_to_canonical_event_shape() {
        let event = WorkflowEventDraft {
            execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
            event_kind: "execution_started".to_string(),
            timestamp: 10.0,
            payload: serde_json::json!({
                "workflow_name": "wf",
                "worktree_path": "/repo",
                "created_from": "cli",
                "request": "ship feature",
                "definition": {
                    "name": "wf",
                    "description": "",
                    "nodes": {
                        "main": {
                            "session": {
                                "provider": "claude"
                            }
                        }
                    }
                }
            }),
        };

        let mapped = event_draft_to_event(&event).unwrap();
        let json = serde_json::to_value(&mapped).unwrap();
        assert_eq!(json["event"], "execution_started");
        assert_eq!(json["execution_id"], event.execution_id);
        assert_eq!(json["workflow_name"], "wf");
        assert_eq!(json["request"], "ship feature");
    }
}
