use super::*;
fn identity(command: &str) -> OperationIdentity {
    OperationIdentity {
        command: command.into(),
        fingerprint: [1; 32],
        target: None,
    }
}
#[test]
fn test_送信台帳_期限と上限と応答確認をdomainが判断する() {
    // Given
    let mut ledger = ClientTransmission::default();
    ledger.restored_state.extend([
        "get_repo_paths".into(),
        "get_performance_telemetry_enabled".into(),
    ]);
    ledger.finish_restoration().unwrap();
    let edit = identity("add_repo_path");
    // When / Then
    assert!(ledger.admit("expired", &edit, 10, 10).is_err());
    for n in 0..policy::MAX_UNACKNOWLEDGED_OPERATIONS {
        ledger.sent(n.to_string(), edit.clone());
    }
    assert!(ledger.admit("new", &edit, 0, 0).is_err());
    assert!(ledger.admit("0", &edit, 0, 0).is_ok());
    assert!(ledger
        .admit("0", &identity("remove_repo_path"), 0, 0)
        .is_err());
    ledger.responded("0", true);
    assert!(ledger.needs_handoff("0"));
    ledger.acknowledged("0");
    assert!(ledger.admit("new", &edit, 0, 0).is_ok());
}
#[test]
fn test_結果不明_前世代の変更だけを復元不能として区別する() {
    // Given
    let mut ledger = ClientTransmission::default();
    ledger.restoration_complete = true;
    // When
    ledger
        .queried("old", identity("add_repo_path"), true, "old", "current")
        .unwrap();
    ledger
        .queried("read", identity("get_app_settings"), true, "old", "current")
        .unwrap();
    ledger
        .queried("unsent", identity("add_repo_path"), false, "old", "current")
        .unwrap();
    // Then
    assert!(ledger.unknown_after_restart("old"));
    assert!(!ledger.unknown_after_restart("read"));
    assert!(!ledger.unknown_after_restart("unsent"));
}

#[test]
fn test_復元_応答だけでは通常操作を受け付けず反映の確認後に再開する() {
    // Given
    let mut ledger = ClientTransmission::default();
    // When / Then
    assert!(ledger.finish_restoration().is_err());
    for command in ["get_repo_paths", "get_performance_telemetry_enabled"] {
        ledger.sent(command.into(), identity(command));
        ledger.responded(command, true);
    }
    assert!(ledger
        .admit("edit", &identity("add_repo_path"), 0, 0)
        .is_err());
    ledger.finish_restoration().unwrap();
    assert!(ledger
        .admit("edit", &identity("add_repo_path"), 0, 0)
        .is_ok());
}

#[test]
fn test_送信失敗_書込み前だけ未送信とし書込み開始後は結果不明にする() {
    assert_eq!(
        WriteProgress::NotStarted.failure(),
        TransmissionFailure::NotSent
    );
    assert_eq!(
        WriteProgress::Attempted.failure(),
        TransmissionFailure::Unknown
    );
}
