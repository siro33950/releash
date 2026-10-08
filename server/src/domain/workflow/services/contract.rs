//! Pure Artifact contract helpers backed by workflow `schemas:`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::Value;

use crate::domain::workflow::services::contract_schema::{self, SchemaViolation};
use crate::domain::workflow::value_objects::{
    ContractValidationResult, ContractViolation, SchemaDef, WorkflowDefinition,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ContractLookupError {
    ExecutionNotFound { execution_id: String },
    InvalidExecutionStartedPayload { details: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactSubmittedSnapshot {
    pub contract: Option<String>,
    pub value: Value,
    pub submitted_at: Option<f64>,
    pub request_id: Option<String>,
    pub timestamp: f64,
}

pub fn lookup_node_contract(workflow: &WorkflowDefinition, node_name: &str) -> Option<String> {
    for node in &workflow.nodes {
        if node.name == node_name {
            return node
                .artifact
                .clone()
                .filter(|contract| !contract.trim().is_empty());
        }
    }
    None
}

pub fn validate_artifact_value(
    schemas: &BTreeMap<String, SchemaDef>,
    contract: &str,
    value: Value,
) -> ContractValidationResult {
    let Some(schema) = schemas.get(contract) else {
        return ContractValidationResult::Invalid(ContractViolation {
            reason: "unknown_schema".to_string(),
            details: format!("Contract schema '{contract}' is not defined."),
        });
    };
    match contract_schema::validate(&value, schema, schemas) {
        Ok(()) => ContractValidationResult::Valid {
            result: infer_result(&value),
            artifact: value,
        },
        Err(violations) => ContractValidationResult::Invalid(ContractViolation {
            reason: "schema_violation".to_string(),
            details: format_schema_violations(&violations),
        }),
    }
}

pub fn render_contract_prompt_guidance(
    schemas: &BTreeMap<String, SchemaDef>,
    contract: &str,
) -> Option<String> {
    let schema = schemas.get(contract)?;
    let schema_json =
        serde_json::to_string_pretty(&contract_schema::schema_def_to_json_value(schema))
            .unwrap_or_else(|_| "null".to_string());
    let additional_fields_guidance = matches!(schema, SchemaDef::Object { .. })
        .then_some("\nFields not listed in `properties` are accepted, but omit fields that downstream nodes do not need.")
        .unwrap_or_default();
    let referenced_schemas = referenced_schema_values(schemas, contract, schema);
    let referenced_schema_guidance = if referenced_schemas.is_empty() {
        String::new()
    } else {
        let referenced_schema_json = serde_json::to_string_pretty(&referenced_schemas)
            .unwrap_or_else(|_| "null".to_string());
        format!(
            "\n\nNames used by `array.items` are Contract references. Every transitive Contract needed by `{contract}` is defined below; do not inspect Releash application data or other session logs to resolve them.\n\n\
## Referenced Contract schemas\n\n\
```json\n{referenced_schema_json}\n\
```"
        )
    };

    Some(format!(
        "## Artifact contract\n\n\
The `--json` argument must be a JSON value matching the `{contract}` schema below. The schema itself is not the value to submit.\n\n\
```json\n{schema_json}\n\
```\
{additional_fields_guidance}{referenced_schema_guidance}"
    ))
}

fn referenced_schema_values(
    schemas: &BTreeMap<String, SchemaDef>,
    contract: &str,
    root: &SchemaDef,
) -> BTreeMap<String, Value> {
    let mut pending = VecDeque::new();
    collect_array_item_references(root, &mut pending);

    let mut visited = BTreeSet::from([contract.to_string()]);
    let mut referenced = BTreeMap::new();
    while let Some(name) = pending.pop_front() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let Some(schema) = schemas.get(&name) else {
            continue;
        };
        collect_array_item_references(schema, &mut pending);
        referenced.insert(name, contract_schema::schema_def_to_json_value(schema));
    }
    referenced
}

fn collect_array_item_references(schema: &SchemaDef, pending: &mut VecDeque<String>) {
    match schema {
        SchemaDef::Object { properties, .. } => {
            for property in properties.values() {
                collect_array_item_references(property, pending);
            }
        }
        SchemaDef::Array { items } => pending.push_back(items.clone()),
        SchemaDef::String { .. } | SchemaDef::Boolean | SchemaDef::Integer | SchemaDef::Number => {}
    }
}

pub fn format_schema_violations(violations: &[SchemaViolation]) -> String {
    violations
        .iter()
        .map(|violation| format!("- {}: {}", violation.path, violation.reason))
        .collect::<Vec<_>>()
        .join("\n")
}

fn infer_result(value: &Value) -> Option<String> {
    ["result", "verdict", "status"]
        .iter()
        .find_map(|field| value.get(field).and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

#[cfg(test)]
#[path = "contract_test.rs"]
mod contract_tests;
