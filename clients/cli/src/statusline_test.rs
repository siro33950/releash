use super::*;

#[test]
fn test_使用率転送_ログを開けなければ起動失敗を返す() {
    // Given / When / Then
    assert!(spawn_transfer(Path::new("/dev/null"), b"{}").is_err());
}

#[test]
fn test_使用率転送_補助ログが書けなくても元の転送エラーを返す() {
    // Given
    let error = ConnectError::unavailable("original transfer failure");
    // When
    let result = record_transfer_result(Path::new("/dev/null"), Err(error));
    // Then
    let error = result.unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::Unavailable);
    assert!(error.to_string().contains("original transfer failure"));
}
