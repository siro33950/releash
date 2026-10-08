use super::*;

#[test]
fn test_ログイン項目_osの全状態と未知の状態を変換する() {
    // Given / When / Then
    for (raw, expected) in [
        (0, LoginItemStatus::NotRegistered),
        (1, LoginItemStatus::Enabled),
        (2, LoginItemStatus::RequiresApproval),
        (3, LoginItemStatus::NotFound),
    ] {
        assert_eq!(decode_status(raw).unwrap(), expected);
    }
    assert_eq!(
        decode_status(4).unwrap_err(),
        "Unknown login item status: 4"
    );
}

#[test]
fn test_登録結果_承認待ちだけを登録済みとして扱い他の失敗を維持する() {
    // Given / When / Then
    assert!(registration_result(Err("requires approval".into()), || decode_status(2)).is_ok());
    assert_eq!(
        registration_result(Err("failed".into()), || decode_status(0)),
        Err("failed".into())
    );
    assert_eq!(
        registration_result(Err("failed".into()), || decode_status(9)),
        Err("Unknown login item status: 9".into())
    );
}

#[test]
fn test_登録希望変換_登録希望だけを書く() {
    use releashd::desktop_api::wire;
    // Given / When / Then
    for requested in [true, false] {
        let wire::command_request::Command::UpdateLoginItemPreference(request) =
            preference_request(requested)
        else {
            panic!("must not overwrite general settings");
        };
        assert_eq!(request.requested, Some(requested));
    }
}

#[test]
fn test_登録可否_サーバの拒否理由と未知の状態を保持する() {
    use releashd::desktop_api::wire::{LoginRegistrationResult, LoginRegistrationStatus as S};
    // Given / When / Then
    assert!(registration_allowed(LoginRegistrationResult {
        status: S::Allowed as i32,
        reason: String::new()
    })
    .is_ok());
    for status in [S::Translocated, S::ReadOnly] {
        assert_eq!(
            registration_allowed(LoginRegistrationResult {
                status: status as i32,
                reason: "Move Releash.app".into()
            }),
            Err("Move Releash.app".into())
        );
    }
    for status in [S::Unspecified as i32, 100] {
        assert!(registration_allowed(LoginRegistrationResult {
            status,
            reason: String::new()
        })
        .is_err());
    }
}
