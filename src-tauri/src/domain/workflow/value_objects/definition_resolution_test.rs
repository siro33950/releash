use super::*;

#[test]
fn test_定義解釈_無関係な未対応定義は対象nodeを制限しない() {
    // Given
    let mut resolution = DefinitionResolution::default();
    resolution
        .node_errors
        .insert("unused".into(), "unknown field".into());

    let definition = WorkflowDefinition {
        nodes: vec![super::super::NodeDefinition {
            name: "current".into(),
            ..Default::default()
        }],
        ..Default::default()
    };

    // When / Then
    assert_eq!(resolution.node_error(&definition, "current"), None);
    assert!(resolution.node_error(&definition, "unused").is_some());
}
