use super::*;
#[test]
fn test_購読開始_重複したreadyで開始要求が増えない() {
    // Given
    let client = client(&ClientConnectionDto {
        url: "http://127.0.0.1:1".into(),
        token: "client".into(),
    })
    .unwrap();
    let mut subscription = SettingsSubscription::default();
    subscription.request_if_needed(&client, "client-id");
    let requested = subscription.has_pending();
    subscription.pending = None;
    // When
    subscription.request_if_needed(&client, "client-id");
    // Then
    assert!(requested);
    assert!(!subscription.has_pending());
}

#[test]
fn test_生存確認_connectの失敗を技術的な失敗の性質へ写す() {
    use connectrpc::ErrorCode;
    // Given / When / Then
    for (code, nature) in [
        (
            ErrorCode::DeadlineExceeded,
            TechnicalFailureNature::TimedOut,
        ),
        (ErrorCode::Unavailable, TechnicalFailureNature::Transient),
        (
            ErrorCode::ResourceExhausted,
            TechnicalFailureNature::Transient,
        ),
        (ErrorCode::Aborted, TechnicalFailureNature::Other),
        (ErrorCode::Unauthenticated, TechnicalFailureNature::Other),
        (ErrorCode::Canceled, TechnicalFailureNature::Cancelled),
        (ErrorCode::Internal, TechnicalFailureNature::Other),
    ] {
        let error = connectrpc::ConnectError::new(code, "reason");
        let message = error.to_string();
        let failure = liveness_failure(error);
        assert_eq!(failure.nature, nature);
        assert_eq!(failure.message, message);
    }
}

#[test]
fn test_ネイティブエラー_detailが無いか不正な場合は元の診断を保持する() {
    // Given / When / Then
    let error = connectrpc::ConnectError::unavailable("connection refused");
    let expected = error.to_string();
    assert_eq!(error_message(error.clone()), expected);
    for (type_url, value) in [
        ("releash.client.v1.CommandError", Some("invalid-base64")),
        ("releash.client.v1.CommandError", Some("AA")),
        ("releash.client.v1.CommandError", None),
        ("other.Error", Some("AA")),
    ] {
        assert_eq!(
            error_message(error.clone().with_detail(connectrpc::ErrorDetail {
                type_url: type_url.into(),
                value: value.map(Into::into),
                debug: None,
            })),
            expected
        );
    }
}
