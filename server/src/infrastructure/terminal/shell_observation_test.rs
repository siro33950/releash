use super::*;

#[test]
fn test_osc7_分割したエスケープとpercent符号化を読む() {
    // Given
    let mut value = Osc7Directory::default();
    // When
    for byte in b"prompt\x1b]7;file://localhost/tmp/a%20b\x1b\\" {
        value.feed(&[*byte]);
    }
    // Then
    assert_eq!(value.cwd.as_deref(), Some("/tmp/a b"));
    value.feed(b"\x1b]7;https://localhost/private\x07");
    assert_eq!(value.cwd.as_deref(), Some("/tmp/a b"));
    value.feed(b"\x1b]7;file:///tmp/next\x07");
    assert_eq!(value.cwd.as_deref(), Some("/tmp/next"));
}

#[test]
fn test_osc7_長すぎる列から回復する() {
    // Given
    let mut value = Osc7Directory::default();
    value.feed(b"\x1b]");
    // When
    value.feed(&vec![b'a'; 10000]);
    value.feed(b"\x1b]7;file:///tmp\x07");
    // Then
    assert_eq!(value.cwd.as_deref(), Some("/tmp"));
}

#[test]
fn test_osc7_ローカルhostだけを採り終端の後は次の列を読める() {
    // Given
    let mut value = Osc7Directory::default();
    let local = local_hostname().unwrap();
    // When / Then
    for host in ["", "localhost", local.as_str()] {
        value.feed(format!("\x1b]7;file://{host}/tmp/local\x07").as_bytes());
        assert_eq!(value.observation(), (Some("/tmp/local".into()), true));
        assert!(value.pending.is_empty());
    }
    for rejected in [
        "file://other-host/tmp/remote",
        "file://[invalid/path",
        "https://localhost/tmp",
    ] {
        value.feed(format!("\x1b]7;{rejected}\x1b\\").as_bytes());
        assert!(value.pending.is_empty());
        if rejected.contains("other-host") {
            assert_eq!(value.observation(), (None, true));
        }
        value.feed(b"\x1b]7;file:///tmp/next\x07");
        assert_eq!(value.observation(), (Some("/tmp/next".into()), true));
    }
}

#[test]
fn test_osc7_観測は最後の生値と更新の有無を返す() {
    // Given
    let mut value = Osc7Directory::default();
    value.feed(b"\x1b]7;file:///tmp/\x07");
    // When / Then
    assert_eq!(value.observation(), (Some("/tmp/".into()), true));
    assert_eq!(value.observation(), (Some("/tmp/".into()), false));
}
