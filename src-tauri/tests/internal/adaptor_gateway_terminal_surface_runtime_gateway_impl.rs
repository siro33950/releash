use releash_lib::test_support::integration::fixtures::fixtures_adaptor_gateway_terminal_surface_runtime_gateway_impl_insert_test_session as insert_test_session;
use releash_lib::test_support::integration::fixtures::fixtures_usecase_terminal_surface_spawn_usecase_workspace_owner as workspace_owner;
use releash_lib::test_support::integration::platform::TechnicalFailure;

use parking_lot::Mutex;
use releash_lib::test_support::integration::platform::IncrementalCheckpointJournal;
use releash_lib::test_support::integration::platform::NativeTerminalCheckpoint;
use releash_lib::test_support::integration::platform::NativeTerminalCheckpointRecord;
use releash_lib::test_support::integration::platform::NativeTerminalEmulator;
use releash_lib::test_support::integration::platform::TechnicalFailureNature;
use releash_lib::test_support::integration::platform::TerminalCheckpointFileStore;
use releash_lib::test_support::integration::terminal::attach_checkpoint;
use releash_lib::test_support::integration::terminal::compact_runtime_checkpoint;
use releash_lib::test_support::integration::terminal::remove_checkpoint_target;
use releash_lib::test_support::integration::terminal::replace_checkpoint_flush;
use releash_lib::test_support::integration::terminal::BackgroundCheckpointFixture;
use releash_lib::test_support::integration::terminal::TerminalRuntimeSpawnRequest;
use releash_lib::test_support::integration::terminal::TerminalSurface;
use releash_lib::test_support::integration::terminal::TerminalSurfaceEventSink;
use releash_lib::test_support::integration::terminal::TerminalSurfaceGateway;
use releash_lib::test_support::integration::terminal::TerminalSurfaceGatewayError;
use releash_lib::test_support::integration::terminal::TerminalSurfaceOutputEvent;
use releash_lib::test_support::integration::terminal::TerminalSurfaceOwner;
use releash_lib::test_support::integration::terminal::TerminalSurfaceRuntimeGatewayFor;
use releash_lib::test_support::integration::terminal::CHECKPOINT_JOURNAL_COMPACTION_BYTES;
use releash_lib::test_support::integration::terminal::TERMINAL_SURFACE_SCROLLBACK_ROWS;
use releash_lib::test_support::integration::workspace::WorkspaceIdentity;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

#[derive(Default)]
struct CapturedTerminalOutput {
    resizes: StdMutex<Vec<(u16, u16, u64)>>,
}

#[test]
pub fn test_ターミナル画面_再起動復元_復元点破損時は新規画面で上書きしない() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let store = TerminalCheckpointFileStore::new(data_dir.path(), TERMINAL_SURFACE_SCROLLBACK_ROWS);
    store
        .save(
            "workspace:5:/repo",
            &NativeTerminalCheckpoint {
                replay: "recoverable".to_string(),
                sequence: 4,
                cols: 80,
                rows: 24,
            },
        )
        .unwrap();
    let checkpoint_path = std::fs::read_dir(data_dir.path().join("terminal-surfaces"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(&checkpoint_path, b"{broken-checkpoint").unwrap();
    let gateway = TerminalSurfaceRuntimeGatewayFor::new(data_dir.path().to_path_buf());

    let result = releash_lib::test_support::integration::terminal::get_or_spawn(
        &releash_lib::test_support::integration::telemetry::TelemetryGateway,
        &gateway,
        &releash_lib::test_support::integration::platform::TerminalSurfaceEventHub::new(),
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        None,
    );
    if let Ok(outcome) = &result {
        releash_lib::test_support::integration::terminal::kill_runtime_generation(
            &gateway,
            outcome.surface.runtime_generation.value(),
        )
        .unwrap();
    }

    assert!(result.is_err());
    assert_eq!(
        std::fs::read(checkpoint_path).unwrap(),
        b"{broken-checkpoint"
    );
}

#[test]
pub fn test_ターミナル画面_pty起動は初期checkpoint永続化を待たない() {
    let data_dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        data_dir.path().join("terminal-surfaces"),
        b"not-a-directory",
    )
    .unwrap();
    let gateway = TerminalSurfaceRuntimeGatewayFor::new(data_dir.path().to_path_buf());

    gateway
        .spawn_runtime(TerminalRuntimeSpawnRequest {
            runtime_generation: 1,
            session_key: "session:test".to_string(),
            rows: 24,
            cols: 80,
            cwd: Some(data_dir.path().to_string_lossy().into_owned()),
            process: None,
            initial_terminal_surface: None,
        })
        .unwrap();

    gateway.request_runtime_stop(1).unwrap();
}

struct BlockingSessionSink {
    blocked_session_key: String,
    started: Arc<(StdMutex<bool>, Condvar)>,
    release: Arc<(StdMutex<bool>, Condvar)>,
}

#[test]
pub fn test_ターミナル画面_イベント順序_別画面の配信を相互に停止させない() {
    let data_dir = tempfile::tempdir().unwrap();
    let first_started = Arc::new((StdMutex::new(false), Condvar::new()));
    let release_first = Arc::new((StdMutex::new(false), Condvar::new()));
    let sink = Arc::new(BlockingSessionSink {
        blocked_session_key: "surface-a".to_string(),
        started: Arc::clone(&first_started),
        release: Arc::clone(&release_first),
    });
    let gateway = Arc::new(TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(
        Arc::new(|_| {}),
        data_dir.path().to_path_buf(),
        sink,
        true,
    ));
    insert_test_session(&gateway, 1, "surface-a", Some("/repo"), None);
    insert_test_session(&gateway, 2, "surface-b", Some("/repo"), None);

    let first = std::thread::spawn({
        let gateway = Arc::clone(&gateway);
        move || gateway.resize("surface-a", 30, 100)
    });
    let (started, changed) = &*first_started;
    let _guard = changed
        .wait_while(started.lock().unwrap(), |started| !*started)
        .unwrap();
    let (completed, observed) = std::sync::mpsc::channel();
    let second = std::thread::spawn({
        let gateway = Arc::clone(&gateway);
        move || {
            let result = gateway.resize("surface-b", 31, 101);
            completed.send(()).unwrap();
            result
        }
    });
    let second_completed = observed.recv_timeout(Duration::from_millis(100)).is_ok();
    let (released, changed) = &*release_first;
    *released.lock().unwrap() = true;
    changed.notify_all();

    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
    assert!(
        second_completed,
        "one Terminal Surface publish must not block another surface"
    );
}

#[test]
pub fn test_ターミナル画面_寸法変更_次の画面_連番で配信する() {
    let data_dir = tempfile::tempdir().unwrap();
    let captured = Arc::new(CapturedTerminalOutput::default());
    let gateway = TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(
        Arc::new(|_| {}),
        data_dir.path().to_path_buf(),
        captured.clone(),
        true,
    );
    insert_test_session(&gateway, 1, "key", Some("/repo"), None);

    gateway.resize("key", 30, 100).unwrap();

    assert_eq!(*captured.resizes.lock().unwrap(), vec![(100, 30, 1)]);
}

#[tokio::test]
pub async fn test_定期保存の期限切れ_子を回収して保留データと保存枠を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        TerminalCheckpointFileStore::new(directory.path(), TERMINAL_SURFACE_SCROLLBACK_ROWS);
    let journal = Arc::new(Mutex::new(IncrementalCheckpointJournal::new(
        NativeTerminalCheckpoint {
            replay: String::new(),
            sequence: 0,
            cols: 80,
            rows: 24,
        },
        false,
    )));
    journal
        .lock()
        .record(NativeTerminalCheckpointRecord::Output {
            sequence: 1,
            data: "kept-output".into(),
        })
        .unwrap();
    let background = BackgroundCheckpointFixture::new(
        store.clone(),
        "deadline".into(),
        1,
        Arc::new(Mutex::new(NativeTerminalEmulator::new(
            80,
            24,
            TERMINAL_SURFACE_SCROLLBACK_ROWS,
        ))),
        journal.clone(),
    );
    let attempt = background.clone();
    // When
    crate::adaptor_gateway_shared_background_worker::assert_expired_releases(async move {
        attempt.flush().await
    })
    .await;
    // Then
    assert!(background.io_available());
    let pending = journal.lock().take_pending();
    assert!(pending.base.is_some());
    assert_eq!(pending.records.len(), 1);
    journal.lock().restore_failed(pending);
    background.flush().await.unwrap();
    let loaded = store.load("deadline").unwrap().unwrap();
    assert_eq!(loaded.sequence, 1);
    assert!(loaded.replay.contains("kept-output"));
}

fn attach_missing_checkpoint_target(
    directory: &std::path::Path,
    background_flush: bool,
) -> TerminalSurfaceRuntimeGatewayFor {
    let gateway = TerminalSurfaceRuntimeGatewayFor::default();
    insert_test_session(&gateway, 1, "missing-checkpoint", Some("/repo"), None);
    let journal = Arc::new(Mutex::new(IncrementalCheckpointJournal::new(
        NativeTerminalCheckpoint {
            replay: String::new(),
            sequence: 0,
            cols: 80,
            rows: 24,
        },
        false,
    )));
    if background_flush {
        journal
            .lock()
            .record(NativeTerminalCheckpointRecord::Output {
                sequence: 1,
                data: "x"
                    .repeat(CHECKPOINT_JOURNAL_COMPACTION_BYTES as usize)
                    .into(),
            })
            .unwrap();
    }
    let store = TerminalCheckpointFileStore::new(directory, TERMINAL_SURFACE_SCROLLBACK_ROWS);
    attach_checkpoint(&gateway, 1, store, journal, background_flush);
    remove_checkpoint_target(&gateway, 1);
    gateway
}

#[test]
pub fn test_checkpoint一括保存_対象不存在を業務の失敗として返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let gateway = attach_missing_checkpoint_target(directory.path(), false);
    // When / Then
    assert_eq!(
        gateway.flush_checkpoints(),
        Err(TerminalSurfaceGatewayError::NotFound(
            "Terminal Surface not found for owner missing-checkpoint".into()
        ))
    );
    let result = compact_runtime_checkpoint(&gateway, 1);
    assert_eq!(
        result,
        Err(TerminalSurfaceGatewayError::NotFound(
            "Terminal Surface for PTY 1 not found".into()
        ))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
pub async fn test_checkpoint背景保存_scheduler経由でも対象不存在を業務の失敗として返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let gateway = attach_missing_checkpoint_target(directory.path(), true);
    // When / Then
    assert_eq!(
        gateway.flush_checkpoints(),
        Err(TerminalSurfaceGatewayError::NotFound(
            "Terminal Surface for PTY 1 not found".into()
        ))
    );
}

#[test]
pub fn test_checkpoint一括保存_schedulerの技術的失敗を保持する() {
    use releash_lib::test_support::integration::platform::TechnicalFailure;
    use releash_lib::test_support::integration::platform::TechnicalFailureNature;
    // Given
    let directory = tempfile::tempdir().unwrap();
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        let gateway = attach_missing_checkpoint_target(directory.path(), false);
        let failure = TechnicalFailure {
            nature,
            message: "scheduler failure".into(),
        };
        let source = failure.clone();
        replace_checkpoint_flush(&gateway, 1, Arc::new(move || Err(source.clone().into())));
        // When / Then
        assert_eq!(
            gateway.flush_checkpoints(),
            Err(TerminalSurfaceGatewayError::Technical(failure))
        );
    }
}

#[test]
pub fn test_checkpoint一括保存_compactのio失敗を保持する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let gateway = attach_missing_checkpoint_target(directory.path(), false);
    gateway.insert_surface(TerminalSurface::new_with_session_key(
        1,
        "missing-checkpoint".into(),
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "missing-checkpoint")
            .unwrap(),
        None,
    ));
    let path = directory.path().join("terminal-surfaces");
    std::fs::write(&path, b"file").unwrap();
    let source = std::fs::create_dir_all(&path).unwrap_err();
    assert_eq!(source.kind(), std::io::ErrorKind::AlreadyExists);
    // When / Then
    assert_eq!(
        gateway.flush_checkpoints(),
        Err(TerminalSurfaceGatewayError::Technical(TechnicalFailure {
            nature: TechnicalFailureNature::Other,
            message: source.to_string(),
        }))
    );
}

impl TerminalSurfaceEventSink for CapturedTerminalOutput {
    fn publish(&self, event: TerminalSurfaceOutputEvent) {
        match event {
            TerminalSurfaceOutputEvent::Resize {
                cols,
                rows,
                sequence,
                ..
            } => {
                self.resizes.lock().unwrap().push((cols, rows, sequence));
            }
            TerminalSurfaceOutputEvent::Output { .. } | TerminalSurfaceOutputEvent::Exit { .. } => {
            }
        }
    }
}

impl TerminalSurfaceEventSink for BlockingSessionSink {
    fn publish(&self, event: TerminalSurfaceOutputEvent) {
        let session_key = match event {
            TerminalSurfaceOutputEvent::Output { session_key, .. }
            | TerminalSurfaceOutputEvent::Resize { session_key, .. }
            | TerminalSurfaceOutputEvent::Exit { session_key, .. } => session_key,
        };
        if session_key != self.blocked_session_key {
            return;
        }
        let (started, changed) = &*self.started;
        *started.lock().unwrap() = true;
        changed.notify_all();
        let (released, changed) = &*self.release;
        let _guard = changed
            .wait_while(released.lock().unwrap(), |released| !*released)
            .unwrap();
    }
}
