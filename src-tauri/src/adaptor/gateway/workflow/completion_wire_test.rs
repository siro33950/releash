use super::*;
use crate::domain::workflow::{NodeDefinition, NodeKind, WorkflowDefinition};
use serde_json::json;

#[test]
fn test_completion保存形式_要求なしの直接シリアライズを拒否する() {
    // Given
    let completion = NodeCompletion::default();
    // When
    let result = serde_json::to_value(&completion);
    // Then
    assert_eq!(
        result.unwrap_err().to_string(),
        "completion must contain at least one requirement"
    );
}

#[test]
fn test_completion保存形式_承認要求を直接シリアライズして復元できる() {
    // Given
    let completion = NodeCompletion::require_approval();
    // When
    let serialized = serde_json::to_value(&completion).unwrap();
    let restored: NodeCompletion = serde_json::from_value(serialized.clone()).unwrap();
    // Then
    assert_eq!(serialized, json!({"require": "approval"}));
    assert_eq!(restored, completion);
}

#[test]
fn test_completion保存形式_要求をmapにし要求なしはnodeから省略する() {
    // Given
    for (completion, expected) in [
        (NodeCompletion::default(), json!({"command": "true"})),
        (
            NodeCompletion::require_approval(),
            json!({"command": "true", "completion": {"require": "approval"}}),
        ),
    ] {
        let workflow = WorkflowDefinition {
            name: "completion".into(),
            nodes: vec![NodeDefinition {
                name: "main".into(),
                kind: NodeKind::Command(crate::domain::workflow::CommandSpec {
                    command: "true".into(),
                    env: Default::default(),
                }),
                completion,
                ..Default::default()
            }],
            entry: "main".into(),
            ..Default::default()
        };
        // When
        let serialized = serde_json::to_value(&workflow).unwrap();
        let restored: WorkflowDefinition = serde_json::from_value(serialized.clone()).unwrap();
        // Then
        assert_eq!(serialized["nodes"]["main"], expected);
        assert_eq!(restored, workflow);
    }
}

#[test]
fn test_completion保存形式_旧形式と不正な要求を拒否する() {
    // Given
    for (value, expected) in [
        (json!("auto"), CompletionShapeError::ExpectedMap),
        (json!("approval"), CompletionShapeError::ExpectedMap),
        (json!(null), CompletionShapeError::ExpectedMap),
        (json!(true), CompletionShapeError::ExpectedMap),
        (json!([]), CompletionShapeError::ExpectedMap),
        (json!({}), CompletionShapeError::Empty),
        (
            json!({"require": "auto"}),
            CompletionShapeError::InvalidRequirement,
        ),
        (
            json!({"require": "unknown"}),
            CompletionShapeError::InvalidRequirement,
        ),
        (
            json!({"require": null}),
            CompletionShapeError::InvalidRequirement,
        ),
        (
            json!({"require": 1}),
            CompletionShapeError::InvalidRequirement,
        ),
        (
            json!({"require": {}}),
            CompletionShapeError::InvalidRequirement,
        ),
        (
            json!({"delegate": "session"}),
            CompletionShapeError::InvalidDelegate("completion delegate must be a map".into()),
        ),
        (
            json!({"require": "approval", "unknown": true}),
            CompletionShapeError::UnknownField,
        ),
    ] {
        // When
        let result = parse_completion(&value);
        let deserialized = serde_json::from_value::<NodeCompletion>(value.clone());
        // Then
        assert_eq!(result.unwrap_err(), expected, "{value}");
        assert!(deserialized.is_err(), "{value}");
    }
}

#[test]
fn test_delegate変換_整数ゼロはdomainの値域検証へ渡す() {
    // Given
    let wire = json!({"delegate": {"child": "verify", "when": "child.ok", "max_iterations": 0}});
    // When
    let completion = parse_completion(&wire).unwrap();
    // Then
    assert_eq!(completion.delegate.unwrap().max_iterations, 0);
}

#[test]
fn test_delegate配線_childrenと同じ宣言順で読み保存復元後も保持する() {
    // Given
    let source = "name: ordered\ndescription: test\nnodes:\n  main:\n    sequence:\n      children:\n        - parent: {inputs: {zebra: request, apple: request, middle: request}}\n  parent:\n    session: {provider: codex}\n    artifact: result\n    completion:\n      delegate:\n        child: check\n        inputs: {zebra: request, apple: request, middle: request}\n        when: child.ok\n        max_iterations: 2\n  check: {command: check}\n";
    // When
    let definition: WorkflowDefinition = serde_saphyr::from_str(source).unwrap();
    let serialized = serde_json::to_string(&definition).unwrap();
    let restored: WorkflowDefinition = serde_json::from_str(&serialized).unwrap();
    // Then
    for definition in [&definition, &restored] {
        let children_inputs = &definition
            .node_by_name("main")
            .unwrap()
            .sequence()
            .unwrap()
            .children[0]
            .inputs;
        let delegate_inputs = &definition
            .node_by_name("parent")
            .unwrap()
            .completion
            .delegate
            .as_ref()
            .unwrap()
            .inputs;
        assert_eq!(delegate_inputs, children_inputs);
        let bindings = crate::domain::workflow::services::reference::resolve_entry_bindings(
            Some(
                &definition
                    .node_by_name("parent")
                    .unwrap()
                    .completion
                    .delegate
                    .as_ref()
                    .unwrap()
                    .child_entry(),
            ),
            &std::collections::HashMap::from([("request".into(), json!("requested"))]),
        );
        let prompt = super::super::workflow_host::prompt_rendering::inject_input_parameters(
            "check",
            definition.node_by_name("check").unwrap(),
            &bindings,
        );
        assert!(prompt.find("## input: zebra").unwrap() < prompt.find("## input: apple").unwrap());
        assert!(prompt.find("## input: apple").unwrap() < prompt.find("## input: middle").unwrap());
        assert_eq!(
            delegate_inputs
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["zebra", "apple", "middle"]
        );
    }
}

#[test]
fn test_delegate配線_childrenと同じく重複パラメータと空の供給元を拒否する() {
    // Given
    for inputs in ["{task: request, task: request}", "{task: ' '}"] {
        let delegate = format!("name: test\ndescription: test\nnodes:\n  main: {{session: {{provider: codex}}, artifact: result, completion: {{delegate: {{child: check, inputs: {inputs}, when: child.ok, max_iterations: 2}}}}}}\n  check: {{command: check}}");
        let sequence = format!("name: test\ndescription: test\nnodes:\n  main: {{sequence: {{children: [{{check: {{inputs: {inputs}}}}}]}}}}\n  check: {{command: check}}");
        // When / Then
        for source in [delegate, sequence] {
            assert!(
                serde_saphyr::from_str::<WorkflowDefinition>(&source).is_err(),
                "{source}"
            );
        }
    }
}
