use super::*;
#[test]
fn test_ログイン項目_配置のos情報を返す() {
    // Given
    let executable = std::env::current_exe().unwrap();
    // When / Then
    assert_eq!(registration_location(&executable).unwrap(), (false, false));
}
#[cfg(target_os = "macos")]
#[test]
fn test_ログイン項目_読み取り専用mountと取得失敗を返す() {
    // Given
    let readonly = std::path::Path::new("/System/Library/CoreServices/SystemVersion.plist");
    let missing = std::path::Path::new("/no-such-releash-bundle/Contents/MacOS/releash");
    // When / Then
    assert_eq!(registration_location(readonly).unwrap(), (false, true));
    assert!(registration_location(missing).is_err());
}
