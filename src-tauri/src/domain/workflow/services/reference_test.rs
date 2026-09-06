use super::*;

#[test]
fn test_sequence参照schema_入れ子を合成し成果のないsessionとfanoutを除く() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(
        r#"
name: nested-schema
description: test
schemas:
  result:
    type: object
    properties:
      passed: {type: boolean}
    required: [passed]
nodes:
  main: {sequence: {children: [nested, silent, fan, command_leaf]}}
  nested: {sequence: {children: [writer]}}
  writer: {session: {provider: codex}, artifact: result}
  silent: {session: {provider: codex}}
  fan: {fanout: {children: [worker]}}
  worker: {command: work}
  command_leaf: {command: check}
"#,
    )
    .unwrap();

    // When
    let node = workflow.node_by_name("main").unwrap();
    let schema = node_reference_schema(&workflow, node).unwrap();

    // Then
    assert!(node_has_artifact(node));
    let SchemaDef::Object {
        properties,
        required,
    } = &schema
    else {
        panic!("expected Object")
    };
    assert!(required.is_empty());
    assert_eq!(
        properties.keys().map(String::as_str).collect::<Vec<_>>(),
        ["command_leaf", "nested"]
    );
    let SchemaDef::Object { required, .. } = &properties["nested"] else {
        panic!("expected nested Object")
    };
    assert!(required.is_empty());
    let resolved = crate::domain::workflow::services::contract_schema::resolve_field_path(
        &schema,
        &FieldPath::from_dotted("nested.writer.passed").unwrap(),
    )
    .unwrap();
    assert_eq!(resolved.schema, &SchemaDef::Boolean);
    assert!(resolved.required);
    let command = crate::domain::workflow::services::contract_schema::resolve_field_path(
        &schema,
        &FieldPath::from_dotted("command_leaf.ok").unwrap(),
    )
    .unwrap();
    assert_eq!(command.schema, &SchemaDef::Boolean);
    assert!(command.required);
}

#[test]
fn test_sequence参照schema_包含cycleへの再訪を打ち切る() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str(
        r#"
name: cycle-schema
description: test
nodes:
  main:
    sequence:
      children:
        - part
        - consumer:
            inputs: {data: part.main.part}
  part: {sequence: {children: [main]}}
  consumer: {command: consume, input: [data]}
"#,
    )
    .unwrap();

    // When
    let schema = node_reference_schema(&workflow, workflow.node_by_name("main").unwrap()).unwrap();
    let errors = crate::domain::workflow::services::validation::validate_all(&workflow);

    // Then
    let SchemaDef::Object { properties, .. } = schema else {
        panic!("expected Object")
    };
    assert_eq!(
        properties["part"],
        SchemaDef::Object {
            properties: Default::default(),
            required: Default::default()
        }
    );
    assert!(errors.iter().any(|error| matches!(error,
        crate::domain::workflow::services::validation::ValidationError::CompositeInclusionCycle { .. }
    )));
}

#[test]
fn test_node参照schema_非objectのcommand契約と参照不能な成果を区別する() {
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
        node_reference_schema(&workflow, workflow.node_by_name("main").unwrap()),
        Err(NodeReferenceSchemaError::ArtifactNotObject)
    );
    for name in ["silent", "missing"] {
        assert_eq!(
            node_reference_schema(&workflow, workflow.node_by_name(name).unwrap()),
            Err(NodeReferenceSchemaError::NoReferenceableArtifact)
        );
    }
}

#[test]
fn test_sequence参照schema_別の枝で訪問済みの子も再展開しない() {
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

    // When
    let schema = node_reference_schema(&workflow, workflow.node_by_name("main").unwrap()).unwrap();

    // Then
    let SchemaDef::Object { properties, .. } = schema else {
        panic!("expected Object")
    };
    let SchemaDef::Object {
        properties: left, ..
    } = &properties["left"]
    else {
        panic!("expected Object")
    };
    assert!(left.contains_key("shared"));
    assert_eq!(
        properties["right"],
        SchemaDef::Object {
            properties: Default::default(),
            required: Default::default()
        }
    );
}
