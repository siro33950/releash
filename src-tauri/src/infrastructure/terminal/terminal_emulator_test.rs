use super::*;
const TEST_SCROLLBACK_ROWS: usize = 1_000;

#[test]
fn test_ターミナル画面再生_旧64kib生出力末尾ではなく画面意味を保持する() {
    let mut surface = NativeTerminalEmulator::new(200, 24, TEST_SCROLLBACK_ROWS);
    let output = format!("oldest-marker{}newest-marker", "x".repeat(70 * 1024));
    surface.apply(&output);
    let checkpoint = surface.snapshot(1);
    let restored = NativeTerminalEmulator::restore(&checkpoint, TEST_SCROLLBACK_ROWS);
    let text = restored.terminal.text().join("\n");

    assert!(text.contains("oldest-marker"));
    assert!(text.contains("newest-marker"));
    assert_eq!(checkpoint.sequence, 1);
}

#[test]
fn test_ターミナル画面再現_復元点生成時だけドメイン連番を受け取る() {
    let mut emulator = NativeTerminalEmulator::new(80, 24, TEST_SCROLLBACK_ROWS);

    assert_eq!(emulator.apply("first"), ());
    assert_eq!(emulator.apply("second"), ());

    let checkpoint = emulator.snapshot(17);
    assert_eq!(checkpoint.sequence, 17);
}

#[test]
fn test_ターミナル画面再現_本番1000行履歴の境界markerを厳密に保持する() {
    const ROWS: u16 = 4;
    const TOTAL_MARKERS: usize = 1_014;
    const FIRST_RETAINED_MARKER: usize = 11;
    let mut emulator = NativeTerminalEmulator::new(
        40,
        ROWS,
        crate::domain::terminal_surface::TERMINAL_SURFACE_SCROLLBACK_ROWS,
    );
    for index in 0..TOTAL_MARKERS {
        emulator.apply(&format!("boundary-marker-{index:04}\r\n"));
    }

    let checkpoint = emulator.snapshot(1);
    let restored = NativeTerminalEmulator::restore(&checkpoint, 10_000);
    let text = restored.terminal.text().join("\n");

    assert_eq!(restored.terminal.lines().count(), 1_005);
    assert!(!text.contains(&format!("boundary-marker-{:04}", FIRST_RETAINED_MARKER - 1)));
    assert!(text.contains(&format!("boundary-marker-{FIRST_RETAINED_MARKER:04}")));
    assert!(text.contains("boundary-marker-1013"));
}

#[test]
fn test_checkpoint失敗_文脈を加えてもio種類を保持する() {
    // Given
    for kind in [
        std::io::ErrorKind::WouldBlock,
        std::io::ErrorKind::TimedOut,
        std::io::ErrorKind::PermissionDenied,
    ] {
        let original = std::io::Error::new(kind, "source failure");
        // When
        let error = checkpoint_io_error(original, "repair", std::path::Path::new("/checkpoint"));
        // Then
        assert_eq!(error.kind(), kind);
        assert_eq!(error.to_string(), "repair /checkpoint: source failure");
    }
}
