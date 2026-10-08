use releashd::test_support::integration::platform::NativeTerminalCheckpoint;
use releashd::test_support::integration::platform::NativeTerminalCheckpointRecord;
use releashd::test_support::integration::platform::NativeTerminalEmulator;
use releashd::test_support::integration::platform::TerminalCheckpointFileStore;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

const TEST_SCROLLBACK_ROWS: usize = 1_000;

#[test]
pub fn test_ターミナル復元点保存_上書きとセッションキー隔離を維持する() {
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
pub fn test_ターミナル復元点削除_対象セッションだけを冪等に削除する() {
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
pub fn test_ターミナル復元点読込_破損または別セッション内容を拒否する() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
    std::fs::create_dir_all(store.test_root()).unwrap();
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
pub fn test_ターミナル増分復元点_output_resize_barrierを順序通り復元する() {
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
    let text = terminal.test_text();
    assert!(text.contains("base"));
    assert!(text.contains("日本語🙂"));
    assert!(text.contains("after-resize"));
    assert_eq!(restored.sequence, 4);
    assert_eq!((restored.cols, restored.rows), (111, 37));
}

#[test]
pub fn test_ターミナル増分復元点_crashで生じたpartial末尾を除去して次のappendを復元する() {
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
    let text = terminal.test_text();
    assert_eq!(restored.sequence, 2);
    assert!(text.contains("durable-before-crash"));
    assert!(text.contains("durable-after-restart"));
    assert!(!text.contains("partial"));
}

#[test]
pub fn test_ターミナル増分復元点_既存journalの生json行を互換読込し書式も維持する() {
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
    assert!(terminal.test_text().contains("legacy-journal"));
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
pub fn test_ターミナル増分復元点_通常append量は既存scrollback全量に比例しない() {
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
pub fn test_ターミナル増分復元点_compact後も同じ状態を復元する() {
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

#[cfg(unix)]
#[test]
pub fn test_ターミナル復元点保存_unixでは非公開権限0700と0600で作成する() {
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
    assert_eq!(mode(store.test_root()), 0o700);
    assert_eq!(mode(&store.path_for("session")), 0o600);
    assert_eq!(mode(&store.journal_path_for("session")), 0o600);
}

#[cfg(unix)]
#[test]
pub fn test_ターミナル増分復元点_unixでは既存journalとrootの権限を是正する() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
    std::fs::create_dir_all(store.test_root()).unwrap();
    std::fs::set_permissions(store.test_root(), std::fs::Permissions::from_mode(0o755)).unwrap();
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
    assert_eq!(mode(store.test_root()), 0o700);
    assert_eq!(mode(&journal_path), 0o600);
}

#[test]
pub fn test_保存中断_不完全なjournal末尾を修復して再送分を重複なく復元する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = TerminalCheckpointFileStore::new(directory.path(), 100);
    store
        .replace_base(
            "session",
            &NativeTerminalCheckpoint {
                replay: String::new(),
                sequence: 0,
                cols: 80,
                rows: 24,
            },
        )
        .unwrap();
    let record = NativeTerminalCheckpointRecord::Output {
        sequence: 1,
        data: "once".into(),
    };
    store
        .append_records("session", std::slice::from_ref(&record))
        .unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(store.journal_path_for("session"))
        .unwrap();
    file.write_all(b"{\"kind\":\"output\",\"sequence\":2")
        .unwrap();
    // When
    store
        .append_records(
            "session",
            &[
                record,
                NativeTerminalCheckpointRecord::Output {
                    sequence: 2,
                    data: "-after".into(),
                },
            ],
        )
        .unwrap();
    // Then
    let checkpoint = store.load("session").unwrap().unwrap();
    assert_eq!(checkpoint.sequence, 2);
    assert!(checkpoint.replay.contains("once-after"));
    assert!(!checkpoint.replay.contains("onceonce"));
}

#[test]
pub fn test_ターミナル増分復元点_出力番号を進めず寸法変更と終了を復元する() {
    // Given
    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TEST_SCROLLBACK_ROWS);
    store
        .replace_base(
            "session",
            &NativeTerminalCheckpoint {
                replay: String::new(),
                sequence: 0,
                cols: 80,
                rows: 24,
            },
        )
        .unwrap();
    // When
    store
        .append_records(
            "session",
            &[
                NativeTerminalCheckpointRecord::Resize {
                    sequence: 0,
                    cols: 100,
                    rows: 30,
                },
                NativeTerminalCheckpointRecord::Output {
                    sequence: 1,
                    data: "one".into(),
                },
                NativeTerminalCheckpointRecord::Resize {
                    sequence: 1,
                    cols: 120,
                    rows: 40,
                },
                NativeTerminalCheckpointRecord::Output {
                    sequence: 2,
                    data: "two".into(),
                },
                NativeTerminalCheckpointRecord::Barrier { sequence: 2 },
            ],
        )
        .unwrap();
    // Then
    let restored = store.load("session").unwrap().unwrap();
    assert_eq!(restored.sequence, 2);
    assert_eq!((restored.cols, restored.rows), (120, 40));
    assert!(
        NativeTerminalEmulator::restore(&restored, TEST_SCROLLBACK_ROWS)
            .test_text()
            .contains("onetwo")
    );
}

#[test]
pub fn test_checkpoint失敗_baseとjournalの操作とpathと元のメッセージを保持する() {
    // Given
    for journal in [false, true] {
        for operation in ["read", "decode", "delete"] {
            let directory = tempfile::tempdir().unwrap();
            let store = TerminalCheckpointFileStore::new(directory.path(), TEST_SCROLLBACK_ROWS);
            store
                .replace_base(
                    "session",
                    &NativeTerminalCheckpoint {
                        replay: String::new(),
                        sequence: 0,
                        cols: 80,
                        rows: 24,
                    },
                )
                .unwrap();
            let path = if journal {
                store.journal_path_for("session")
            } else {
                store.path_for("session")
            };
            if operation == "decode" {
                std::fs::write(
                    &path,
                    if journal {
                        b"{\n".as_slice()
                    } else {
                        b"{".as_slice()
                    },
                )
                .unwrap();
            } else {
                let _ = std::fs::remove_file(&path);
                std::fs::create_dir(&path).unwrap();
            }
            // When
            let error = if operation == "delete" {
                store.delete("session").unwrap_err()
            } else {
                store.load("session").err().unwrap()
            };
            // Then
            assert!(error
                .to_string()
                .starts_with(&format!("{operation} {}: ", path.display())));
            if operation == "decode" {
                assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
                assert!(error.to_string().contains("EOF while parsing an object"));
            } else {
                let original = if operation == "read" {
                    std::fs::read(&path).unwrap_err()
                } else {
                    std::fs::remove_file(&path).unwrap_err()
                };
                assert_eq!(error.kind(), original.kind());
                assert!(error.to_string().ends_with(&original.to_string()));
            }
        }
    }
}

#[cfg(unix)]
#[test]
pub fn test_checkpoint失敗_journal修復で操作とpathとio種類を保持する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = TerminalCheckpointFileStore::new(directory.path(), TEST_SCROLLBACK_ROWS);
    store
        .replace_base(
            "session",
            &NativeTerminalCheckpoint {
                replay: String::new(),
                sequence: 0,
                cols: 80,
                rows: 24,
            },
        )
        .unwrap();
    let path = store.journal_path_for("session");
    std::fs::write(&path, b"{").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
    let original = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap_err();
    // When
    let error = store.load("session").err().unwrap();
    // Then
    assert_eq!(error.kind(), original.kind());
    assert_eq!(
        error.to_string(),
        format!("repair {}: {original}", path.display())
    );
}
