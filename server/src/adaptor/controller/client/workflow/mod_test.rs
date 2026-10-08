use super::*;

#[test]
fn workflow_tab_error_is_redacted() {
    let err = redacted_workflow_tab_error("workflow_node_session_rejected");
    assert_eq!(
        err,
        "workflow_node_session_rejected: workflow node tab operation failed"
    );
    assert!(!err.contains("/repo"));
    assert!(!err.contains("agent-session"));
    assert!(!err.contains("message body"));
}

/// Spec issues-1011 finding 12: command 入口の `validate_execution_id` は path traversal や
/// 形式不正な execution_id を拒否し、後段の Execution Store / engine に到達させない。
/// abort_workflow / get_workflow_state / approve_workflow_node /
/// get_workflow_execution / get_workflow_execution_log / get_workflow_execution_state /
/// resolve_worktree_by_execution の全 command で共通に使われるため、入力種別ごとに
/// 受理/拒否を一括で担保する。
#[test]
fn validate_execution_id_table_accepts_uuid_and_rejects_invalid_inputs() {
    // 受理: 正規 UUID（生成値）と既知サンプル
    let generated = uuid::Uuid::new_v4().to_string();
    let accepted = [
        generated.as_str(),
        "550e8400-e29b-41d4-a716-446655440000",
        "00000000-0000-0000-0000-000000000000",
    ];
    for input in accepted {
        assert!(
            validate_execution_id(input).is_ok(),
            "valid UUID must be accepted: {input}"
        );
    }

    // 拒否: 空文字 / 非 UUID / path traversal / 不正文字 / 余分なスペース / 長さ違い
    let rejected = [
        "",
        "not-a-uuid",
        "../etc/passwd",
        "../../workflow_executions/secret",
        "execution-1",
        "550e8400-e29b-41d4-a716-44665544000",   // 1 文字不足
        "550e8400-e29b-41d4-a716-4466554400000", // 1 文字過剰
        "550e8400-e29b-41d4-a716-44665544000g",  // 非 hex
        "550e8400-e29b-41d4-a716-446655440000\n",
        " 550e8400-e29b-41d4-a716-446655440000",
        "550e8400-e29b-41d4-a716-446655440000 ",
    ];
    for input in rejected {
        assert!(
            validate_execution_id(input).is_err(),
            "invalid execution_id must be rejected: {input:?}"
        );
    }
}

#[test]
fn parse_execution_origin_rejects_unknown_values() {
    assert!(matches!(
        parse_execution_origin(None).unwrap(),
        crate::domain::workflow::ExecutionOrigin::DesktopUi
    ));
    for invalid in ["remote", "unknown"] {
        let err = parse_execution_origin(Some(invalid.to_string()))
            .expect_err("unknown execution origins must be rejected");
        assert!(err.contains("unknown created_from"));
    }
}

#[test]
fn parse_facet_kind_unknown_returns_error() {
    assert!(parse_facet_kind("unknown").is_err());
    assert!(parse_facet_kind("").is_err());
}

/// Gherkin: parse_facet_kind を経由する Tauri コマンドは 3種それぞれの種別指定で
/// 正常経路に到達する（種別解決層）
#[test]
fn parse_facet_kind_resolves_all_three_kinds_for_command_routing() {
    for kind in ["policy", "knowledge", "instruction"] {
        assert!(
            parse_facet_kind(kind).is_ok(),
            "kind '{kind}' should be accepted"
        );
    }
}

#[test]
fn validate_template_variables_artifact_refs_ok() {
    assert!(validate_template_variables("Use {{ request }} and {{ plan.summary }}").is_ok());
}

#[test]
fn validate_template_variables_invalid_ref_fails() {
    let result = validate_template_variables("Use {{ bad ref }}");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("bad ref"));
}

#[test]
fn validate_template_variables_no_vars_ok() {
    assert!(validate_template_variables("No variables here").is_ok());
}

#[test]
fn test_template変数検証_0段と多段の参照を受理する() {
    let result = validate_template_variables("{{ goal }} and {{ goal.a.b }}");
    assert!(result.is_ok());
}

#[test]
fn duplicate_workflow_rejects_invalid_name() {
    let result = crate::domain::workflow::validation::validate_name("bad name!");
    assert!(result.is_err());
}

#[test]
fn validation_errors_return_stable_kind_prefix_for_commands() {
    let err = crate::domain::workflow::validation::validate_name("bad name!")
        .map_err(validation_error_string)
        .unwrap_err();
    assert!(err.starts_with("validation_error:"));
}

#[test]
fn duplicate_facet_rejects_invalid_key() {
    let result = crate::adaptor::gateway::workflow::facet::validate_facet_key("../evil");
    assert!(result.is_err());
}
