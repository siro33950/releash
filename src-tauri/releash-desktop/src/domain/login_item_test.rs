use super::*;

#[test]
fn test_ログイン項目_承認待ちは無効で登録喪失だけを復旧する() {
    // Given
    for status in [
        LoginItemStatus::NotRegistered,
        LoginItemStatus::Enabled,
        LoginItemStatus::RequiresApproval,
        LoginItemStatus::NotFound,
    ] {
        // When
        let enabled = status.enabled();
        let requested = status.needs_registration(true);
        let disabled = status.needs_registration(false);
        // Then
        assert_eq!(enabled, status == LoginItemStatus::Enabled);
        assert!(!disabled);
        assert_eq!(
            requested,
            matches!(
                status,
                LoginItemStatus::NotRegistered | LoginItemStatus::NotFound
            )
        );
    }
}

#[test]
fn test_登録規則_承認待ちの解除と登録後の状態を確認する() {
    // Given / When / Then
    assert_eq!(
        LoginItemStatus::RequiresApproval.registration_change(false),
        RegistrationChange::Unregister
    );
    assert_eq!(
        LoginItemStatus::NotFound.registration_change(false),
        RegistrationChange::None
    );
    assert_eq!(
        LoginItemStatus::RequiresApproval.requested_after_change(true),
        Ok(true)
    );
    assert!(!LoginItemStatus::RequiresApproval.enabled());
    assert!(LoginItemStatus::RequiresApproval
        .requested_after_change(false)
        .is_err());
}
