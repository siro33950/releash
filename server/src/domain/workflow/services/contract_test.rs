pub(crate) mod contract_service_tests {
    use super::super::*;
    use crate::domain::workflow::value_objects::{
        FacetRefs, FanoutSpec, NodeDefinition, NodeKind, SchemaDef, SessionSpec, WorkflowDefinition,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: BTreeMap::from([(
                "review".to_string(),
                SchemaDef::Object {
                    properties: BTreeMap::from([(
                        "verdict".to_string(),
                        SchemaDef::String {
                            r#enum: Some(vec!["LGTM".to_string(), "FIX".to_string()]),
                        },
                    )]),
                    required: BTreeSet::from(["verdict".to_string()]),
                },
            )]),
            nodes: vec![
                NodeDefinition {
                    name: "fanout".to_string(),
                    kind: NodeKind::Fanout(FanoutSpec {
                        children: vec![crate::domain::workflow::ChildEntry::reference("child")],
                        items: None,
                    }),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "child".to_string(),
                    kind: NodeKind::Session(SessionSpec {
                        facets: FacetRefs {
                            instruction: Some("review".to_string()),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    artifact: Some("review".to_string()),
                    ..Default::default()
                },
            ],
            entry: "fanout".to_string(),
        }
    }

    #[test]
    fn test_lookup_node_contract_top_level_fanout_childも探索する() {
        assert_eq!(
            lookup_node_contract(&workflow(), "child").as_deref(),
            Some("review")
        );
    }

    #[test]
    fn test_validate_artifact_value_schemaで検証する() {
        let valid = validate_artifact_value(
            &workflow().schemas,
            "review",
            serde_json::json!({
                "verdict": "LGTM"
            }),
        );
        assert!(matches!(
            valid,
            ContractValidationResult::Valid {
                result: Some(_),
                ..
            }
        ));

        let invalid = validate_artifact_value(
            &workflow().schemas,
            "review",
            serde_json::json!({
                "verdict": "MAYBE"
            }),
        );
        assert!(matches!(
            invalid,
            ContractValidationResult::Invalid(ContractViolation {
                reason,
                ..
            }) if reason == "schema_violation"
        ));
    }

    #[test]
    fn test_contract_prompt_guidance_explains_object_schema_and_extra_fields() {
        let schemas = BTreeMap::from([(
            "spec-directory".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::from([(
                    "spec_dir".to_string(),
                    SchemaDef::String { r#enum: None },
                )]),
                required: BTreeSet::from(["spec_dir".to_string()]),
            },
        )]);

        let guidance = render_contract_prompt_guidance(&schemas, "spec-directory").unwrap();

        assert!(guidance.contains("\"spec_dir\": \"string\""));
        assert!(guidance.contains("\"required\": ["));
        assert!(!guidance.contains("additionalProperties"));
        assert!(guidance.contains("Fields not listed in `properties` are accepted"));
        assert!(render_contract_prompt_guidance(&schemas, "missing").is_none());
    }

    #[test]
    fn test_contract_prompt_guidance_includes_all_transitive_item_schemas() {
        let schemas = BTreeMap::from([
            (
                "plan-review-result".to_string(),
                SchemaDef::Object {
                    properties: BTreeMap::from([(
                        "findings".to_string(),
                        SchemaDef::Array {
                            items: "plan-review-finding".to_string(),
                        },
                    )]),
                    required: BTreeSet::from(["findings".to_string()]),
                },
            ),
            (
                "plan-review-finding".to_string(),
                SchemaDef::Object {
                    properties: BTreeMap::from([(
                        "evidence".to_string(),
                        SchemaDef::Array {
                            items: "workflow-text".to_string(),
                        },
                    )]),
                    required: BTreeSet::from(["evidence".to_string()]),
                },
            ),
            (
                "workflow-text".to_string(),
                SchemaDef::String { r#enum: None },
            ),
        ]);

        let guidance = render_contract_prompt_guidance(&schemas, "plan-review-result").unwrap();

        assert!(guidance.contains("## Referenced Contract schemas"));
        assert!(guidance.contains("\"plan-review-finding\": {"));
        assert!(guidance.contains("\"workflow-text\": \"string\""));
        assert!(guidance.contains("Every transitive Contract needed"));
        assert!(guidance.contains("do not inspect Releash application data"));
    }

    #[test]
    fn test_contract_prompt_guidance_handles_schema_reference_cycles_once() {
        let schemas = BTreeMap::from([
            (
                "root".to_string(),
                SchemaDef::Array {
                    items: "child".to_string(),
                },
            ),
            (
                "child".to_string(),
                SchemaDef::Array {
                    items: "root".to_string(),
                },
            ),
        ]);

        let guidance = render_contract_prompt_guidance(&schemas, "root").unwrap();

        assert_eq!(guidance.matches("\"child\": {").count(), 1);
        assert!(!guidance.contains("\"root\": {"));
    }

    #[test]
    fn test_session_spec_default_is_available_for_artifact_nodes() {
        let _node = NodeDefinition {
            name: "review".to_string(),
            kind: NodeKind::Session(SessionSpec::default()),
            artifact: Some("review".to_string()),
            ..Default::default()
        };
    }
}
