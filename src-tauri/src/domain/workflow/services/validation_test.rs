use super::*;

#[test]
fn test_参照schema集約_配線の既存診断messageを維持する() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(
        r#"
name: wiring-message
description: test
schemas: {text: string}
nodes:
  main: {command: check, artifact: text}
  source: {session: {provider: codex}, artifact: unknown}
"#,
    )
    .unwrap();
    let path = FieldPath::from_dotted("passed").unwrap();

    // When / Then
    for (name, expected) in [
        (
            "main",
            "source node 'main' Artifact Contract is not an object",
        ),
        (
            "source",
            "source node 'source' Artifact has no field path 'passed'",
        ),
    ] {
        assert_eq!(
            validate_node_source_field_path(&workflow, workflow.node_by_name(name).unwrap(), &path),
            Err(expected.to_string())
        );
    }
}

#[test]
fn test_参照検証_非object契約と合成子経由の配線の原因段を両方収集する() {
    // Given
    let mut workflow: WorkflowDefinition = serde_saphyr::from_str(
        r#"
name: nonobject-leaf
description: test
schemas: {text: string}
nodes:
  outer:
    sequence:
      children:
        - main
        - consume:
            inputs: {value: main.part.bad.ok}
  main: {sequence: {children: [part]}}
  part: {sequence: {children: [bad]}}
  bad: {command: check, artifact: text}
  consume: {command: consume, input: [value]}
"#,
    )
    .unwrap();
    workflow.entry = "outer".to_string();

    // When
    let errors = validate_all(&workflow);

    // Then
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::InvalidArtifactSchema { node, contract }
            if node == "bad" && contract == "text"
    )));
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::InvalidInputWiring(violation)
            if violation.child == "consume"
                && violation.source == "main.part.bad.ok"
                && violation.kind == InputWiringKind::UnknownSourceField
                && violation.reason == "source node 'main' Artifact cannot resolve segment 2 ('bad') from a non-object value"
    )), "{errors:?}");
}

#[test]
fn test_fanout参照検証_配線とitemsの失敗位置とmap終端のmessageを維持する() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/adaptor/gateway/workflow/fixtures/valid/fanout-map-references.yml"
    )))
    .unwrap();
    for (path, suffix) in [
        (
            "nested_fan.unknown.passed",
            "does not declare segment 2 ('unknown')",
        ),
        (
            "nested_fan.nested_a.unknown",
            "does not declare segment 3 ('unknown')",
        ),
        (
            "nested_fan.nested_a.passed.field",
            "cannot resolve segment 4 ('field') from a non-object value",
        ),
    ] {
        let field_path = FieldPath::from_dotted(path).unwrap();

        // When / Then
        assert_eq!(
            validate_node_source_field_path(
                &workflow,
                workflow.node_by_name("seq").unwrap(),
                &field_path
            ),
            Err(format!("source node 'seq' Artifact {suffix}"))
        );
        assert_eq!(
            reference::artifact_field_schema(&workflow, "seq", &field_path),
            Err(format!("node 'seq' Artifact {suffix}"))
        );
    }
    let mut workflow = workflow;
    workflow
        .nodes
        .iter_mut()
        .find(|node| node.name == "expand")
        .unwrap()
        .kind = NodeKind::Fanout(crate::domain::workflow::FanoutSpec {
        children: vec![crate::domain::workflow::ChildEntry::reference("worker")],
        items: Some(ItemsSource::ArtifactField {
            node: "seq".to_string(),
            field_path: FieldPath::new(["nested_fan"]),
        }),
    });

    // When
    let errors = validate_all(&workflow);

    // Then
    assert!(errors.iter().any(|error| matches!(error, ValidationError::InvalidFanoutItemsReference { reason, .. } if reason == "items reference must resolve to an array field")));
}

#[test]
fn test_fanout参照検証_items供給元の直接child参照と既存エラーを維持する() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(
        r#"
name: items-errors
description: test
schemas: {text: string}
nodes:
  main: {sequence: {children: [fan, invalid, silent, missing]}}
  fan: {fanout: {children: [a]}}
  a: {command: collect}
  invalid: {command: collect, artifact: text}
  silent: {session: {provider: codex}}
  missing: {session: {provider: codex}, artifact: unknown}
"#,
    )
    .unwrap();
    for (name, expected) in [
        (
            "a",
            "node 'a' is a fanout child and its Artifact is not referenceable",
        ),
        (
            "invalid",
            "node 'invalid' Artifact Contract is not an object",
        ),
        ("silent", "node 'silent' does not produce an Artifact"),
        (
            "missing",
            "node 'missing' has no Artifact field path 'tasks'",
        ),
        ("unknown", "unknown Artifact-producing node 'unknown'"),
    ] {
        // When / Then
        assert_eq!(
            reference::artifact_field_schema(&workflow, name, &FieldPath::new(["tasks"])),
            Err(expected.to_string())
        );
    }
}

#[test]
fn test_delegate値域_直接構築された定義でもゼロを拒否し正の回数を受理する() {
    use crate::domain::workflow::{
        CommandSpec, NodeCompletion, Predicate, SessionDelegate, SessionSpec,
    };
    // Given
    let mut workflow = WorkflowDefinition {
        name: "delegate-limit".into(),
        entry: "main".into(),
        nodes: vec![
            NodeDefinition {
                name: "main".into(),
                kind: NodeKind::Session(SessionSpec::default()),
                artifact: Some("result".into()),
                completion: NodeCompletion {
                    require: None,
                    delegate: Some(SessionDelegate {
                        child: "verify".into(),
                        inputs: Vec::new(),
                        when: Predicate::Ref("child.ok".into()),
                        max_iterations: 0,
                    }),
                },
                ..Default::default()
            },
            NodeDefinition {
                name: "verify".into(),
                kind: NodeKind::Command(CommandSpec {
                    command: "check".into(),
                    env: Default::default(),
                }),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    for count in [0, 1, u32::MAX] {
        workflow.nodes[0]
            .completion
            .delegate
            .as_mut()
            .unwrap()
            .max_iterations = count;
        // When
        let errors = collect_delegate_errors(&workflow);
        // Then
        assert_eq!(
            errors.iter().any(|error| matches!(
                error,
                ValidationError::InvalidDelegate {
                    kind: InvalidDelegateKind::MaxIterations,
                    ..
                }
            )),
            count == 0
        );
    }
}

#[test]
fn test_delegate検査kind_宣言fieldとmessageの位置を同じ対応から取得する() {
    for (kind, field) in [
        (
            InvalidDelegateKind::UnsupportedNodeKind,
            "completion.delegate",
        ),
        (
            InvalidDelegateKind::MissingArtifactContract,
            "completion.delegate",
        ),
        (
            InvalidDelegateKind::ChildWithoutArtifact,
            "completion.delegate.child",
        ),
        (
            InvalidDelegateKind::MaxIterations,
            "completion.delegate.max_iterations",
        ),
        (
            InvalidDelegateKind::WhenFieldNotBoolean,
            "completion.delegate.when",
        ),
    ] {
        // Given
        let error = ValidationError::InvalidDelegate {
            node: "worker".into(),
            kind,
            reason: "invalid".into(),
        };
        // When / Then
        assert_eq!(kind.field_path(), field);
        assert_eq!(error.to_string(), format!("node 'worker' {field}: invalid"));
    }
}
