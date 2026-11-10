use super::*;

#[test]
fn test_使用率拒否応答_資格の拒否理由をrpcの失敗に残す() {
    // Given / When / Then
    for (reason, expected) in [
        (
            ProviderLifecycleRejection::InvalidCapability,
            "invalid_capability",
        ),
        (
            ProviderLifecycleRejection::BindingExpired,
            "binding_expired",
        ),
        (
            ProviderLifecycleRejection::BindingNotActive,
            "binding_not_active",
        ),
    ] {
        let error = usage_result(Ok(ProviderLifecycleIngressResult::Rejected(reason))).unwrap_err();
        assert_eq!(error.code, connectrpc::ErrorCode::InvalidArgument);
        assert!(error.to_string().contains(expected));
    }
    assert!(usage_result(Ok(ProviderLifecycleIngressResult::Applied)).is_ok());
}
