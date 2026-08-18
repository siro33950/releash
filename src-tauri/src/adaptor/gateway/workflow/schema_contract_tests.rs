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
    fn 純直列_無名commandエントリは合成内部名でカタログへ正規化され隣接辺で進む() {
        let yaml = r#"
name: pure-serial
description: unnamed command entries
nodes:
  main:
    sequence:
      children:
      - command: "cargo test --workspace"
      - command: "cargo clippy -- -D warnings"
"#;
        let wf: WorkflowDefinitionYaml = serde_saphyr::from_str(yaml).unwrap();

        let names: Vec<&str> = wf.nodes.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(names, vec!["main", "main#0", "main#1"]);
        assert_eq!(
            wf.nodes[0].sequence().unwrap().children,
            vec![
                crate::domain::workflow::ChildEntry::reference("main#0"),
                crate::domain::workflow::ChildEntry::reference("main#1"),
            ]
        );
        assert_eq!(wf.nodes[1].command(), Some("cargo test --workspace"));
        assert!(crate::domain::workflow::validation::validate(&wf).is_ok());

        // 隣接辺: 先頭 → 次 → 終端。
        assert_eq!(
            wf.initial_execution_node().map(|node| node.name.as_str()),
            Some("main#0")
        );
        let route = |name: &str| {
            crate::domain::workflow::services::routing::route(
                &wf,
                wf.nodes.iter().position(|node| node.name == name).unwrap(),
                None,
                &std::collections::HashMap::new(),
            )
            .unwrap()
        };
        assert_eq!(
            route("main#0"),
            crate::domain::workflow::services::routing::RouteDecision::TransitionTo(
                "main#1".to_string()
            )
        );
        assert_eq!(
            route("main#1"),
            crate::domain::workflow::services::routing::RouteDecision::Completed
        );
    }

    #[test]
    fn 無名エントリ入り定義は正規形serializeを経てroundtripする() {
        let yaml = r#"
name: unnamed-roundtrip
description: unnamed entries survive the event wire shape
nodes:
  main:
    sequence:
      children:
      - command: "cargo test --workspace"
      - command: "cargo clippy -- -D warnings"
"#;
        let wf: WorkflowDefinitionYaml = serde_saphyr::from_str(yaml).unwrap();

        // ExecutionStarted と同じ経路: 正規形を serialize → deserialize で同値。
        let json = serde_json::to_value(&wf).unwrap();
        let restored: WorkflowDefinitionYaml = serde_json::from_value(json).unwrap();
        assert_eq!(restored, wf);

        let yaml_normal_form = serde_saphyr::to_string(&wf).unwrap();
        let reparsed: WorkflowDefinitionYaml = serde_saphyr::from_str(&yaml_normal_form).unwrap();
        assert_eq!(reparsed, wf);
    }

    #[test]
    fn インライン宣言はカタログ参照へ正規化され直書きと同じ正規形になる() {
        let inline = r#"
name: inline-sugar
description: inline declaration
nodes:
  main:
    sequence:
      children:
      - quick_check:
          command: "cargo check"
"#;
        let expanded = r#"
name: inline-sugar
description: inline declaration
nodes:
  main:
    sequence:
      children:
      - quick_check
  quick_check:
    command: "cargo check"
"#;
        let inline_wf: WorkflowDefinitionYaml = serde_saphyr::from_str(inline).unwrap();
        let expanded_wf: WorkflowDefinitionYaml = serde_saphyr::from_str(expanded).unwrap();

        assert_eq!(inline_wf, expanded_wf);
        assert_eq!(
            serde_saphyr::to_string(&inline_wf).unwrap(),
            serde_saphyr::to_string(&expanded_wf).unwrap()
        );
    }

    #[test]
    fn インライン宣言の名前がカタログと衝突したら拒否される() {
        let yaml = r#"
name: inline-collision
description: duplicate inline name
nodes:
  main:
    sequence:
      children:
      - quick_check:
          command: "cargo check"
  quick_check:
    command: "cargo build"
"#;
        let error = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap_err();
        assert!(error.to_string().contains("is duplicated"), "{error}");
    }

    #[test]
    fn 再帰カウント_インライン宣言もノード数上限ガードに数える() {
        use crate::domain::workflow::validation;
        use crate::domain::workflow::value_objects::MAX_NODES_PER_WORKFLOW;

        // カタログ直書きは main の1つだけで、残りは全てインライン宣言。
        // 正規化がカタログへ登録するため、総数 = 1 + インライン数で上限を超える。
        let mut yaml = String::from(
            "name: inline-bomb\ndescription: inline nodes exceed the guard\nnodes:\n  main:\n    sequence:\n      children:\n",
        );
        for index in 0..MAX_NODES_PER_WORKFLOW {
            yaml.push_str(&format!(
                "      - inline_{index}:\n          command: \"echo {index}\"\n"
            ));
        }

        let wf: WorkflowDefinitionYaml = serde_saphyr::from_str(&yaml).unwrap();
        assert_eq!(wf.nodes.len(), MAX_NODES_PER_WORKFLOW + 1);
        assert!(validation::validate_all(&wf)
            .iter()
            .any(|error| matches!(error, validation::ValidationError::TooManyNodes { .. })));
    }

    #[test]
    fn parse_fanout_node_with_artifact_items() {
        let yaml = r#"
name: fanout
description: fanout test
nodes:
  main:
    fanout:
      children:
      - arch-review
      - security-review
      items: plan.targets
"#;
        let wf: WorkflowDefinitionYaml = serde_saphyr::from_str(yaml).unwrap();
        let fanout = wf.nodes[0].fanout().unwrap();
        assert_eq!(
            fanout.children,
            vec![
                crate::domain::workflow::ChildEntry::reference("arch-review"),
                crate::domain::workflow::ChildEntry::reference("security-review"),
            ]
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
            serialized_value["nodes"]["main"]["fanout"]["children"],
            serde_json::json!(["arch-review", "security-review"])
        );
        assert_eq!(
            serialized_value["nodes"]["main"]["fanout"]["items"],
            Value::String("plan.targets".to_string())
        );
    }

    #[test]
    fn parse_fanout_children_and_literal_items() {
        let yaml = r#"
name: fanout
description: fanout test
nodes:
  main:
    fanout:
      children:
      - reviewer
      items:
      - api
      - cli
"#;

        let wf: WorkflowDefinitionYaml = serde_saphyr::from_str(yaml).unwrap();
        let fanout = wf.nodes[0].fanout().unwrap();

        assert_eq!(
            fanout.children,
            vec![crate::domain::workflow::ChildEntry::reference("reviewer")]
        );
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
            serialized_value["nodes"]["main"]["fanout"]["children"],
            serde_json::json!(["reviewer"])
        );
    }

    #[test]
    fn rejects_fanout_items_outside_literal_array_or_node_field_reference() {
        for items in ["source", "source.field.nested", "request"] {
            let yaml = format!(
                r#"
name: invalid-items
description: invalid
nodes:
  main:
    fanout:
      children:
      - reviewer
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
    sequence:
      children:
      - check:
          rules:
          - when:
              on: ok
              then: done
            next: fix
          - loop_guard:
              max_iterations: 3
              on_exhausted: give_up
      - triage:
          rules:
          - switch:
              on: verdict
              cases:
                LGTM: done
                NEEDS_FIX: fix
            next: give_up
          - next: done
  check:
    session:
      provider: claude
  triage:
    session:
      provider: claude
"#;
        let wf = serde_saphyr::from_str::<WorkflowDefinitionYaml>(yaml).unwrap();

        let sequence = wf.nodes[0].sequence().unwrap();
        let check_rules = sequence.children[0].rules.as_deref().unwrap();
        let triage_rules = sequence.children[1].rules.as_deref().unwrap();
        assert!(matches!(
            &check_rules[0],
            Rule::When { on, then, next }
                if on == "ok" && then == "done" && next == "fix"
        ));
        assert!(matches!(
            &check_rules[1],
            Rule::LoopGuard {
                max_iterations: 3,
                on_exhausted,
            } if on_exhausted == "give_up"
        ));
        assert!(matches!(
            &triage_rules[0],
            Rule::Switch { on, cases, next }
                if on == "verdict"
                    && cases.get("LGTM").map(String::as_str) == Some("done")
                    && cases.get("NEEDS_FIX").map(String::as_str) == Some("fix")
                    && next.as_deref() == Some("give_up")
        ));
        assert!(matches!(
            &triage_rules[1],
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
    sequence:
      children:
      - work:
          rules:
          - when:
              on: ok
              then: done
            switch:
              on: verdict
              cases:
                LGTM: done
            next: fix
  work:
    session:
      provider: claude
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
    sequence:
      children:
      - work:
          rules:
          - when:
              on: ok
              then: done
  work:
    session:
      provider: claude
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
    sequence:
      children:
      - build:
          rules:
          - when:
              on: verdict
              then: review
            next: review
      - review:
          rules:
          - switch:
              on: verdict
              cases:
                LGTM: dispatch
                NEEDS_FIX: build
            next: dispatch
      - dispatch:
          rules:
          - loop_guard:
              max_iterations: 2
              on_exhausted: review
  build:
    command: cargo build
    artifact: review
  review:
    session:
      provider: claude
      facets:
        policy: coding
        knowledge:
          - workflow
        instruction: review
    artifact: review
  dispatch:
    fanout:
      children:
      - worker
      items:
        - api
        - cli
  worker:
    session:
      provider: codex
    input:
    - item
    completion: approval
"#;
        assert!(serde_saphyr::from_str::<WorkflowDefinitionYaml>(known).is_ok());

        for (label, anchor, indent) in [
            ("root", "description: known values", ""),
            ("node", "    command: cargo build", "    "),
            ("session", "      provider: claude", "      "),
            ("session facets", "        policy: coding", "        "),
            ("fanout", "      items:", "      "),
            ("when rule", "              then: review", "              "),
            ("rule element", "            next: review", "            "),
            (
                "switch rule",
                "                NEEDS_FIX: build",
                "              ",
            ),
            (
                "loop_guard rule",
                "              on_exhausted: review",
                "              ",
            ),
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
