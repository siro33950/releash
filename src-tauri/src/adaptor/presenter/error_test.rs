use super::*;

#[test]
fn test_表示失敗_cause設定でstatusと文言を保ち二重に包まない() {
    // Given
    for source in [
        AppError::new("message"),
        AppError::new("message").with_status(connectrpc::ErrorCode::Unavailable),
    ] {
        let kind = source.connect_code();
        // When
        let result = source.with_cause(Some("source failure".into()));
        // Then
        assert_eq!(result.connect_code(), kind);
        assert_eq!(result.cause(), Some("source failure"));
        assert_eq!(result.to_string(), "message");
        let AppError::Presented { error, .. } = result else {
            panic!("expected presented")
        };
        assert!(matches!(*error, AppError::Internal(_)));
    }
}
