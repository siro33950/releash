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
