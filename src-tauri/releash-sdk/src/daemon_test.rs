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
    let daemon = executable().unwrap();
    // Then
    assert_eq!(daemon.parent(), current.parent());
    assert_eq!(daemon.file_name().unwrap(), "releashd");
}
