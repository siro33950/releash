use super::*;
#[test]
fn test_引継ぎ参照_ファイル識別子と固定長hashを検証する() {
    // Given
    let ids = ["", "../other", "/tmp/other", "a.json", "request-123"];
    // When
    let valid_ids = ids.map(|id| validate_id(id).is_ok());
    let fingerprints = [
        validate_fingerprint(&[0; 32], &[]),
        validate_fingerprint(&[0; 32], &[1; 32]),
        validate_fingerprint(&[], &[]),
        validate_fingerprint(&[0; 32], &[1]),
    ]
    .map(|result| result.is_ok());
    // Then
    assert_eq!(valid_ids, [false, false, false, false, true]);
    assert_eq!(fingerprints, [true, true, false, false]);
}
#[test]
fn test_引継ぎ参照_不正なcommandを永続化しない() {
    // Given
    let commands = [
        "",
        "GET_SETTINGS",
        "update/../../settings",
        "update_app_settings",
    ];
    // When
    let valid = commands.map(|command| validate_reference("id", command, &[0; 32], &[]).is_ok());
    // Then
    assert_eq!(valid, [false, false, false, true]);
}
#[test]
fn test_引継ぎ操作_前世代の変更だけ確認済みにできる() {
    // Given
    let cases = [
        ("add_repo_path", "old", "new"),
        ("get_app_settings", "old", "new"),
        ("add_repo_path", "new", "new"),
        ("add_repo_path", "", "new"),
    ];
    // When
    let restored =
        cases.map(|(command, previous, current)| restored_unknown(command, previous, current));
    // Then
    assert_eq!(restored, [true, false, false, false]);
}
