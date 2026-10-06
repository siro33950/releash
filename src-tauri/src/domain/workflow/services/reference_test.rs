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
