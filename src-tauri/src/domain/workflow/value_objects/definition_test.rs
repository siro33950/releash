use super::*;

#[test]
fn test_環境変数名_形式とengine予約prefixを検証する() {
    assert_eq!(
        EnvironmentVariableName::new("DOC_2").unwrap().as_str(),
        "DOC_2"
    );
    assert_eq!(
        EnvironmentVariableName::new("_DOC").unwrap().as_str(),
        "_DOC"
    );
    assert!(matches!(
        EnvironmentVariableName::new("2DOC"),
        Err(EnvironmentVariableNameError::Invalid(_))
    ));
    assert!(matches!(
        EnvironmentVariableName::new("DOC-NAME"),
        Err(EnvironmentVariableNameError::Invalid(_))
    ));
    assert!(matches!(
        EnvironmentVariableName::new("RELEASH_WORKTREE_PATH"),
        Err(EnvironmentVariableNameError::Reserved(_))
    ));
}

#[test]
fn test_inputパラメータ参照_パラメータと多段fieldを受理する() {
    let parameter = InputParameterRef::new("document").unwrap();
    assert_eq!(parameter.parameter(), "document");
    assert!(parameter.field_path().is_empty());

    let field = InputParameterRef::new("document.body.text").unwrap();
    assert_eq!(field.parameter(), "document");
    assert_eq!(field.field_path().segments(), ["body", "text"]);
    assert_eq!(field.as_string(), "document.body.text");
    assert!(InputParameterRef::new("document bad").is_err());
}

#[test]
fn test_command_env_空mapを省略し既存snapshotを空mapとして読む() {
    let snapshot = r#"{
            "name":"wf",
            "description":"",
            "nodes":{"main":{"command":"true"}}
        }"#;

    let workflow = serde_json::from_str::<WorkflowDefinition>(snapshot).unwrap();
    let command = workflow.nodes[0].command_spec().unwrap();
    assert!(command.env.is_empty());

    let serialized = serde_json::to_value(&workflow).unwrap();
    assert_eq!(serialized["nodes"]["main"]["command"], "true");
    assert!(serialized["nodes"]["main"].get("env").is_none());
}

#[test]
fn test_command_env_宣言をdefinition_snapshotで往復する() {
    let workflow = serde_saphyr::from_str::<WorkflowDefinition>(
        r#"name: wf
description: test
nodes:
  main:
    command: printf
    env:
      DOC: document.body
    input:
      - document
"#,
    )
    .unwrap();

    let serialized = serde_json::to_string(&workflow).unwrap();
    let restored = serde_json::from_str::<WorkflowDefinition>(&serialized).unwrap();

    assert_eq!(restored, workflow);
    assert_eq!(
        restored.nodes[0]
            .command_spec()
            .unwrap()
            .env
            .get(&EnvironmentVariableName::new("DOC").unwrap())
            .map(InputParameterRef::as_string),
        Some("document.body".to_string())
    );
}

#[test]
fn test_fanout_items_多段artifact参照を直列化表記で往復する() {
    // Given
    let source = "plan.payload.targets";

    // When
    let items = serde_json::from_value::<ItemsSource>(Value::String(source.to_string())).unwrap();
    let serialized = serde_json::to_value(&items).unwrap();

    // Then
    assert_eq!(
        items,
        ItemsSource::ArtifactField {
            node: "plan".to_string(),
            field_path: FieldPath::new(["payload", "targets"]),
        }
    );
    assert_eq!(serialized, Value::String(source.to_string()));
}

#[test]
fn test_session_permission_4値は文字列構築とserdeで同じ値を往復する() {
    let cases = [
        ("manual", SessionPermission::Manual),
        ("auto", SessionPermission::Auto),
        ("bypass", SessionPermission::Bypass),
        ("read-only", SessionPermission::ReadOnly),
    ];

    for (serialized, expected) in cases {
        assert_eq!(serialized.parse::<SessionPermission>().unwrap(), expected);
        assert_eq!(expected.as_str(), serialized);
        assert_eq!(
            serde_json::to_string(&expected).unwrap(),
            format!("\"{serialized}\"")
        );
        assert_eq!(
            serde_json::from_str::<SessionPermission>(&format!("\"{serialized}\"")).unwrap(),
            expected
        );
        assert_eq!(
            serde_saphyr::from_str::<SessionPermission>(serialized).unwrap(),
            expected
        );
    }
}

#[test]
fn test_session_permission_未知値とprovider固有値は文字列構築とserdeで拒否する() {
    for invalid in [
        "unknown",
        "acceptEdits",
        "danger-full-access",
        "workspace-write",
        "bypassPermissions",
        "plan",
    ] {
        assert!(invalid.parse::<SessionPermission>().is_err());
        assert!(serde_json::from_str::<SessionPermission>(&format!("\"{invalid}\"")).is_err());
        assert!(serde_saphyr::from_str::<SessionPermission>(invalid).is_err());
    }
}

// serde_json の直接デシリアライズ（イベント payload 復元経路）は上流に
// 重複キー拒否が無いため、children エントリ本体の全フィールドが
// 自前の重複検出で守られていることを検証する。
#[test]
fn test_子エントリ本体は重複completionキーを拒否する() {
    let error = serde_json::from_str::<RawChildBody>(
        r#"{"command":"echo hi","completion":"auto","completion":"approval"}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("duplicate field `completion`"));
}

#[test]
fn test_子エントリ本体は空リスト後の重複inputキーを拒否する() {
    let error = serde_json::from_str::<RawChildBody>(
        r#"{"command":"echo hi","input":[],"input":["spec"]}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("duplicate field `input`"));
}

#[test]
fn test_onfailure_ignoreとretryが子エントリの扱いとしてパースされる() {
    let body = serde_json::from_str::<RawChildBody>(r#"{"on_failure":"ignore"}"#).unwrap();
    assert_eq!(body.on_failure, Some(OnFailure::Ignore));

    let body = serde_json::from_str::<RawChildBody>(r#"{"on_failure":{"retry":3}}"#).unwrap();
    assert_eq!(body.on_failure, Some(OnFailure::Retry(3)));
}

#[test]
fn test_onfailure_不正な値を拒否する() {
    let error = serde_json::from_str::<OnFailure>(r#""abort""#).unwrap_err();
    assert!(error.to_string().contains("on_failure must be"));

    let error = serde_json::from_str::<OnFailure>(r#"{"retry":0}"#).unwrap_err();
    assert!(error.to_string().contains("at least 1"));

    let error = serde_json::from_str::<OnFailure>(r#"{"retry":1,"backoff":true}"#).unwrap_err();
    assert!(error.to_string().contains("only field"));

    let error =
        serde_json::from_str::<RawChildBody>(r#"{"on_failure":"ignore","on_failure":"ignore"}"#)
            .unwrap_err();
    assert!(error.to_string().contains("duplicate field `on_failure`"));
}

#[test]
fn test_onfailure_子エントリのserializeで往復する() {
    let ignore = ChildEntry {
        on_failure: Some(OnFailure::Ignore),
        ..ChildEntry::reference("flaky")
    };
    assert_eq!(
        serde_json::to_value(&ignore).unwrap(),
        serde_json::json!({"flaky": {"on_failure": "ignore"}})
    );

    let retry = ChildEntry {
        on_failure: Some(OnFailure::Retry(2)),
        ..ChildEntry::reference("flaky")
    };
    assert_eq!(
        serde_json::to_value(&retry).unwrap(),
        serde_json::json!({"flaky": {"on_failure": {"retry": 2}}})
    );

    // 扱いなしは文字列参照へ畳まれる（現行のまま）。
    assert_eq!(
        serde_json::to_value(ChildEntry::reference("plain")).unwrap(),
        serde_json::json!("plain")
    );
}

#[test]
fn test_node_definition_facet参照を検出する() {
    let mut node = NodeDefinition::default();
    assert!(!node.has_facet_refs());
    if let Some(session) = node.session_mut() {
        session.facets.knowledge = vec!["releash-thread-cli".to_string()];
    }
    assert!(node.has_facet_refs());
}

#[test]
fn test_entry解決_mainが先頭以外でもrootとして解決される() {
    let node = |name: &str| NodeDefinition {
        name: name.to_string(),
        ..Default::default()
    };
    let workflow = WorkflowDefinition {
        nodes: vec![node("helper"), node("main"), node("done")],
        ..Default::default()
    };

    assert_eq!(workflow.entry_index(), Some(1));
    assert_eq!(workflow.entry_node().map(|n| n.name.as_str()), Some("main"));
}

#[test]
fn test_entry解決_entryと同名nodeが無ければ解決しない() {
    let workflow = WorkflowDefinition {
        nodes: vec![NodeDefinition {
            name: "helper".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };

    assert_eq!(workflow.entry_index(), None);
    assert!(workflow.entry_node().is_none());
}

#[test]
fn test_実効辺_rules省略は隣接辺で末尾は終端() {
    let sequence = SequenceSpec {
        entry: None,
        children: vec![
            ChildEntry::reference("first"),
            ChildEntry::reference("second"),
        ],
    };

    assert_eq!(
        sequence.effective_rules("first"),
        EffectiveRules::AdjacentNext("second")
    );
    assert_eq!(sequence.effective_rules("second"), EffectiveRules::Terminal);
    assert_eq!(
        sequence.effective_rules("unlisted"),
        EffectiveRules::Terminal
    );
}

#[test]
fn test_実効辺_明示rulesが隣接辺より優先され空rulesは終端() {
    let sequence = SequenceSpec {
        entry: None,
        children: vec![
            ChildEntry {
                on_failure: None,
                name: "first".to_string(),
                inputs: Vec::new(),
                rules: Some(vec![Rule::Next("third".to_string())]),
            },
            ChildEntry {
                on_failure: None,
                name: "second".to_string(),
                inputs: Vec::new(),
                rules: Some(Vec::new()),
            },
            ChildEntry::reference("third"),
        ],
    };

    assert_eq!(
        sequence.effective_rules("first"),
        EffectiveRules::Rules(&[Rule::Next("third".to_string())][..])
    );
    assert_eq!(
        sequence.effective_rules("second"),
        EffectiveRules::Rules(&[][..])
    );
}

#[test]
fn test_entry解決_sequenceのentry省略時はリスト先頭() {
    let sequence = SequenceSpec {
        entry: None,
        children: vec![
            ChildEntry::reference("first"),
            ChildEntry::reference("second"),
        ],
    };
    assert_eq!(sequence.entry_child_name(), Some("first"));

    let explicit = SequenceSpec {
        entry: Some("second".to_string()),
        ..sequence
    };
    assert_eq!(explicit.entry_child_name(), Some("second"));
}

#[test]
fn test_node名前空間_明示名と自動生成名を同じ空間で一意にする() {
    let mut namespace = NodeNamespace::default();

    assert_eq!(namespace.register_explicit("prepare").unwrap(), "prepare");
    assert_eq!(namespace.register_synthesized("main", 0).unwrap(), "main#0");
    assert!(namespace.contains("prepare"));
    assert!(namespace.contains("main#0"));
    assert_eq!(
        namespace.register("main#0"),
        Err(NodeNamespaceError::Duplicate("main#0".to_string()))
    );
}

#[test]
fn test_node名前空間_明示名では予約語を拒否する() {
    let mut namespace = NodeNamespace::default();

    assert_eq!(
        namespace.register_explicit("children"),
        Err(NodeNamespaceError::Reserved("children".to_string()))
    );
    assert!(!namespace.contains("children"));
    assert_eq!(
        namespace.register_explicit("artifact"),
        Err(NodeNamespaceError::Reserved("artifact".to_string()))
    );
    assert!(!namespace.contains("artifact"));
    assert!(namespace.register_explicit("output").is_ok());
    assert!(namespace.contains("output"));
}

#[test]
fn test_fanoutキー_itemsなしではエントリ名で相互解決する() {
    // Given
    let spec = FanoutSpec {
        children: vec![ChildEntry::reference("a"), ChildEntry::reference("b")],
        items: None,
    };

    for (child_index, name) in ["a", "b"].into_iter().enumerate() {
        // When
        let key = spec.artifact_key(FanoutSlot {
            item_index: None,
            child_index,
        });
        let entry = spec.child_entry_for_artifact_key(name);

        // Then
        assert_eq!(key.as_deref(), Some(name));
        assert_eq!(entry, spec.children.get(child_index));
    }
    assert_eq!(spec.child_entry_for_artifact_key("unknown"), None);
    assert_eq!(spec.child_entry_for_artifact_key("0"), None);
}

#[test]
fn test_fanoutキー_itemsありでは単一と複数childrenをフラットな添字で相互解決する() {
    // Given
    for names in [vec!["a"], vec!["a", "b"]] {
        let spec = FanoutSpec {
            children: names.into_iter().map(ChildEntry::reference).collect(),
            items: Some(ItemsSource::Literal(vec![])),
        };
        for item_index in 0..3 {
            for child_index in 0..spec.children.len() {
                // When
                let key = spec
                    .artifact_key(FanoutSlot {
                        item_index: Some(item_index),
                        child_index,
                    })
                    .unwrap();
                let entry = spec.child_entry_for_artifact_key(&key);

                // Then
                assert_eq!(
                    key,
                    (item_index * spec.children.len() + child_index).to_string()
                );
                assert_eq!(entry, spec.children.get(child_index));
            }
        }
    }
}

#[test]
fn test_fanoutキー_非正準な添字は解決しない() {
    // Given
    let spec = FanoutSpec {
        children: vec![ChildEntry::reference("a")],
        items: Some(ItemsSource::Literal(vec![])),
    };
    for key in [
        "007",
        "00",
        "+1",
        "1.0",
        " 1",
        "1 ",
        "-1",
        "",
        "a",
        "18446744073709551616",
    ] {
        // When
        let entry = spec.child_entry_for_artifact_key(key);

        // Then
        assert_eq!(entry, None, "{key}");
    }
}

#[test]
fn test_fanoutキー_空childrenと不正な座標は解決しない() {
    // Given
    for items in [None, Some(ItemsSource::Literal(vec![]))] {
        let spec = FanoutSpec {
            children: vec![],
            items,
        };

        // When / Then
        assert_eq!(
            spec.artifact_key(FanoutSlot {
                item_index: Some(0),
                child_index: 0
            }),
            None
        );
        assert_eq!(spec.child_entry_for_artifact_key("0"), None);
    }
    let spec = FanoutSpec {
        children: vec![ChildEntry::reference("a"), ChildEntry::reference("b")],
        items: Some(ItemsSource::Literal(vec![])),
    };
    for slot in [
        FanoutSlot {
            item_index: None,
            child_index: 0,
        },
        FanoutSlot {
            item_index: Some(0),
            child_index: 2,
        },
        FanoutSlot {
            item_index: Some(usize::MAX),
            child_index: 0,
        },
    ] {
        // When / Then
        assert_eq!(spec.artifact_key(slot), None);
    }
}

#[test]
fn test_述語定義の保存_単一参照と合成とネストの構造と遷移先を保持する() {
    // Given
    for on in [
        serde_json::json!("passed"),
        serde_json::json!({"and": ["passed", "details.passed"]}),
        serde_json::json!({"or": ["passed"]}),
        serde_json::json!({"and": ["passed", {"or": ["clean", "skipped"]}]}),
    ] {
        let value = serde_json::json!({"when": {"on": on, "then": "done"}, "next": "fix"});
        let rule: Rule = serde_json::from_value(value.clone()).unwrap();
        // When
        let encoded = serde_json::to_value(&rule).unwrap();
        let decoded: Rule =
            serde_saphyr::from_str(&serde_saphyr::to_string(&rule).unwrap()).unwrap();
        // Then
        assert_eq!(encoded, value);
        assert_eq!(decoded, rule);
    }
}

#[test]
fn test_述語定義の保存_workflow全体の現在形式を再読込できる() {
    // Given
    let source =
        include_str!("../../../adaptor/gateway/workflow/fixtures/valid/predicate-routing.yml");
    for on in [
        "passed",
        "{and: [passed, details.passed]}",
        "{or: [clean, skipped]}",
        "{and: [passed, {or: [clean, skipped]}]}",
    ] {
        let workflow: WorkflowDefinition =
            serde_saphyr::from_str(&source.replace("{and: [passed, {or: [clean, skipped]}]}", on))
                .unwrap();
        // When
        let serialized = serde_json::to_string(&workflow).unwrap();
        let restored: WorkflowDefinition = serde_json::from_str(&serialized).unwrap();
        // Then
        assert_eq!(restored, workflow);
    }
}
