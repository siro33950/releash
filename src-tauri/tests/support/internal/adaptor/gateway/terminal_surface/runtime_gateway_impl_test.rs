use super::test_support::{
    attach_checkpoint, compact_runtime_checkpoint, insert_runtime, remove_checkpoint_target,
    replace_checkpoint_flush, BackgroundCheckpointFixture,
};
use super::*;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;
use releash_lib::test_support::integration::method_traits::TerminalSurfaceGateway;
use std::sync::{Condvar, Mutex as StdMutex};
use std::time::Duration;

fn workspace_owner(path: &str) -> TerminalSurfaceOwner {
    TerminalSurfaceOwner::workspace(WorkspaceIdentity::new(path)).unwrap()
}

struct MockWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for MockWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
struct MockKiller {
    killed: Arc<std::sync::atomic::AtomicBool>,
}

struct MockResizer {
    rows: u16,
    cols: u16,
}

impl NativePtyResizer for MockResizer {
    fn resize(
        &mut self,
        rows: u16,
        cols: u16,
    ) -> Result<(), crate::infrastructure::terminal::native_pty::NativePtyError> {
        self.rows = rows;
        self.cols = cols;
        Ok(())
    }
}

fn insert_test_session_with_resizer(
    gateway: &TerminalSurfaceRuntimeGatewayFor,
    runtime_generation: u64,
    session_key: &str,
    worktree_path: Option<&str>,
    label: Option<&str>,
    resizer: Box<dyn NativePtyResizer + Send>,
) -> (Arc<std::sync::atomic::AtomicBool>, Arc<Mutex<Vec<u8>>>) {
    let written = Arc::new(Mutex::new(Vec::<u8>::new()));
    let killed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let terminal_surface = Arc::new(Mutex::new(NativeTerminalEmulator::new(
        80,
        24,
        TERMINAL_SURFACE_SCROLLBACK_ROWS,
    )));
    terminal_surface.lock().apply("buffered data");
    let workspace = WorkspaceIdentity::new(worktree_path.unwrap_or("/"));
    let mut session = TerminalSurface::new_with_session_key(
        runtime_generation,
        session_key.to_string(),
        TerminalSurfaceOwner::session(workspace, session_key).unwrap(),
        label.map(str::to_string),
    );
    session.worktree_path = worktree_path.map(str::to_string);
    let checkpoint = terminal_surface.lock().snapshot(1);
    assert!(session.apply_checkpoint(runtime_generation, to_domain_checkpoint(&checkpoint)));
    insert_runtime(
        gateway,
        runtime_generation,
        session_key.to_string(),
        NativePtyRuntime::from_parts(
            Box::new(MockWriter(Arc::clone(&written))),
            Box::new(MockKiller {
                killed: Arc::clone(&killed),
            }),
            resizer,
        ),
        terminal_surface,
    );
    gateway.insert_surface(session);
    (killed, written)
}

fn insert_test_session(
    gateway: &TerminalSurfaceRuntimeGatewayFor,
    runtime_generation: u64,
    session_key: &str,
    worktree_path: Option<&str>,
    label: Option<&str>,
) -> Arc<std::sync::atomic::AtomicBool> {
    insert_test_session_with_resizer(
        gateway,
        runtime_generation,
        session_key,
        worktree_path,
        label,
        Box::new(MockResizer { rows: 24, cols: 80 }),
    )
    .0
}

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

    let result = crate::usecase::terminal_surface::spawn_usecase::get_or_spawn(
        &crate::adaptor::gateway::telemetry::TelemetryGateway,
        &gateway,
        &crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub::new(),
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        None,
    );
    if let Ok(outcome) = &result {
        crate::usecase::terminal_surface::lifecycle_usecase::kill_runtime_generation(
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
    super::super::super::shared::background_worker::background_worker_integration_tests::assert_expired_releases(async move { attempt.flush().await }).await;
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
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
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
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
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

impl portable_pty::ChildKiller for MockKiller {
    fn kill(&mut self) -> Result<(), std::io::Error> {
        self.killed.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        Box::new(MockKiller {
            killed: Arc::clone(&self.killed),
        })
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

impl portable_pty::Child for MockKiller {
    fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
        Ok(Some(portable_pty::ExitStatus::with_exit_code(0)))
    }

    fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
        Ok(portable_pty::ExitStatus::with_exit_code(0))
    }

    fn process_id(&self) -> Option<u32> {
        None
    }

    #[cfg(windows)]
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        None
    }
}
