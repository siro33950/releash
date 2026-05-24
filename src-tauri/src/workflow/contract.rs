//! [08] contract validator。
//!
//! 旧来「`<workflow_output>` ブロックを agent 自由文から抽出して contract を判定する」
//! prose 抽出経路は本 issue で完全廃止された（spec [08] L137 / Rule 4 構造化出力の
//! 確定経路は明示的提出のみに統一）。本モジュールは CLI / Tauri 経由の
//! `SubmitOutput` から呼ばれる pure validator (`validate_contract_value`) を唯一の
//! 検証入口として提供する。

use serde_json::Value;

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

    // ---- [08] resolve_step_output_contract_from_events: CLI / Tauri 共通の解決経路 ----

    fn workflow_with_review_contract(contract: Option<&str>) -> Workflow {
        use crate::workflow::schema::{NodeDefinition, NodeType};
        Workflow {
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
            "review-verdict",
        )))];
        let contract = resolve_step_output_contract_from_events(&events, "review")
            .expect("contract should be resolved");
        assert_eq!(contract, "review-verdict");
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
