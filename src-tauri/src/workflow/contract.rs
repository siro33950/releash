//! [08] contract validator。
//!
//! 旧来「`<workflow_output>` ブロックを agent 自由文から抽出して contract を判定する」
//! prose 抽出経路は本 issue で完全廃止された（spec [08] L137 / Rule 4 構造化出力の
//! 確定経路は明示的提出のみに統一）。本モジュールは CLI / Tauri 経由の
//! `SubmitOutput` から呼ばれる pure validator (`validate_contract_value`) を唯一の
//! 検証入口として提供する。

use serde_json::Value;

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
    validate_contract_specific(contract_type, value)
}

/// contract typeごとのバリデーション。
fn validate_contract_specific(contract_type: &str, json: Value) -> ContractValidationResult {
    match contract_type {
        "review-verdict" => validate_review_verdict(json),
        "fix-result" => validate_fix_result(json),
        "spec-file-path" => validate_spec_file_path(json),
        "approved-fix-policy" => validate_approved_fix_policy(json),
        _ => {
            // 未知のcontract typeはJSONが有効ならそのまま通す
            ContractValidationResult::Valid {
                structured_output: json,
                result: None,
            }
        }
    }
}

fn validate_approved_fix_policy(json: Value) -> ContractValidationResult {
    // approved-fix-policy はユーザーが指示書で任意のスキーマを定義する契約。
    // フィールド検証・unknown フィールド除去・サイズ制限を行わず、入力 JSON をそのまま渡す。
    ContractValidationResult::Valid {
        structured_output: json,
        result: Some("approved".to_string()),
    }
}

fn validate_review_verdict(json: Value) -> ContractValidationResult {
    let verdict = json.get("verdict").and_then(|v| v.as_str());
    match verdict {
        Some("LGTM") => ContractValidationResult::Valid {
            result: Some("LGTM".to_string()),
            structured_output: json,
        },
        Some("NEEDS_FIX") => {
            // NEEDS_FIX時はfindingsが非空配列であり、各findingにseverityとmessageがあることを検証
            let findings = json.get("findings").and_then(|v| v.as_array());
            match findings {
                Some(arr) if !arr.is_empty() => {
                    let invalid_finding = arr.iter().find(|f| {
                        f.get("severity").and_then(|v| v.as_str()).is_none()
                            || f.get("message").and_then(|v| v.as_str()).is_none()
                    });
                    if let Some(bad) = invalid_finding {
                        ContractValidationResult::Invalid(ContractViolation {
                            reason: "invalid_finding".to_string(),
                            details: format!(
                                "Each finding must have \"severity\" and \"message\". Invalid finding: {}",
                                bad
                            ),
                        })
                    } else {
                        ContractValidationResult::Valid {
                            result: Some("NEEDS_FIX".to_string()),
                            structured_output: json,
                        }
                    }
                }
                _ => ContractValidationResult::Invalid(ContractViolation {
                    reason: "missing_findings".to_string(),
                    details:
                        "verdict is \"NEEDS_FIX\" but \"findings\" is missing or empty. At least one finding is required."
                            .to_string(),
                }),
            }
        }
        Some(other) => ContractValidationResult::Invalid(ContractViolation {
            reason: "invalid_verdict".to_string(),
            details: format!("verdict must be \"LGTM\" or \"NEEDS_FIX\", got \"{other}\"."),
        }),
        None => ContractValidationResult::Invalid(ContractViolation {
            reason: "missing_field".to_string(),
            details: "Missing required field \"verdict\" (must be \"LGTM\" or \"NEEDS_FIX\")."
                .to_string(),
        }),
    }
}

fn validate_fix_result(json: Value) -> ContractValidationResult {
    let status = json.get("status").and_then(|v| v.as_str());
    match status {
        Some("FIXED") | Some("PARTIAL") | Some("BLOCKED") => ContractValidationResult::Valid {
            result: Some(status.unwrap().to_string()),
            structured_output: json,
        },
        Some(other) => ContractValidationResult::Invalid(ContractViolation {
            reason: "invalid_status".to_string(),
            details: format!(
                "status must be \"FIXED\", \"PARTIAL\", or \"BLOCKED\", got \"{other}\"."
            ),
        }),
        None => ContractValidationResult::Invalid(ContractViolation {
            reason: "missing_field".to_string(),
            details:
                "Missing required field \"status\" (must be \"FIXED\", \"PARTIAL\", or \"BLOCKED\")."
                    .to_string(),
        }),
    }
}

fn validate_spec_file_path(json: Value) -> ContractValidationResult {
    let path = json.get("spec_file_path").and_then(|v| v.as_str());
    match path {
        Some(p) if !p.is_empty() => {
            let is_drive_letter_abs = p.len() >= 3
                && p.as_bytes()[0].is_ascii_alphabetic()
                && p.as_bytes()[1] == b':'
                && (p.as_bytes()[2] == b'/' || p.as_bytes()[2] == b'\\');
            if p.starts_with('/') || p.starts_with('\\') || is_drive_letter_abs {
                return ContractValidationResult::Invalid(ContractViolation {
                    reason: "invalid_path".to_string(),
                    details: format!(
                        "spec_file_path must be a relative path, got absolute path: \"{p}\""
                    ),
                });
            }
            if p.contains("..") {
                return ContractValidationResult::Invalid(ContractViolation {
                    reason: "invalid_path".to_string(),
                    details: format!("spec_file_path must not contain \"..\": \"{p}\""),
                });
            }
            ContractValidationResult::Valid {
                result: None,
                structured_output: json,
            }
        }
        _ => ContractValidationResult::Invalid(ContractViolation {
            reason: "missing_field".to_string(),
            details: "Missing required field \"spec_file_path\".".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- [08] validate_contract_value: pure validator (CLI / engine 共通入口) ----

    #[test]
    fn validate_contract_value_accepts_compliant_review_verdict_lgtm() {
        match validate_contract_value("review-verdict", json!({"verdict": "LGTM"})) {
            ContractValidationResult::Valid {
                result,
                structured_output,
            } => {
                assert_eq!(result, Some("LGTM".to_string()));
                assert_eq!(structured_output["verdict"], "LGTM");
            }
            other => panic!("expected Valid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_accepts_compliant_review_verdict_needs_fix() {
        match validate_contract_value(
            "review-verdict",
            json!({"verdict": "NEEDS_FIX", "findings": [{"severity": "error", "message": "bug"}]}),
        ) {
            ContractValidationResult::Valid { result, .. } => {
                assert_eq!(result, Some("NEEDS_FIX".to_string()));
            }
            other => panic!("expected Valid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_review_verdict_missing_verdict() {
        match validate_contract_value("review-verdict", json!({"summary": "looks ok"})) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "missing_field");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_invalid_verdict() {
        match validate_contract_value("review-verdict", json!({"verdict": "MAYBE"})) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_verdict");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_needs_fix_without_findings() {
        match validate_contract_value("review-verdict", json!({"verdict": "NEEDS_FIX"})) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "missing_findings");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_needs_fix_with_empty_findings() {
        match validate_contract_value(
            "review-verdict",
            json!({"verdict": "NEEDS_FIX", "findings": []}),
        ) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "missing_findings");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_finding_missing_severity() {
        match validate_contract_value(
            "review-verdict",
            json!({"verdict": "NEEDS_FIX", "findings": [{"message": "bug"}]}),
        ) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_finding");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_finding_missing_message() {
        match validate_contract_value(
            "review-verdict",
            json!({"verdict": "NEEDS_FIX", "findings": [{"severity": "error"}]}),
        ) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_finding");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_accepts_fix_result_statuses() {
        for status in &["FIXED", "PARTIAL", "BLOCKED"] {
            match validate_contract_value("fix-result", json!({"status": status})) {
                ContractValidationResult::Valid { result, .. } => {
                    assert_eq!(result, Some(status.to_string()));
                }
                other => panic!("expected Valid for status {status}, got {:?}", other),
            }
        }
    }

    #[test]
    fn validate_contract_value_rejects_invalid_fix_result_status() {
        match validate_contract_value("fix-result", json!({"status": "DONE"})) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_status");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_missing_fix_result_status() {
        match validate_contract_value("fix-result", json!({"summary": "done"})) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "missing_field");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_accepts_relative_spec_file_path() {
        match validate_contract_value(
            "spec-file-path",
            json!({"spec_file_path": "docs/spec/issues-1029.md"}),
        ) {
            ContractValidationResult::Valid { result, .. } => {
                assert_eq!(result, None);
            }
            other => panic!("expected Valid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_absolute_spec_file_path() {
        match validate_contract_value("spec-file-path", json!({"spec_file_path": "/etc/passwd"})) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_path");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_spec_file_path_traversal() {
        match validate_contract_value(
            "spec-file-path",
            json!({"spec_file_path": "docs/../../etc/passwd"}),
        ) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_path");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_rejects_windows_drive_spec_file_path() {
        for path in &["C:\\repo\\spec.md", "C:/repo/spec.md", "D:\\file.md"] {
            match validate_contract_value("spec-file-path", json!({"spec_file_path": path})) {
                ContractValidationResult::Invalid(v) => {
                    assert_eq!(v.reason, "invalid_path", "path: {path}");
                }
                other => panic!("expected Invalid for path '{path}', got {:?}", other),
            }
        }
    }

    #[test]
    fn validate_contract_value_rejects_missing_spec_file_path_field() {
        match validate_contract_value("spec-file-path", json!({"path": "some/path.md"})) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "missing_field");
            }
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_contract_value_accepts_approved_fix_policy_with_arbitrary_fields() {
        let input = json!({
            "policy": "Fix only NEEDS_FIX findings.",
            "review_step": "code_review_parallel",
            "extra": {"nested": "value"}
        });
        match validate_contract_value("approved-fix-policy", input.clone()) {
            ContractValidationResult::Valid {
                result,
                structured_output,
            } => {
                assert_eq!(result, Some("approved".to_string()));
                assert_eq!(structured_output, input);
            }
            other => panic!("expected Valid, got {:?}", other),
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
}
