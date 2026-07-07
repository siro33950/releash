//! Pure output contract validation.

use std::collections::HashMap;

use serde_json::Value;

use crate::domain::workflow::value_objects::{
    ContractValidationResult, ContractViolation, WorkflowDefinition,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ContractLookupError {
    RunNotFound { run_id: String },
    InvalidRunStartedPayload { details: String },
    NoOutputContract { workflow_name: String, step: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputSubmittedSnapshot {
    pub contract: String,
    pub structured_output: Value,
    pub submitted_at: Option<f64>,
    pub request_id: Option<String>,
    pub timestamp: f64,
}

pub fn lookup_step_output_contract(
    workflow: &WorkflowDefinition,
    step_name: &str,
) -> Option<String> {
    for node in &workflow.nodes {
        if node.name == step_name {
            return node
                .output_contract
                .clone()
                .filter(|contract| !contract.trim().is_empty());
        }
        if let Some(fanout) = node.fanout() {
            let children = &fanout.parallel_children;
            for child in children {
                if child.name == step_name {
                    return child
                        .output_contract
                        .clone()
                        .filter(|contract| !contract.trim().is_empty());
                }
            }
        }
    }
    None
}

pub fn extract_workflow_variables_from_contract_output(
    output_contract: Option<&str>,
    structured_output: Option<&Value>,
) -> HashMap<String, String> {
    let mut variables = HashMap::new();
    if output_contract != Some("spec-directory") {
        return variables;
    }
    if let Some(path) = structured_output
        .and_then(|output| output.get("spec_dir"))
        .and_then(|value| value.as_str())
    {
        variables.insert("spec_dir".to_string(), path.to_string());
    }
    variables
}

pub fn build_missing_output_repair_prompt(
    cli_alias: &str,
    run_id: &str,
    step_name: &str,
    contract: &str,
    contract_definition: Option<&str>,
) -> String {
    let contract_section = contract_definition
        .filter(|body| !body.trim().is_empty())
        .map(|body| {
            format!(
                "\n\nContract definition (type: {contract}):\n\n```text\n{}\n```",
                body.trim()
            )
        })
        .unwrap_or_default();
    format!(
        "The required structured output for this workflow step has not been submitted.\n\n\
Submit it by running this command with a JSON object that satisfies the `{contract}` contract:{contract_section}\n\n\
```sh\n\
{cli_alias} workflow output submit {run_id} \\\n  --step {step_name} \\\n  --type {contract} \\\n  --json '{{...}}'\n```\n\n\
Do not create a temporary JSON file for this. Do not finish the step until the command succeeds."
    )
}

pub fn validate_contract_value_with_definition(
    value: Value,
    contract_definition: Option<&str>,
) -> ContractValidationResult {
    validate_contract_against_metadata(value, contract_definition)
}

pub fn strip_contract_validation_metadata(contract_definition: &str) -> String {
    let mut remaining = contract_definition;
    let mut output = String::new();
    let opening = "```contract-validation";

    while let Some(start) = remaining.find(opening) {
        output.push_str(&remaining[..start]);
        let after_opening = &remaining[start + opening.len()..];
        let body_start = after_opening.find('\n').map(|pos| pos + 1).unwrap_or(0);
        let body = &after_opening[body_start..];
        if let Some(end) = body.find("```") {
            remaining = &body[end + 3..];
        } else {
            remaining = "";
            break;
        }
    }
    output.push_str(remaining);
    output.trim().to_string()
}

#[derive(Debug, Default)]
struct ContractValidationMetadata {
    result_field: Option<String>,
    result: Option<String>,
    required: Vec<String>,
    enums: HashMap<String, Vec<String>>,
    non_empty_array_when: Vec<ConditionalArrayRule>,
    array_items_required: HashMap<String, Vec<String>>,
    relative_paths: Vec<String>,
}

#[derive(Debug)]
struct ConditionalArrayRule {
    field: String,
    equals: Value,
    array: String,
}

fn validate_contract_against_metadata(
    json: Value,
    contract_definition: Option<&str>,
) -> ContractValidationResult {
    let metadata = match contract_definition.and_then(extract_validation_metadata) {
        Some(Ok(metadata)) => metadata,
        Some(Err(details)) => {
            return ContractValidationResult::Invalid(ContractViolation {
                reason: "invalid_contract_validation_metadata".to_string(),
                details,
            });
        }
        None => ContractValidationMetadata::default(),
    };

    if let Err(violation) = validate_metadata_rules(&json, &metadata) {
        return ContractValidationResult::Invalid(violation);
    }

    let result = metadata
        .result
        .or_else(|| {
            metadata
                .result_field
                .as_deref()
                .and_then(|field| json.get(field))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            ["result", "verdict", "status"]
                .iter()
                .find_map(|field| json.get(field).and_then(|value| value.as_str()))
                .map(ToOwned::to_owned)
        });

    ContractValidationResult::Valid {
        structured_output: json,
        result,
    }
}

fn extract_validation_metadata(
    contract_definition: &str,
) -> Option<Result<ContractValidationMetadata, String>> {
    let opening = "```contract-validation";
    let start = contract_definition.find(opening)?;
    let after_opening = &contract_definition[start + opening.len()..];
    let body_start = after_opening.find('\n').map(|pos| pos + 1).unwrap_or(0);
    let body = &after_opening[body_start..];
    let end = body.find("```").unwrap_or(body.len());
    let json = body[..end].trim();
    Some(parse_contract_validation_metadata(json))
}

fn parse_contract_validation_metadata(json: &str) -> Result<ContractValidationMetadata, String> {
    let value: Value = serde_json::from_str(json)
        .map_err(|err| format!("Invalid contract-validation metadata JSON: {err}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "Invalid contract-validation metadata JSON: expected object".to_string())?;
    Ok(ContractValidationMetadata {
        result_field: optional_string_field(object, "result_field")?,
        result: optional_string_field(object, "result")?,
        required: string_array_field(object, "required")?,
        enums: string_array_map_field(object, "enums")?,
        non_empty_array_when: conditional_array_rules_field(object, "non_empty_array_when")?,
        array_items_required: string_array_map_field(object, "array_items_required")?,
        relative_paths: string_array_field(object, "relative_paths")?,
    })
}

fn optional_string_field(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("metadata field \"{key}\" must be a string")),
    }
}

fn string_array_field(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| format!("metadata field \"{key}\" must contain strings"))
            })
            .collect(),
        Some(_) => Err(format!("metadata field \"{key}\" must be an array")),
    }
}

fn string_array_map_field(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<HashMap<String, Vec<String>>, String> {
    let Some(value) = object.get(key) else {
        return Ok(HashMap::new());
    };
    let Some(map) = value.as_object() else {
        return Err(format!("metadata field \"{key}\" must be an object"));
    };
    map.iter()
        .map(|(field, value)| {
            let values = value
                .as_array()
                .ok_or_else(|| format!("metadata field \"{key}.{field}\" must be an array"))?
                .iter()
                .map(|item| {
                    item.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                        format!("metadata field \"{key}.{field}\" must contain strings")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((field.clone(), values))
        })
        .collect()
}

fn conditional_array_rules_field(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<ConditionalArrayRule>, String> {
    let Some(value) = object.get(key) else {
        return Ok(Vec::new());
    };
    let Some(rules) = value.as_array() else {
        return Err(format!("metadata field \"{key}\" must be an array"));
    };
    rules
        .iter()
        .enumerate()
        .map(|(idx, rule)| {
            let Some(rule) = rule.as_object() else {
                return Err(format!("metadata field \"{key}[{idx}]\" must be an object"));
            };
            let field = required_string(rule, key, idx, "field")?;
            let array = required_string(rule, key, idx, "array")?;
            let equals = rule
                .get("equals")
                .cloned()
                .ok_or_else(|| format!("metadata field \"{key}[{idx}].equals\" is required"))?;
            Ok(ConditionalArrayRule {
                field,
                equals,
                array,
            })
        })
        .collect()
}

fn required_string(
    object: &serde_json::Map<String, Value>,
    parent_key: &str,
    idx: usize,
    key: &str,
) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("metadata field \"{parent_key}[{idx}].{key}\" must be a string"))
}

fn validate_metadata_rules(
    json: &Value,
    metadata: &ContractValidationMetadata,
) -> Result<(), ContractViolation> {
    for field in &metadata.required {
        match json.get(field) {
            Some(Value::String(value)) if value.is_empty() => {
                return Err(ContractViolation {
                    reason: "missing_field".to_string(),
                    details: format!("Required field \"{field}\" must not be empty."),
                });
            }
            Some(_) => {}
            None => {
                return Err(ContractViolation {
                    reason: "missing_field".to_string(),
                    details: format!("Missing required field \"{field}\"."),
                });
            }
        }
    }

    for (field, allowed) in &metadata.enums {
        let Some(value) = json.get(field) else {
            continue;
        };
        let Some(actual) = value.as_str() else {
            return Err(ContractViolation {
                reason: "invalid_enum".to_string(),
                details: format!("Field \"{field}\" must be a string."),
            });
        };
        if !allowed.iter().any(|candidate| candidate == actual) {
            return Err(ContractViolation {
                reason: "invalid_enum".to_string(),
                details: format!(
                    "Field \"{field}\" must be one of [{}], got \"{actual}\".",
                    allowed
                        .iter()
                        .map(|value| format!("\"{value}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }
    }

    for rule in &metadata.non_empty_array_when {
        if json.get(&rule.field) == Some(&rule.equals) {
            match json.get(&rule.array).and_then(|value| value.as_array()) {
                Some(values) if !values.is_empty() => {}
                _ => {
                    return Err(ContractViolation {
                        reason: "missing_array".to_string(),
                        details: format!(
                            "Field \"{}\" must be a non-empty array when \"{}\" is {}.",
                            rule.array, rule.field, rule.equals
                        ),
                    });
                }
            }
        }
    }

    for (array_field, required_fields) in &metadata.array_items_required {
        let array = match json.get(array_field) {
            None => continue,
            Some(Value::Array(array)) => array,
            Some(_) => {
                return Err(ContractViolation {
                    reason: "invalid_array".to_string(),
                    details: format!("Field \"{array_field}\" must be an array."),
                });
            }
        };
        for item in array {
            for field in required_fields {
                if item.get(field).and_then(|value| value.as_str()).is_none() {
                    return Err(ContractViolation {
                        reason: "invalid_array_item".to_string(),
                        details: format!(
                            "Each item in \"{array_field}\" must have string field \"{field}\". Invalid item: {item}"
                        ),
                    });
                }
            }
        }
    }

    for field in &metadata.relative_paths {
        let path = match json.get(field) {
            None => continue,
            Some(Value::String(path)) => path,
            Some(_) => {
                return Err(ContractViolation {
                    reason: "invalid_path".to_string(),
                    details: format!("Field \"{field}\" must be a string relative path."),
                });
            }
        };
        validate_relative_contract_path(field, path)?;
    }

    Ok(())
}

fn validate_relative_contract_path(field: &str, path: &str) -> Result<(), ContractViolation> {
    let is_drive_letter_abs = path.len() >= 3
        && path.as_bytes()[0].is_ascii_alphabetic()
        && path.as_bytes()[1] == b':'
        && (path.as_bytes()[2] == b'/' || path.as_bytes()[2] == b'\\');
    if path.starts_with('/') || path.starts_with('\\') || is_drive_letter_abs {
        return Err(ContractViolation {
            reason: "invalid_path".to_string(),
            details: format!(
                "Field \"{field}\" must be a relative path, got absolute path: \"{path}\""
            ),
        });
    }
    if path.contains("..") {
        return Err(ContractViolation {
            reason: "invalid_path".to_string(),
            details: format!("Field \"{field}\" must not contain \"..\": \"{path}\""),
        });
    }
    if path.ends_with('/') || path.ends_with('\\') {
        return Err(ContractViolation {
            reason: "invalid_path".to_string(),
            details: format!("Field \"{field}\" must not end with a path separator: \"{path}\""),
        });
    }
    Ok(())
}

#[cfg(test)]
mod contract_service_tests {
    use super::*;
    use crate::domain::workflow::value_objects::{
        FanoutSpec, InterimChild, NodeDefinition, NodeKind, WorkflowDefinition,
    };
    use serde_json::json;

    #[test]
    fn test_contract_validation_metadata_result_fieldを抽出する() {
        let definition = r#"
```contract-validation
{
  "result_field": "status",
  "required": ["status"],
  "enums": {"status": ["FIXED", "BLOCKED"]}
}
```
"#;
        match validate_contract_value_with_definition(json!({"status": "FIXED"}), Some(definition))
        {
            ContractValidationResult::Valid { result, .. } => {
                assert_eq!(result.as_deref(), Some("FIXED"));
            }
            other => panic!("expected valid, got {other:?}"),
        }
    }

    #[test]
    fn test_contract_validation_relative_pathは絶対パスを拒否する() {
        match validate_contract_value_with_definition(
            json!({"spec_dir": "/tmp/spec"}),
            Some(
                r#"
```contract-validation
{"relative_paths":["spec_dir"]}
```
"#,
            ),
        ) {
            ContractValidationResult::Invalid(violation) => {
                assert_eq!(violation.reason, "invalid_path");
            }
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    #[test]
    fn test_lookup_step_output_contract_parallel_childも探索する() {
        let workflow = WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            variables: Default::default(),
            nodes: vec![NodeDefinition {
                name: "parallel".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    parallel_children: vec![InterimChild {
                        name: "child".to_string(),
                        output_contract: Some("review-verdict".to_string()),
                        ..Default::default()
                    }],
                    aggregate: None,
                }),
                ..Default::default()
            }],
        };
        assert_eq!(
            lookup_step_output_contract(&workflow, "child").as_deref(),
            Some("review-verdict")
        );
    }

    #[test]
    fn extract_workflow_variables_spec_directoryだけspec_dirを返す() {
        let output = serde_json::json!({"spec_dir": "docs/specs/issues-978"});
        assert_eq!(
            extract_workflow_variables_from_contract_output(Some("spec-directory"), Some(&output))
                .get("spec_dir")
                .map(String::as_str),
            Some("docs/specs/issues-978")
        );
        assert!(extract_workflow_variables_from_contract_output(
            Some("approved-fix-policy"),
            Some(&output)
        )
        .is_empty());
        assert!(extract_workflow_variables_from_contract_output(
            Some("spec-directory"),
            Some(&serde_json::json!({"other": "value"}))
        )
        .is_empty());
    }

    #[test]
    fn missing_output_repair_prompt_uses_inline_json_command() {
        let prompt = build_missing_output_repair_prompt(
            "releash-dev",
            "run-1",
            "review",
            "review-verdict",
            Some("  decision: LGTM  "),
        );

        assert!(prompt.contains("releash-dev workflow output submit run-1 \\"));
        assert!(prompt.contains("--step review \\"));
        assert!(prompt.contains("--type review-verdict \\"));
        assert!(prompt.contains("--json '{...}'"));
        assert!(prompt.contains("Contract definition (type: review-verdict):"));
        assert!(prompt.contains("decision: LGTM"));
        assert!(prompt.contains("Do not create a temporary JSON file"));
    }
}
