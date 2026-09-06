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
