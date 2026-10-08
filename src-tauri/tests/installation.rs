use releashd::test_support::integration::installation::{cli_link, create_cli_link, read_only};
#[test]
fn test_cli設置_os上でリンクを作り古いリンクを置き換えファイルを保護する() {
    // Given
    let tmp = tempfile::tempdir().unwrap();
    let link = tmp.path().join("bin/releash");
    let target = tmp.path().join("new/releash");
    // When / Then
    assert_eq!(cli_link(&link).unwrap(), None);
    create_cli_link(&target, &link).unwrap();
    assert_eq!(cli_link(&link).unwrap(), Some(Some(target.clone())));
    let replacement = tmp.path().join("replacement/releash");
    create_cli_link(&replacement, &link).unwrap();
    assert_eq!(cli_link(&link).unwrap(), Some(Some(replacement)));
    std::fs::remove_file(&link).unwrap();
    std::fs::write(&link, "user command").unwrap();
    assert_eq!(cli_link(&link).unwrap(), Some(None));
    assert!(create_cli_link(&target, &link).is_err());
    assert_eq!(std::fs::read_to_string(link).unwrap(), "user command");
}
#[test]
fn test_配置観測_現在の実行ファイルと取得失敗を返す() {
    // Given / When / Then
    assert!(!read_only(&std::env::current_exe().unwrap()).unwrap());
    assert!(read_only(std::path::Path::new("/no-such-releash-bundle/releashd")).is_err());
}
#[cfg(target_os = "macos")]
#[test]
fn test_配置観測_読み取り専用ボリュームを検出する() {
    // Given / When / Then
    assert!(read_only(std::path::Path::new(
        "/System/Library/CoreServices/SystemVersion.plist"
    ))
    .unwrap());
}
