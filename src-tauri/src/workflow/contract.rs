//! [08] contract validator。
//!
//! 旧来「`<workflow_output>` ブロックを agent 自由文から抽出して contract を判定する」
//! prose 抽出経路は本 issue で完全廃止された（spec [08] L137 / Rule 4 構造化出力の
//! 確定経路は明示的提出のみに統一）。本モジュールは CLI / Tauri 経由の
//! `SubmitOutput` から呼ばれる pure validator (`validate_contract_value`) を唯一の
//! 検証入口として提供する。

use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::workflow::event::WorkflowEvent;
use crate::workflow::schema::Workflow;

/// [08] event 列から step の `output_contract` を解決する際の決定論的エラー。
///
/// CLI / Tauri 経路は本エラーをそれぞれ自層の表現（`CliError::NotFound /
/// InvalidInput` や `String`）に map するだけで、event log → workflow definition →
/// contract resolution の経路自体を共有する（spec [08] アーキテクチャ概要:
/// contract resolution は engine と CLI 双方から再利用される pure 関数）。
#[derive(Debug, Clone, PartialEq)]
pub enum ContractLookupError {
    /// 当該 run の `RunStarted` event が存在しない（run 不在 / 認可外）。
    RunNotFound,
    /// step が `output_contract` を持たない（あるいは空文字）。
    NoOutputContract { workflow_name: String, step: String },
}

/// [08] event 列の `RunStarted` から workflow definition を取り出し、step の
/// `output_contract` を解決する pure helper。
///
/// CLI `workflow output validate / submit` の事前検証と Tauri `workflow_validate_output`
/// が完全に同一ロジックで contract type を引いていたため共通化する（spec [08]
/// アーキテクチャ概要: CLI と engine の双方から再利用される pure 関数）。
pub fn resolve_step_output_contract_from_events(
    events: &[WorkflowEvent],
    step: &str,
) -> Result<String, ContractLookupError> {
    let workflow = events
        .iter()
        .find_map(|e| match e {
            WorkflowEvent::RunStarted {
                workflow_definition,
                ..
            } => Some(workflow_definition.clone()),
            _ => None,
        })
        .ok_or(ContractLookupError::RunNotFound)?;
    lookup_step_output_contract(&workflow, step).ok_or_else(|| {
        ContractLookupError::NoOutputContract {
            workflow_name: workflow.name,
            step: step.to_string(),
        }
    })
}

/// [08] workflow definition から `step_name` の `output_contract` を解決する pure helper。
///
/// top-level node / parallel child の両方を探索し、`output_contract` が空文字 /
/// 未設定の step は「提出対象として妥当でない」として `None` を返す。
///
/// engine の `WorkflowCommand::SubmitOutput` handler と CLI `workflow output submit /
/// validate` の双方から再利用される pure 関数として contract モジュールに置く
/// （spec [08] アーキテクチャ概要: contract resolution は engine と CLI 双方から
/// 再利用される pure 関数。CLI 層が engine internals に依存しないようにする境界）。
pub fn lookup_step_output_contract(workflow: &Workflow, step_name: &str) -> Option<String> {
    for node in &workflow.nodes {
        if node.name == step_name {
            return node
                .output_contract
                .clone()
                .filter(|c| !c.trim().is_empty());
        }
        if let Some(children) = &node.parallel_children {
            for child in children {
                if child.name == step_name {
                    return child
                        .output_contract
                        .clone()
                        .filter(|c| !c.trim().is_empty());
                }
            }
        }
    }
    None
}

/// contract検証結果。
#[derive(Debug, Clone)]
pub enum ContractValidationResult {
    Valid {
        structured_output: Value,
        result: Option<String>,
    },
    Invalid(ContractViolation),
}

#[derive(Debug, Clone)]
pub struct ContractViolation {
    pub reason: String,
    pub details: String,
}

/// [08] `validate_contract` の Value-only 入口。
///
/// CLI `workflow output submit` / `validate` および engine の
/// `WorkflowCommand::SubmitOutput` handler から再利用される pure validator。
/// caller は既に「contract type」と「JSON value」を typed 入力として持っているため
/// prose 抽出を経由せず、`<workflow_output>` block の存在判定は行わない。
/// type 名の照合は caller (CLI / engine) 側の責務に閉じる。
pub fn validate_contract_value(contract_type: &str, value: Value) -> ContractValidationResult {
    let definition = crate::workflow::builtin::get_builtin_facet(
        crate::workflow::facet::FacetKind::Contract,
        contract_type,
    );
    validate_contract_value_with_definition(value, definition)
}

pub fn validate_contract_value_with_definition(
    value: Value,
    contract_definition: Option<&str>,
) -> ContractValidationResult {
    validate_contract_against_metadata(value, contract_definition)
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ContractValidationMetadata {
    result_field: Option<String>,
    result: Option<String>,
    required: Vec<String>,
    enums: HashMap<String, Vec<String>>,
    non_empty_array_when: Vec<ConditionalArrayRule>,
    array_items_required: HashMap<String, Vec<String>>,
    relative_paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
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
    Some(
        serde_json::from_str(json)
            .map_err(|err| format!("Invalid contract-validation metadata JSON: {err}")),
    )
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
mod tests {
    use super::*;
    use serde_json::json;

    // ---- [08] validate_contract_value: pure validator (CLI / engine 共通入口) ----

    #[test]
    fn validate_contract_value_accepts_fix_result_statuses() {
        let definition = r#"
```contract-validation
{
  "result_field": "status",
  "required": ["status"],
  "enums": {
    "status": ["FIXED", "PARTIAL", "BLOCKED"]
  }
}
```
"#;
        for status in &["FIXED", "PARTIAL", "BLOCKED"] {
            match validate_contract_value_with_definition(
                json!({"status": status}),
                Some(definition),
            ) {
                ContractValidationResult::Valid { result, .. } => {
                    assert_eq!(result, Some(status.to_string()));
                }
                other => panic!("expected Valid for status {status}, got {:?}", other),
            }
        }
    }

    #[test]
    fn validate_contract_value_rejects_invalid_fix_result_status() {
        let definition = r#"
```contract-validation
{
  "result_field": "status",
  "required": ["status"],
  "enums": {
    "status": ["FIXED", "PARTIAL", "BLOCKED"]
  }
}
```
"#;
        match validate_contract_value_with_definition(json!({"status": "DONE"}), Some(definition)) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_enum");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_missing_fix_result_status() {
        let definition = r#"
```contract-validation
{
  "result_field": "status",
  "required": ["status"],
  "enums": {
    "status": ["FIXED", "PARTIAL", "BLOCKED"]
  }
}
```
"#;
        match validate_contract_value_with_definition(json!({"summary": "done"}), Some(definition))
        {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "missing_field");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_accepts_relative_spec_dir() {
        match validate_contract_value(
            "spec-directory",
            json!({"spec_dir": "docs/spec/issues-1029.md"}),
        ) {
            ContractValidationResult::Valid { result, .. } => {
                assert_eq!(result, None);
            }
            other => panic!("expected Valid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_absolute_spec_dir() {
        match validate_contract_value("spec-directory", json!({"spec_dir": "/etc/passwd"})) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_path");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_spec_dir_traversal() {
        match validate_contract_value(
            "spec-directory",
            json!({"spec_dir": "docs/../../etc/passwd"}),
        ) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_path");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_windows_drive_spec_dir() {
        for path in &["C:\\repo\\spec.md", "C:/repo/spec.md", "D:\\file.md"] {
            match validate_contract_value("spec-directory", json!({"spec_dir": path})) {
                ContractValidationResult::Invalid(v) => {
                    assert_eq!(v.reason, "invalid_path", "path: {path}");
                }
                other => panic!("expected Invalid for path '{path}', got {:?}", other),
            }
        }
    }

    #[test]
    fn validate_contract_value_rejects_missing_spec_dir_field() {
        match validate_contract_value("spec-directory", json!({"path": "some/path.md"})) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "missing_field");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_non_string_spec_dir() {
        match validate_contract_value("spec-directory", json!({"spec_dir": 123})) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_path");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_passes_through_unknown_contract_type() {
        match validate_contract_value("custom-thing", json!({"anything": "goes"})) {
            ContractValidationResult::Valid { result, .. } => {
                assert_eq!(result, None);
            }
            other => panic!("expected Valid, got {:?}", other),
        }
    }

    // ---- [08] resolve_step_output_contract_from_events: CLI / Tauri 共通の解決経路 ----

    fn workflow_with_review_contract(contract: Option<&str>) -> Workflow {
        use crate::workflow::schema::{NodeDefinition, NodeType};
        Workflow {
            variables: Default::default(),
            name: "wf-resolve".to_string(),
            description: "".to_string(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                node_type: NodeType::Agent,
                instruction: Some("review".to_string()),
                output_contract: contract.map(str::to_string),
                ..NodeDefinition::default()
            }],
        }
    }

    fn run_started_event(workflow: Workflow) -> WorkflowEvent {
        WorkflowEvent::RunStarted {
            run_id: "run-1".to_string(),
            workflow_name: workflow.name.clone(),
            workflow_file_stem: workflow.name.clone(),
            worktree_path: "/wt".to_string(),
            workflow_definition: workflow,
            timestamp: 1.0,
        }
    }

    /// spec [08]: RunStarted から workflow_definition を引き、step の contract を解決する。
    #[test]
    fn resolve_step_output_contract_from_events_resolves_compliant_contract() {
        let events = vec![run_started_event(workflow_with_review_contract(Some(
            "spec-directory",
        )))];
        let contract = resolve_step_output_contract_from_events(&events, "review")
            .expect("contract should be resolved");
        assert_eq!(contract, "spec-directory");
    }

    /// spec [08]: RunStarted event がない event 列は `RunNotFound`。
    #[test]
    fn resolve_step_output_contract_from_events_returns_run_not_found_without_run_started() {
        let err = resolve_step_output_contract_from_events(&[], "review")
            .expect_err("missing RunStarted should yield error");
        assert_eq!(err, ContractLookupError::RunNotFound);
    }

    /// spec [08]: step に `output_contract` が無い場合は `NoOutputContract`。
    #[test]
    fn resolve_step_output_contract_from_events_returns_no_output_contract_when_unset() {
        let events = vec![run_started_event(workflow_with_review_contract(None))];
        let err = resolve_step_output_contract_from_events(&events, "review")
            .expect_err("missing output_contract should yield error");
        assert_eq!(
            err,
            ContractLookupError::NoOutputContract {
                workflow_name: "wf-resolve".to_string(),
                step: "review".to_string(),
            }
        );
    }
}
