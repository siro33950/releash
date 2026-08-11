use super::schema::*;
use crate::domain::workflow::services::contract_schema;
use serde_json::Value;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_session_providerを必須とし旧modelとpermissionを拒否する() {
        let with_provider = r#"
name: provider-session
description: Provider session
nodes:
  - name: implement
    session:
      provider: codex
      gate: auto
"#;
        let missing_provider = r#"
name: missing-provider
description: Missing Provider
nodes:
  - name: implement
    session:
      gate: auto
"#;
        let legacy_settings = r#"
name: legacy-settings
description: Legacy settings
nodes:
  - name: implement
    session:
      provider: claude
      model: legacy-model
      permission: edit
      gate: auto
"#;

        assert!(serde_saphyr::from_str::<WorkflowDefinitionYaml>(with_provider).is_ok());
        assert!(serde_saphyr::from_str::<WorkflowDefinitionYaml>(missing_provider).is_err());
        assert!(serde_saphyr::from_str::<WorkflowDefinitionYaml>(legacy_settings).is_err());
    }

    #[test]
    fn parse_session_node() {
        let yaml = r#"
name: session-only
description: 単一セッション
nodes:
  - name: implement
    session:
      provider: claude
      gate: auto
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
        assert_eq!(session.gate, SessionGate::Auto);
    }

    #[test]
    fn session_knowledge_accepts_scalar_and_sequence_and_preserves_order() {
        let yaml = r#"
name: knowledge-shapes
description: knowledge scalar and sequence
nodes:
  - name: scalar
    session:
      provider: claude
      gate: auto
      facets:
        knowledge: releash-thread-cli
  - name: sequence
    session:
      provider: claude
      gate: auto
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
                    name: "scalar".to_string(),
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
                    name: "sequence".to_string(),
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
            value["nodes"][0]["session"]["facets"]["knowledge"],
            Value::String("releash-thread-cli".to_string())
        );
        assert_eq!(
            value["nodes"][1]["session"]["facets"]["knowledge"],
            serde_json::json!(["releash-thread-cli", "requirements-design"])
        );
    }

    #[test]
    fn session_knowledge_rejects_non_string_sequence_elements() {
        let yaml = r#"
name: invalid-knowledge
description: invalid knowledge element
nodes:
  - name: review
    session:
      provider: claude
      gate: auto
      facets:
        knowledge:
          - releash-thread-cli
          - 42
"#;

        assert!(serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).is_err());
    }

    #[test]
    fn parse_approval_gate_session_node() {
        let yaml = r#"
name: approval-only
description: 承認セッション
nodes:
  - name: approve
    session:
      provider: claude
      gate: approval
      facets:
        instruction: approve
        policy: planning
"#;
        let wf: WorkflowDefinitionYaml = serde_saphyr::from_str(yaml).unwrap();
        let node = &wf.nodes[0];
        assert!(node.is_approval_session());
        assert_eq!(
            node.session().unwrap().facets.instruction.as_deref(),
            Some("approve")
        );
    }

    #[test]
    fn parse_session_node_requires_gate() {
        let yaml = r#"
name: missing-gate
description: gate is required
nodes:
  - name: implement
    session:
      provider: claude
"#;

        let error = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();

        assert!(error.to_string().contains("missing field `gate`"));
    }

    #[test]
    fn parse_command_node() {
        let yaml = r#"
name: command-only
description: command node
nodes:
  - name: build
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
  - name: review
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
            serialized_value["nodes"][0]["fanout"]["child"],
            serde_json::json!(["arch-review", "security-review"])
        );
        assert_eq!(
            serialized_value["nodes"][0]["fanout"]["items"],
            Value::String("plan.targets".to_string())
        );
    }

    #[test]
    fn parse_fanout_single_child_and_literal_items() {
        let yaml = r#"
name: fanout
description: fanout test
nodes:
  - name: review
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
            serialized_value["nodes"][0]["fanout"]["child"],
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
  - name: review
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
  - name: judge
    session:
      provider: claude
      gate: auto
    rules:
      - when: { on: ok, then: done }
        next: fix
      - loop_guard: { max_iterations: 3, on_exhausted: give_up, reset_on: judge }
  - name: triage
    session:
      provider: claude
      gate: auto
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
            } if on_exhausted == "give_up" && reset_on == "judge"
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
    fn rejects_legacy_rule_match_key() {
        let yaml = r#"
name: legacy-rule
description: invalid
nodes:
  - name: review
    session:
      provider: claude
      gate: auto
    rules:
      - match: NEEDS_FIX
        next: fix
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("match"));
    }

    #[test]
    fn rejects_removed_reject_and_rerun_rules() {
        for removed_action in ["reject", "rerun"] {
            let yaml = format!(
                r#"
name: removed-rule
description: invalid
nodes:
  - name: review
    session:
      provider: claude
      gate: approval
    rules:
      - match: {removed_action}
        next: fix
"#
            );

            let error = serde_saphyr::from_str::<WorkflowDefinitionYaml>(&yaml).unwrap_err();
            assert!(error.to_string().contains("match"));
        }
    }

    #[test]
    fn rejects_rule_with_multiple_discriminators() {
        let yaml = r#"
name: invalid-rule
description: invalid
nodes:
  - name: review
    session:
      provider: claude
      gate: auto
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
  - name: review
    session:
      provider: claude
      gate: auto
    rules:
      - when: { on: ok, then: done }
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("when rule requires sibling next"));
    }

    #[test]
    fn rejects_node_direct_cycle_guard_and_resets_cycle_for() {
        let yaml = r#"
name: legacy-guards
description: invalid
nodes:
  - name: fix
    session:
      provider: claude
      gate: auto
    cycle_guard:
      max_iterations: 2
    resets_cycle_for:
      - fix
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(
            err.to_string().contains("cycle_guard") || err.to_string().contains("resets_cycle_for")
        );
    }

    #[test]
    fn rejects_missing_kind_block() {
        let yaml = r#"
name: invalid
description: invalid
nodes:
  - name: missing
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
  - name: duplicate
    command: "echo hi"
    session:
      provider: claude
      gate: auto
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("exactly one kind block"));
    }

    #[test]
    fn rejects_legacy_type_field() {
        let yaml = r#"
name: old-type
description: invalid
nodes:
  - name: implement
    type: agent
    instruction: implement
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("type"));
    }

    #[test]
    fn rejects_legacy_output_contract_field() {
        let yaml = r#"
name: old-output-contract
description: invalid
nodes:
  - name: review
    session:
      provider: claude
      gate: auto
    output_contract: review-verdict
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("output_contract"));
    }

    #[test]
    fn rejects_legacy_input_contracts_field() {
        let yaml = r#"
name: old-input-contracts
description: invalid
nodes:
  - name: implement
    session:
      provider: claude
      gate: auto
    input_contracts:
      - spec-directory
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("input_contracts"));
    }

    #[test]
    fn schemas_accept_scalar_string_contract() {
        let yaml = r#"
name: scalar-schema
description: valid
schemas:
  request_text: string
nodes:
  - name: review
    session:
      provider: claude
      gate: auto
    input: request_text
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
    fn schemas_reject_array_extra_keywords_with_allowed_field_message() {
        let err = serde_json::from_value::<SchemaDef>(serde_json::json!({
            "type": "array",
            "items": "review-item",
            "required": []
        }))
        .unwrap_err();

        assert!(err.to_string().contains("array schema supports only items"));
    }

    #[test]
    fn schemas_reject_retired_additional_properties_keyword() {
        let err = serde_json::from_value::<SchemaDef>(serde_json::json!({
            "type": "object",
            "additionalProperties": false
        }))
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("object schema supports only properties and required"));
    }

    #[test]
    fn schemas_reject_subset_outside_keywords() {
        let yaml = r#"
name: invalid-schema-keyword
description: invalid
schemas:
  review:
    type: object
    oneOf: []
nodes:
  - name: review
    session:
      provider: claude
      gate: auto
"#;
        assert!(serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).is_err());
    }

    #[test]
    fn rejects_flat_session_facets() {
        let yaml = r#"
name: flat-facet
description: invalid
nodes:
  - name: implement
    session:
      provider: claude
      gate: auto
    instruction: implement
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("instruction"));
    }

    #[test]
    fn rejects_inline_prompt() {
        let yaml = r#"
name: inline-test
description: invalid
nodes:
  - name: quick
    session:
      provider: claude
      gate: auto
    inline_prompt: "Do a quick analysis"
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("inline_prompt"));
    }

    #[test]
    fn rejects_session_block_command_field() {
        let yaml = r#"
name: invalid-session
description: invalid
nodes:
  - name: implement
    session:
      provider: claude
      command: "cargo build"
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("command"));
    }

    #[test]
    fn rejects_command_spec_session_fields() {
        let yaml = r#"
command: "cargo build"
facets:
  instruction: implement
"#;
        let err = serde_saphyr::from_str::<CommandSpec>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("facets"));
    }

    #[test]
    fn rejects_command_node_session_fields() {
        let yaml = r#"
name: invalid-command
description: invalid
nodes:
  - name: build
    command: "cargo build"
    facets:
      instruction: implement
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("facets"));
    }

    #[test]
    fn rejects_fanout_block_session_fields() {
        let yaml = r#"
name: invalid-fanout
description: invalid
nodes:
  - name: review
    fanout:
      facets:
        instruction: review
      child: []
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("facets"));
    }

    #[test]
    fn rejects_legacy_parallel_children() {
        let yaml = r#"
name: invalid-parallel-children
description: invalid
nodes:
  - name: review
    fanout:
      parallel_children:
        - name: child
"#;
        let err = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("parallel_children"));
    }
}
