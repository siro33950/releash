use super::*;

#[test]
fn validate_name_rejects_leading_special_chars() {
    assert!(validation::validate_name("-starts-with-dash").is_err());
    assert!(validation::validate_name("_starts-with-underscore").is_err());
    assert!(validation::validate_name("a-valid-name").is_ok());
    assert!(validation::validate_name("1-starts-with-digit").is_ok());
}

#[test]
fn test_診断失敗_domainと同じ文言を返す() {
    // Given
    let items = vec![DiagnosticItem::new(
        "WFS006",
        Severity::Error,
        DiagnosticStage::ParseShape,
        None,
        "duplicate",
    )];
    let error = StorageError::Diagnostics(items);
    // When
    let message = error.to_string();
    // Then
    assert_eq!(message, "workflow_diagnostics: WFS006: duplicate");
}
