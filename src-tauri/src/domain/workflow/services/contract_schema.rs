use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::domain::workflow::SchemaDef;

pub const COMMAND_RESERVED_FIELDS: &[&str] = &["ok", "exit_code", "stdout", "stderr", "duration"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaViolation {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingFieldKind {
    Boolean,
    Enum,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingFieldError {
    NotObject,
    MissingProperty { field: String },
    NotRequired { field: String },
    NotBooleanOrEnum { field: String },
}

pub fn schema_def_from_json(value: &Value) -> Result<SchemaDef, String> {
    if let Some(scalar) = value.as_str() {
        if scalar == "string" {
            return Ok(SchemaDef::String { r#enum: None });
        }
        return Err(format!(
            "scalar schema supports only 'string', got '{scalar}'"
        ));
    }

    let object = value
        .as_object()
        .ok_or_else(|| "schema must be an object or scalar 'string'".to_string())?;
    let schema_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "schema.type must be a string".to_string())?;

    match schema_type {
        "object" => {
            reject_schema_keywords(
                object.keys().map(String::as_str),
                &["type", "properties", "required", "additionalProperties"],
                "object schema supports only properties, required, and additionalProperties",
            )?;
            let properties = match object.get("properties") {
                Some(value) => value
                    .as_object()
                    .ok_or_else(|| "properties must be an object".to_string())?
                    .iter()
                    .map(|(field, schema)| {
                        schema_def_from_json(schema)
                            .map(|schema| (field.clone(), schema))
                            .map_err(|reason| format!("properties.{field}: {reason}"))
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>()?,
                None => BTreeMap::new(),
            };
            let required = match object.get("required") {
                Some(value) => parse_string_array(value, "required")?.into_iter().collect(),
                None => BTreeSet::new(),
            };
            let additional_properties = object
                .get("additionalProperties")
                .map(|value| {
                    value
                        .as_bool()
                        .ok_or_else(|| "additionalProperties must be a boolean".to_string())
                })
                .transpose()?
                .unwrap_or(true);
            Ok(SchemaDef::Object {
                properties,
                required,
                additional_properties,
            })
        }
        "array" => {
            reject_schema_keywords(
                object.keys().map(String::as_str),
                &["type", "items"],
                "array schema supports only items",
            )?;
            let items = object
                .get("items")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "array.items must be a non-empty Contract name".to_string())?;
            Ok(SchemaDef::Array {
                items: items.to_string(),
            })
        }
        "string" => {
            reject_schema_keywords(
                object.keys().map(String::as_str),
                &["type", "enum"],
                "string schema supports only enum",
            )?;
            let r#enum = object
                .get("enum")
                .map(|value| parse_string_array(value, "enum"))
                .transpose()?;
            Ok(SchemaDef::String { r#enum })
        }
        "boolean" => scalar_schema_from_json(object, SchemaDef::Boolean),
        "integer" => scalar_schema_from_json(object, SchemaDef::Integer),
        "number" => scalar_schema_from_json(object, SchemaDef::Number),
        other => Err(format!("unsupported schema type '{other}'")),
    }
}

pub fn schema_def_to_json_value(schema: &SchemaDef) -> Value {
    match schema {
        SchemaDef::Object {
            properties,
            required,
            additional_properties,
        } => {
            let mut object = serde_json::Map::new();
            object.insert("type".to_string(), Value::String("object".to_string()));
            if !properties.is_empty() {
                object.insert(
                    "properties".to_string(),
                    Value::Object(
                        properties
                            .iter()
                            .map(|(field, schema)| {
                                (field.clone(), schema_def_to_json_value(schema))
                            })
                            .collect(),
                    ),
                );
            }
            if !required.is_empty() {
                object.insert(
                    "required".to_string(),
                    Value::Array(required.iter().cloned().map(Value::String).collect()),
                );
            }
            if !additional_properties {
                object.insert("additionalProperties".to_string(), Value::Bool(false));
            }
            Value::Object(object)
        }
        SchemaDef::Array { items } => serde_json::json!({
            "type": "array",
            "items": items,
        }),
        SchemaDef::String { r#enum: None } => Value::String("string".to_string()),
        SchemaDef::String {
            r#enum: Some(values),
        } => serde_json::json!({
            "type": "string",
            "enum": values,
        }),
        SchemaDef::Boolean => serde_json::json!({ "type": "boolean" }),
        SchemaDef::Integer => serde_json::json!({ "type": "integer" }),
        SchemaDef::Number => serde_json::json!({ "type": "number" }),
    }
}

pub fn validate(
    value: &Value,
    schema: &SchemaDef,
    schemas: &BTreeMap<String, SchemaDef>,
) -> Result<(), Vec<SchemaViolation>> {
    let mut violations = Vec::new();
    validate_at(value, schema, schemas, "$", &mut violations);
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

pub fn routing_field_kind(
    schema: &SchemaDef,
    field: &str,
) -> Result<RoutingFieldKind, RoutingFieldError> {
    let SchemaDef::Object {
        properties,
        required,
        ..
    } = schema
    else {
        return Err(RoutingFieldError::NotObject);
    };
    let Some(property) = properties.get(field) else {
        return Err(RoutingFieldError::MissingProperty {
            field: field.to_string(),
        });
    };
    if !required.contains(field) {
        return Err(RoutingFieldError::NotRequired {
            field: field.to_string(),
        });
    }
    match property {
        SchemaDef::Boolean => Ok(RoutingFieldKind::Boolean),
        SchemaDef::String {
            r#enum: Some(values),
        } if !values.is_empty() => Ok(RoutingFieldKind::Enum),
        _ => Err(RoutingFieldError::NotBooleanOrEnum {
            field: field.to_string(),
        }),
    }
}

pub fn schema_declares_command_reserved_field(schema: &SchemaDef) -> Option<String> {
    let SchemaDef::Object { properties, .. } = schema else {
        return None;
    };
    COMMAND_RESERVED_FIELDS
        .iter()
        .find(|field| properties.contains_key(**field))
        .map(|field| (*field).to_string())
}

pub fn is_safe_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn validate_at(
    value: &Value,
    schema: &SchemaDef,
    schemas: &BTreeMap<String, SchemaDef>,
    path: &str,
    violations: &mut Vec<SchemaViolation>,
) {
    match schema {
        SchemaDef::Object {
            properties,
            required,
            additional_properties,
        } => {
            let Some(object) = value.as_object() else {
                push(violations, path, "expected object");
                return;
            };
            for field in required {
                if !object.contains_key(field) {
                    push(
                        violations,
                        &format!("{path}.{field}"),
                        "required field missing",
                    );
                }
            }
            for (field, property_schema) in properties {
                if let Some(child) = object.get(field) {
                    validate_at(
                        child,
                        property_schema,
                        schemas,
                        &format!("{path}.{field}"),
                        violations,
                    );
                }
            }
            if !additional_properties {
                for field in object.keys() {
                    if !properties.contains_key(field) {
                        push(
                            violations,
                            &format!("{path}.{field}"),
                            "additional property not allowed",
                        );
                    }
                }
            }
        }
        SchemaDef::Array { items } => {
            let Some(array) = value.as_array() else {
                push(violations, path, "expected array");
                return;
            };
            let Some(item_schema) = schemas.get(items) else {
                push(
                    violations,
                    path,
                    format!("array item schema '{items}' is not defined"),
                );
                return;
            };
            for (idx, item) in array.iter().enumerate() {
                validate_at(
                    item,
                    item_schema,
                    schemas,
                    &format!("{path}[{idx}]"),
                    violations,
                );
            }
        }
        SchemaDef::String { r#enum } => {
            let Some(text) = value.as_str() else {
                push(violations, path, "expected string");
                return;
            };
            if let Some(values) = r#enum {
                if !values.iter().any(|value| value == text) {
                    push(
                        violations,
                        path,
                        format!("expected one of [{}]", values.join(", ")),
                    );
                }
            }
        }
        SchemaDef::Boolean => {
            if !value.is_boolean() {
                push(violations, path, "expected boolean");
            }
        }
        SchemaDef::Integer => {
            if !(value.is_i64() || value.is_u64()) {
                push(violations, path, "expected integer");
            }
        }
        SchemaDef::Number => {
            if !value.is_number() {
                push(violations, path, "expected number");
            }
        }
    }
}

fn push(violations: &mut Vec<SchemaViolation>, path: &str, reason: impl Into<String>) {
    violations.push(SchemaViolation {
        path: path.to_string(),
        reason: reason.into(),
    });
}

fn reject_schema_keywords<'a>(
    keys: impl Iterator<Item = &'a str>,
    allowed: &[&str],
    message: &str,
) -> Result<(), String> {
    if keys.into_iter().any(|key| !allowed.contains(&key)) {
        return Err(message.to_string());
    }
    Ok(())
}

fn scalar_schema_from_json(
    object: &serde_json::Map<String, Value>,
    schema: SchemaDef,
) -> Result<SchemaDef, String> {
    reject_schema_keywords(
        object.keys().map(String::as_str),
        &["type"],
        "boolean, integer, and number schemas do not support extra keywords",
    )?;
    Ok(schema)
}

fn parse_string_array(value: &Value, field: &str) -> Result<Vec<String>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{field} must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| format!("{field} entries must be strings"))
        })
        .collect()
}

#[cfg(test)]
mod contract_schema_tests {
    use super::*;
    use std::collections::BTreeSet;

    fn object(properties: BTreeMap<String, SchemaDef>, required: &[&str]) -> SchemaDef {
        SchemaDef::Object {
            properties,
            required: required.iter().map(|value| (*value).to_string()).collect(),
            additional_properties: false,
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
        assert!(violations.iter().any(|v| v.path == "$.extra"));
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
            "required": ["verdict"],
            "additionalProperties": false
        }))
        .unwrap();
        assert!(matches!(
            object_schema,
            SchemaDef::Object {
                additional_properties: false,
                ..
            }
        ));
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
                "object schema supports only properties, required, and additionalProperties",
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
                "required": ["verdict"],
                "additionalProperties": false
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
            additional_properties: true,
        };
        assert_eq!(
            routing_field_kind(&schema, "flag"),
            Ok(RoutingFieldKind::Boolean)
        );
        assert_eq!(
            routing_field_kind(&schema, "verdict"),
            Ok(RoutingFieldKind::Enum)
        );
        assert!(matches!(
            routing_field_kind(&schema, "note"),
            Err(RoutingFieldError::NotRequired { .. })
        ));
    }
}
