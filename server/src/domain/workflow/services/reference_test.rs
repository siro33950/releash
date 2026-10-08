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
pub(crate) mod tests {
    use super::super::*;
    use crate::domain::workflow::value_objects::InputSourceRef;
    use crate::domain::workflow::{CommandSpec, InputParam, NodeKind};

    fn command_node_with_params(
        name: &str,
        command: &str,
        params: Vec<InputParam>,
    ) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: command.to_string(),
                env: Default::default(),
            }),
            artifact: None,
            input: params,
            completion: Default::default(),
            worktree: None,
        }
    }

    fn untyped(name: &str) -> InputParam {
        InputParam {
            name: name.to_string(),
            contract: None,
        }
    }

    fn command_env(
        entries: &[(&str, &str)],
    ) -> BTreeMap<EnvironmentVariableName, InputParameterRef> {
        entries
            .iter()
            .map(|(name, reference)| {
                (
                    EnvironmentVariableName::new(*name).unwrap(),
                    InputParameterRef::new(*reference).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn test_本文検証_宣言済みパラメータ名のみ参照できる() {
        let node = command_node_with_params(
            "delete",
            "rm -f -- '{{ spec }}/behavior.md'",
            vec![untyped("spec")],
        );
        let errors =
            validate_template_references_for_node(&node, &BTreeMap::new(), node.command().unwrap());
        assert!(errors.is_empty());
    }

    #[test]
    fn test_本文検証_未宣言の参照を拒否する() {
        let node = command_node_with_params("echo", "echo '{{ item }}'", vec![untyped("task")]);
        let errors =
            validate_template_references_for_node(&node, &BTreeMap::new(), node.command().unwrap());
        assert!(errors.iter().any(|error| matches!(
            error,
            ReferenceResolveError::UnknownParameter { name } if name == "item"
        )));
    }

    #[test]
    fn test_command環境解決_stringは無変換で非stringはcompact_jsonになる() {
        let env = command_env(&[
            ("DOC", "document"),
            ("META", "metadata"),
            ("COUNT", "metadata.count"),
        ]);
        let bindings = vec![
            (
                "document".to_string(),
                Value::String("{{ untouched }}; `still data`\n$HOME".to_string()),
            ),
            (
                "metadata".to_string(),
                serde_json::json!({"count": 2, "ready": true}),
            ),
        ];

        let resolved = resolve_command_environment(&env, &bindings).unwrap();

        assert!(resolved.contains(&(
            "DOC".to_string(),
            "{{ untouched }}; `still data`\n$HOME".to_string()
        )));
        assert!(resolved.contains(&(
            "META".to_string(),
            r#"{"count":2,"ready":true}"#.to_string()
        )));
        assert!(resolved.contains(&("COUNT".to_string(), "2".to_string())));
    }

    #[test]
    fn test_command環境解決_束縛またはfieldが無ければ全体を失敗する() {
        let missing_parameter = command_env(&[("DOC", "document")]);
        assert!(matches!(
            resolve_command_environment(&missing_parameter, &[]),
            Err(CommandEnvironmentResolutionError::MissingParameter { .. })
        ));

        let missing_field = command_env(&[("DOC", "document.body")]);
        let bindings = vec![("document".to_string(), serde_json::json!({"title": "x"}))];
        assert!(matches!(
            resolve_command_environment(&missing_field, &bindings),
            Err(CommandEnvironmentResolutionError::MissingField { .. })
        ));
    }

    #[test]
    fn test_command環境参照検証_未宣言inputと型ありinputの未知fieldを拒否する() {
        let mut node = command_node_with_params(
            "main",
            "true",
            vec![InputParam {
                name: "document".to_string(),
                contract: Some("document-contract".to_string()),
            }],
        );
        let NodeKind::Command(command) = &mut node.kind else {
            unreachable!();
        };
        command.env = command_env(&[("UNKNOWN", "missing"), ("FIELD", "document.body")]);
        let workflow = WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            schemas: [(
                "document-contract".to_string(),
                SchemaDef::Object {
                    properties: BTreeMap::new(),
                    required: Default::default(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![node],
            entry: "main".to_string(),
            ..Default::default()
        };

        let errors = validate_workflow_command_environment_references(&workflow);

        assert!(errors.iter().any(|error| matches!(
            &error.source,
            ReferenceResolveError::UnknownParameter { .. }
        )));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.source, ReferenceResolveError::UnknownField { .. })));
    }

    #[test]
    fn test_束縛解決_sequenceは兄弟とrequestを解決する() {
        let entry = ChildEntry {
            name: "consume".to_string(),
            inputs: vec![
                ("spec".to_string(), InputSourceRef::new("collect.spec_dir")),
                ("goal".to_string(), InputSourceRef::new("request")),
            ],
            rules: None,
        };
        let mut artifacts = HashMap::new();
        artifacts.insert(
            "collect".to_string(),
            serde_json::json!({"spec_dir": "specs/x"}),
        );
        artifacts.insert(
            REQUEST_ARTIFACT.to_string(),
            Value::String("build it".to_string()),
        );

        let bindings = resolve_entry_bindings(Some(&entry), &artifacts);

        assert_eq!(
            bindings,
            vec![
                ("spec".to_string(), Value::String("specs/x".to_string())),
                ("goal".to_string(), Value::String("build it".to_string())),
            ]
        );
    }

    #[test]
    fn test_束縛解決_fanout子は親パラメータとrequestとitemsを解決する() {
        let node = command_node_with_params(
            "worker",
            "echo",
            vec![untyped("thread"), untyped("spec"), untyped("goal")],
        );
        let entry = ChildEntry {
            name: "worker".to_string(),
            inputs: vec![
                ("thread".to_string(), InputSourceRef::new("items")),
                ("spec".to_string(), InputSourceRef::new("context.spec_dir")),
                ("goal".to_string(), InputSourceRef::new("request")),
            ],
            rules: None,
        };
        let mut parent_parameters = HashMap::new();
        parent_parameters.insert(
            "context".to_string(),
            serde_json::json!({"spec_dir": "specs/x"}),
        );
        let request = Value::String("build it".to_string());
        let item = serde_json::json!({"thread_id": "t-1"});

        let bindings = resolve_fanout_child_bindings(
            Some(&entry),
            &node,
            &parent_parameters,
            Some(&request),
            Some(&item),
        );

        assert_eq!(
            bindings,
            vec![
                ("thread".to_string(), item.clone()),
                ("spec".to_string(), Value::String("specs/x".to_string())),
                ("goal".to_string(), Value::String("build it".to_string())),
            ]
        );
    }

    #[test]
    fn test_束縛解決_fanout子は兄弟nodeを直接参照できない() {
        let node = command_node_with_params("worker", "echo", vec![untyped("spec")]);
        let entry = ChildEntry {
            name: "worker".to_string(),
            inputs: vec![("spec".to_string(), InputSourceRef::new("collect"))],
            rules: None,
        };

        let bindings =
            resolve_fanout_child_bindings(Some(&entry), &node, &HashMap::new(), None, None);

        assert!(bindings.is_empty());
    }

    #[test]
    fn test_束縛解決_単一パラメータへのitems自動束縛() {
        let node = command_node_with_params("worker", "echo", vec![untyped("task")]);
        let item = serde_json::json!({"task_id": "T1"});

        let bindings =
            resolve_fanout_child_bindings(None, &node, &HashMap::new(), None, Some(&item));

        assert_eq!(bindings, vec![("task".to_string(), item)]);
    }

    #[test]
    fn test_束縛解決_解決できない供給元は束縛から除かれる() {
        let entry = ChildEntry {
            name: "consume".to_string(),
            inputs: vec![("spec".to_string(), InputSourceRef::new("missing_node"))],
            rules: None,
        };

        let bindings = resolve_entry_bindings(Some(&entry), &HashMap::new());

        assert!(bindings.is_empty());
    }
}

mod reference_path_test {
    use super::super::*;

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
            name: "consume".to_string(),
            inputs: vec![(
                "value".to_string(),
                crate::domain::workflow::value_objects::InputSourceRef::new(
                    "produce.outer.missing",
                ),
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
            kind: crate::domain::workflow::NodeKind::Command(
                crate::domain::workflow::CommandSpec {
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
                },
            ),
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

    #[test]
    fn test_正本サンプルのfanout配線_統合と整合確認と修正統合へ同じslot集合を渡す() {
        // Given
        let workflow: WorkflowDefinition = serde_saphyr::from_str(include_str!(concat!(
            env!("OUT_DIR"),
            "/full-cycle-development.yml"
        )))
        .unwrap();
        let implementations = serde_json::json!({"0": {"verify_task": {"task_id": "one", "complete": true}}, "1": {"verify_task": {"task_id": "two", "complete": false}}});
        let fixes = serde_json::json!({"0": {"verify_fix": {"complete": true}}, "2": {"verify_fix": {"complete": false}}});
        let artifacts = HashMap::from([
            ("implement_all".to_string(), implementations.clone()),
            ("fix_all".to_string(), fixes.clone()),
        ]);
        for (scope, target, expected) in [
            ("implementation", "merge_implementations", &implementations),
            ("implementation", "check_integration", &implementations),
            ("fix_round", "merge_fixes", &fixes),
        ] {
            let sequence = workflow.node_by_name(scope).unwrap().sequence().unwrap();
            let entry = sequence.child_entry(target).unwrap();

            // When
            let bindings = resolve_entry_bindings(Some(entry), &artifacts);

            // Then
            assert_eq!(
                workflow
                    .node_by_name(target)
                    .unwrap()
                    .input_parameter("results")
                    .unwrap()
                    .contract,
                None
            );
            assert_eq!(
                bindings
                    .iter()
                    .find(|(name, _)| name == "results")
                    .map(|(_, value)| value),
                Some(expected)
            );
        }
    }
}
