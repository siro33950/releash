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
fn test_登録希望変換_登録希望だけを読み書きし不正な応答は拒否する() {
    use crate::adaptor::controller::api::protocol::client as wire;
    // Given / When / Then
    for requested in [true, false] {
        assert_eq!(
            preference_result(wire::command_result::Command::GetAppSettings(
                wire::AppSection {
                    auto_launch: Some(requested),
                    ..Default::default()
                }
            )),
            Ok(requested)
        );
        let wire::command_request::Command::UpdateLoginItemPreference(request) =
            preference_request(requested)
        else {
            panic!("must not overwrite general settings");
        };
        assert_eq!(request.requested, Some(requested));
    }
    assert_eq!(
        preference_result(wire::command_result::Command::GetAppSettings(
            Default::default()
        )),
        Err("Missing login preference".into())
    );
    assert_eq!(
        preference_result(wire::command_result::Command::UpdateAppSettings(
            wire::Unit {}
        )),
        Err("Unexpected login preference result".into())
    );
}
