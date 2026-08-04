use super::*;

#[derive(serde::Deserialize)]
struct BackendXtermCheckpointFixture {
    initial_cols: u16,
    initial_rows: u16,
    resized_cols: u16,
    resized_rows: u16,
    sequence: u64,
    before_resize: String,
    after_resize: String,
    checkpoint: NativeTerminalCheckpoint,
}

const TEST_SCROLLBACK_ROWS: usize = 1_000;

#[test]
fn test_ターミナル画面再現_本番の履歴上限を所有しない() {
    let source = include_str!("terminal_emulator.rs");
    let production = source.split("#[cfg(test)]").next().unwrap();

    assert!(!production.contains("TERMINAL_SCROLLBACK_ROWS: usize"));
}

#[test]
fn test_ターミナル復元点保存_上書きとセッションキー隔離を維持する() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path());
    let first = NativeTerminalCheckpoint {
        replay: "first screen".to_string(),
        sequence: 1,
        cols: 80,
        rows: 24,
    };
    let second = NativeTerminalCheckpoint {
        replay: "second screen".to_string(),
        sequence: 2,
        cols: 111,
        rows: 37,
    };

    store.save("session-a", &first).unwrap();
    store.save("session-b", &second).unwrap();
    store.save("session-a", &second).unwrap();

    let session_a = store.load("session-a").unwrap().unwrap();
    let session_b = store.load("session-b").unwrap().unwrap();
    assert_eq!(session_a.replay, "second screen");
    assert_eq!(session_a.sequence, 2);
    assert_eq!((session_a.cols, session_a.rows), (111, 37));
    assert_eq!(session_b.replay, "second screen");
    assert_eq!(session_b.sequence, 2);
    assert_ne!(store.path_for("session-a"), store.path_for("session-b"));
    assert!(!store
        .path_for("session-a")
        .display()
        .to_string()
        .contains("session-a"));
}

#[test]
fn test_ターミナル復元点削除_対象セッションだけを冪等に削除する() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path());
    let checkpoint = NativeTerminalCheckpoint {
        replay: "screen".to_string(),
        sequence: 1,
        cols: 80,
        rows: 24,
    };
    store.save("session-a", &checkpoint).unwrap();
    store.save("session-b", &checkpoint).unwrap();

    store.delete("session-a").unwrap();
    store.delete("session-a").unwrap();

    assert!(store.load("session-a").unwrap().is_none());
    assert!(store.load("session-b").unwrap().is_some());
}

#[test]
fn test_ターミナル復元点読込_破損または別セッション内容を拒否する() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path());
    std::fs::create_dir_all(&store.root).unwrap();
    std::fs::write(store.path_for("corrupt"), b"not-json").unwrap();
    assert!(store.load("corrupt").is_err());

    let checkpoint = NativeTerminalCheckpoint {
        replay: "screen".to_string(),
        sequence: 4,
        cols: 80,
        rows: 24,
    };
    store.save("source", &checkpoint).unwrap();
    std::fs::copy(store.path_for("source"), store.path_for("target")).unwrap();
    assert!(store.load("target").is_err());
}

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
fn test_ターミナル画面再現_backend生成checkpointが実xterm用golden_fixtureと一致する() {
    let fixture: BackendXtermCheckpointFixture = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/terminal-surface-checkpoint-v1.json"
    ))
    .unwrap();
    let mut emulator = NativeTerminalEmulator::new(
        fixture.initial_cols,
        fixture.initial_rows,
        crate::domain::terminal_surface::TERMINAL_SURFACE_SCROLLBACK_ROWS,
    );
    emulator.apply(&fixture.before_resize);
    emulator.resize(fixture.resized_cols, fixture.resized_rows);
    emulator.apply(&fixture.after_resize);

    let generated = emulator.snapshot(fixture.sequence);

    assert_eq!(generated.replay, fixture.checkpoint.replay);
    assert_eq!(generated.sequence, fixture.checkpoint.sequence);
    assert_eq!(generated.cols, fixture.checkpoint.cols);
    assert_eq!(generated.rows, fixture.checkpoint.rows);
}
