mod contract_schema_tests {
    use super::super::*;
    use std::collections::BTreeSet;

    fn object(properties: BTreeMap<String, SchemaDef>, required: &[&str]) -> SchemaDef {
        SchemaDef::Object {
            properties,
            required: required.iter().map(|value| (*value).to_string()).collect(),
        }
    }

    #[test]
    fn test_schema検証_object_required_boolean_enumを検証する() {
        let schema = object(
            BTreeMap::from([
                ("ok".to_string(), SchemaDef::Boolean),
                (
                    "verdict".to_string(),
                    SchemaDef::String {
                        r#enum: Some(vec!["SHIP".to_string(), "HOLD".to_string()]),
                    },
                ),
            ]),
            &["ok", "verdict"],
        );
        assert!(validate(
            &serde_json::json!({"ok": true, "verdict": "SHIP"}),
            &schema,
            &BTreeMap::new(),
        )
        .is_ok());
        let violations = validate(
            &serde_json::json!({"ok": "yes", "extra": 1}),
            &schema,
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert!(violations.iter().any(|v| v.path == "$.ok"));
        assert!(violations.iter().any(|v| v.path == "$.verdict"));
        assert!(!violations.iter().any(|v| v.path == "$.extra"));
    }

    #[test]
    fn test_schema検証_objectは未宣言fieldを受理する() {
        let schema = object(
            BTreeMap::from([("spec_dir".to_string(), SchemaDef::String { r#enum: None })]),
            &["spec_dir"],
        );

        assert!(validate(
            &serde_json::json!({
                "spec_dir": "docs/specs/issues-1469",
                "spec_id": "issues-1469",
                "metadata": {"source": "agent"}
            }),
            &schema,
            &BTreeMap::new(),
        )
        .is_ok());
    }

    #[test]
    fn test_schema検証_array_itemsは名前付きschema参照を使う() {
        let item = object(
            BTreeMap::from([("thread_id".to_string(), SchemaDef::String { r#enum: None })]),
            &["thread_id"],
        );
        let schemas = BTreeMap::from([("thread".to_string(), item)]);
        let schema = SchemaDef::Array {
            items: "thread".to_string(),
        };
        assert!(validate(&serde_json::json!([{"thread_id": "1"}]), &schema, &schemas,).is_ok());
        assert!(validate(&serde_json::json!([{}]), &schema, &schemas).is_err());
    }

    #[test]
    fn test_schema検証_spec_dirという名前だけではpath制約を適用しない() {
        let schema = object(
            BTreeMap::from([("spec_dir".to_string(), SchemaDef::String { r#enum: None })]),
            &["spec_dir"],
        );
        for value in [
            "docs/specs/issues-123",
            "/tmp/spec",
            "../outside",
            "docs/specs/",
            "C:\\tmp\\spec",
        ] {
            assert!(
                validate(
                    &serde_json::json!({"spec_dir": value}),
                    &schema,
                    &BTreeMap::new(),
                )
                .is_ok(),
                "generic schema engine must not infer path constraints from field name for {value}"
            );
        }
    }

    #[test]
    fn test_schema_def_from_jsonは全type分岐を構築する() {
        assert_eq!(
            schema_def_from_json(&serde_json::json!("string")).unwrap(),
            SchemaDef::String { r#enum: None }
        );
        assert_eq!(
            schema_def_from_json(&serde_json::json!({"type": "boolean"})).unwrap(),
            SchemaDef::Boolean
        );
        assert_eq!(
            schema_def_from_json(&serde_json::json!({"type": "integer"})).unwrap(),
            SchemaDef::Integer
        );
        assert_eq!(
            schema_def_from_json(&serde_json::json!({"type": "number"})).unwrap(),
            SchemaDef::Number
        );
        assert_eq!(
            schema_def_from_json(&serde_json::json!({"type": "array", "items": "thread"})).unwrap(),
            SchemaDef::Array {
                items: "thread".to_string()
            }
        );
        assert_eq!(
            schema_def_from_json(&serde_json::json!({
                "type": "string",
                "enum": ["LGTM", "NEEDS_FIX"]
            }))
            .unwrap(),
            SchemaDef::String {
                r#enum: Some(vec!["LGTM".to_string(), "NEEDS_FIX".to_string()])
            }
        );

        let object_schema = schema_def_from_json(&serde_json::json!({
            "type": "object",
            "properties": {"verdict": {"type": "string", "enum": ["LGTM"]}},
            "required": ["verdict"]
        }))
        .unwrap();
        assert!(matches!(object_schema, SchemaDef::Object { .. }));
    }

    #[test]
    fn test_schema_def_from_jsonは不正構文を固定する() {
        for (value, expected) in [
            (
                serde_json::json!(true),
                "schema must be an object or scalar 'string'",
            ),
            (
                serde_json::json!("number"),
                "scalar schema supports only 'string', got 'number'",
            ),
            (
                serde_json::json!({"type": "array", "items": ""}),
                "array.items must be a non-empty Contract name",
            ),
            (
                serde_json::json!({"type": "string", "enum": "LGTM"}),
                "enum must be an array",
            ),
            (
                serde_json::json!({"type": "object", "items": "x"}),
                "object schema supports only properties and required",
            ),
            (
                serde_json::json!({"type": "object", "future_keyword": false}),
                "object schema supports only properties and required",
            ),
            (
                serde_json::json!({"type": "boolean", "enum": ["yes"]}),
                "boolean, integer, and number schemas do not support extra keywords",
            ),
            (
                serde_json::json!({"type": "unknown"}),
                "unsupported schema type 'unknown'",
            ),
        ] {
            assert_eq!(schema_def_from_json(&value).unwrap_err(), expected);
        }
    }

    #[test]
    fn test_schema_def_from_jsonは全型の未知keywordを拒否する() {
        for value in [
            serde_json::json!({"type": "object", "properties": {}, "future_keyword": "x"}),
            serde_json::json!({"type": "array", "items": "review", "future_keyword": "x"}),
            serde_json::json!({"type": "string", "enum": ["LGTM"], "future_keyword": "x"}),
            serde_json::json!({"type": "boolean", "future_keyword": "x"}),
            serde_json::json!({"type": "integer", "future_keyword": "x"}),
            serde_json::json!({"type": "number", "future_keyword": "x"}),
        ] {
            assert!(
                schema_def_from_json(&value).is_err(),
                "unknown keyword must be rejected: {value}"
            );
        }
    }

    #[test]
    fn test_schema_def_to_json_valueは単一renderを行う() {
        let schema = object(
            BTreeMap::from([(
                "verdict".to_string(),
                SchemaDef::String {
                    r#enum: Some(vec!["LGTM".to_string()]),
                },
            )]),
            &["verdict"],
        );

        assert_eq!(
            schema_def_to_json_value(&schema),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "verdict": {"type": "string", "enum": ["LGTM"]}
                },
                "required": ["verdict"]
            })
        );
    }

    #[test]
    fn test_routing_field判定_required_boolean_or_enumだけ許可する() {
        let schema = SchemaDef::Object {
            properties: BTreeMap::from([
                ("flag".to_string(), SchemaDef::Boolean),
                (
                    "verdict".to_string(),
                    SchemaDef::String {
                        r#enum: Some(vec!["YES".to_string(), "NO".to_string()]),
                    },
                ),
                ("note".to_string(), SchemaDef::String { r#enum: None }),
            ]),
            required: BTreeSet::from(["flag".to_string(), "verdict".to_string()]),
        };
        assert_eq!(
            routing_field_kind(
                match &schema {
                    SchemaDef::Object { properties, .. } => &properties["flag"],
                    _ => unreachable!(),
                },
                true,
                "flag",
            ),
            Ok(RoutingFieldKind::Boolean)
        );
        assert_eq!(
            routing_field_kind(
                match &schema {
                    SchemaDef::Object { properties, .. } => &properties["verdict"],
                    _ => unreachable!(),
                },
                true,
                "verdict",
            ),
            Ok(RoutingFieldKind::Enum)
        );
        assert!(matches!(
            routing_field_kind(
                match &schema {
                    SchemaDef::Object { properties, .. } => &properties["note"],
                    _ => unreachable!(),
                },
                false,
                "note",
            ),
            Err(RoutingFieldError::NotRequired { .. })
        ));
    }
}

mod contract_schema_path_test {
    use super::super::*;

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
}
