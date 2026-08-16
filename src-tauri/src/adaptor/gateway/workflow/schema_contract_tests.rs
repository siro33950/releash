use super::schema::*;
use crate::domain::workflow::services::contract_schema;
use serde_json::Value;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_session_providerを必須とする() {
        let with_provider = r#"
name: provider-session
description: Provider session
nodes:
  main:
    session:
      provider: codex
"#;
        let missing_provider = r#"
name: missing-provider
description: Missing Provider
nodes:
  main:
    session:
      model: opus
"#;

        assert!(serde_saphyr::from_str::<WorkflowDefinitionYaml>(with_provider).is_ok());
        assert!(serde_saphyr::from_str::<WorkflowDefinitionYaml>(missing_provider).is_err());
    }

    #[test]
    fn parse_session_node() {
        let yaml = r#"
name: session-only
description: 単一セッション
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: implement
        policy: coding
"#;
        let wf: WorkflowDefinitionYaml = serde_saphyr::from_str(yaml).unwrap();
        let node = &wf.nodes[0];
        assert_eq!(node.kind_name(), NodeKindName::Session);
        let session = node.session().unwrap();
        assert_eq!(session.facets.instruction.as_deref(), Some("implement"));
        assert_eq!(session.facets.policy.as_deref(), Some("coding"));
        assert_eq!(node.completion, NodeCompletion::Auto);
    }

    #[test]
    fn session_knowledge_accepts_scalar_and_sequence_and_preserves_order() {
        let yaml = r#"
name: knowledge-shapes
description: knowledge scalar and sequence
nodes:
  main:
    session:
      provider: claude
      facets:
        knowledge: releash-thread-cli
  many:
    session:
      provider: claude
      facets:
        knowledge:
          - releash-thread-cli
          - requirements-design
"#;

        let workflow = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap();

        assert_eq!(
            workflow.nodes[0].session().unwrap().facets.knowledge,
            vec!["releash-thread-cli"]
        );
        assert_eq!(
            workflow.nodes[1].session().unwrap().facets.knowledge,
            vec!["releash-thread-cli", "requirements-design"]
        );
    }

    #[test]
    fn session_knowledge_serializes_one_as_scalar_and_many_as_sequence() {
        let workflow = WorkflowDefinitionYaml {
            name: "knowledge-shapes".to_string(),
            description: String::new(),
            nodes: vec![
                NodeDefinition {
                    name: "main".to_string(),
                    kind: NodeKind::Session(SessionSpec {
                        facets: FacetRefs {
                            knowledge: vec!["releash-thread-cli".to_string()],
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "many".to_string(),
                    kind: NodeKind::Session(SessionSpec {
                        facets: FacetRefs {
                            knowledge: vec![
                                "releash-thread-cli".to_string(),
                                "requirements-design".to_string(),
                            ],
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let serialized = serde_saphyr::to_string(&workflow).unwrap();
        let value = serde_saphyr::from_str::<Value>(&serialized).unwrap();

        assert_eq!(
            value["nodes"]["main"]["session"]["facets"]["knowledge"],
            Value::String("releash-thread-cli".to_string())
        );
        assert_eq!(
            value["nodes"]["many"]["session"]["facets"]["knowledge"],
            serde_json::json!(["releash-thread-cli", "requirements-design"])
        );
    }

    #[test]
    fn session_knowledge_rejects_non_string_sequence_elements() {
        let yaml = r#"
name: invalid-knowledge
description: invalid knowledge element
nodes:
  main:
    session:
      provider: claude
      facets:
        knowledge:
          - releash-thread-cli
          - 42
"#;

        assert!(serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).is_err());
    }

    #[test]
    fn test_schema契約_completion承認のsession_nodeをパースする() {
        let yaml = r#"
name: approval-only
description: 承認セッション
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: approve
        policy: planning
    completion: approval
"#;
        let wf: WorkflowDefinitionYaml = serde_saphyr::from_str(yaml).unwrap();
        let node = &wf.nodes[0];
        assert!(node.requires_approval_completion());
        assert_eq!(
            node.session().unwrap().facets.instruction.as_deref(),
            Some("approve")
        );
    }

    #[test]
    fn parse_command_node() {
        let yaml = r#"
name: command-only
description: command node
nodes:
  main:
    command: "cargo build"
"#;
        let wf: WorkflowDefinitionYaml = serde_saphyr::from_str(yaml).unwrap();
        let node = &wf.nodes[0];
        assert_eq!(node.kind_name(), NodeKindName::Command);
        match &node.kind {
            NodeKind::Command(spec) => assert_eq!(spec.command, "cargo build"),
            other => panic!("expected command node, got {other:?}"),
        }
    }

    #[test]
    fn parse_fanout_node_with_artifact_items() {
        let yaml = r#"
name: fanout
description: fanout test
nodes:
  main:
    fanout:
      child: [arch-review, security-review]
      items: plan.targets
"#;
        let wf: WorkflowDefinitionYaml = serde_saphyr::from_str(yaml).unwrap();
        let fanout = wf.nodes[0].fanout().unwrap();
        assert_eq!(
            fanout.child,
            vec!["arch-review".to_string(), "security-review".to_string()]
        );
        assert_eq!(
            fanout.items,
            Some(ItemsSource::ArtifactField {
                node: "plan".to_string(),
                field: "targets".to_string(),
            })
        );
        let serialized = serde_saphyr::to_string(&wf).unwrap();
        let serialized_value: Value = serde_saphyr::from_str(&serialized).unwrap();
        assert_eq!(
            serialized_value["nodes"]["main"]["fanout"]["child"],
            serde_json::json!(["arch-review", "security-review"])
        );
        assert_eq!(
            serialized_value["nodes"]["main"]["fanout"]["items"],
            Value::String("plan.targets".to_string())
        );
    }

    #[test]
    fn parse_fanout_single_child_and_literal_items() {
        let yaml = r#"
name: fanout
description: fanout test
nodes:
  main:
    fanout:
      child: reviewer
      items: [api, cli]
"#;

        let wf: WorkflowDefinitionYaml = serde_saphyr::from_str(yaml).unwrap();
        let fanout = wf.nodes[0].fanout().unwrap();

        assert_eq!(fanout.child, vec!["reviewer"]);
        assert_eq!(
            fanout.items,
            Some(ItemsSource::Literal(vec![
                Value::String("api".to_string()),
                Value::String("cli".to_string()),
            ]))
        );
        let serialized = serde_saphyr::to_string(&wf).unwrap();
        let serialized_value: Value = serde_saphyr::from_str(&serialized).unwrap();
        assert_eq!(
            serialized_value["nodes"]["main"]["fanout"]["child"],
            Value::String("reviewer".to_string())
        );
    }

    #[test]
    fn rejects_fanout_items_outside_literal_array_or_node_field_reference() {
        for items in ["source", "source.field.nested", "item.field", "request"] {
            let yaml = format!(
                r#"
name: invalid-items
description: invalid
nodes:
  main:
    fanout:
      child: reviewer
      items: {items}
"#
            );

            let error = serde_saphyr::from_str::<WorkflowDefinitionYaml>(&yaml).unwrap_err();
            assert!(
                error.to_string().contains("fanout.items"),
                "{items}: {error}"
            );
        }
    }

    #[test]
    fn parse_rules_tagged_enum_shapes() {
        let yaml = r#"
name: rules
description: rules test
nodes:
  main:
    session:
      provider: claude
    rules:
      - when: { on: ok, then: done }
        next: fix
      - loop_guard: { max_iterations: 3, on_exhausted: give_up, reset_on: main }
  triage:
    session:
      provider: claude
    rules:
      - switch:
          on: verdict
          cases:
            LGTM: done
            NEEDS_FIX: fix
        next: give_up
      - next: done
"#;
        let wf = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap();

        assert!(matches!(
            &wf.nodes[0].rules[0],
            Rule::When { on, then, next }
                if on == "ok" && then == "done" && next == "fix"
        ));
        assert!(matches!(
            &wf.nodes[0].rules[1],
            Rule::LoopGuard {
                max_iterations: 3,
                on_exhausted,
                reset_on: Some(reset_on),
            } if on_exhausted == "give_up" && reset_on == "main"
        ));
        assert!(matches!(
            &wf.nodes[1].rules[0],
            Rule::Switch { on, cases, next }
                if on == "verdict"
                    && cases.get("LGTM").map(String::as_str) == Some("done")
                    && cases.get("NEEDS_FIX").map(String::as_str) == Some("fix")
                    && next.as_deref() == Some("give_up")
        ));
        assert!(matches!(
            &wf.nodes[1].rules[1],
            Rule::Next(next) if next == "done"
        ));
    }

    #[test]
    fn rejects_rule_with_multiple_discriminators() {
        let yaml = r#"
name: invalid-rule
description: invalid
nodes:
  main:
    session:
      provider: claude
    rules:
      - when: { on: ok, then: done }
        switch:
          on: verdict
          cases:
            LGTM: done
        next: fix
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err
            .to_string()
            .contains("discriminator keys when, switch, and loop_guard are mutually exclusive"));
    }

    #[test]
    fn rejects_when_without_sibling_next() {
        let yaml = r#"
name: invalid-when
description: invalid
nodes:
  main:
    session:
      provider: claude
    rules:
      - when: { on: ok, then: done }
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("when rule requires sibling next"));
    }

    #[test]
    fn rejects_missing_kind_block() {
        let yaml = r#"
name: invalid
description: invalid
nodes:
  main:
    artifact: review
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("exactly one kind block"));
    }

    #[test]
    fn rejects_multiple_kind_blocks() {
        let yaml = r#"
name: invalid
description: invalid
nodes:
  main:
    command: "echo hi"
    session:
      provider: claude
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("exactly one kind block"));
    }

    #[test]
    fn schemas_accept_scalar_string_contract() {
        let yaml = r#"
name: scalar-schema
description: valid
schemas:
  request_text: string
nodes:
  main:
    session:
      provider: claude
    input:
      - item: request_text
"#;
        let workflow = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap();
        assert!(matches!(
            workflow.schemas.get("request_text"),
            Some(SchemaDef::String { r#enum: None })
        ));
    }

    #[test]
    fn schemas_serde_matches_domain_schema_helper_for_supported_shapes() {
        for value in [
            serde_json::json!("string"),
            serde_json::json!({"type": "string", "enum": ["LGTM", "NEEDS_FIX"]}),
            serde_json::json!({"type": "array", "items": "review-item"}),
            serde_json::json!({"type": "boolean"}),
            serde_json::json!({"type": "integer"}),
            serde_json::json!({"type": "number"}),
            serde_json::json!({
                "type": "object",
                "properties": {"verdict": {"type": "string", "enum": ["LGTM"]}},
                "required": ["verdict"]
            }),
        ] {
            let gateway_schema: SchemaDef = serde_json::from_value(value.clone()).unwrap();
            let domain_schema = contract_schema::schema_def_from_json(&value).unwrap();
            assert_eq!(
                crate::adaptor::gateway::workflow::domain_mapping::schema_def_to_domain(
                    &gateway_schema
                ),
                domain_schema
            );
        }
    }

    #[test]
    fn deserializeは未知fieldとkeywordを拒否する() {
        let known = r#"
name: strict-grammar
description: known values
schemas:
  review:
    type: object
    properties:
      verdict:
        type: string
        enum:
          - LGTM
          - NEEDS_FIX
    required:
      - verdict
nodes:
  main:
    command: cargo build
    artifact: review
    rules:
      - when:
          on: verdict
          then: LGTM
        next: review
  review:
    session:
      provider: claude
      facets:
        policy: coding
        knowledge:
          - workflow
        instruction: review
    rules:
      - switch:
          on: verdict
          cases:
            LGTM: dispatch
            NEEDS_FIX: main
        next: dispatch
  dispatch:
    fanout:
      child: worker
      items:
        - api
        - cli
    rules:
      - loop_guard:
          max_iterations: 2
          on_exhausted: review
          reset_on: main
  worker:
    session:
      provider: codex
    completion: approval
"#;
        assert!(serde_saphyr::from_str::<WorkflowDefinitionYaml>(known).is_ok());

        for (label, anchor, indent) in [
            ("root", "description: known values", ""),
            ("node", "    command: cargo build", "    "),
            ("session", "      provider: claude", "      "),
            ("session facets", "        policy: coding", "        "),
            ("fanout", "      child: worker", "      "),
            ("when rule", "          then: LGTM", "          "),
            ("rule element", "        next: review", "        "),
            ("switch rule", "            NEEDS_FIX: main", "          "),
            ("loop_guard rule", "          reset_on: main", "          "),
            ("schema", "    type: object", "    "),
        ] {
            let source = known.replace(anchor, &format!("{anchor}\n{indent}future_field: ignored"));
            assert_ne!(source, known, "{label} anchor is missing");
            assert!(
                serde_saphyr::from_str::<serde_json::Value>(&source).is_ok(),
                "{label} source must stay syntactically valid YAML"
            );
            let error = serde_saphyr::from_str::<WorkflowDefinitionYaml>(&source).unwrap_err();
            assert!(
                error.to_string().contains("future_field"),
                "unknown field at {label} must be rejected: {error}"
            );
        }

        assert!(serde_saphyr::from_str::<CommandSpec>("command: cargo build").is_ok());
        assert!(serde_saphyr::from_str::<CommandSpec>(
            "command: cargo build\nfuture_field: ignored\n"
        )
        .is_err());
    }
}
