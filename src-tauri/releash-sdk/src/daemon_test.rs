use super::*;
#[test]
fn test_起動診断_stderrの末尾だけを返し空ファイルも扱う() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("stderr");
    let text = "x".repeat(9000) + "failure";
    std::fs::write(&log, &text).unwrap();
    // When / Then
    let tail = stderr_tail(&log).unwrap();
    assert_eq!(tail.len(), 8192);
    assert!(tail.ends_with("failure"));
    std::fs::write(&log, []).unwrap();
    assert_eq!(stderr_tail(&log).unwrap(), "");
}

#[test]
fn test_同梱サーバパス_実行ファイルの実体の隣を返す() {
    // Given
    let current = std::env::current_exe().unwrap().canonicalize().unwrap();
    // When
    let resolved = current_executable().unwrap();
    let daemon = executable(&resolved);
    // Then
    assert_eq!(resolved, current);
    assert_eq!(daemon.parent(), resolved.parent());
    assert_eq!(daemon.file_name().unwrap(), "releashd");
}

#[test]
fn test_同梱サーバパス_存在しない実行ファイルでもパスだけを返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let current = directory.path().join("releash");
    // When
    let daemon = executable(&current);
    // Then
    assert_eq!(daemon, directory.path().join("releashd"));
    assert!(!daemon.exists());
}
