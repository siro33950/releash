use super::*;

#[test]
fn test_入力モード_複合設定と追跡方式を最後の指定通り保持する() {
    // Given
    let mut modes = TerminalInputModes::default();
    // When
    modes.apply("\x1b[?1000;1002;1003;1006;2004;1004h\x1b[?1007l");
    // Then
    assert_eq!(modes.mouse_tracking, Some(1003));
    assert_eq!(modes.mouse_encoding, Some(1006));
    assert_eq!(modes.alternate_scroll, Some(false));
    assert_eq!(modes.focus, Some(true));
    assert_eq!(modes.paste, Some(true));
    // When
    modes.apply("\x1b[?1002l\x1b[?1006l\x1b[?1004;2004l\x1b[?1007h");
    // Then
    assert_eq!(modes.mouse_tracking, Some(0));
    assert_eq!(modes.mouse_encoding, Some(0));
    assert_eq!(modes.alternate_scroll, Some(true));
    assert_eq!(modes.focus, Some(false));
    assert_eq!(modes.paste, Some(false));
}

#[test]
fn test_入力モード_座標形式の変更はマウス追跡を解除しない() {
    // Given
    let mut modes = TerminalInputModes::default();
    modes.apply("\x1b[?1003h");
    for encoding in [1005, 1006, 1015, 1016] {
        // When
        modes.apply(&format!("\x1b[?{encoding}h"));
        // Then
        assert_eq!(modes.mouse_encoding, Some(encoding));
        // When
        modes.apply(&format!("\x1b[?{encoding}l"));
        // Then
        assert_eq!(modes.mouse_encoding, Some(0));
        assert_eq!(modes.mouse_tracking, Some(1003));
    }
}

#[test]
fn test_入力モード_文字列制御内や取消された設定を適用しない() {
    // Given
    let mut modes = TerminalInputModes::default();
    // When
    modes.apply("\x1b]title[?1003h\x07\x1bPq[?2004h\x1b\\\x1b_foo[>7u\x1b\\");
    modes.apply("\x1b[?1004\x18h\x1b[?1003:2h\x1b[?1003 h\x1b[?1003$h");
    // Then
    assert_eq!(modes.mouse_tracking, None);
    assert_eq!(modes.focus, None);
    assert_eq!(modes.paste, None);
    assert_eq!(modes.keyboards[0].flags, 0);
}

#[test]
fn test_キーボード設定_変更方式と省略値と不正な方式を扱う() {
    // Given
    let mut modes = TerminalInputModes::default();
    for (output, flags) in [
        ("\x1b[=7u", 7),
        ("\x1b[=8;2u", 15),
        ("\x1b[=0008;02u", 15),
        ("\x1b[=3;3u", 12),
        ("\x1b[=1;4u", 12),
        ("\x1b[=63u", 31),
        ("\x1b[=u", 0),
        ("\x1b[>7u\x1b[>u\x1b[<u", 7),
        ("\x1b[<0u", 0),
        ("\x1b[>7u\x1b[<99u", 0),
        ("\x1b[>7u\x1b[<65536u", 0),
    ] {
        // When
        modes.apply(output);
        // Then
        assert_eq!(modes.keyboards[0].flags, flags, "{output:?}");
    }
}

#[test]
fn test_入力モード_長いパラメータ列の保持量を制限して次の制御列を受け付ける() {
    // Given
    let mut modes = TerminalInputModes::default();
    // When
    modes.apply(&format!("\x1b[?{}", "1;".repeat(10_000)));
    // Then
    assert_eq!(modes.parameters.len(), 32);
    // When
    modes.apply("h\x1b[?1003h\x1b[>7u");
    // Then
    assert_eq!(modes.mouse_tracking, Some(1003));
    assert_eq!(modes.keyboards[0].flags, 7);
}

#[test]
fn test_キーボード設定_画面別の保存とスタック上限を維持する() {
    // Given
    let mut modes = TerminalInputModes::default();
    modes.apply("\x1b[>1u\x1b[>3u\x1b[?1049h\x1b[>7u\x1b[>15u");
    // When
    modes.apply("\x1b[<u\x1b[?1049l\x1b[<u");
    // Then
    assert_eq!(modes.keyboards[0].flags, 1);
    assert_eq!(modes.keyboards[1].flags, 7);
    // When
    for flags in 0..20 {
        modes.apply(&format!("\x1b[>{flags}u"));
    }
    // Then
    assert_eq!(modes.keyboards[0].stack.len(), 16);
    assert_eq!(modes.keyboards[0].stack[0], 3);
    // When
    modes.apply("\x1b[<16u");
    // Then
    assert_eq!(modes.keyboards[0].flags, 3);
    assert!(modes.keyboards[0].stack.is_empty());
}

#[test]
fn test_入力モード_ソフトリセットは保持しハードリセットは初期化する() {
    // Given
    let mut modes = TerminalInputModes::default();
    modes.apply("\x1b[?1003;1006;1004;2004h\x1b[?1007l\x1b[>7u");
    // When
    modes.apply("\x1b[!p");
    // Then
    assert_eq!(modes.mouse_tracking, Some(1003));
    assert_eq!(modes.keyboards[0].flags, 7);
    // When
    modes.apply("\x1bc");
    // Then
    assert_eq!(modes.mouse_tracking, Some(0));
    assert_eq!(modes.alternate_scroll, Some(true));
    assert_eq!(modes.paste, Some(false));
    assert_eq!(modes.keyboards[0].flags, 0);
    assert!(modes.keyboards[0].stack.is_empty());
    // SwiftTerm preserves focus reporting and coordinate encoding across RIS.
    assert_eq!(modes.mouse_encoding, Some(1006));
    assert_eq!(modes.focus, Some(true));
}
