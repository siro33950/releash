use super::*;

fn object(
    properties: impl IntoIterator<Item = (&'static str, SchemaDef)>,
    required: &[&str],
) -> SchemaDef {
    SchemaDef::Object {
        properties: properties
            .into_iter()
            .map(|(name, schema)| (name.to_string(), schema))
            .collect(),
        required: required.iter().map(|name| (*name).to_string()).collect(),
    }
}

fn path(reference: &str) -> FieldPath {
    FieldPath::from_reference(reference).unwrap().1
}

#[test]
fn test_field_path静的解決_多段の終端schemaと直上のrequiredを返す() {
    // Given
    let schema = object(
        [("outer", object([("leaf", SchemaDef::Boolean)], &["leaf"]))],
        &[],
    );

    // When
    let resolved = resolve_field_path(&schema, &path("root.outer.leaf")).unwrap();

    // Then
    assert_eq!(resolved.schema, &SchemaDef::Boolean);
    assert!(resolved.required);
}

#[test]
fn test_field_path静的解決_中間段にrequiredを要求しない() {
    // Given
    let schema = object(
        [(
            "optional",
            object([("leaf", SchemaDef::Boolean)], &["leaf"]),
        )],
        &[],
    );

    // When
    let result = resolve_field_path(&schema, &path("root.optional.leaf"));

    // Then
    assert!(result.is_ok());
}

#[test]
fn test_field_path静的解決_非objectの中間段と失敗位置を返す() {
    for non_object in [
        SchemaDef::Array {
            items: "item".to_string(),
        },
        SchemaDef::String { r#enum: None },
        SchemaDef::Boolean,
        SchemaDef::Integer,
        SchemaDef::Number,
    ] {
        // Given
        let schema = object([("value", non_object)], &["value"]);

        // When
        let result = resolve_field_path(&schema, &path("root.value.leaf"));

        // Then
        assert_eq!(
            result,
            Err(FieldPathResolutionError {
                position: 1,
                segment: "leaf".to_string(),
                kind: FieldPathResolutionErrorKind::NonObject,
            })
        );
    }
}

#[test]
fn test_field_path静的解決_存在しないfieldと失敗位置を返す() {
    // Given
    let schema = object(
        [("outer", object([("leaf", SchemaDef::Boolean)], &["leaf"]))],
        &["outer"],
    );

    // When
    let result = resolve_field_path(&schema, &path("root.outer.missing"));

    // Then
    assert_eq!(
        result,
        Err(FieldPathResolutionError {
            position: 1,
            segment: "missing".to_string(),
            kind: FieldPathResolutionErrorKind::MissingProperty,
        })
    );
}

#[test]
fn test_field_path静的解決_段0個は起点schemaを返す() {
    // Given
    let schema = SchemaDef::Number;
    let path = FieldPath::default();

    // When
    let resolved = resolve_field_path(&schema, &path).unwrap();

    // Then
    assert_eq!(resolved.schema, &schema);
    assert!(!resolved.required);
}

#[test]
fn test_command参照schema_artifactと予約fieldを合成する() {
    // Given
    let artifact = object([("payload", SchemaDef::Number)], &["payload"]);

    // When
    let schema = command_reference_schema(Some(&artifact)).unwrap();

    // Then
    let SchemaDef::Object {
        properties,
        required,
    } = schema
    else {
        panic!("command reference schema must be an object");
    };
    assert_eq!(properties.get("payload"), Some(&SchemaDef::Number));
    assert_eq!(properties.get("ok"), Some(&SchemaDef::Boolean));
    assert_eq!(properties.get("exit_code"), Some(&SchemaDef::Integer));
    assert_eq!(
        properties.get("stdout"),
        Some(&SchemaDef::String { r#enum: None })
    );
    assert_eq!(
        properties.get("stderr"),
        Some(&SchemaDef::String { r#enum: None })
    );
    assert_eq!(properties.get("duration"), Some(&SchemaDef::Integer));
    assert!(required.contains("payload"));
    assert!(COMMAND_RESERVED_FIELDS
        .iter()
        .all(|field| required.contains(*field)));
}

#[test]
fn test_command参照schema_artifact無しは予約fieldだけを合成する() {
    // Given / When
    let schema = command_reference_schema(None).unwrap();

    // Then
    let SchemaDef::Object {
        properties,
        required,
    } = schema
    else {
        panic!("command reference schema must be an object");
    };
    assert_eq!(properties.len(), COMMAND_RESERVED_FIELDS.len());
    assert_eq!(required.len(), COMMAND_RESERVED_FIELDS.len());
}

#[test]
fn test_command参照schema_artifactがobject以外なら拒否する() {
    // Given / When
    let result = command_reference_schema(Some(&SchemaDef::Boolean));

    // Then
    assert_eq!(result, Err(CommandReferenceSchemaError::ArtifactNotObject));
}
