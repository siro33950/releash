use super::*;

fn path(reference: &str) -> FieldPath {
    FieldPath::from_reference(reference).unwrap().1
}

#[test]
fn test_実行時path解決_多段の終端値を返す() {
    // Given
    let value = serde_json::json!({"outer": {"inner": {"leaf": 42}}});

    // When
    let resolved = resolve_value_at_path(&value, &path("root.outer.inner.leaf"));

    // Then
    assert_eq!(resolved, Some(&serde_json::json!(42)));
}

#[test]
fn test_実行時path解決_非objectと存在しないkeyは未解決になる() {
    // Given
    let value = serde_json::json!({"scalar": "text", "object": {}});

    // When / Then
    assert_eq!(
        resolve_value_at_path(&value, &path("root.scalar.leaf")),
        None
    );
    assert_eq!(
        resolve_value_at_path(&value, &path("root.object.missing")),
        None
    );
}

#[test]
fn test_実行時path解決_段0個と1段の値を従来通り返す() {
    // Given
    let value = serde_json::json!({"leaf": true});

    // When / Then
    assert_eq!(
        resolve_value_at_path(&value, &FieldPath::default()),
        Some(&value)
    );
    assert_eq!(
        resolve_value_at_path(&value, &path("root.leaf")),
        Some(&serde_json::json!(true))
    );
}

#[test]
fn test_実行時path解決_多段の配線とenvを解決する() {
    // Given
    let entry = ChildEntry {
        on_failure: None,
        name: "consume".to_string(),
        inputs: vec![(
            "value".to_string(),
            crate::domain::workflow::value_objects::InputSourceRef::new("produce.outer.leaf"),
        )],
        rules: None,
    };
    let artifacts = HashMap::from([(
        "produce".to_string(),
        serde_json::json!({"outer": {"leaf": "wired"}}),
    )]);
    let env = BTreeMap::from([(
        EnvironmentVariableName::new("VALUE").unwrap(),
        InputParameterRef::new("input.outer.leaf").unwrap(),
    )]);
    let bindings = vec![(
        "input".to_string(),
        serde_json::json!({"outer": {"leaf": {"ok": true}}}),
    )];

    // When
    let wired = resolve_entry_bindings(Some(&entry), &artifacts);
    let environment = resolve_command_environment(&env, &bindings).unwrap();

    // Then
    assert_eq!(
        wired,
        vec![("value".to_string(), serde_json::json!("wired"))]
    );
    assert_eq!(
        environment,
        vec![("VALUE".to_string(), r#"{"ok":true}"#.to_string())]
    );
}

#[test]
fn test_実行時path解決_多段の未解決配線は束縛から除く() {
    // Given
    let entry = ChildEntry {
        on_failure: None,
        name: "consume".to_string(),
        inputs: vec![(
            "value".to_string(),
            crate::domain::workflow::value_objects::InputSourceRef::new("produce.outer.missing"),
        )],
        rules: None,
    };
    let artifacts = HashMap::from([("produce".to_string(), serde_json::json!({"outer": {}}))]);

    // When
    let wired = resolve_entry_bindings(Some(&entry), &artifacts);

    // Then
    assert!(wired.is_empty());
}

#[test]
fn test_実行時path解決_fanout子とtemplateで多段を解決する() {
    // Given
    let node = NodeDefinition {
        name: "worker".to_string(),
        input: vec![crate::domain::workflow::InputParam {
            name: "value".to_string(),
            contract: None,
        }],
        ..Default::default()
    };
    let entry = ChildEntry {
        on_failure: None,
        name: "worker".to_string(),
        inputs: vec![(
            "value".to_string(),
            crate::domain::workflow::value_objects::InputSourceRef::new("context.outer.leaf"),
        )],
        rules: None,
    };
    let parameters = HashMap::from([(
        "context".to_string(),
        serde_json::json!({"outer": {"leaf": "resolved"}}),
    )]);

    // When
    let bindings = resolve_fanout_child_bindings(Some(&entry), &node, &parameters, None, None);
    let template = resolve_template_value("context", &path("root.outer.leaf"), &parameters);

    // Then
    assert_eq!(
        bindings,
        vec![("value".to_string(), serde_json::json!("resolved"))]
    );
    assert_eq!(template, Some(&serde_json::json!("resolved")));
    assert_eq!(
        resolve_template_value("context", &path("root.outer.missing"), &parameters),
        None
    );
}

fn command_with_env(
    parameters: Vec<crate::domain::workflow::InputParam>,
    references: &[(&str, &str)],
) -> NodeDefinition {
    NodeDefinition {
        name: "main".to_string(),
        kind: crate::domain::workflow::NodeKind::Command(crate::domain::workflow::CommandSpec {
            command: "true".to_string(),
            env: references
                .iter()
                .map(|(variable, reference)| {
                    (
                        EnvironmentVariableName::new(*variable).unwrap(),
                        InputParameterRef::new(*reference).unwrap(),
                    )
                })
                .collect(),
        }),
        input: parameters,
        ..Default::default()
    }
}

fn nested_parameter_schema() -> SchemaDef {
    SchemaDef::Object {
        properties: BTreeMap::from([
            (
                "outer".to_string(),
                SchemaDef::Object {
                    properties: BTreeMap::from([(
                        "leaf".to_string(),
                        SchemaDef::String { r#enum: None },
                    )]),
                    required: Default::default(),
                },
            ),
            ("scalar".to_string(), SchemaDef::Boolean),
        ]),
        required: Default::default(),
    }
}

#[test]
fn test_command_env参照検証_型ありinputの多段fieldが通る() {
    // Given
    let workflow = WorkflowDefinition {
        name: "wf".to_string(),
        schemas: BTreeMap::from([("document".to_string(), nested_parameter_schema())]),
        nodes: vec![command_with_env(
            vec![crate::domain::workflow::InputParam {
                name: "doc".to_string(),
                contract: Some("document".to_string()),
            }],
            &[("VALUE", "doc.outer.leaf")],
        )],
        ..Default::default()
    };

    // When
    let errors = validate_workflow_command_environment_references(&workflow);

    // Then
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn test_command_env参照検証_存在しない段と非object中間段を拒否する() {
    // Given
    let workflow = WorkflowDefinition {
        name: "wf".to_string(),
        schemas: BTreeMap::from([("document".to_string(), nested_parameter_schema())]),
        nodes: vec![command_with_env(
            vec![crate::domain::workflow::InputParam {
                name: "doc".to_string(),
                contract: Some("document".to_string()),
            }],
            &[
                ("MISSING", "doc.outer.missing"),
                ("NON_OBJECT", "doc.scalar.leaf"),
            ],
        )],
        ..Default::default()
    };

    // When
    let errors = validate_workflow_command_environment_references(&workflow);

    // Then
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors
        .iter()
        .all(|error| matches!(error.source, ReferenceResolveError::UnknownField { .. })));
}

#[test]
fn test_command_env参照検証_型なしinputの多段は静的検査しない() {
    // Given
    let workflow = WorkflowDefinition {
        name: "wf".to_string(),
        nodes: vec![command_with_env(
            vec![crate::domain::workflow::InputParam {
                name: "doc".to_string(),
                contract: None,
            }],
            &[
                ("WHOLE", "doc"),
                ("ONE", "doc.field"),
                ("DEEP", "doc.any.depth"),
            ],
        )],
        ..Default::default()
    };

    // When
    let errors = validate_workflow_command_environment_references(&workflow);

    // Then
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn test_template参照検証_型ありinputの多段fieldを解決する() {
    // Given
    let node = command_with_env(
        vec![crate::domain::workflow::InputParam {
            name: "doc".to_string(),
            contract: Some("document".to_string()),
        }],
        &[],
    );
    let schemas = BTreeMap::from([("document".to_string(), nested_parameter_schema())]);

    // When
    let valid =
        validate_template_references_for_node(&node, &schemas, "command {{ doc.outer.leaf }}");
    let invalid = validate_template_references_for_node(
        &node,
        &schemas,
        "{{ doc.outer.missing }} {{ doc.scalar.leaf }}",
    );

    // Then
    assert!(valid.is_empty(), "{valid:?}");
    assert_eq!(invalid.len(), 2, "{invalid:?}");
    assert!(invalid
        .iter()
        .all(|error| matches!(error, ReferenceResolveError::UnknownField { .. })));
}

#[test]
fn test_template参照検証_型なしinputの多段は静的検査しない() {
    // Given
    let node = command_with_env(
        vec![crate::domain::workflow::InputParam {
            name: "doc".to_string(),
            contract: None,
        }],
        &[],
    );

    // When
    let errors = validate_template_references_for_node(
        &node,
        &BTreeMap::new(),
        "{{ doc }} {{ doc.field }} {{ doc.any.depth }}",
    );

    // Then
    assert!(errors.is_empty(), "{errors:?}");
}
