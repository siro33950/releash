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
fn test_登録規則_一時配置とread_onlyを拒否し解除の可否を決める() {
    // Given / When / Then
    for (translocated, read_only, allowed) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        assert_eq!(
            RegistrationLocation {
                translocated,
                read_only
            }
            .ensure_registration_allowed()
            .is_ok(),
            allowed
        );
    }
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
