use super::*;

#[test]
fn test_provider拒否応答_全理由のjsonを維持する() {
    // Given
    let cases = [
        (
            ProviderLifecycleRejection::BindingNotActive,
            "binding_not_active",
        ),
        (
            ProviderLifecycleRejection::InvalidCapability,
            "invalid_capability",
        ),
        (
            ProviderLifecycleRejection::BindingMismatch,
            "binding_mismatch",
        ),
        (
            ProviderLifecycleRejection::ProviderMismatch,
            "provider_mismatch",
        ),
        (ProviderLifecycleRejection::ScopeMismatch, "scope_mismatch"),
        (
            ProviderLifecycleRejection::BindingExpired,
            "binding_expired",
        ),
        (
            ProviderLifecycleRejection::SessionAlreadyAssociated,
            "session_already_associated",
        ),
        (
            ProviderLifecycleRejection::SessionNotAssociated,
            "session_not_associated",
        ),
        (
            ProviderLifecycleRejection::ProviderSessionMismatch,
            "provider_session_mismatch",
        ),
        (
            ProviderLifecycleRejection::TranscriptMismatch,
            "transcript_mismatch",
        ),
    ];
    // When / Then
    for (reason, expected) in cases {
        let response = ProviderLifecycleReceiveResponse::from(
            ProviderLifecycleIngressResult::Rejected(reason),
        );
        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({"status": "rejected", "reason": expected})
        );
    }
}
