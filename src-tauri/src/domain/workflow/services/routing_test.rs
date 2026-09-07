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
