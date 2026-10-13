use releashd::test_support::integration::provider_usage::ReverseLines;

#[test]
fn test_末尾読み取り_未完結行を無視しpageをまたぐ行を返す() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("lines");
    let long = "x".repeat(70 * 1024);
    std::fs::write(&path, format!("first\n{long}\nlast\npartial")).unwrap();
    let mut lines = ReverseLines::open(&path).unwrap();
    // When / Then
    assert_eq!(lines.read_previous().unwrap().unwrap(), b"last");
    assert_eq!(lines.read_previous().unwrap().unwrap(), long.as_bytes());
    assert_eq!(lines.read_previous().unwrap().unwrap(), b"first");
    assert!(lines.read_previous().unwrap().is_none());
    std::fs::write(&path, b"only\n").unwrap();
    assert_eq!(
        ReverseLines::open(&path)
            .unwrap()
            .read_previous()
            .unwrap()
            .unwrap(),
        b"only"
    );
}

#[test]
fn test_末尾読み取り_複数chunkの長い行と空行を逆順に返す() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("lines");
    let first = "a".repeat(3 * 64 * 1024 + 1);
    let second = "b".repeat(2 * 64 * 1024 + 1);
    std::fs::write(&path, format!("{first}\n\n{second}\nunfinished")).unwrap();
    let mut lines = ReverseLines::open(&path).unwrap();
    // When / Then
    for expected in [second.as_bytes(), b"", first.as_bytes()] {
        assert_eq!(lines.read_previous().unwrap().unwrap(), expected);
    }
    assert!(lines.read_previous().unwrap().is_none());
}
