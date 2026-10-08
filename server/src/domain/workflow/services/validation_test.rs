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
pub(crate) mod tests {
    use super::super::*;
    use crate::domain::provider_lifecycle::ProviderKind;
    use crate::domain::workflow::value_objects::{
        ChildEntry, CommandSpec, FacetRefs, FanoutSpec, InputSourceRef, NodeCompletion, NodeKind,
        Rule, SequenceSpec, SessionSpec,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn command_node(name: &str, command: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: command.to_string(),
                env: Default::default(),
            }),
            artifact: None,
            input: Vec::new(),
            completion: NodeCompletion::default(),
            worktree: None,
        }
    }

    fn session_node(name: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                provider: ProviderKind::Claude,
                model: None,
                permission: None,
                facets: FacetRefs {
                    policy: None,
                    knowledge: Vec::new(),
                    instruction: Some("do-it".to_string()),
                },
            }),
            artifact: None,
            input: Vec::new(),
            completion: NodeCompletion::default(),
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
            completion: NodeCompletion::default(),
            worktree: None,
        }
    }

    fn fanout_node(
        name: &str,
        children: Vec<ChildEntry>,
        items: Option<ItemsSource>,
    ) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Fanout(FanoutSpec { children, items }),
            artifact: None,
            input: Vec::new(),
            completion: NodeCompletion::default(),
            worktree: None,
        }
    }

    fn entry(name: &str, inputs: Vec<(&str, &str)>) -> ChildEntry {
        ChildEntry {
            name: name.to_string(),
            inputs: inputs
                .into_iter()
                .map(|(parameter, source)| (parameter.to_string(), InputSourceRef::new(source)))
                .collect(),
            rules: None,
        }
    }

    fn untyped_param(name: &str) -> InputParam {
        InputParam {
            name: name.to_string(),
            contract: None,
        }
    }

    fn typed_param(name: &str, contract: &str) -> InputParam {
        InputParam {
            name: name.to_string(),
            contract: Some(contract.to_string()),
        }
    }

    fn workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: BTreeMap::new(),
            nodes,
            entry: "main".to_string(),
        }
    }

    fn object_schema(fields: &[&str]) -> SchemaDef {
        SchemaDef::Object {
            properties: fields
                .iter()
                .map(|field| ((*field).to_string(), SchemaDef::String { r#enum: None }))
                .collect(),
            required: fields
                .iter()
                .map(|field| (*field).to_string())
                .collect::<BTreeSet<_>>(),
        }
    }

    #[test]
    fn test_検証_純直列のsequenceが通る() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("first"),
                    ChildEntry::reference("second"),
                ],
            ),
            command_node("first", "echo one"),
            command_node("second", "echo two"),
        ]);
        assert!(validate(&wf).is_ok(), "{:?}", validate(&wf));
        assert!(validate_all(&wf).is_empty(), "{:?}", validate_all(&wf));
    }

    #[test]
    fn test_検証_mainがleafなら単独実行として通る() {
        let wf = workflow(vec![session_node("main")]);
        assert!(validate(&wf).is_ok(), "{:?}", validate(&wf));
    }

    #[test]
    fn test_検証_main不在はエラー() {
        let wf = workflow(vec![command_node("helper", "echo hi")]);
        assert!(matches!(
            validate(&wf),
            Err(ValidationError::MissingEntryNode { .. })
        ));
    }

    #[test]
    fn test_配線_request供給とfieldパス付き兄弟供給が通る() {
        let mut collect = command_node("collect", "echo '{}'");
        collect.artifact = Some("collected".to_string());
        let mut consume = command_node("consume", "echo '{{ spec }}' '{{ goal }}'");
        consume.input = vec![untyped_param("spec"), untyped_param("goal")];
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("collect"),
                    entry(
                        "consume",
                        vec![("spec", "collect.spec_dir"), ("goal", "request")],
                    ),
                ],
            ),
            collect,
            consume,
        ]);
        wf.schemas
            .insert("collected".to_string(), object_schema(&["spec_dir"]));

        assert!(validate(&wf).is_ok(), "{:?}", validate(&wf));
    }

    #[test]
    fn test_配線_兄弟artifactと型あり自inputの多段fieldが通る() {
        // Given
        let mut collect = command_node("collect", "echo '{}'");
        collect.artifact = Some("nested".to_string());
        let mut consume = command_node("consume", "echo");
        consume.input = vec![
            untyped_param("artifact_value"),
            untyped_param("input_value"),
        ];
        let mut main = sequence_node(
            "main",
            vec![
                ChildEntry::reference("collect"),
                entry(
                    "consume",
                    vec![
                        ("artifact_value", "collect.outer.leaf"),
                        ("input_value", "context.outer.leaf"),
                    ],
                ),
            ],
        );
        main.input = vec![typed_param("context", "nested")];
        let mut wf = workflow(vec![main, collect, consume]);
        wf.schemas.insert(
            "nested".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::from([(
                    "outer".to_string(),
                    SchemaDef::Object {
                        properties: BTreeMap::from([(
                            "leaf".to_string(),
                            SchemaDef::String { r#enum: None },
                        )]),
                        required: BTreeSet::new(),
                    },
                )]),
                required: BTreeSet::new(),
            },
        );

        // When
        let errors = validate_all(&wf);

        // Then
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn test_配線_多段の存在しない段と非object中間段を拒否する() {
        // Given
        let mut collect = command_node("collect", "echo '{}'");
        collect.artifact = Some("nested".to_string());
        let mut consume = command_node("consume", "echo");
        consume.input = vec![untyped_param("missing"), untyped_param("non_object")];
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("collect"),
                    entry(
                        "consume",
                        vec![
                            ("missing", "collect.outer.missing"),
                            ("non_object", "collect.scalar.leaf"),
                        ],
                    ),
                ],
            ),
            collect,
            consume,
        ]);
        wf.schemas.insert(
            "nested".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::from([
                    (
                        "outer".to_string(),
                        SchemaDef::Object {
                            properties: BTreeMap::new(),
                            required: BTreeSet::new(),
                        },
                    ),
                    ("scalar".to_string(), SchemaDef::String { r#enum: None }),
                ]),
                required: BTreeSet::new(),
            },
        );

        // When
        let errors = validate_all(&wf);

        // Then
        let violations = errors
            .iter()
            .filter(|error| {
                matches!(
                    error,
                    ValidationError::InvalidInputWiring(violation)
                        if violation.kind == InputWiringKind::UnknownSourceField
                )
            })
            .count();
        assert_eq!(violations, 2, "{errors:?}");
    }

    #[test]
    fn test_配線_型なしinputの多段fieldは静的検査せずcommand予約fieldは受理する() {
        // Given
        let collect = command_node("collect", "true");
        let mut consume = command_node("consume", "echo");
        consume.input = vec![untyped_param("dynamic"), untyped_param("ok")];
        let mut main = sequence_node(
            "main",
            vec![
                ChildEntry::reference("collect"),
                entry(
                    "consume",
                    vec![("dynamic", "context.any.depth"), ("ok", "collect.ok")],
                ),
            ],
        );
        main.input = vec![untyped_param("context")];
        let wf = workflow(vec![main, collect, consume]);

        // When
        let errors = validate_all(&wf);

        // Then
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn test_配線_requestとitemsのfield参照は拒否する() {
        // Given
        let mut sequence_consumer = command_node("sequence-consumer", "echo");
        sequence_consumer.input = vec![untyped_param("value")];
        let mut fanout_consumer = command_node("fanout-consumer", "echo");
        fanout_consumer.input = vec![untyped_param("value")];
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry("sequence-consumer", vec![("value", "request.field")]),
                    ChildEntry::reference("fan"),
                ],
            ),
            sequence_consumer,
            fanout_node(
                "fan",
                vec![entry("fanout-consumer", vec![("value", "items.field")])],
                Some(ItemsSource::Literal(vec![serde_json::json!(1)])),
            ),
            fanout_consumer,
        ]);

        // When
        let errors = validate_all(&wf);

        // Then
        let invalid_formats = errors
            .iter()
            .filter(|error| {
                matches!(
                    error,
                    ValidationError::InvalidInputWiring(violation)
                        if violation.kind == InputWiringKind::InvalidSourceFormat
                )
            })
            .count();
        assert_eq!(invalid_formats, 2, "{errors:?}");
    }

    #[test]
    fn test_配線_空・空白・空の段は従来と同じ形式不正になる() {
        // Given
        let mut consume = command_node("consume", "echo");
        consume.input = vec![
            untyped_param("empty"),
            untyped_param("space"),
            untyped_param("middle"),
            untyped_param("trailing"),
        ];
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![entry(
                    "consume",
                    vec![
                        ("empty", ""),
                        ("space", "bad source"),
                        ("middle", "source..leaf"),
                        ("trailing", "source."),
                    ],
                )],
            ),
            consume,
        ]);

        // When
        let errors = validate_all(&wf);

        // Then
        let invalid_formats = errors
            .iter()
            .filter(|error| {
                matches!(
                    error,
                    ValidationError::InvalidInputWiring(violation)
                        if violation.kind == InputWiringKind::InvalidSourceFormat
                )
            })
            .count();
        assert_eq!(invalid_formats, 4, "{errors:?}");
    }

    #[test]
    fn test_配線_未知の供給元を拒否する() {
        let mut consume = command_node("consume", "echo");
        consume.input = vec![untyped_param("spec")];
        let wf = workflow(vec![
            sequence_node("main", vec![entry("consume", vec![("spec", "ghost")])]),
            consume,
        ]);

        assert!(validate_all(&wf).iter().any(|error| matches!(
            error,
            ValidationError::InvalidInputWiring(violation)
                if violation.kind == InputWiringKind::UnknownSource
        )));
    }

    #[test]
    fn test_配線_子が宣言しないパラメータへの配線を拒否する() {
        let consume = command_node("consume", "echo");
        let wf = workflow(vec![
            sequence_node("main", vec![entry("consume", vec![("spec", "request")])]),
            consume,
        ]);

        assert!(validate_all(&wf).iter().any(|error| matches!(
            error,
            ValidationError::InvalidInputWiring(violation)
                if violation.kind == InputWiringKind::UnknownParameter
        )));
    }

    #[test]
    fn test_配線_兄弟名と自パラメータ名の衝突を拒否する() {
        let owner_children = vec![
            ChildEntry::reference("spec"),
            entry("consume", vec![("spec", "spec")]),
        ];
        let mut nested = sequence_node("part", owner_children);
        nested.input = vec![untyped_param("spec")];
        let mut consume = command_node("consume", "echo");
        consume.input = vec![untyped_param("spec")];
        let wf = workflow(vec![
            sequence_node("main", vec![ChildEntry::reference("part")]),
            nested,
            command_node("spec", "echo spec"),
            consume,
        ]);

        assert!(validate_all(&wf).iter().any(|error| matches!(
            error,
            ValidationError::InvalidInputWiring(violation)
                if violation.kind == InputWiringKind::AmbiguousSource
        )));
    }

    #[test]
    fn test_配線_fanout外のitems供給元を拒否する() {
        let mut consume = command_node("consume", "echo");
        consume.input = vec![untyped_param("task")];
        let wf = workflow(vec![
            sequence_node("main", vec![entry("consume", vec![("task", "items")])]),
            consume,
        ]);

        assert!(validate_all(&wf).iter().any(|error| matches!(
            error,
            ValidationError::InvalidInputWiring(violation)
                if violation.kind == InputWiringKind::ItemsUnavailable
        )));
    }

    #[test]
    fn test_予約パラメータ名_requestとitemsは宣言できない() {
        let mut node = command_node("main", "echo");
        node.input = vec![untyped_param("request")];
        let wf = workflow(vec![node]);

        assert!(validate_all(&wf).iter().any(|error| matches!(
            error,
            ValidationError::ReservedInputParameterName { parameter, .. } if parameter == "request"
        )));
    }

    #[test]
    fn test_配線_fanout子は兄弟nodeを供給元にできない() {
        let mut collect = command_node("collect", "echo '{}'");
        collect.artifact = Some("collected".to_string());
        let mut worker = command_node("worker", "echo");
        worker.input = vec![untyped_param("spec")];
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("collect"),
                    ChildEntry::reference("fan"),
                ],
            ),
            collect,
            fanout_node(
                "fan",
                vec![entry("worker", vec![("spec", "collect")])],
                None,
            ),
            worker,
        ]);
        wf.schemas
            .insert("collected".to_string(), object_schema(&["spec_dir"]));

        assert!(validate_all(&wf).iter().any(|error| matches!(
            error,
            ValidationError::InvalidInputWiring(violation)
                if violation.kind == InputWiringKind::UnknownSource && violation.node == "fan"
        )));
    }

    #[test]
    fn test_配線_fanout子は親fanoutのパラメータを供給元にできる() {
        let mut collect = command_node("collect", "echo '{}'");
        collect.artifact = Some("collected".to_string());
        let mut worker = command_node("worker", "echo '{{ spec }}'");
        worker.input = vec![untyped_param("spec")];
        let mut fan = fanout_node(
            "fan",
            vec![entry("worker", vec![("spec", "context")])],
            None,
        );
        fan.input = vec![untyped_param("context")];
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("collect"),
                    entry("fan", vec![("context", "collect")]),
                ],
            ),
            collect,
            fan,
            worker,
        ]);
        wf.schemas
            .insert("collected".to_string(), object_schema(&["spec_dir"]));

        assert!(validate(&wf).is_ok(), "{:?}", validate(&wf));
    }

    #[test]
    fn test_ネスト_rulesターゲットのsequence参照が通る() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![ChildEntry {
                    name: "work".to_string(),
                    inputs: Vec::new(),
                    rules: Some(vec![Rule::Next("part".to_string())]),
                }],
            ),
            command_node("work", "echo hi"),
            sequence_node("part", vec![ChildEntry::reference("leaf")]),
            command_node("leaf", "echo hi"),
        ]);

        assert!(validate(&wf).is_ok(), "{:?}", validate(&wf));
        assert!(validate_all(&wf).is_empty(), "{:?}", validate_all(&wf));
    }

    #[test]
    fn test_検証_sequenceのartifact宣言を拒否する() {
        let mut root = sequence_node("main", vec![ChildEntry::reference("leaf")]);
        root.artifact = Some("result".to_string());
        let mut wf = workflow(vec![root, command_node("leaf", "echo hi")]);
        wf.schemas
            .insert("result".to_string(), object_schema(&["note"]));

        assert!(validate_all(&wf).iter().any(|error| matches!(
            error,
            ValidationError::SequenceArtifactDeclaration { node } if node == "main"
        )));
    }

    #[test]
    fn test_ネスト_sequenceの子のsequenceが通る() {
        let wf = workflow(vec![
            sequence_node("main", vec![ChildEntry::reference("part")]),
            sequence_node("part", vec![ChildEntry::reference("leaf")]),
            command_node("leaf", "echo hi"),
        ]);

        assert!(validate(&wf).is_ok(), "{:?}", validate(&wf));
        assert!(validate_all(&wf).is_empty(), "{:?}", validate_all(&wf));
    }

    #[test]
    fn test_ネスト_fanoutの子のsequenceが通る() {
        let wf = workflow(vec![
            sequence_node("main", vec![ChildEntry::reference("fan")]),
            fanout_node("fan", vec![ChildEntry::reference("part")], None),
            sequence_node("part", vec![ChildEntry::reference("worker")]),
            command_node("worker", "echo hi"),
        ]);

        assert!(validate(&wf).is_ok(), "{:?}", validate(&wf));
        assert!(validate_all(&wf).is_empty(), "{:?}", validate_all(&wf));
    }

    #[test]
    fn test_包含循環_相互包含のsequenceを検出する() {
        let wf = workflow(vec![
            sequence_node("main", vec![ChildEntry::reference("outer")]),
            sequence_node("outer", vec![ChildEntry::reference("inner")]),
            sequence_node("inner", vec![ChildEntry::reference("outer")]),
        ]);

        let errors = validate_all(&wf);
        assert!(
            errors.iter().any(|error| matches!(
                error,
                ValidationError::CompositeInclusionCycle { node, cycle }
                    if node == "outer" && cycle == "outer -> inner -> outer"
            )),
            "{errors:?}"
        );
        // 相互包含は W2 の子参照一意性にも触れるため、validate() の最初の
        // エラーは ChildReferenceViolation で安定する（循環自体は上の
        // collect_inclusion_cycle_errors の assert が固定する）。
        assert!(matches!(
            validate(&wf),
            Err(ValidationError::ChildReferenceViolation { .. })
        ));
    }

    #[test]
    fn test_包含循環_自己参照のsequenceを検出する() {
        let wf = workflow(vec![
            sequence_node("main", vec![ChildEntry::reference("part")]),
            sequence_node("part", vec![ChildEntry::reference("part")]),
        ]);

        assert!(validate_all(&wf).iter().any(|error| matches!(
            error,
            ValidationError::CompositeInclusionCycle { node, cycle }
                if node == "part" && cycle == "part -> part"
        )));
    }

    #[test]
    fn test_root_sequenceのapprovalが通る() {
        let mut root = sequence_node("main", vec![ChildEntry::reference("leaf")]);
        root.completion = NodeCompletion::require_approval();
        let wf = workflow(vec![root, command_node("leaf", "echo hi")]);

        assert!(validate(&wf).is_ok(), "{:?}", validate(&wf));
        assert!(validate_all(&wf).is_empty(), "{:?}", validate_all(&wf));
    }

    #[test]
    fn test_items検証_明示配線と型一致が通る() {
        let mut list = command_node("list", "echo '{}'");
        list.artifact = Some("scan".to_string());
        let mut worker = command_node("worker", "echo '{{ thread.thread_id }}'");
        worker.input = vec![typed_param("thread", "thread-ref")];
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![ChildEntry::reference("list"), ChildEntry::reference("fan")],
            ),
            list,
            fanout_node(
                "fan",
                vec![entry("worker", vec![("thread", "items")])],
                Some(ItemsSource::ArtifactField {
                    node: "list".to_string(),
                    field_path: crate::domain::workflow::FieldPath::new(["threads"]),
                }),
            ),
            worker,
        ]);
        wf.schemas
            .insert("thread-ref".to_string(), object_schema(&["thread_id"]));
        wf.schemas.insert(
            "scan".to_string(),
            SchemaDef::Object {
                properties: [(
                    "threads".to_string(),
                    SchemaDef::Array {
                        items: "thread-ref".to_string(),
                    },
                )]
                .into_iter()
                .collect(),
                required: ["threads".to_string()].into_iter().collect(),
            },
        );

        assert!(validate(&wf).is_ok(), "{:?}", validate(&wf));
    }

    #[test]
    fn test_items検証_多段の終端arrayと子inputの要素contractが一致する() {
        // Given
        let mut list = command_node("list", "echo '{}'");
        list.artifact = Some("scan".to_string());
        let mut worker = command_node("worker", "echo");
        worker.input = vec![typed_param("thread", "thread-ref")];
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![ChildEntry::reference("list"), ChildEntry::reference("fan")],
            ),
            list,
            fanout_node(
                "fan",
                vec![entry("worker", vec![("thread", "items")])],
                Some(ItemsSource::ArtifactField {
                    node: "list".to_string(),
                    field_path: crate::domain::workflow::FieldPath::new(["payload", "threads"]),
                }),
            ),
            worker,
        ]);
        wf.schemas
            .insert("thread-ref".to_string(), object_schema(&["thread_id"]));
        wf.schemas.insert(
            "scan".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::from([(
                    "payload".to_string(),
                    SchemaDef::Object {
                        properties: BTreeMap::from([(
                            "threads".to_string(),
                            SchemaDef::Array {
                                items: "thread-ref".to_string(),
                            },
                        )]),
                        required: BTreeSet::new(),
                    },
                )]),
                required: BTreeSet::new(),
            },
        );

        // When
        let errors = validate_all(&wf);

        // Then
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn test_items検証_多段の存在しない段と非object中間段と非array終端を拒否する() {
        // Given
        let mut list = command_node("list", "echo '{}'");
        list.artifact = Some("scan".to_string());
        let fan = |name: &str, worker: &str, path: &[&str]| {
            fanout_node(
                name,
                vec![ChildEntry::reference(worker)],
                Some(ItemsSource::ArtifactField {
                    node: "list".to_string(),
                    field_path: crate::domain::workflow::FieldPath::new(path.iter().copied()),
                }),
            )
        };
        let mut workers = ["worker-missing", "worker-scalar", "worker-not-array"]
            .map(|name| command_node(name, "echo"));
        for worker in &mut workers {
            worker.input = vec![untyped_param("item")];
        }
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("list"),
                    ChildEntry::reference("fan-missing"),
                    ChildEntry::reference("fan-scalar"),
                    ChildEntry::reference("fan-not-array"),
                ],
            ),
            list,
            fan("fan-missing", "worker-missing", &["payload", "missing"]),
            fan("fan-scalar", "worker-scalar", &["scalar", "leaf"]),
            fan(
                "fan-not-array",
                "worker-not-array",
                &["payload", "not-array"],
            ),
            workers[0].clone(),
            workers[1].clone(),
            workers[2].clone(),
        ]);
        wf.schemas.insert(
            "scan".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::from([
                    (
                        "payload".to_string(),
                        SchemaDef::Object {
                            properties: BTreeMap::from([(
                                "not-array".to_string(),
                                SchemaDef::String { r#enum: None },
                            )]),
                            required: BTreeSet::new(),
                        },
                    ),
                    ("scalar".to_string(), SchemaDef::Boolean),
                ]),
                required: BTreeSet::new(),
            },
        );

        // When
        let errors = validate_all(&wf);

        // Then
        let invalid_items = errors
            .iter()
            .filter(|error| matches!(error, ValidationError::InvalidFanoutItemsReference { .. }))
            .count();
        assert_eq!(invalid_items, 3, "{errors:?}");
    }

    #[test]
    fn test_items検証_受け手のいないitemsを拒否する() {
        let worker = command_node("worker", "echo hi");
        let mut list = command_node("list", "echo '{}'");
        list.artifact = Some("scan".to_string());
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![ChildEntry::reference("list"), ChildEntry::reference("fan")],
            ),
            list,
            fanout_node(
                "fan",
                vec![ChildEntry::reference("worker")],
                Some(ItemsSource::ArtifactField {
                    node: "list".to_string(),
                    field_path: crate::domain::workflow::FieldPath::new(["threads"]),
                }),
            ),
            worker,
        ]);
        wf.schemas
            .insert("thread-ref".to_string(), object_schema(&["thread_id"]));
        wf.schemas.insert(
            "scan".to_string(),
            SchemaDef::Object {
                properties: [(
                    "threads".to_string(),
                    SchemaDef::Array {
                        items: "thread-ref".to_string(),
                    },
                )]
                .into_iter()
                .collect(),
                required: ["threads".to_string()].into_iter().collect(),
            },
        );

        assert!(validate_all(&wf)
            .iter()
            .any(|error| matches!(error, ValidationError::FanoutInputMismatch { .. })));
    }

    #[test]
    fn test_items検証_単一パラメータへの自動束縛が通る() {
        let mut worker = command_node("worker", "echo '{{ task }}'");
        worker.input = vec![untyped_param("task")];
        let wf = workflow(vec![
            sequence_node("main", vec![ChildEntry::reference("fan")]),
            fanout_node(
                "fan",
                vec![ChildEntry::reference("worker")],
                Some(ItemsSource::Literal(vec![serde_json::json!("a")])),
            ),
            worker,
        ]);

        assert!(validate(&wf).is_ok(), "{:?}", validate(&wf));
    }

    #[test]
    fn test_本文検証_未宣言参照を拒否する() {
        let wf = workflow(vec![command_node("main", "echo '{{ item }}'")]);

        assert!(validate_all(&wf).iter().any(|error| matches!(
            error,
            ValidationError::InvalidArtifactReference {
                kind: InvalidArtifactReferenceKind::UnknownParameter,
                ..
            }
        )));
    }

    #[test]
    fn test_再帰カウント_上限超過を拒否する() {
        let mut nodes = vec![sequence_node(
            "main",
            (0..MAX_NODES_PER_WORKFLOW)
                .map(|i| ChildEntry::reference(format!("n{i}")))
                .collect(),
        )];
        for i in 0..MAX_NODES_PER_WORKFLOW {
            nodes.push(command_node(&format!("n{i}"), "echo hi"));
        }
        let wf = workflow(nodes);

        assert!(matches!(
            validate(&wf),
            Err(ValidationError::TooManyNodes { .. })
        ));
    }

    #[test]
    fn test_fanout_children数_上限超過を拒否する() {
        let child_names = (0..=MAX_FANOUT_CHILDREN)
            .map(|index| format!("child-{index}"))
            .collect::<Vec<_>>();
        let mut nodes = vec![fanout_node(
            "main",
            child_names.iter().map(ChildEntry::reference).collect(),
            None,
        )];
        nodes.extend(child_names.iter().map(|name| command_node(name, "echo hi")));
        let wf = workflow(nodes);

        assert!(matches!(
            validate(&wf),
            Err(ValidationError::TooManyFanoutChildren { node, count, max })
                if node == "main"
                    && count == MAX_FANOUT_CHILDREN + 1
                    && max == MAX_FANOUT_CHILDREN
        ));
    }

    #[test]
    fn test_検証_sessionのfacet無しを拒否する() {
        let mut node = session_node("main");
        if let NodeKind::Session(spec) = &mut node.kind {
            spec.facets = FacetRefs::default();
        }
        let wf = workflow(vec![node]);

        assert!(matches!(
            validate(&wf),
            Err(ValidationError::MissingFacet { .. })
        ));
    }

    #[test]
    fn test_検証_ルール付きループとloop_guardが通る() {
        let mut check = command_node("check", "echo '{}'");
        check.artifact = Some("verdict".to_string());
        let mut wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry {
                        name: "check".to_string(),
                        inputs: Vec::new(),
                        rules: Some(vec![Rule::When {
                            on: crate::domain::workflow::Predicate::Ref("done".to_string()),
                            then: "finish".to_string(),
                            next: "fix".to_string(),
                        }]),
                    },
                    ChildEntry {
                        name: "fix".to_string(),
                        inputs: Vec::new(),
                        rules: Some(vec![
                            Rule::LoopGuard {
                                max_iterations: 2,
                                on_exhausted: "finish".to_string(),
                            },
                            Rule::Next("check".to_string()),
                        ]),
                    },
                    ChildEntry::reference("finish"),
                ],
            ),
            check,
            command_node("fix", "echo fix"),
            command_node("finish", "echo done"),
        ]);
        wf.schemas.insert(
            "verdict".to_string(),
            SchemaDef::Object {
                properties: [("done".to_string(), SchemaDef::Boolean)]
                    .into_iter()
                    .collect(),
                required: ["done".to_string()].into_iter().collect(),
            },
        );

        assert!(validate(&wf).is_ok(), "{:?}", validate(&wf));
        assert!(validate_all(&wf).is_empty(), "{:?}", validate_all(&wf));
    }
}
