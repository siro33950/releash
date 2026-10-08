use super::*;

#[test]
fn test_review_scanの辺_正本サンプルの統合mapで既存の遷移先を維持する() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../workflows/examples/full-cycle-development.yml"
    )))
    .unwrap();
    let sequence = workflow.node_by_name("review").unwrap().sequence().unwrap();

    // When / Then
    assert!(validate_rules(&workflow).is_empty());
    for (has_open_threads, target) in [(true, "fix_round"), (false, "implementation_confirmation")]
    {
        let decision = route_in_scope(
            &workflow,
            sequence,
            "review_scan",
            Some(&serde_json::json!({
                "check_full_review_threads": {"ok": true, "has_open_threads": has_open_threads}
            })),
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(decision, RouteDecision::TransitionTo(target.to_string()));
    }
}

#[test]
fn test_参照schema集約_辺の既存診断messageを維持する() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(
        r#"
name: routing-message
description: test
schemas: {text: string}
nodes:
  main: {command: check, artifact: text}
  missing: {session: {provider: codex}, artifact: unknown}
  silent: {session: {provider: codex}}
"#,
    )
    .unwrap();

    // When / Then
    for (name, expected) in [
        ("main", "command Artifact Contract is not an object"),
        (
            "missing",
            "artifact Contract 'unknown' is not declared in schemas",
        ),
        (
            "silent",
            "routing field 'passed' requires an artifact Contract on this node",
        ),
    ] {
        assert_eq!(
            validate_routing_field(
                &workflow,
                workflow.node_by_name(name).unwrap(),
                "passed",
                RoutingFieldKind::Boolean
            ),
            Err(expected.to_string())
        );
    }
}

#[test]
fn test_fanoutの辺_名前と添字とsequence経由の値に従って遷移する() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/adaptor/gateway/workflow/fixtures/valid/fanout-map-routing.yml"
    )))
    .unwrap();
    let sequence = workflow.node_by_name("main").unwrap().sequence().unwrap();
    assert!(validate_rules(&workflow).is_empty());
    for (node, artifact, target) in [
        ("fan", serde_json::json!({"a": {"passed": true}}), "indexed"),
        (
            "fan",
            serde_json::json!({"a": {"passed": false}}),
            "finished",
        ),
        ("fan", serde_json::json!({}), "finished"),
        (
            "indexed",
            serde_json::json!({"0": {"verdict": "READY"}}),
            "classifier",
        ),
        (
            "indexed",
            serde_json::json!({"0": {"verdict": "HOLD"}}),
            "finished",
        ),
        (
            "classifier",
            serde_json::json!({"classify": {"verdict": "READY"}}),
            "seq",
        ),
        (
            "classifier",
            serde_json::json!({"classify": {"verdict": "HOLD"}}),
            "finished",
        ),
        (
            "seq",
            serde_json::json!({"nested_fan": {"nested_a": {"passed": true}}}),
            "ready",
        ),
        (
            "seq",
            serde_json::json!({"nested_fan": {"nested_a": {"passed": false}}}),
            "finished",
        ),
        ("seq", serde_json::json!({"nested_fan": {}}), "finished"),
    ] {
        // When
        let decision =
            route_in_scope(&workflow, sequence, node, Some(&artifact), &HashMap::new()).unwrap();

        // Then
        assert_eq!(
            decision,
            RouteDecision::TransitionTo(target.to_string()),
            "{node}: {artifact}"
        );
    }
}

#[test]
fn test_fanoutの辺_switchのslot欠番は網羅ならエラーで非網羅ならnextへ遷移する() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/adaptor/gateway/workflow/fixtures/valid/fanout-map-routing.yml"
    )))
    .unwrap();
    let sequence = workflow.node_by_name("main").unwrap().sequence().unwrap();
    let artifact = serde_json::json!({});
    assert!(validate_rules(&workflow).is_empty());
    for node in ["indexed", "classifier"] {
        // When
        let decision = route_in_scope(&workflow, sequence, node, Some(&artifact), &HashMap::new());

        // Then
        assert_eq!(
            decision,
            Err(ScopeRoutingError::Rule(WorkflowError::validation(format!(
                "No matching switch case for node '{node}' and no next catch-all"
            )))),
            "{node}"
        );
    }

    // When
    let decision = route_in_scope(
        &workflow,
        sequence,
        "partial_classifier",
        Some(&artifact),
        &HashMap::new(),
    );

    // Then
    assert_eq!(
        decision,
        Ok(RouteDecision::TransitionTo("finished".to_string()))
    );
}

#[test]
fn test_fanoutの辺_未知段と非objectとmap終端の診断messageを維持する() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/adaptor/gateway/workflow/fixtures/valid/fanout-map-routing.yml"
    )))
    .unwrap();
    for (field, reason) in [
        ("nested_fan.missing", "routing field 'nested_fan.missing' has undeclared segment 2 ('missing')"),
        ("nested_fan.nested_a.passed.field", "routing field 'nested_fan.nested_a.passed.field' cannot resolve segment 4 ('field') from a non-object value"),
        ("nested_fan", "routing field 'nested_fan' must be boolean or string enum"),
    ] {
        // When
        let result = validate_routing_field(&workflow, workflow.node_by_name("seq").unwrap(), field, RoutingFieldKind::Boolean);

        // Then
        assert_eq!(result, Err(reason.to_string()));
    }
}

#[test]
fn test_正本サンプルのfanout集約後の辺_整合確認の判断で既存の遷移先を維持する() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../workflows/examples/full-cycle-development.yml"
    )))
    .unwrap();
    let sequence = workflow
        .node_by_name("implementation")
        .unwrap()
        .sequence()
        .unwrap();
    for (complete, target) in [(true, "implementation_report"), (false, "fix_integration")] {
        // When
        let decision = route_in_scope(
            &workflow,
            sequence,
            "check_integration",
            Some(&serde_json::json!({"complete": complete})),
            &HashMap::new(),
        )
        .unwrap();

        // Then
        assert_eq!(decision, RouteDecision::TransitionTo(target.to_string()));
    }
}

#[test]
fn test_述語の辺_真理値と欠損と非boolean値から遷移を決める() {
    use crate::domain::workflow::Predicate::{And, Or, Ref};
    // Given
    let reference = |name: &str| Ref(name.to_string());
    for passed in [
        None,
        Some(Value::Bool(false)),
        Some(Value::Bool(true)),
        Some(Value::String("true".into())),
    ] {
        for clean in [false, true] {
            for skipped in [false, true] {
                let mut artifact = serde_json::json!({"clean": clean, "skipped": skipped});
                if let Some(value) = &passed {
                    artifact["passed"] = value.clone();
                }
                let p = passed.as_ref().and_then(Value::as_bool).unwrap_or(false);
                for (on, expected) in [
                    (reference("passed"), p),
                    (
                        And(vec![reference("passed"), reference("clean")]),
                        p && clean,
                    ),
                    (
                        Or(vec![reference("passed"), reference("clean")]),
                        p || clean,
                    ),
                    (
                        And(vec![
                            reference("passed"),
                            Or(vec![reference("clean"), reference("skipped")]),
                        ]),
                        p && (clean || skipped),
                    ),
                ] {
                    let rules = [Rule::When {
                        on,
                        then: "done".into(),
                        next: "fix".into(),
                    }];
                    // When
                    let target = raw_target("judge", &rules, Some(&artifact)).unwrap();
                    // Then
                    assert_eq!(
                        target.as_deref(),
                        Some(if expected { "done" } else { "fix" }),
                        "{artifact}: {rules:?}"
                    );
                    assert_eq!(
                        raw_target("judge", &rules, None).unwrap().as_deref(),
                        Some("fix")
                    );
                }
            }
        }
    }
}

#[test]
fn test_述語の辺_多段objectの途中と末端の欠損は葉単位でfalseになる() {
    // Given
    for artifact in [
        serde_json::json!({}),
        serde_json::json!({"details": {}}),
        serde_json::json!({"details": false}),
        serde_json::json!({"details": {"passed": 1}}),
        serde_json::json!({"details": {"passed": true}}),
    ] {
        for operator in ["and", "or"] {
            let rule: Rule = serde_json::from_value(serde_json::json!({"when": {"on": {operator: ["details.passed", "ok"]}, "then": "done"}, "next": "fix"})).unwrap();
            let mut artifact = artifact.clone();
            artifact["ok"] = Value::Bool(true);
            // When
            let target = raw_target("judge", &[rule], Some(&artifact)).unwrap();
            // Then
            let expected = operator == "or"
                || artifact.pointer("/details/passed").and_then(Value::as_bool) == Some(true);
            assert_eq!(
                target.as_deref(),
                Some(if expected { "done" } else { "fix" })
            );
        }
    }
}

#[test]
fn test_述語の辺_合成子のmapを経由して単一参照と同じ値を読む() {
    // Given
    for (source, field, node, artifact, target, fallback) in [
        (
            include_str!(
                "../../../adaptor/gateway/workflow/fixtures/valid/sequence-merged-references.yml"
            ),
            "check_full_review_threads.has_open_threads",
            "review_scan",
            serde_json::json!({"check_full_review_threads": {"has_open_threads": true}}),
            "consume",
            "finished",
        ),
        (
            include_str!("../../../adaptor/gateway/workflow/fixtures/valid/fanout-map-routing.yml"),
            "a.passed",
            "fan",
            serde_json::json!({"a": {"passed": true}}),
            "indexed",
            "finished",
        ),
        (
            include_str!("../../../adaptor/gateway/workflow/fixtures/valid/fanout-map-routing.yml"),
            "nested_fan.nested_a.passed",
            "seq",
            serde_json::json!({"nested_fan": {"nested_a": {"passed": true}}}),
            "ready",
            "finished",
        ),
    ] {
        let source = source.replace(
            &format!("on: {field}"),
            &format!("on: {{and: [{field}, {{or: [{field}]}}]}}"),
        );
        let workflow: WorkflowDefinition = serde_saphyr::from_str(&source).unwrap();
        let sequence = workflow.entry_node().unwrap().sequence().unwrap();
        // When / Then
        assert!(validate_rules(&workflow).is_empty());
        for (artifact, expected) in [(Some(&artifact), target), (None, fallback)] {
            assert_eq!(
                route_in_scope(&workflow, sequence, node, artifact, &HashMap::new()).unwrap(),
                RouteDecision::TransitionTo(expected.to_string())
            );
        }
    }
}
pub(crate) mod routing_tests {
    use super::super::*;
    use crate::domain::workflow::value_objects::{
        CommandSpec, FanoutSpec, InputSourceRef, NodeKind, SequenceSpec,
    };

    fn command_node(name: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: "echo hi".to_string(),
                env: Default::default(),
            }),
            artifact: None,
            input: Vec::new(),
            completion: Default::default(),
            worktree: None,
        }
    }

    fn sequence_node(name: &str, children: Vec<ChildEntry>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Sequence(SequenceSpec {
                entry: None,
                children,
            }),
            artifact: None,
            input: Vec::new(),
            completion: Default::default(),
            worktree: None,
        }
    }

    fn fanout_node(name: &str, children: Vec<ChildEntry>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                children,
                items: None,
            }),
            artifact: None,
            input: Vec::new(),
            completion: Default::default(),
            worktree: None,
        }
    }

    fn entry_with_rules(name: &str, rules: Vec<Rule>) -> ChildEntry {
        ChildEntry {
            name: name.to_string(),
            inputs: Vec::new(),
            rules: Some(rules),
        }
    }

    fn workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            nodes,
            ..Default::default()
        }
    }

    fn node_index(workflow: &WorkflowDefinition, name: &str) -> usize {
        workflow
            .nodes
            .iter()
            .position(|node| node.name == name)
            .expect("node exists")
    }

    fn route_from(
        workflow: &WorkflowDefinition,
        name: &str,
        artifact: Option<&Value>,
    ) -> RouteDecision {
        route(
            workflow,
            node_index(workflow, name),
            artifact,
            &HashMap::new(),
        )
        .expect("route succeeds")
    }

    fn nested_schema(field: &str, schema: SchemaDef, required: bool) -> SchemaDef {
        SchemaDef::Object {
            properties: BTreeMap::from([(
                "outer".to_string(),
                SchemaDef::Object {
                    properties: BTreeMap::from([(field.to_string(), schema)]),
                    required: if required {
                        BTreeSet::from([field.to_string()])
                    } else {
                        BTreeSet::new()
                    },
                },
            )]),
            required: BTreeSet::new(),
        }
    }

    #[test]
    fn test_when_空白入り1段propertyをrequired_booleanとして分岐する() {
        // Given
        let mut work = command_node("work");
        work.artifact = Some("result".to_string());
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules(
                        "work",
                        vec![Rule::When {
                            on: crate::domain::workflow::Predicate::Ref("legacy flag".to_string()),
                            then: "yes".to_string(),
                            next: "no".to_string(),
                        }],
                    ),
                    ChildEntry::reference("yes"),
                    ChildEntry::reference("no"),
                ],
            ),
            work,
            command_node("yes"),
            command_node("no"),
        ]);
        wf.schemas.insert(
            "result".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::from([("legacy flag".to_string(), SchemaDef::Boolean)]),
                required: BTreeSet::from(["legacy flag".to_string()]),
            },
        );

        // When
        let errors = validate_rules(&wf);
        let when_true = route_from(&wf, "work", Some(&serde_json::json!({"legacy flag": true})));
        let when_false = route_from(
            &wf,
            "work",
            Some(&serde_json::json!({"legacy flag": false})),
        );

        // Then
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(when_true, RouteDecision::TransitionTo("yes".to_string()));
        assert_eq!(when_false, RouteDecision::TransitionTo("no".to_string()));
    }

    #[test]
    fn test_switch_空白入り1段propertyをrequired_string_enumとして分岐する() {
        // Given
        let mut work = command_node("work");
        work.artifact = Some("result".to_string());
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules(
                        "work",
                        vec![Rule::Switch {
                            on: "legacy kind".to_string(),
                            cases: BTreeMap::from([
                                ("A".to_string(), "a".to_string()),
                                ("B".to_string(), "b".to_string()),
                            ]),
                            next: Some("failed".to_string()),
                        }],
                    ),
                    ChildEntry::reference("a"),
                    ChildEntry::reference("b"),
                    ChildEntry::reference("failed"),
                ],
            ),
            work,
            command_node("a"),
            command_node("b"),
            command_node("failed"),
        ]);
        wf.schemas.insert(
            "result".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::from([(
                    "legacy kind".to_string(),
                    SchemaDef::String {
                        r#enum: Some(vec!["A".to_string(), "B".to_string()]),
                    },
                )]),
                required: BTreeSet::from(["legacy kind".to_string()]),
            },
        );

        // When
        let errors = validate_rules(&wf);
        let decision = route_from(&wf, "work", Some(&serde_json::json!({"legacy kind": "B"})));

        // Then
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(decision, RouteDecision::TransitionTo("b".to_string()));

        // When
        let NodeKind::Sequence(sequence) = &mut wf.nodes[0].kind else {
            unreachable!();
        };
        let Rule::Switch {
            cases,
            next: switch_next,
            ..
        } = sequence.children[0]
            .rules
            .as_mut()
            .unwrap()
            .first_mut()
            .unwrap()
        else {
            unreachable!();
        };
        cases.remove("B");
        *switch_next = None;
        let errors = validate_rules(&wf);

        // Then
        assert!(errors.iter().any(|error| matches!(
            error,
            RoutingValidationError::SwitchMissingCases { missing, .. }
                if missing == &vec!["B".to_string()]
        )));
    }

    #[test]
    fn test_when_空白入り中間段を経由してrequired_booleanを解決する() {
        // Given
        let mut work = command_node("work");
        work.artifact = Some("result".to_string());
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules(
                        "work",
                        vec![Rule::When {
                            on: crate::domain::workflow::Predicate::Ref(
                                "legacy flag.enabled".to_string(),
                            ),
                            then: "yes".to_string(),
                            next: "no".to_string(),
                        }],
                    ),
                    ChildEntry::reference("yes"),
                    ChildEntry::reference("no"),
                ],
            ),
            work,
            command_node("yes"),
            command_node("no"),
        ]);
        wf.schemas.insert(
            "result".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::from([(
                    "legacy flag".to_string(),
                    SchemaDef::Object {
                        properties: BTreeMap::from([("enabled".to_string(), SchemaDef::Boolean)]),
                        required: BTreeSet::from(["enabled".to_string()]),
                    },
                )]),
                required: BTreeSet::new(),
            },
        );

        // When
        let errors = validate_rules(&wf);
        let decision = route_from(
            &wf,
            "work",
            Some(&serde_json::json!({"legacy flag": {"enabled": true}})),
        );

        // Then
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(decision, RouteDecision::TransitionTo("yes".to_string()));
    }

    #[test]
    fn test_述語_on全体の前後空白を段数によらず拒否する() {
        // Given
        let node = command_node("work");
        let wf = workflow(vec![node]);
        let node = wf.node_by_name("work").unwrap();

        for field in [" ok", "ok ", " outer.enabled", "outer.enabled "] {
            // When
            let result = validate_routing_field(&wf, node, field, RoutingFieldKind::Boolean);

            // Then
            assert!(result.is_err(), "{field:?} must be rejected");
        }
    }

    #[test]
    fn test_when多段参照_中間段がoptionalでもrequired_booleanで分岐する() {
        // Given
        let mut work = command_node("work");
        work.artifact = Some("result".to_string());
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules(
                        "work",
                        vec![Rule::When {
                            on: crate::domain::workflow::Predicate::Ref("outer.flag".to_string()),
                            then: "yes".to_string(),
                            next: "no".to_string(),
                        }],
                    ),
                    ChildEntry::reference("yes"),
                    ChildEntry::reference("no"),
                ],
            ),
            work,
            command_node("yes"),
            command_node("no"),
        ]);
        wf.schemas.insert(
            "result".to_string(),
            nested_schema("flag", SchemaDef::Boolean, true),
        );

        // When
        let errors = validate_rules(&wf);
        let when_true = route_from(
            &wf,
            "work",
            Some(&serde_json::json!({"outer": {"flag": true}})),
        );
        let when_false = route_from(
            &wf,
            "work",
            Some(&serde_json::json!({"outer": {"flag": false}})),
        );

        // Then
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(when_true, RouteDecision::TransitionTo("yes".to_string()));
        assert_eq!(when_false, RouteDecision::TransitionTo("no".to_string()));
    }

    #[test]
    fn test_switch多段参照_required_string_enumで分岐し網羅性を検査する() {
        // Given
        let mut work = command_node("work");
        work.artifact = Some("result".to_string());
        let cases = BTreeMap::from([
            ("A".to_string(), "a".to_string()),
            ("B".to_string(), "b".to_string()),
        ]);
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules(
                        "work",
                        vec![Rule::Switch {
                            on: "outer.verdict".to_string(),
                            cases,
                            next: Some("failed".to_string()),
                        }],
                    ),
                    ChildEntry::reference("a"),
                    ChildEntry::reference("b"),
                    ChildEntry::reference("failed"),
                ],
            ),
            work,
            command_node("a"),
            command_node("b"),
            command_node("failed"),
        ]);
        wf.schemas.insert(
            "result".to_string(),
            nested_schema(
                "verdict",
                SchemaDef::String {
                    r#enum: Some(vec!["A".to_string(), "B".to_string()]),
                },
                true,
            ),
        );

        // When
        let errors = validate_rules(&wf);
        let decision = route_from(
            &wf,
            "work",
            Some(&serde_json::json!({"outer": {"verdict": "B"}})),
        );

        // Then
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(decision, RouteDecision::TransitionTo("b".to_string()));

        // When
        let NodeKind::Sequence(sequence) = &mut wf.nodes[0].kind else {
            unreachable!();
        };
        let Rule::Switch {
            cases,
            next: switch_next,
            ..
        } = sequence.children[0]
            .rules
            .as_mut()
            .unwrap()
            .first_mut()
            .unwrap()
        else {
            unreachable!();
        };
        cases.remove("B");
        *switch_next = None;
        let errors = validate_rules(&wf);

        // Then
        assert!(errors.iter().any(|error| matches!(
            error,
            RoutingValidationError::SwitchMissingCases { missing, .. }
                if missing == &vec!["B".to_string()]
        )));
    }

    #[test]
    fn test_多段述語_存在しない段と非object中間段と末端型不一致を拒否する() {
        // Given
        let mut node = command_node("work");
        node.artifact = Some("result".to_string());
        let mut wf = workflow(vec![node]);
        wf.schemas.insert(
            "result".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::from([
                    ("scalar".to_string(), SchemaDef::String { r#enum: None }),
                    (
                        "outer".to_string(),
                        SchemaDef::Object {
                            properties: BTreeMap::from([
                                ("text".to_string(), SchemaDef::String { r#enum: None }),
                                ("optional".to_string(), SchemaDef::Boolean),
                                (
                                    "empty".to_string(),
                                    SchemaDef::String {
                                        r#enum: Some(Vec::new()),
                                    },
                                ),
                            ]),
                            required: BTreeSet::from(["text".to_string(), "empty".to_string()]),
                        },
                    ),
                ]),
                required: BTreeSet::new(),
            },
        );
        let node = wf.node_by_name("work").unwrap();

        // When / Then
        assert!(
            validate_routing_field(&wf, node, "outer.missing", RoutingFieldKind::Boolean).is_err()
        );
        assert!(
            validate_routing_field(&wf, node, "scalar.leaf", RoutingFieldKind::Boolean).is_err()
        );
        assert!(
            validate_routing_field(&wf, node, "outer.text", RoutingFieldKind::Boolean).is_err()
        );
        assert!(
            validate_routing_field(&wf, node, "outer.optional", RoutingFieldKind::Boolean).is_err()
        );
        assert!(validate_routing_field(&wf, node, "outer.empty", RoutingFieldKind::Enum).is_err());
    }

    #[test]
    fn test_述語_1段のcommand_okとartifact_fieldの従来挙動を維持する() {
        // Given
        let command = command_node("command");
        let mut session = command_node("session");
        session.artifact = Some("result".to_string());
        let mut wf = workflow(vec![command, session]);
        wf.schemas.insert(
            "result".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::from([("flag".to_string(), SchemaDef::Boolean)]),
                required: BTreeSet::from(["flag".to_string()]),
            },
        );

        // When / Then
        assert!(validate_routing_field(
            &wf,
            wf.node_by_name("command").unwrap(),
            "ok",
            RoutingFieldKind::Boolean,
        )
        .is_ok());
        assert!(validate_routing_field(
            &wf,
            wf.node_by_name("session").unwrap(),
            "flag",
            RoutingFieldKind::Boolean,
        )
        .is_ok());
    }

    #[test]
    fn test_隣接辺_rules無しエントリはリストの次へ進み末尾で完了する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("first"),
                    ChildEntry::reference("second"),
                ],
            ),
            command_node("first"),
            command_node("second"),
        ]);

        assert_eq!(
            route_from(&wf, "first", None),
            RouteDecision::TransitionTo("second".to_string())
        );
        assert_eq!(route_from(&wf, "second", None), RouteDecision::Completed);
    }

    #[test]
    fn test_明示終端_空rulesは隣接辺を持たず完了する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules("first", Vec::new()),
                    ChildEntry::reference("second"),
                ],
            ),
            command_node("first"),
            command_node("second"),
        ]);

        assert_eq!(route_from(&wf, "first", None), RouteDecision::Completed);
    }

    #[test]
    fn test_children外ターゲット_遷移後は出る辺が無く完了する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![entry_with_rules(
                    "first",
                    vec![Rule::Next("target_only".to_string())],
                )],
            ),
            command_node("first"),
            command_node("target_only"),
        ]);

        assert_eq!(
            route_from(&wf, "first", None),
            RouteDecision::TransitionTo("target_only".to_string())
        );
        assert_eq!(
            route_from(&wf, "target_only", None),
            RouteDecision::Completed
        );
    }

    #[test]
    fn test_単独実行_rootがleafなら完了する() {
        let wf = workflow(vec![command_node("main")]);
        assert_eq!(route_from(&wf, "main", None), RouteDecision::Completed);
    }

    #[test]
    fn test_loop_guard_上限到達でon_exhaustedへ迂回する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules("work", vec![Rule::Next("retry".to_string())]),
                    entry_with_rules(
                        "retry",
                        vec![
                            Rule::LoopGuard {
                                max_iterations: 2,
                                on_exhausted: "done".to_string(),
                            },
                            Rule::Next("work".to_string()),
                        ],
                    ),
                    ChildEntry::reference("done"),
                ],
            ),
            command_node("work"),
            command_node("retry"),
            command_node("done"),
        ]);

        let mut counts = HashMap::new();
        counts.insert("retry".to_string(), 1u32);
        let decision = route(&wf, node_index(&wf, "work"), None, &counts).unwrap();
        assert_eq!(decision, RouteDecision::TransitionTo("retry".to_string()));

        counts.insert("retry".to_string(), 2u32);
        let decision = route(&wf, node_index(&wf, "work"), None, &counts).unwrap();
        assert_eq!(decision, RouteDecision::TransitionTo("done".to_string()));
    }

    #[test]
    fn test_検証_同一合成子への重複子参照を拒否する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("first"),
                    ChildEntry::reference("first"),
                ],
            ),
            command_node("first"),
        ]);

        let errors = validate_rules(&wf);
        assert!(errors.iter().any(|error| matches!(
            error,
            RoutingValidationError::DuplicateChildReference { child, .. } if child == "first"
        )));
        assert!(
            !errors.iter().any(|error| matches!(
                error,
                RoutingValidationError::ChildReferenceViolation { .. }
            )),
            "同一合成子内の重複は DuplicateChildReference のみで報告する: {errors:?}"
        );
    }

    #[test]
    fn test_検証_未知の子参照を拒否する() {
        let wf = workflow(vec![sequence_node(
            "main",
            vec![ChildEntry::reference("ghost")],
        )]);

        assert!(validate_rules(&wf).iter().any(|error| matches!(
            error,
            RoutingValidationError::UnknownChildReference { child, .. } if child == "ghost"
        )));
    }

    #[test]
    fn test_検証_fanout子エントリのrulesを拒否する() {
        let wf = workflow(vec![
            sequence_node("main", vec![ChildEntry::reference("fan")]),
            fanout_node(
                "fan",
                vec![entry_with_rules(
                    "worker",
                    vec![Rule::Next("worker".to_string())],
                )],
            ),
            command_node("worker"),
        ]);

        assert!(validate_rules(&wf).iter().any(|error| matches!(
            error,
            RoutingValidationError::RulesOnFanoutChildEntry { child, .. } if child == "worker"
        )));
    }

    #[test]
    fn test_検証_他合成子の子への遷移ターゲットを拒否する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules("first", vec![Rule::Next("worker".to_string())]),
                    ChildEntry::reference("fan"),
                ],
            ),
            command_node("first"),
            fanout_node("fan", vec![ChildEntry::reference("worker")]),
            command_node("worker"),
        ]);

        assert!(validate_rules(&wf).iter().any(|error| matches!(
            error,
            RoutingValidationError::ChildReferenceViolation { child, .. } if child == "worker"
        )));
    }

    #[test]
    fn test_検証_隣接辺を含む閉路にloop_guardが無ければ拒否する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("first"),
                    entry_with_rules("second", vec![Rule::Next("first".to_string())]),
                ],
            ),
            command_node("first"),
            command_node("second"),
        ]);

        assert!(validate_rules(&wf)
            .iter()
            .any(|error| matches!(error, RoutingValidationError::CycleWithoutLoopGuard { .. })));
    }

    #[test]
    fn test_検証_entryがchildren外ならエラー() {
        let mut seq = SequenceSpec {
            entry: Some("outside".to_string()),
            children: vec![ChildEntry::reference("first")],
        };
        let wf = workflow(vec![
            NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Sequence(std::mem::take(&mut seq)),
                artifact: None,
                input: Vec::new(),
                completion: Default::default(),
                worktree: None,
            },
            command_node("first"),
            command_node("outside"),
        ]);

        assert!(validate_rules(&wf).iter().any(|error| matches!(
            error,
            RoutingValidationError::SequenceEntryNotChild { entry, .. } if entry == "outside"
        )));
    }

    #[test]
    fn test_到達可能性_スコープ内で辿れない子とカタログ未参照nodeを検出する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules("first", Vec::new()),
                    ChildEntry::reference("orphan_child"),
                ],
            ),
            command_node("first"),
            command_node("orphan_child"),
            command_node("unreferenced"),
        ]);

        let unreachable: Vec<_> = validate_reachability(&wf)
            .into_iter()
            .map(|error| match error {
                RoutingValidationError::UnreachableNode { node } => node,
                other => panic!("unexpected error: {other:?}"),
            })
            .collect();
        assert!(unreachable.contains(&"orphan_child".to_string()));
        assert!(unreachable.contains(&"unreferenced".to_string()));
        assert!(!unreachable.contains(&"first".to_string()));
    }

    #[test]
    fn test_パラメータ配線_供給元参照のroot分解() {
        let source = InputSourceRef::new("collect_inputs.spec_dir");
        assert_eq!(source.root(), "collect_inputs");
        assert_eq!(
            source.raw().split_once('.').map(|(_, field)| field),
            Some("spec_dir")
        );
    }
}
