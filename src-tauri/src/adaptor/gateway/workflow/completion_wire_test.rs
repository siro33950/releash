use super::*;
use crate::domain::workflow::{NodeDefinition, NodeKind, WorkflowDefinition};
use serde_json::json;

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
            CompletionShapeError::UnknownField,
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
