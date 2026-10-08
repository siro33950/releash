pub(crate) mod tests {
    use super::super::*;
    use crate::domain::workflow::{
        FacetRefs, FanoutSpec, InputParam, ItemsSource, NodeDefinition, NodeKind, SessionSpec,
    };

    #[test]
    fn workflow_summary_serializes_like_existing_wire_shape() {
        // Given
        let domain = domain::WorkflowSummary {
            failure: None,
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            is_running: true,
            source_format: domain::WorkflowSourceFormat::Yaml,
        };
        // When
        let mapped = domain_workflow_summary_to_schema(domain.clone());
        // Then
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
                        children: vec![domain::ChildEntry {
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
