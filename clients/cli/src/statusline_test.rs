use super::*;

#[test]
fn test_使用率転送_ログを開けなければ起動失敗を返す() {
    // Given / When / Then
    assert!(spawn_transfer(Path::new("/dev/null"), b"{}").is_err());
}
