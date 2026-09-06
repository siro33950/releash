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
