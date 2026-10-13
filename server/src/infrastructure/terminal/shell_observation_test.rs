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
fn test_osc7_更新後は優先しosの移動後は古い値を破棄する() {
    let mut value = Osc7Directory::default();
    assert_eq!(value.observe_cwd("/tmp".into()), "/tmp");
    value.feed(b"\x1b]7;file:///tmp/\x07");
    assert_eq!(value.observe_cwd("/tmp".into()), "/tmp/");
    assert_eq!(value.observe_cwd("/".into()), "/");
    value.feed(b"\x1b]7;file:///next/\x07");
    assert_eq!(value.observe_cwd("/next".into()), "/next/");
}
