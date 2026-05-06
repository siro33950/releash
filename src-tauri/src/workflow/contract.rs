use serde_json::Value;

/// `<workflow_output>` ブロックの抽出結果。
#[derive(Debug, Clone)]
pub enum ExtractionResult {
    Found { type_name: String, json: Value },
    NoBlock,
    MultipleBlocks,
    InvalidJson(String),
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

/// LLMがコードフェンスで囲むパターン（```json ... ``` や ``` ... ```）を除去する。
fn strip_code_fences(s: &str) -> &str {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        // ```json\n...\n``` → skip language tag line
        let body = if let Some(newline_pos) = rest.find('\n') {
            &rest[newline_pos + 1..]
        } else {
            rest
        };
        body.strip_suffix("```").unwrap_or(body).trim()
    } else {
        trimmed
    }
}

/// assistant responseからXMLブロック `<workflow_output type="...">{ JSON }</workflow_output>` を抽出する。
pub fn extract_workflow_output(text: &str) -> ExtractionResult {
    let open_tag = "<workflow_output";
    let close_tag = "</workflow_output>";

    let mut found: Vec<(String, &str)> = Vec::new();
    let mut search_start = 0;

    while let Some(tag_start) = text[search_start..].find(open_tag) {
        let abs_tag_start = search_start + tag_start;
        let after_tag = &text[abs_tag_start + open_tag.len()..];

        // type属性を抽出
        let type_name = if let Some(type_start) = after_tag.find("type=\"") {
            let value_start = type_start + 6;
            if let Some(value_end) = after_tag[value_start..].find('"') {
                after_tag[value_start..value_start + value_end].to_string()
            } else {
                String::new()
            }
        } else if let Some(type_start) = after_tag.find("type='") {
            let value_start = type_start + 6;
            if let Some(value_end) = after_tag[value_start..].find('\'') {
                after_tag[value_start..value_start + value_end].to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // `>` を見つけてcontent開始位置を確定
        if let Some(gt_pos) = after_tag.find('>') {
            let content_start = abs_tag_start + open_tag.len() + gt_pos + 1;
            if let Some(close_pos) = text[content_start..].find(close_tag) {
                let content = text[content_start..content_start + close_pos].trim();
                found.push((type_name, content));
                search_start = content_start + close_pos + close_tag.len();
            } else {
                search_start = content_start;
            }
        } else {
            search_start = abs_tag_start + open_tag.len();
        }
    }

    match found.len() {
        0 => ExtractionResult::NoBlock,
        1 => {
            let (type_name, content) = &found[0];
            let stripped = strip_code_fences(content);
            match serde_json::from_str::<Value>(stripped) {
                Ok(json) => ExtractionResult::Found {
                    type_name: type_name.clone(),
                    json,
                },
                Err(e) => ExtractionResult::InvalidJson(e.to_string()),
            }
        }
        _ => ExtractionResult::MultipleBlocks,
    }
}

/// type照合 + contract-specific validation。
pub fn validate_contract(
    expected_type: &str,
    extraction: ExtractionResult,
) -> ContractValidationResult {
    match extraction {
        ExtractionResult::NoBlock => ContractValidationResult::Invalid(ContractViolation {
            reason: "no_block".to_string(),
            details: "No <workflow_output> block found in the response.".to_string(),
        }),
        ExtractionResult::MultipleBlocks => ContractValidationResult::Invalid(ContractViolation {
            reason: "multiple_blocks".to_string(),
            details: "Multiple <workflow_output> blocks found. Provide exactly one.".to_string(),
        }),
        ExtractionResult::InvalidJson(err) => {
            ContractValidationResult::Invalid(ContractViolation {
                reason: "invalid_json".to_string(),
                details: format!("JSON parse error in <workflow_output>: {err}"),
            })
        }
        ExtractionResult::Found { type_name, json } => {
            if type_name != expected_type {
                return ContractValidationResult::Invalid(ContractViolation {
                    reason: "type_mismatch".to_string(),
                    details: format!(
                        "Expected type=\"{expected_type}\", got type=\"{type_name}\"."
                    ),
                });
            }
            validate_contract_specific(expected_type, json)
        }
    }
}

/// contract typeごとのバリデーション。
fn validate_contract_specific(contract_type: &str, json: Value) -> ContractValidationResult {
    match contract_type {
        "review-verdict" => validate_review_verdict(json),
        "fix-result" => validate_fix_result(json),
        "spec-file-path" => validate_spec_file_path(json),
        _ => {
            // 未知のcontract typeはJSONが有効ならそのまま通す
            ContractValidationResult::Valid {
                structured_output: json,
                result: None,
            }
        }
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
            if p.starts_with('/') || p.starts_with('\\') {
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

/// contract violation時のrepair promptを生成する。
/// `contract_definition` にファセットのmarkdown定義を渡すと、エージェントが正しい形式を把握できる。
pub fn build_repair_prompt(
    contract_type: &str,
    violation: &ContractViolation,
    contract_definition: Option<&str>,
) -> String {
    let definition_section = match contract_definition {
        Some(def) => {
            format!("\n\n--- Contract Definition ---\n{def}\n--- End Contract Definition ---")
        }
        None => String::new(),
    };
    format!(
        "Your previous response did not satisfy the output contract \"{contract_type}\".\n\
         Reason: {reason}\n\
         Details: {details}\n\n\
         Please provide your response again with a valid <workflow_output type=\"{contract_type}\"> block containing well-formed JSON.\n\
         The output must satisfy all required fields for the \"{contract_type}\" contract.{definition_section}",
        reason = violation.reason,
        details = violation.details,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_valid_block() {
        let text = r#"Some text before
<workflow_output type="review-verdict">
{"verdict": "LGTM", "summary": "All good"}
</workflow_output>
Some text after"#;
        match extract_workflow_output(text) {
            ExtractionResult::Found { type_name, json } => {
                assert_eq!(type_name, "review-verdict");
                assert_eq!(json["verdict"], "LGTM");
            }
            other => panic!("Expected Found, got {:?}", other),
        }
    }

    #[test]
    fn extract_no_block() {
        let text = "Just some regular text without any blocks";
        assert!(matches!(
            extract_workflow_output(text),
            ExtractionResult::NoBlock
        ));
    }

    #[test]
    fn extract_multiple_blocks() {
        let text = r#"<workflow_output type="a">{"x":1}</workflow_output>
<workflow_output type="b">{"y":2}</workflow_output>"#;
        assert!(matches!(
            extract_workflow_output(text),
            ExtractionResult::MultipleBlocks
        ));
    }

    #[test]
    fn extract_invalid_json() {
        let text = r#"<workflow_output type="test">{not valid json}</workflow_output>"#;
        assert!(matches!(
            extract_workflow_output(text),
            ExtractionResult::InvalidJson(_)
        ));
    }

    #[test]
    fn validate_review_verdict_lgtm() {
        let extraction = ExtractionResult::Found {
            type_name: "review-verdict".to_string(),
            json: json!({"verdict": "LGTM"}),
        };
        match validate_contract("review-verdict", extraction) {
            ContractValidationResult::Valid {
                result,
                structured_output,
            } => {
                assert_eq!(result, Some("LGTM".to_string()));
                assert_eq!(structured_output["verdict"], "LGTM");
            }
            other => panic!("Expected Valid, got {:?}", other),
        }
    }

    #[test]
    fn validate_review_verdict_needs_fix() {
        let extraction = ExtractionResult::Found {
            type_name: "review-verdict".to_string(),
            json: json!({"verdict": "NEEDS_FIX", "findings": [{"severity": "error", "message": "bug"}]}),
        };
        match validate_contract("review-verdict", extraction) {
            ContractValidationResult::Valid { result, .. } => {
                assert_eq!(result, Some("NEEDS_FIX".to_string()));
            }
            other => panic!("Expected Valid, got {:?}", other),
        }
    }

    #[test]
    fn validate_review_verdict_missing_field() {
        let extraction = ExtractionResult::Found {
            type_name: "review-verdict".to_string(),
            json: json!({"summary": "looks ok"}),
        };
        assert!(matches!(
            validate_contract("review-verdict", extraction),
            ContractValidationResult::Invalid(_)
        ));
    }

    #[test]
    fn validate_type_mismatch() {
        let extraction = ExtractionResult::Found {
            type_name: "fix-result".to_string(),
            json: json!({"status": "FIXED"}),
        };
        match validate_contract("review-verdict", extraction) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "type_mismatch");
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_fix_result_valid() {
        let extraction = ExtractionResult::Found {
            type_name: "fix-result".to_string(),
            json: json!({"status": "FIXED"}),
        };
        match validate_contract("fix-result", extraction) {
            ContractValidationResult::Valid { result, .. } => {
                assert_eq!(result, Some("FIXED".to_string()));
            }
            other => panic!("Expected Valid, got {:?}", other),
        }
    }

    #[test]
    fn validate_spec_file_path_valid() {
        let extraction = ExtractionResult::Found {
            type_name: "spec-file-path".to_string(),
            json: json!({"spec_file_path": "docs/spec/issues-898.md"}),
        };
        match validate_contract("spec-file-path", extraction) {
            ContractValidationResult::Valid { result, .. } => {
                assert_eq!(result, None);
            }
            other => panic!("Expected Valid, got {:?}", other),
        }
    }

    #[test]
    fn validate_spec_file_path_missing() {
        let extraction = ExtractionResult::Found {
            type_name: "spec-file-path".to_string(),
            json: json!({"path": "some/path.md"}),
        };
        assert!(matches!(
            validate_contract("spec-file-path", extraction),
            ContractValidationResult::Invalid(_)
        ));
    }

    #[test]
    fn validate_no_block() {
        match validate_contract("review-verdict", ExtractionResult::NoBlock) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "no_block");
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_multiple_blocks() {
        match validate_contract("review-verdict", ExtractionResult::MultipleBlocks) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "multiple_blocks");
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_invalid_json_error() {
        match validate_contract(
            "review-verdict",
            ExtractionResult::InvalidJson("bad json".to_string()),
        ) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_json");
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_unknown_contract_type_passes() {
        let extraction = ExtractionResult::Found {
            type_name: "custom-thing".to_string(),
            json: json!({"anything": "goes"}),
        };
        match validate_contract("custom-thing", extraction) {
            ContractValidationResult::Valid { result, .. } => {
                assert_eq!(result, None);
            }
            other => panic!("Expected Valid, got {:?}", other),
        }
    }

    #[test]
    fn validate_review_verdict_needs_fix_no_findings() {
        let extraction = ExtractionResult::Found {
            type_name: "review-verdict".to_string(),
            json: json!({"verdict": "NEEDS_FIX"}),
        };
        match validate_contract("review-verdict", extraction) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "missing_findings");
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_review_verdict_needs_fix_empty_findings() {
        let extraction = ExtractionResult::Found {
            type_name: "review-verdict".to_string(),
            json: json!({"verdict": "NEEDS_FIX", "findings": []}),
        };
        match validate_contract("review-verdict", extraction) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "missing_findings");
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_fix_result_partial() {
        let extraction = ExtractionResult::Found {
            type_name: "fix-result".to_string(),
            json: json!({"status": "PARTIAL"}),
        };
        match validate_contract("fix-result", extraction) {
            ContractValidationResult::Valid { result, .. } => {
                assert_eq!(result, Some("PARTIAL".to_string()));
            }
            other => panic!("Expected Valid, got {:?}", other),
        }
    }

    #[test]
    fn validate_fix_result_blocked() {
        let extraction = ExtractionResult::Found {
            type_name: "fix-result".to_string(),
            json: json!({"status": "BLOCKED"}),
        };
        match validate_contract("fix-result", extraction) {
            ContractValidationResult::Valid { result, .. } => {
                assert_eq!(result, Some("BLOCKED".to_string()));
            }
            other => panic!("Expected Valid, got {:?}", other),
        }
    }

    #[test]
    fn validate_fix_result_invalid_status() {
        let extraction = ExtractionResult::Found {
            type_name: "fix-result".to_string(),
            json: json!({"status": "DONE"}),
        };
        match validate_contract("fix-result", extraction) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_status");
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn build_repair_prompt_contains_info() {
        let violation = ContractViolation {
            reason: "missing_field".to_string(),
            details: "Missing \"verdict\" field.".to_string(),
        };
        let prompt = build_repair_prompt("review-verdict", &violation, None);
        assert!(prompt.contains("review-verdict"));
        assert!(prompt.contains("missing_field"));
        assert!(prompt.contains("Missing \"verdict\" field."));
        assert!(!prompt.contains("Contract Definition"));
    }

    #[test]
    fn build_repair_prompt_includes_definition() {
        let violation = ContractViolation {
            reason: "no_block".to_string(),
            details: "No <workflow_output> block found.".to_string(),
        };
        let definition = "You MUST include exactly one `<workflow_output>` block.\nFormat: ...";
        let prompt = build_repair_prompt("review-verdict", &violation, Some(definition));
        assert!(prompt.contains("review-verdict"));
        assert!(prompt.contains("no_block"));
        assert!(prompt.contains("Contract Definition"));
        assert!(prompt.contains(definition));
    }

    // ---- R3-01: findings severity/message validation ----

    #[test]
    fn validate_review_verdict_needs_fix_finding_missing_severity() {
        let extraction = ExtractionResult::Found {
            type_name: "review-verdict".to_string(),
            json: json!({"verdict": "NEEDS_FIX", "findings": [{"message": "bug"}]}),
        };
        match validate_contract("review-verdict", extraction) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_finding");
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_review_verdict_needs_fix_finding_missing_message() {
        let extraction = ExtractionResult::Found {
            type_name: "review-verdict".to_string(),
            json: json!({"verdict": "NEEDS_FIX", "findings": [{"severity": "error"}]}),
        };
        match validate_contract("review-verdict", extraction) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_finding");
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_review_verdict_needs_fix_finding_empty_object() {
        let extraction = ExtractionResult::Found {
            type_name: "review-verdict".to_string(),
            json: json!({"verdict": "NEEDS_FIX", "findings": [{}]}),
        };
        match validate_contract("review-verdict", extraction) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_finding");
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_review_verdict_needs_fix_valid_findings() {
        let extraction = ExtractionResult::Found {
            type_name: "review-verdict".to_string(),
            json: json!({"verdict": "NEEDS_FIX", "findings": [
                {"severity": "error", "message": "bug found"},
                {"severity": "warning", "message": "style issue"}
            ]}),
        };
        match validate_contract("review-verdict", extraction) {
            ContractValidationResult::Valid { result, .. } => {
                assert_eq!(result, Some("NEEDS_FIX".to_string()));
            }
            other => panic!("Expected Valid, got {:?}", other),
        }
    }

    // ---- R5-01: spec-file-path path traversal prevention ----

    #[test]
    fn validate_spec_file_path_absolute_path_rejected() {
        let extraction = ExtractionResult::Found {
            type_name: "spec-file-path".to_string(),
            json: json!({"spec_file_path": "/etc/passwd"}),
        };
        match validate_contract("spec-file-path", extraction) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_path");
                assert!(v.details.contains("absolute"));
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_spec_file_path_traversal_rejected() {
        let extraction = ExtractionResult::Found {
            type_name: "spec-file-path".to_string(),
            json: json!({"spec_file_path": "docs/../../../etc/passwd"}),
        };
        match validate_contract("spec-file-path", extraction) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_path");
                assert!(v.details.contains(".."));
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn validate_spec_file_path_backslash_absolute_rejected() {
        let extraction = ExtractionResult::Found {
            type_name: "spec-file-path".to_string(),
            json: json!({"spec_file_path": "\\\\server\\share"}),
        };
        match validate_contract("spec-file-path", extraction) {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "invalid_path");
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    // ---- R4-02: contract retry flow (pure function tests) ----

    #[test]
    fn contract_retry_flow_no_block_then_valid() {
        // 1st attempt: no block → Invalid
        let text1 = "I reviewed the code and it looks good.";
        let extraction1 = extract_workflow_output(text1);
        let result1 = validate_contract("review-verdict", extraction1);
        assert!(matches!(result1, ContractValidationResult::Invalid(_)));

        // Build repair prompt from violation
        if let ContractValidationResult::Invalid(v) = result1 {
            assert_eq!(v.reason, "no_block");
            let repair = build_repair_prompt("review-verdict", &v, None);
            assert!(repair.contains("review-verdict"));
            assert!(repair.contains("no_block"));
        }

        // 2nd attempt: valid → success
        let text2 = r#"<workflow_output type="review-verdict">
{"verdict": "LGTM"}
</workflow_output>"#;
        let extraction2 = extract_workflow_output(text2);
        let result2 = validate_contract("review-verdict", extraction2);
        match result2 {
            ContractValidationResult::Valid { result, .. } => {
                assert_eq!(result, Some("LGTM".to_string()));
            }
            other => panic!("Expected Valid, got {:?}", other),
        }
    }

    #[test]
    fn contract_retry_flow_type_mismatch_then_valid() {
        // 1st attempt: wrong type
        let text1 = r#"<workflow_output type="fix-result">
{"status": "FIXED"}
</workflow_output>"#;
        let extraction1 = extract_workflow_output(text1);
        let result1 = validate_contract("review-verdict", extraction1);
        if let ContractValidationResult::Invalid(v) = &result1 {
            assert_eq!(v.reason, "type_mismatch");
            let repair = build_repair_prompt("review-verdict", v, None);
            assert!(repair.contains("type_mismatch"));
        } else {
            panic!("Expected Invalid");
        }

        // 2nd attempt: correct type
        let text2 = r#"<workflow_output type="review-verdict">
{"verdict": "NEEDS_FIX", "findings": [{"severity": "error", "message": "bug"}]}
</workflow_output>"#;
        let extraction2 = extract_workflow_output(text2);
        let result2 = validate_contract("review-verdict", extraction2);
        assert!(matches!(result2, ContractValidationResult::Valid { .. }));
    }

    #[test]
    fn contract_retry_flow_invalid_json_then_valid() {
        let text1 = r#"<workflow_output type="review-verdict">{bad json}</workflow_output>"#;
        let extraction1 = extract_workflow_output(text1);
        let result1 = validate_contract("review-verdict", extraction1);
        if let ContractValidationResult::Invalid(v) = &result1 {
            assert_eq!(v.reason, "invalid_json");
            let repair = build_repair_prompt("review-verdict", v, None);
            assert!(repair.contains("invalid_json"));
        } else {
            panic!("Expected Invalid");
        }
    }

    #[test]
    fn contract_retry_flow_contract_specific_failure() {
        // Missing verdict field
        let text1 = r#"<workflow_output type="review-verdict">
{"summary": "looks good"}
</workflow_output>"#;
        let extraction1 = extract_workflow_output(text1);
        let result1 = validate_contract("review-verdict", extraction1);
        if let ContractValidationResult::Invalid(v) = &result1 {
            assert_eq!(v.reason, "missing_field");
            let repair = build_repair_prompt("review-verdict", v, None);
            assert!(repair.contains("missing_field"));
            assert!(repair.contains("review-verdict"));
        } else {
            panic!("Expected Invalid");
        }
    }

    #[test]
    fn contract_retry_flow_multiple_blocks() {
        let text = r#"<workflow_output type="review-verdict">{"verdict":"LGTM"}</workflow_output>
<workflow_output type="review-verdict">{"verdict":"NEEDS_FIX","findings":[{"severity":"error","message":"x"}]}</workflow_output>"#;
        let extraction = extract_workflow_output(text);
        let result = validate_contract("review-verdict", extraction);
        if let ContractValidationResult::Invalid(v) = &result {
            assert_eq!(v.reason, "multiple_blocks");
            let repair = build_repair_prompt("review-verdict", v, None);
            assert!(repair.contains("multiple_blocks"));
        } else {
            panic!("Expected Invalid");
        }
    }

    // ---- strip_code_fences ----

    #[test]
    fn strip_code_fences_json_block() {
        let input = "```json\n{\"verdict\": \"LGTM\"}\n```";
        assert_eq!(strip_code_fences(input), r#"{"verdict": "LGTM"}"#);
    }

    #[test]
    fn strip_code_fences_no_language_tag() {
        let input = "```\n{\"verdict\": \"LGTM\"}\n```";
        assert_eq!(strip_code_fences(input), r#"{"verdict": "LGTM"}"#);
    }

    #[test]
    fn strip_code_fences_plain_json() {
        let input = r#"{"verdict": "LGTM"}"#;
        assert_eq!(strip_code_fences(input), input);
    }

    #[test]
    fn strip_code_fences_with_surrounding_whitespace() {
        let input = "  \n```json\n{\"x\":1}\n```\n  ";
        assert_eq!(strip_code_fences(input), r#"{"x":1}"#);
    }

    #[test]
    fn strip_code_fences_no_closing_fence() {
        let input = "```json\n{\"x\":1}";
        assert_eq!(strip_code_fences(input), r#"{"x":1}"#);
    }

    #[test]
    fn extract_with_code_fences_in_workflow_output() {
        let text = r#"<workflow_output type="review-verdict">
```json
{"verdict": "LGTM", "summary": "All good"}
```
</workflow_output>"#;
        match extract_workflow_output(text) {
            ExtractionResult::Found { type_name, json } => {
                assert_eq!(type_name, "review-verdict");
                assert_eq!(json["verdict"], "LGTM");
            }
            other => panic!("Expected Found, got {:?}", other),
        }
    }

    // ---- R4-01: output_text None → NoBlock validation ----

    #[test]
    fn no_block_extraction_triggers_retry_flow() {
        // engine.rsでoutput_text: Noneの場合、ExtractionResult::NoBlockとしてvalidate_contractに渡される
        let result = validate_contract("review-verdict", ExtractionResult::NoBlock);
        match result {
            ContractValidationResult::Invalid(v) => {
                assert_eq!(v.reason, "no_block");
                // repair promptが生成できることを確認
                let repair = build_repair_prompt("review-verdict", &v, None);
                assert!(repair.contains("review-verdict"));
                assert!(repair.contains("No <workflow_output> block found"));
            }
            other => panic!("Expected Invalid(no_block), got {:?}", other),
        }
    }
}
