//! Pure Artifact contract helpers backed by workflow `schemas:`.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::domain::workflow::services::contract_schema::{self, SchemaViolation};
use crate::domain::workflow::value_objects::{
    ContractValidationResult, ContractViolation, SchemaDef, WorkflowDefinition,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ContractLookupError {
    RunNotFound { run_id: String },
    InvalidRunStartedPayload { details: String },
    NoArtifactContract { workflow_name: String, node: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactSubmittedSnapshot {
    pub contract: Option<String>,
    pub value: Value,
    pub submitted_at: Option<f64>,
    pub request_id: Option<String>,
    pub timestamp: f64,
}

pub fn lookup_node_artifact_contract(
    workflow: &WorkflowDefinition,
    node_name: &str,
) -> Option<String> {
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
            structured_output: value,
        },
        Err(violations) => ContractValidationResult::Invalid(ContractViolation {
            reason: "schema_violation".to_string(),
            details: format_schema_violations(&violations),
        }),
    }
}

pub fn build_missing_artifact_repair_prompt(
    cli_alias: &str,
    run_id: &str,
    node_name: &str,
    contract: &str,
) -> String {
    format!(
        "The required Artifact for this workflow node has not been submitted.\n\n\
Submit it by running this command with a JSON value that satisfies the `{contract}` schema:\n\n\
```sh\n\
{cli_alias} workflow output submit {run_id} \\\n  --node {node_name} \\\n  --type {contract} \\\n  --json '{{...}}'\n\
```\n\n\
Do not create a temporary JSON file for this. Do not finish the node until the command succeeds."
    )
}

pub fn build_schema_violation_repair_prompt(
    cli_alias: &str,
    run_id: &str,
    node_name: &str,
    contract: &str,
    violations: &[SchemaViolation],
) -> String {
    let details = format_schema_violations(violations);
    format!(
        "The submitted Artifact did not satisfy the `{contract}` schema.\n\n\
Schema violations:\n{details}\n\n\
Submit a corrected Artifact with:\n\n\
```sh\n\
{cli_alias} workflow output submit {run_id} \\\n  --node {node_name} \\\n  --type {contract} \\\n  --json '{{...}}'\n\
```"
    )
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
mod contract_service_tests {
    use super::*;
    use crate::domain::workflow::value_objects::{
        FacetRefs, FanoutSpec, NodeDefinition, NodeKind, SchemaDef, SessionSpec, WorkflowDefinition,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: BTreeMap::from([(
                "review".to_string(),
                SchemaDef::Object {
                    properties: BTreeMap::from([(
                        "verdict".to_string(),
                        SchemaDef::String {
                            r#enum: Some(vec!["LGTM".to_string(), "FIX".to_string()]),
                        },
                    )]),
                    required: BTreeSet::from(["verdict".to_string()]),
                    additional_properties: true,
                },
            )]),
            nodes: vec![
                NodeDefinition {
                    name: "fanout".to_string(),
                    kind: NodeKind::Fanout(FanoutSpec {
                        child: vec!["child".to_string()],
                        items: None,
                    }),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "child".to_string(),
                    kind: NodeKind::Session(SessionSpec {
                        facets: FacetRefs {
                            instruction: Some("review".to_string()),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    artifact: Some("review".to_string()),
                    ..Default::default()
                },
            ],
        }
    }

    #[test]
    fn test_lookup_node_artifact_contract_top_level_fanout_childも探索する() {
        assert_eq!(
            lookup_node_artifact_contract(&workflow(), "child").as_deref(),
            Some("review")
        );
    }

    #[test]
    fn test_validate_artifact_value_schemaで検証する() {
        let valid = validate_artifact_value(
            &workflow().schemas,
            "review",
            serde_json::json!({
                "verdict": "LGTM"
            }),
        );
        assert!(matches!(
            valid,
            ContractValidationResult::Valid {
                result: Some(_),
                ..
            }
        ));

        let invalid = validate_artifact_value(
            &workflow().schemas,
            "review",
            serde_json::json!({
                "verdict": "MAYBE"
            }),
        );
        assert!(matches!(
            invalid,
            ContractValidationResult::Invalid(ContractViolation {
                reason,
                ..
            }) if reason == "schema_violation"
        ));
    }

    #[test]
    fn test_missing_artifact_repair_prompt_uses_schema_vocabulary_and_node_flag() {
        let prompt =
            build_missing_artifact_repair_prompt("releash-dev", "run-1", "review", "review");
        assert!(prompt.contains("Artifact"));
        assert!(prompt.contains("schema"));
        assert!(prompt.contains(
            "```sh\nreleash-dev workflow output submit run-1 \\\n  --node review \\\n  --type review \\\n  --json '{...}'\n```"
        ));
        assert!(!prompt.contains("\n+  --"));
        assert!(!prompt.contains("--step"));
    }

    #[test]
    fn test_schema_violation_repair_prompt_contains_copyable_command() {
        let prompt = build_schema_violation_repair_prompt(
            "releash-dev",
            "run-1",
            "review",
            "review",
            &[SchemaViolation {
                path: "$.verdict".to_string(),
                reason: "expected one of [LGTM, FIX]".to_string(),
            }],
        );

        assert!(prompt.contains("- $.verdict: expected one of [LGTM, FIX]"));
        assert!(prompt.contains(
            "```sh\nreleash-dev workflow output submit run-1 \\\n  --node review \\\n  --type review \\\n  --json '{...}'\n```"
        ));
        assert!(!prompt.contains("\n+  --"));
    }

    #[test]
    fn test_session_spec_default_is_available_for_artifact_nodes() {
        let _node = NodeDefinition {
            name: "review".to_string(),
            kind: NodeKind::Session(SessionSpec::default()),
            artifact: Some("review".to_string()),
            ..Default::default()
        };
    }
}
