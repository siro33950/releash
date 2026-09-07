use super::*;

fn composite_workflow() -> WorkflowDefinition {
    serde_saphyr::from_str(
        r#"
name: composite-reference
description: test
schemas:
  result:
    type: object
    properties:
      passed: {type: boolean}
      optional: {type: boolean}
      outer:
        type: object
        properties:
          passed: {type: boolean}
          scalar: string
        required: [passed]
      tasks: {type: array, items: text}
    required: [passed, tasks]
  text: string
nodes:
  main: {sequence: {children: [nested, silent, fan, indexed, command_leaf]}}
  nested: {sequence: {children: [fan]}}
  fan: {fanout: {children: [writer, silent]}}
  indexed: {fanout: {children: [writer, command_leaf], items: []}}
  writer: {session: {provider: codex}, artifact: result}
  silent: {session: {provider: codex}}
  command_leaf: {command: check}
"#,
    )
    .unwrap()
}

#[test]
fn test_node参照解決_合成子の名前キーと添字キーを経由してleaf契約へ届く() {
    // Given
    let workflow = composite_workflow();
    for (name, path) in [
        ("fan", "writer.passed"),
        ("indexed", "0.passed"),
        ("indexed", "2.passed"),
        ("indexed", "1.ok"),
        ("indexed", "99.ok"),
        ("main", "fan.writer.passed"),
        ("main", "nested.fan.writer.outer.passed"),
        ("main", "command_leaf.ok"),
    ] {
        // When
        let resolved = resolve_node_field_path(
            &workflow,
            workflow.node_by_name(name).unwrap(),
            &FieldPath::from_dotted(path).unwrap(),
        );

        // Then
        assert_eq!(
            resolved,
            Ok(ResolvedNodeField::Leaf {
                schema: SchemaDef::Boolean,
                required: true
            }),
            "{name}.{path}"
        );
    }
}

#[test]
fn test_node参照解決_終端がmapかleafかを区別し合成子の段はrequiredにしない() {
    // Given
    let workflow = composite_workflow();
    for path in [
        FieldPath::default(),
        FieldPath::new(["nested"]),
        FieldPath::new(["nested", "fan"]),
    ] {
        // When / Then
        assert_eq!(
            resolve_node_field_path(&workflow, workflow.node_by_name("main").unwrap(), &path),
            Ok(ResolvedNodeField::Map)
        );
    }
    for path in ["writer", "writer.optional"] {
        // When
        let resolved = resolve_node_field_path(
            &workflow,
            workflow.node_by_name("fan").unwrap(),
            &FieldPath::from_dotted(path).unwrap(),
        )
        .unwrap();

        // Then
        assert!(matches!(
            resolved,
            ResolvedNodeField::Leaf {
                required: false,
                ..
            }
        ));
    }
    assert!(node_has_artifact(workflow.node_by_name("fan").unwrap()));
}

#[test]
fn test_node参照解決_未知キーと成果のない子と非objectを絶対位置で報告する() {
    // Given
    let workflow = composite_workflow();
    for (path, position, segment, kind) in [
        (
            "fan.missing",
            1,
            "missing",
            contract_schema::FieldPathResolutionErrorKind::MissingProperty,
        ),
        (
            "fan.silent",
            1,
            "silent",
            contract_schema::FieldPathResolutionErrorKind::MissingProperty,
        ),
        (
            "indexed.007.passed",
            1,
            "007",
            contract_schema::FieldPathResolutionErrorKind::MissingProperty,
        ),
        (
            "nested.fan.writer.outer.missing",
            4,
            "missing",
            contract_schema::FieldPathResolutionErrorKind::MissingProperty,
        ),
        (
            "nested.fan.writer.outer.scalar.passed",
            5,
            "passed",
            contract_schema::FieldPathResolutionErrorKind::NonObject,
        ),
    ] {
        // When
        let result = resolve_node_field_path(
            &workflow,
            workflow.node_by_name("main").unwrap(),
            &FieldPath::from_dotted(path).unwrap(),
        );

        // Then
        assert_eq!(
            result,
            Err(NodeFieldPathError::Segment(
                contract_schema::FieldPathResolutionError {
                    position,
                    segment: segment.to_string(),
                    kind
                }
            )),
            "{path}"
        );
    }
}

#[test]
fn test_node参照解決_包含cycleへの再訪をその段で打ち切る() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(
        r#"
name: cycle-schema
description: test
nodes:
  main: {sequence: {children: [part]}}
  part: {fanout: {children: [main]}}
"#,
    )
    .unwrap();

    // When
    let result = resolve_node_field_path(
        &workflow,
        workflow.node_by_name("main").unwrap(),
        &FieldPath::new(["part", "main", "part"]),
    );
    let errors = crate::domain::workflow::services::validation::validate_all(&workflow);

    // Then
    assert_eq!(result, Err(missing_node_field(1, "main")));
    assert!(errors.iter().any(|error| matches!(error, crate::domain::workflow::services::validation::ValidationError::CompositeInclusionCycle { .. })));
}

#[test]
fn test_node参照解決_非objectのcommand契約と参照不能な成果を区別する() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(
        r#"
name: invalid-schema
description: test
schemas: {text: string}
nodes:
  main: {command: check, artifact: text}
  silent: {session: {provider: codex}}
  missing: {session: {provider: codex}, artifact: missing}
"#,
    )
    .unwrap();

    // When / Then
    assert_eq!(
        resolve_node_field_path(
            &workflow,
            workflow.node_by_name("main").unwrap(),
            &FieldPath::new(["passed"])
        ),
        Err(NodeFieldPathError::ArtifactNotObject)
    );
    for name in ["silent", "missing"] {
        assert_eq!(
            resolve_node_field_path(
                &workflow,
                workflow.node_by_name(name).unwrap(),
                &FieldPath::new(["passed"])
            ),
            Err(NodeFieldPathError::NoReferenceableArtifact)
        );
    }
}

#[test]
fn test_node参照解決_合成子経由の非object契約をleafの直前段に帰属させる() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(
        r#"
name: nonobject-leaf
description: test
schemas: {text: string}
nodes:
  main: {sequence: {children: [part]}}
  part: {sequence: {children: [bad]}}
  bad: {command: check, artifact: text}
"#,
    )
    .unwrap();
    for (name, path, expected) in [
        (
            "main",
            "part.bad.ok",
            NodeFieldPathError::Segment(contract_schema::FieldPathResolutionError {
                position: 1,
                segment: "bad".to_string(),
                kind: contract_schema::FieldPathResolutionErrorKind::NonObject,
            }),
        ),
        (
            "part",
            "bad.ok",
            NodeFieldPathError::Segment(contract_schema::FieldPathResolutionError {
                position: 0,
                segment: "bad".to_string(),
                kind: contract_schema::FieldPathResolutionErrorKind::NonObject,
            }),
        ),
        ("bad", "ok", NodeFieldPathError::ArtifactNotObject),
    ] {
        // When
        let result = resolve_node_field_path(
            &workflow,
            workflow.node_by_name(name).unwrap(),
            &FieldPath::from_dotted(path).unwrap(),
        );

        // Then
        assert_eq!(result, Err(expected), "{name}.{path}");
    }
}

#[test]
fn test_node参照解決_訪問済み集合は要求されたpathごとに閉じる() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(
        r#"
name: shared-schema
description: test
nodes:
  main: {sequence: {children: [left, right]}}
  left: {sequence: {children: [shared]}}
  right: {sequence: {children: [shared]}}
  shared: {command: check}
"#,
    )
    .unwrap();
    for side in ["left", "right"] {
        // When
        let result = resolve_node_field_path(
            &workflow,
            workflow.node_by_name("main").unwrap(),
            &FieldPath::new([side, "shared", "ok"]),
        );

        // Then
        assert_eq!(
            result,
            Ok(ResolvedNodeField::Leaf {
                schema: SchemaDef::Boolean,
                required: true
            })
        );
    }
}

#[test]
fn test_node参照解決_fanout内のsequenceとfanoutを経由してleafへ届く() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(
        r#"
name: nested-fanout
description: test
nodes:
  main: {fanout: {children: [seq], items: []}}
  seq: {sequence: {children: [fan]}}
  fan: {fanout: {children: [leaf, missing]}}
  leaf: {command: check}
  missing: {session: {provider: codex}, artifact: unknown}
"#,
    )
    .unwrap();
    let node = workflow.node_by_name("main").unwrap();

    // When
    let resolved =
        resolve_node_field_path(&workflow, node, &FieldPath::new(["0", "fan", "leaf", "ok"]));
    let missing = resolve_node_field_path(
        &workflow,
        node,
        &FieldPath::new(["0", "fan", "missing", "passed"]),
    );

    // Then
    assert_eq!(
        resolved,
        Ok(ResolvedNodeField::Leaf {
            schema: SchemaDef::Boolean,
            required: true
        })
    );
    assert_eq!(missing, Err(missing_node_field(2, "missing")));
}
