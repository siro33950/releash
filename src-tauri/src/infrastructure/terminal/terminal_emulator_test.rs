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
fn test_ターミナル復元点保存_上書きとセッションキー隔離を維持する() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
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
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
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
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
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

#[test]
fn test_ターミナル増分復元点_output_resize_barrierを順序通り復元する() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
    let base = NativeTerminalCheckpoint {
        replay: "base".to_string(),
        sequence: 0,
        cols: 80,
        rows: 24,
    };
    store.replace_base("session", &base).unwrap();

    store
        .append_records(
            "session",
            &[
                NativeTerminalCheckpointRecord::Output {
                    sequence: 1,
                    data: "\r\n日本語🙂".into(),
                },
                NativeTerminalCheckpointRecord::Resize {
                    sequence: 2,
                    cols: 111,
                    rows: 37,
                },
                NativeTerminalCheckpointRecord::Output {
                    sequence: 3,
                    data: "\r\nafter-resize".into(),
                },
                NativeTerminalCheckpointRecord::Barrier { sequence: 4 },
            ],
        )
        .unwrap();

    let restored = store.load("session").unwrap().unwrap();
    let terminal = NativeTerminalEmulator::restore(&restored, TEST_SCROLLBACK_ROWS);
    let text = terminal.terminal.text().join("\n");
    assert!(text.contains("base"));
    assert!(text.contains("日本語🙂"));
    assert!(text.contains("after-resize"));
    assert_eq!(restored.sequence, 4);
    assert_eq!((restored.cols, restored.rows), (111, 37));
}

#[test]
fn test_ターミナル増分復元点_crashで生じたpartial末尾を除去して次のappendを復元する() {
    use std::io::Write;

    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
    let base = NativeTerminalCheckpoint {
        replay: String::new(),
        sequence: 0,
        cols: 80,
        rows: 24,
    };
    store.replace_base("session", &base).unwrap();
    store
        .append_records(
            "session",
            &[NativeTerminalCheckpointRecord::Output {
                sequence: 1,
                data: "durable-before-crash".into(),
            }],
        )
        .unwrap();
    let journal_path = store.journal_path_for("session");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&journal_path)
        .unwrap()
        .write_all(br#"{"kind":"output","sequence":2,"data":"partial"#)
        .unwrap();

    let restored_after_crash = store.load("session").unwrap().unwrap();
    assert_eq!(restored_after_crash.sequence, 1);
    store
        .append_records(
            "session",
            &[NativeTerminalCheckpointRecord::Output {
                sequence: 2,
                data: "durable-after-restart".into(),
            }],
        )
        .unwrap();

    let restored = store.load("session").unwrap().unwrap();
    let terminal = NativeTerminalEmulator::restore(&restored, TEST_SCROLLBACK_ROWS);
    let text = terminal.terminal.text().join("\n");
    assert_eq!(restored.sequence, 2);
    assert!(text.contains("durable-before-crash"));
    assert!(text.contains("durable-after-restart"));
    assert!(!text.contains("partial"));
}

#[test]
fn test_ターミナル増分復元点_既存journalの生json行を互換読込し書式も維持する() {
    use std::io::Write;

    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
    let base = NativeTerminalCheckpoint {
        replay: String::new(),
        sequence: 0,
        cols: 80,
        rows: 24,
    };
    store.replace_base("session", &base).unwrap();
    let raw_line = r#"{"kind":"output","sequence":1,"data":"legacy-journal"}"#;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(store.journal_path_for("session"))
        .unwrap()
        .write_all(format!("{raw_line}\n").as_bytes())
        .unwrap();

    let restored = store.load("session").unwrap().unwrap();
    let terminal = NativeTerminalEmulator::restore(&restored, TEST_SCROLLBACK_ROWS);
    assert_eq!(restored.sequence, 1);
    assert!(terminal
        .terminal
        .text()
        .join("\n")
        .contains("legacy-journal"));
    assert_eq!(
        serde_json::to_string(&NativeTerminalCheckpointRecord::Output {
            sequence: 1,
            data: "legacy-journal".into(),
        })
        .unwrap(),
        raw_line
    );
}

#[test]
fn test_ターミナル増分復元点_通常append量は既存scrollback全量に比例しない() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
    let small = NativeTerminalCheckpoint {
        replay: String::new(),
        sequence: 0,
        cols: 80,
        rows: 24,
    };
    let large = NativeTerminalCheckpoint {
        replay: "history\r\n".repeat(1_000),
        sequence: 0,
        cols: 80,
        rows: 24,
    };
    store.replace_base("small", &small).unwrap();
    store.replace_base("large", &large).unwrap();
    let delta = [NativeTerminalCheckpointRecord::Output {
        sequence: 1,
        data: "same-delta".into(),
    }];

    let small_bytes = store.append_records("small", &delta).unwrap();
    let large_bytes = store.append_records("large", &delta).unwrap();

    assert_eq!(small_bytes, large_bytes);
    assert!(small_bytes < 256);
}

#[test]
fn test_ターミナル増分復元点_compact後も同じ状態を復元する() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
    let base = NativeTerminalCheckpoint {
        replay: String::new(),
        sequence: 0,
        cols: 80,
        rows: 24,
    };
    store.replace_base("session", &base).unwrap();
    store
        .append_records(
            "session",
            &[NativeTerminalCheckpointRecord::Output {
                sequence: 1,
                data: "durable-output".into(),
            }],
        )
        .unwrap();
    let before = store.load("session").unwrap().unwrap();

    store.replace_base("session", &before).unwrap();

    assert_eq!(store.journal_len("session").unwrap(), 0);
    assert_eq!(
        store.load("session").unwrap().unwrap().replay,
        before.replay
    );
}

#[test]
fn test_ターミナル増分復元点_旧version1をmigrationせず無視する() {
    use sha2::{Digest, Sha256};

    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
    let root = data_dir.path().join("terminal-surfaces");
    std::fs::create_dir_all(&root).unwrap();
    let digest = Sha256::digest(b"legacy-session");
    let legacy = root.join(format!("{}.json", hex::encode(digest)));
    std::fs::write(
        legacy,
        br#"{"version":1,"session_key":"legacy-session","checkpoint":{"replay":"legacy","sequence":1,"cols":80,"rows":24}}"#,
    )
    .unwrap();

    assert!(store.load("legacy-session").unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn test_ターミナル復元点保存_unixでは非公開権限0700と0600で作成する() {
    use std::os::unix::fs::PermissionsExt;

    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
    let checkpoint = NativeTerminalEmulator::new(80, 24, TEST_SCROLLBACK_ROWS).snapshot(0);
    store.save("session", &checkpoint).unwrap();
    store
        .append_records(
            "session",
            &[NativeTerminalCheckpointRecord::Output {
                sequence: 1,
                data: "output".into(),
            }],
        )
        .unwrap();

    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&store.root), 0o700);
    assert_eq!(mode(&store.path_for("session")), 0o600);
    assert_eq!(mode(&store.journal_path_for("session")), 0o600);
}

#[cfg(unix)]
#[test]
fn test_ターミナル増分復元点_unixでは既存journalとrootの権限を是正する() {
    use std::os::unix::fs::PermissionsExt;

    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
    std::fs::create_dir_all(&store.root).unwrap();
    std::fs::set_permissions(&store.root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let journal_path = store.journal_path_for("session");
    std::fs::write(&journal_path, b"").unwrap();
    std::fs::set_permissions(&journal_path, std::fs::Permissions::from_mode(0o644)).unwrap();

    store
        .append_records(
            "session",
            &[NativeTerminalCheckpointRecord::Output {
                sequence: 1,
                data: "output".into(),
            }],
        )
        .unwrap();

    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&store.root), 0o700);
    assert_eq!(mode(&journal_path), 0o600);
}
