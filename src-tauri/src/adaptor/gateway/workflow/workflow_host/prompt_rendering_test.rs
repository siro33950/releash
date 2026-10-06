use super::*;
use crate::domain::workflow::WorkflowDefinition;

#[test]
fn test_delegate_起動指示は同一sessionでの再提出とturn終了と予約キーを説明する() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str("name: test\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: implement}}, artifact: result, completion: {delegate: {child: check, when: child.ok, max_iterations: 2}}}\n  check: {command: check}\nschemas:\n  result: {type: object, properties: {done: {type: boolean}}, required: [done]}").unwrap();
    let facets = crate::domain::workflow::FacetContents {
        instruction: Some("implement".into()),
        ..Default::default()
    };
    // When
    let (_, prompt) = super::build_leaf_prompt(
        workflow.node_by_name("main").unwrap(),
        Some(&facets),
        "node-1",
        &[],
        &workflow.schemas,
    )
    .unwrap();
    // Then
    assert!(prompt.contains("同じnode-executionへArtifactを再提出"));
    assert!(prompt.contains("childキーはengineが管理"));
    assert!(prompt.contains("turnを終了"));
    assert!(prompt.contains("--node-execution node-1"));
    assert!(prompt.contains("--type result"));
}

#[test]
fn test_commandテンプレート_既存のparameterとfield展開を維持する() {
    let bindings = vec![
        (
            "document".to_string(),
            Value::String("it's {{ literal }}".to_string()),
        ),
        ("metadata".to_string(), serde_json::json!({"count": 2})),
    ];

    let rendered = render_parameter_references(
        "printf '%s' '{{ document }}'; printf '%s' '{{ metadata.count }}'",
        &bindings,
    );

    assert_eq!(
        rendered,
        "printf '%s' 'it's {{ literal }}'; printf '%s' '2'"
    );
}

#[test]
fn test_commandテンプレート_未解決参照を従来どおり残す() {
    assert_eq!(
        render_parameter_references("echo '{{ missing }}'", &[]),
        "echo '{{ missing }}'"
    );
}

#[test]
fn test_commandテンプレート_多段fieldを終端値へ展開する() {
    // Given
    let bindings = vec![(
        "document".to_string(),
        serde_json::json!({"outer": {"inner": {"text": "rendered"}}}),
    )];

    // When
    let rendered = render_parameter_references("echo '{{ document.outer.inner.text }}'", &bindings);

    // Then
    assert_eq!(rendered, "echo 'rendered'");
}

#[test]
fn test_sessionファセット_システムとユーザー本文の多段fieldを展開する() {
    // Given
    let node = NodeDefinition {
        name: "main".to_string(),
        kind: crate::domain::workflow::NodeKind::Session(crate::domain::workflow::SessionSpec {
            facets: crate::domain::workflow::FacetRefs {
                policy: Some("policy".to_string()),
                instruction: Some("instruction".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }),
        input: vec![crate::domain::workflow::InputParam {
            name: "context".to_string(),
            contract: None,
        }],
        ..Default::default()
    };
    let facets = FacetContents {
        policy: Some("Policy {{ context.outer.value }}".to_string()),
        instruction: Some("Do {{ context.outer.value }}".to_string()),
        ..Default::default()
    };
    let bindings = vec![(
        "context".to_string(),
        serde_json::json!({"outer": {"value": "nested"}}),
    )];

    // When
    let (system, user) = build_leaf_prompt(
        &node,
        Some(&facets),
        "00000000-0000-4000-8000-000000000001",
        &bindings,
        &BTreeMap::new(),
    )
    .unwrap();

    // Then
    assert!(system.unwrap().contains("Policy nested"));
    assert!(user.contains("Do nested"));
}
