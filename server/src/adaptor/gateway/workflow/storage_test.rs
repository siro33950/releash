use super::*;

#[test]
fn validate_name_rejects_leading_special_chars() {
    assert!(validation::validate_name("-starts-with-dash").is_err());
    assert!(validation::validate_name("_starts-with-underscore").is_err());
    assert!(validation::validate_name("a-valid-name").is_ok());
    assert!(validation::validate_name("1-starts-with-digit").is_ok());
}

#[test]
fn test_workflow保存先_指定されたディレクトリを使い空指定は既定へ戻る() {
    // Given
    let _lock = crate::test_support::TEST_ENV_LOCK.lock();
    let expected = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("releash")
        .join("workflows");
    let _empty = crate::test_support::EnvVarGuard::set_value("RELEASH_WORKFLOWS_DIR", "");
    // When / Then
    assert_eq!(workflows_dir(), expected);
    let _directory =
        crate::test_support::EnvVarGuard::set_value("RELEASH_WORKFLOWS_DIR", "/isolated/workflows");
    assert_eq!(workflows_dir(), PathBuf::from("/isolated/workflows"));
}
