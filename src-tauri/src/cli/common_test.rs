use super::*;

#[test]
fn exit_code_mapping_is_stable() {
    assert_eq!(cli_result_exit_code(Ok(CliSuccess::ok(String::new()))), 0);
    assert_eq!(
        cli_result_exit_code(Ok(CliSuccess::with_exit_code(
            String::new(),
            DIAGNOSTIC_ERRORS_EXIT_CODE,
        ))),
        3
    );
    assert_eq!(
        cli_error_exit_code(&CliError::InvalidInput("bad".to_string())),
        2
    );
    assert_eq!(
        cli_error_exit_code(&CliError::NotFound("missing".to_string())),
        4
    );
    assert_eq!(cli_error_exit_code(&CliError::Other("io".to_string())), 1);
}

#[test]
fn cli_error_stderr_mapping_is_stable() {
    assert_eq!(
        cli_error_stderr(&CliError::InvalidInput("bad".to_string())),
        "error: bad"
    );
    assert_eq!(
        cli_error_stderr(&CliError::NotFound("missing".to_string())),
        "missing"
    );
    assert_eq!(
        cli_error_stderr(&CliError::Other("io".to_string())),
        "error: io"
    );
}

#[test]
fn validate_execution_id_rejects_non_uuid() {
    assert!(validate_execution_id("not-a-uuid").is_err());
    assert!(validate_execution_id("").is_err());
    assert!(validate_execution_id("../etc/passwd").is_err());
}

#[test]
fn validate_execution_id_accepts_valid_uuid() {
    assert!(validate_execution_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
}

/// spec [01] 解決順序「明示指定 > alias 内包値」: RELEASH_DATA_DIR が明示
/// 指定されている場合は、その値がそのまま採用される（PathBuf 化のみ）。
#[test]
fn resolve_data_dir_uses_explicit_env_when_set() {
    let resolved = resolve_data_dir_from_env(Some("/explicit/path".to_string())).unwrap();
    assert_eq!(resolved, std::path::PathBuf::from("/explicit/path"));
}
