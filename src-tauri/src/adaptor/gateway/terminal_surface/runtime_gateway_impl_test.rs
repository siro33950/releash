use super::*;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex as StdMutex};
use std::time::Duration;

fn workspace_owner(path: &str) -> TerminalSurfaceOwner {
    TerminalSurfaceOwner::workspace(WorkspaceIdentity::new(path)).unwrap()
}

fn session_owner(path: &str, session_id: &str) -> TerminalSurfaceOwner {
    TerminalSurfaceOwner::session(WorkspaceIdentity::new(path), session_id).unwrap()
}

struct BlockingFirstEventSink {
    first_started: Arc<(StdMutex<bool>, Condvar)>,
    release_first: Arc<(StdMutex<bool>, Condvar)>,
    sequences: StdMutex<Vec<u64>>,
}

#[derive(Default)]
struct RecordingEventSink {
    events: StdMutex<Vec<TerminalSurfaceEvent>>,
}

impl TerminalSurfaceEventSink for RecordingEventSink {
    fn publish(&self, event: TerminalSurfaceEvent) {
        self.events.lock().unwrap().push(event);
    }
}

impl TerminalSurfaceEventSink for BlockingFirstEventSink {
    fn publish(&self, event: TerminalSurfaceEvent) {
        let sequence = match event {
            TerminalSurfaceEvent::Output { sequence, .. }
            | TerminalSurfaceEvent::Resize { sequence, .. }
            | TerminalSurfaceEvent::Exit { sequence, .. } => sequence,
        };
        if sequence == 1 {
            let (started, changed) = &*self.first_started;
            *started.lock().unwrap() = true;
            changed.notify_all();
            let (released, changed) = &*self.release_first;
            let _guard = changed
                .wait_while(released.lock().unwrap(), |released| !*released)
                .unwrap();
        }
        self.sequences.lock().unwrap().push(sequence);
    }
}

#[test]
fn test_ターミナル画面イベント_連番採番と配信を一つの順序操作にする() {
    let order = Arc::new(TerminalSurfaceEventOrder::default());
    let next_sequence = Arc::new(AtomicU64::new(0));
    let first_started = Arc::new((StdMutex::new(false), Condvar::new()));
    let release_first = Arc::new((StdMutex::new(false), Condvar::new()));
    let sink = Arc::new(BlockingFirstEventSink {
        first_started: Arc::clone(&first_started),
        release_first: Arc::clone(&release_first),
        sequences: StdMutex::new(Vec::new()),
    });

    let first = std::thread::spawn({
        let order = Arc::clone(&order);
        let next_sequence = Arc::clone(&next_sequence);
        let sink = Arc::clone(&sink);
        move || {
            order.advance_and_publish(Some(sink.as_ref()), || {
                let sequence = next_sequence.fetch_add(1, Ordering::SeqCst) + 1;
                Some((
                    sequence,
                    TerminalSurfaceEvent::Output {
                        session_key: "surface".to_string(),
                        data: "first".into(),
                        sequence,
                    },
                ))
            })
        }
    });
    let (started, changed) = &*first_started;
    let _guard = changed
        .wait_while(started.lock().unwrap(), |started| !*started)
        .unwrap();
    let second = std::thread::spawn({
        let order = Arc::clone(&order);
        let next_sequence = Arc::clone(&next_sequence);
        let sink = Arc::clone(&sink);
        move || {
            order.advance_and_publish(Some(sink.as_ref()), || {
                let sequence = next_sequence.fetch_add(1, Ordering::SeqCst) + 1;
                Some((
                    sequence,
                    TerminalSurfaceEvent::Resize {
                        session_key: "surface".to_string(),
                        cols: 120,
                        rows: 40,
                        sequence,
                    },
                ))
            })
        }
    });
    let (released, changed) = &*release_first;
    *released.lock().unwrap() = true;
    changed.notify_all();

    assert_eq!(first.join().unwrap(), Some(1));
    assert_eq!(second.join().unwrap(), Some(2));
    assert_eq!(*sink.sequences.lock().unwrap(), vec![1, 2]);
}

#[derive(Default)]
struct CapturedTerminalOutput {
    resizes: StdMutex<Vec<(u16, u16, u64)>>,
}

impl TerminalSurfaceEventSink for CapturedTerminalOutput {
    fn publish(&self, event: TerminalSurfaceEvent) {
        match event {
            TerminalSurfaceEvent::Resize {
                cols,
                rows,
                sequence,
                ..
            } => {
                self.resizes.lock().unwrap().push((cols, rows, sequence));
            }
            TerminalSurfaceEvent::Output { .. } | TerminalSurfaceEvent::Exit { .. } => {}
        }
    }
}

#[test]
fn test_ターミナル画面_再起動復元_復元点破損時は新規画面で上書きしない() {
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
fn test_ターミナル画面_実行環境_初期状態では画面を持たない() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    assert!(gateway.list_summaries().is_empty());
}

#[test]
fn test_ターミナル画面_pty起動は初期checkpoint永続化を待たない() {
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

#[test]
fn test_ターミナル画面_一覧概要取得時に画面再現器の全画面再生を再構成しない() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    insert_test_session(&gateway, 1, "key", Some("/repo"), None);
    gateway
        .runtimes
        .lock()
        .get(&1)
        .unwrap()
        .terminal_surface
        .lock()
        .apply("runtime-only-output");

    let surfaces = gateway.list_summaries();

    assert_eq!(surfaces.len(), 1);
    assert_eq!(surfaces[0].session_key, "key");
    assert!(!gateway
        .registry
        .lock()
        .get(1)
        .unwrap()
        .checkpoint
        .replay
        .contains("runtime-only-output"));
}

#[test]
fn test_ターミナル画面_取得または生成_既存画面の概要取得で全画面再生を再構成しない() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    let owner = session_owner("/repo", "agent");
    let session_key = owner.stable_key();
    insert_test_session(&gateway, 1, &session_key, Some("/repo"), None);
    {
        let mut registry = gateway.registry.lock();
        let mut surface = registry.remove(1).unwrap();
        surface.owner = owner.clone();
        registry.insert(surface);
    }
    gateway
        .runtimes
        .lock()
        .get(&1)
        .unwrap()
        .terminal_surface
        .lock()
        .apply("runtime-only-output");

    let outcome = crate::usecase::terminal_surface::spawn_usecase::get_or_spawn(
        &crate::adaptor::gateway::telemetry::TelemetryGateway,
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        owner,
        None,
    )
    .unwrap();

    let _: TerminalSurfaceSummary = outcome.surface;
}

#[test]
fn test_ターミナル画面入力_存在しないptyはエラーを返す() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    let result = gateway.write("missing", "hello");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_ターミナル画面寸法変更_存在しないptyはエラーを返す() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    let result = gateway.resize("missing", 24, 80);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_ターミナル画面_寸法変更_零の寸法を無視する() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    assert!(gateway.resize("missing", 0, 80).is_ok());
    assert!(gateway.resize("missing", 24, 0).is_ok());
    assert!(gateway.resize("missing", 0, 0).is_ok());
}

#[test]
fn test_ターミナル画面出力_正しいutf8を保持する() {
    let mut pending = Vec::new();
    let result = process_pty_output(b"hello world", &mut pending);
    assert_eq!(result.as_deref(), Some("hello world"));
    assert!(pending.is_empty());
}

#[test]
fn test_ターミナル画面出力_未完了utf8を次の断片まで保持する() {
    let mut pending = Vec::new();
    assert!(process_pty_output(&[0xE3, 0x81], &mut pending).is_none());
    assert_eq!(pending.len(), 2);
    let result = process_pty_output(&[0x82], &mut pending);
    assert_eq!(result.as_deref(), Some("あ"));
    assert!(pending.is_empty());
}

#[test]
fn test_ターミナル画面出力_空入力では出力しない() {
    let mut pending = Vec::new();
    assert!(process_pty_output(b"", &mut pending).is_none());
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

struct MockResizer {
    rows: u16,
    cols: u16,
}

impl NativePtyResizer for MockResizer {
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String> {
        self.rows = rows;
        self.cols = cols;
        Ok(())
    }
}

struct BlockingResizer {
    started: Arc<(StdMutex<bool>, Condvar)>,
    release: Arc<(StdMutex<bool>, Condvar)>,
}

impl NativePtyResizer for BlockingResizer {
    fn resize(&mut self, _rows: u16, _cols: u16) -> Result<(), String> {
        let (started, changed) = &*self.started;
        *started.lock().unwrap() = true;
        changed.notify_all();
        let (released, changed) = &*self.release;
        let _guard = changed
            .wait_while(released.lock().unwrap(), |released| !*released)
            .unwrap();
        Ok(())
    }
}

struct BlockingSessionSink {
    blocked_session_key: String,
    started: Arc<(StdMutex<bool>, Condvar)>,
    release: Arc<(StdMutex<bool>, Condvar)>,
}

impl TerminalSurfaceEventSink for BlockingSessionSink {
    fn publish(&self, event: TerminalSurfaceEvent) {
        let session_key = match event {
            TerminalSurfaceEvent::Output { session_key, .. }
            | TerminalSurfaceEvent::Resize { session_key, .. }
            | TerminalSurfaceEvent::Exit { session_key, .. } => session_key,
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
    gateway.runtimes.lock().insert(
        runtime_generation,
        AttachedTerminalRuntime {
            native_pty: NativePtyRuntime::from_parts(
                Box::new(MockWriter(Arc::clone(&written))),
                Box::new(MockKiller {
                    killed: Arc::clone(&killed),
                }),
                resizer,
            ),
            output: None,
            event_order: Arc::new(TerminalSurfaceEventOrder::default()),
            terminal_surface,
            checkpoint_scheduler: None,
            session_key: session_key.to_string(),
            output_drained: Arc::new((Mutex::new(true), parking_lot::Condvar::new())),
            checkpoint_journal: None,
            checkpoint_store: None,
            checkpoint_io: None,
            pending_input_traces: Arc::new(Mutex::new(VecDeque::new())),
        },
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

#[test]
fn test_ターミナル画面入力_存在するptyへ書き込む() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    insert_test_session(&gateway, 1, "key", Some("/repo"), None);
    assert!(gateway.write("key", "hello").is_ok());
}

#[test]
fn test_ターミナル画面入力_attachment_sequenceをpty書込順へ変換する() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    let (_, written) = insert_test_session_with_resizer(
        &gateway,
        1,
        "key",
        Some("/repo"),
        None,
        Box::new(MockResizer { rows: 24, cols: 80 }),
    );
    gateway.activate_input_attachment("key", "attachment-a");

    assert!(gateway
        .write_attached("key", "attachment-a", 1, "second")
        .is_ok());
    assert!(written.lock().is_empty());
    assert!(gateway
        .write_attached("key", "attachment-a", 0, "first")
        .is_ok());

    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while written.lock().len() < "firstsecond".len() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(written.lock().as_slice(), b"firstsecond");
}

#[test]
fn test_ターミナル画面入力_連続する入力不能は応答で失敗し購読へ通知しない() {
    let recorded = Arc::new(RecordingEventSink::default());
    let gateway = TerminalSurfaceRuntimeGateway {
        event_sink: Some(recorded.clone()),
        input_ingress: Mutex::new(TerminalSurfaceInputIngressRegistry::with_pending_capacity(
            1,
        )),
        ..Default::default()
    };
    gateway.activate_input_attachment("key", "attachment-a");

    assert!(gateway
        .write_attached("key", "attachment-a", 2, "third")
        .is_ok());
    assert!(gateway
        .write_attached("key", "attachment-a", 3, "fourth")
        .is_err());
    assert!(gateway
        .write_attached("key", "attachment-a", 4, "fifth")
        .is_err());

    assert!(recorded.events.lock().unwrap().is_empty());
}

#[test]
fn test_ターミナル画面入力_失効attachmentのwrite失敗は応答だけで伝える() {
    let recorded = Arc::new(RecordingEventSink::default());
    let gateway = TerminalSurfaceRuntimeGateway {
        event_sink: Some(recorded.clone()),
        ..Default::default()
    };
    gateway.activate_input_attachment("key", "attachment-b");

    let result = gateway.write_attached("key", "attachment-a", 0, "stale");

    assert!(matches!(
        result,
        Err(error) if error.message() == "Terminal input attachment is no longer active"
    ));
    assert!(recorded.events.lock().unwrap().is_empty());
}

#[test]
fn test_ターミナル画面入力_runtime書込失敗は応答だけで伝え未処理入力を保持する() {
    // Given
    let recorded = Arc::new(RecordingEventSink::default());
    let gateway = TerminalSurfaceRuntimeGateway {
        event_sink: Some(recorded.clone()),
        ..Default::default()
    };
    gateway.activate_input_attachment("key", "attachment-a");

    // When
    let result = gateway.write_attached("key", "attachment-a", 0, "input");

    // Then
    assert!(result.is_err());
    assert!(recorded.events.lock().unwrap().is_empty());
    let pending = gateway
        .input_ingress
        .lock()
        .admit("key", "attachment-a", 1, "next".into())
        .unwrap();
    assert_eq!(
        pending
            .iter()
            .map(|input| input.data.as_str())
            .collect::<Vec<_>>(),
        vec!["input", "next"]
    );
}

fn journal_output_context(
    journal_enabled: bool,
    flush_calls: Arc<std::sync::atomic::AtomicUsize>,
) -> (
    TerminalOutputReaderContext,
    Arc<Mutex<IncrementalCheckpointJournal>>,
) {
    let registry = Arc::new(Mutex::new(TerminalSurfaceRegistry::default()));
    registry
        .lock()
        .insert(TerminalSurface::new_with_session_key(
            1,
            "journal-key".to_string(),
            session_owner("/repo", "journal"),
            None,
        ));
    let journal = Arc::new(Mutex::new(IncrementalCheckpointJournal::new(
        NativeTerminalCheckpoint {
            replay: String::new(),
            sequence: 0,
            cols: 80,
            rows: 24,
        },
        true,
    )));
    let background_calls = flush_calls.clone();
    let scheduler = DirtyCheckpointScheduler::spawn(
        crate::usecase::work_queue::shared().clone(),
        uuid::Uuid::new_v4().to_string(),
        Duration::from_millis(10),
        Arc::new(move || {
            flush_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
        Arc::new(move || {
            let calls = background_calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }),
    );
    let context = TerminalOutputReaderContext {
        event_sink: None,
        event_order: Arc::new(TerminalSurfaceEventOrder::default()),
        registry,
        runtime_generation: 1,
        terminal_surface: Arc::new(Mutex::new(NativeTerminalEmulator::new(
            80,
            24,
            TERMINAL_SURFACE_SCROLLBACK_ROWS,
        ))),
        checkpoint_scheduler: Some(scheduler),
        session_key: "journal-key".to_string(),
        output_drained: Arc::new((Mutex::new(false), parking_lot::Condvar::new())),
        checkpoint_journal: Some(Arc::clone(&journal)),
        journal_enabled,
        first_provider_byte_started_at: Instant::now(),
        pending_input_traces: Arc::new(Mutex::new(VecDeque::new())),
    };
    (context, journal)
}

#[test]
fn test_ターミナル出力journal_有効時はrecordとmark_dirtyを実行する() {
    let flush_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (context, journal) = journal_output_context(true, Arc::clone(&flush_calls));

    publish_terminal_output(&context, "journal-data".to_string(), Vec::new());

    let pending = journal.lock().take_pending();
    assert!(matches!(
        pending.records.as_slice(),
        [NativeTerminalCheckpointRecord::Output { sequence: 1, data }]
            if data.as_ref() == "journal-data"
    ));
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while flush_calls.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(flush_calls.load(Ordering::SeqCst) > 0);
}

#[test]
fn test_ターミナル出力journal_無効時はrecordとmark_dirtyを実行しない() {
    let flush_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (context, journal) = journal_output_context(false, Arc::clone(&flush_calls));

    publish_terminal_output(&context, "journal-data".to_string(), Vec::new());

    assert!(journal.lock().take_pending().records.is_empty());
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(flush_calls.load(Ordering::SeqCst), 0);
    let registry = context.registry.lock();
    let surface = registry.find_by_session_key("journal-key").unwrap();
    assert_eq!(surface.latest_sequence, 1);
}

#[test]
fn test_ターミナル画面入力_provider_process終了後はwriterが残っていても拒否する() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    insert_test_session(&gateway, 1, "key", Some("/repo"), None);
    assert!(gateway.registry.lock().mark_exited(1, Some(0)).is_some());

    let result = gateway.write("key", "must-not-be-accepted");

    assert!(matches!(
        result,
        Err(error) if error.message() == "Terminal Surface is not writable for owner key"
    ));
}

#[test]
fn test_ターミナル画面_寸法変更_存在するptyと画面再現器を変更する() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    insert_test_session(&gateway, 1, "key", Some("/repo"), None);
    assert!(gateway.resize("key", 30, 100).is_ok());
}

#[test]
fn test_ターミナル画面_寸法変更_実pty変更中の出力適用を同じ画面内で直列化する() {
    let gateway = Arc::new(TerminalSurfaceRuntimeGateway::default());
    let resize_started = Arc::new((StdMutex::new(false), Condvar::new()));
    let release_resize = Arc::new((StdMutex::new(false), Condvar::new()));
    insert_test_session_with_resizer(
        gateway.as_ref(),
        1,
        "key",
        Some("/repo"),
        None,
        Box::new(BlockingResizer {
            started: Arc::clone(&resize_started),
            release: Arc::clone(&release_resize),
        }),
    );
    let terminal_surface = Arc::clone(&gateway.runtimes.lock().get(&1).unwrap().terminal_surface);
    let event_order = Arc::clone(&gateway.runtimes.lock().get(&1).unwrap().event_order);
    let registry = Arc::clone(&gateway.registry);

    let resize = std::thread::spawn({
        let gateway = Arc::clone(&gateway);
        move || gateway.resize("key", 30, 100)
    });
    let (started, changed) = &*resize_started;
    let _guard = changed
        .wait_while(started.lock().unwrap(), |started| !*started)
        .unwrap();

    let (completed, observed) = std::sync::mpsc::channel();
    let output = std::thread::spawn(move || {
        event_order.advance_and_publish(None, || {
            terminal_surface.lock().apply("output-after-resize");
            let sequence = registry.lock().record_output(1, Instant::now())?;
            Some((
                sequence,
                TerminalSurfaceEvent::Output {
                    session_key: "key".to_string(),
                    data: "output-after-resize".into(),
                    sequence,
                },
            ))
        });
        completed.send(()).unwrap();
    });
    let completed_before_resize = observed.recv_timeout(Duration::from_millis(100)).is_ok();
    let (released, changed) = &*release_resize;
    *released.lock().unwrap() = true;
    changed.notify_all();

    resize.join().unwrap().unwrap();
    output.join().unwrap();
    assert!(
        !completed_before_resize,
        "output must not be applied between native PTY resize and emulator resize"
    );
}

#[test]
fn test_ターミナル画面_イベント順序_別画面の配信を相互に停止させない() {
    let data_dir = tempfile::tempdir().unwrap();
    let first_started = Arc::new((StdMutex::new(false), Condvar::new()));
    let release_first = Arc::new((StdMutex::new(false), Condvar::new()));
    let sink = Arc::new(BlockingSessionSink {
        blocked_session_key: "surface-a".to_string(),
        started: Arc::clone(&first_started),
        release: Arc::clone(&release_first),
    });
    let gateway = Arc::new(TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(
        crate::usecase::work_queue::shared().clone(),
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
fn test_ターミナル画面_寸法変更_次の画面_連番で配信する() {
    let data_dir = tempfile::tempdir().unwrap();
    let captured = Arc::new(CapturedTerminalOutput::default());
    let gateway = TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(
        crate::usecase::work_queue::shared().clone(),
        data_dir.path().to_path_buf(),
        captured.clone(),
        true,
    );
    insert_test_session(&gateway, 1, "key", Some("/repo"), None);

    gateway.resize("key", 30, 100).unwrap();

    assert_eq!(*captured.resizes.lock().unwrap(), vec![(100, 30, 1)]);
}

#[test]
fn test_ターミナル画面参照_登録簿の概要を返す() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    insert_test_session(&gateway, 1, "key", Some("/repo"), Some("dev"));
    let found = gateway.find_summary_by_session_key("key").unwrap();
    assert_eq!(found.runtime_generation.value(), 1);
    assert_eq!(found.label.as_deref(), Some("dev"));
    assert_eq!(found.latest_sequence, 1);
}

#[test]
fn test_ターミナル画面終了_登録簿と実行環境を削除する() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    let killed = insert_test_session(&gateway, 1, "key", Some("/repo"), None);

    crate::usecase::terminal_surface::lifecycle_usecase::kill_runtime_generation(&gateway, 1)
        .unwrap();

    assert!(killed.load(std::sync::atomic::Ordering::SeqCst));
    assert!(gateway.snapshot(1).is_none());
    assert!(gateway.runtimes.lock().get(&1).is_none());
}

#[test]
fn test_ターミナル画面一括終了_登録簿が選んだ作業木だけを終了する() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    insert_test_session(&gateway, 1, "key-1", Some("/repo"), Some("dev"));
    insert_test_session(&gateway, 2, "key-2", Some("/repo"), Some("test"));
    insert_test_session(&gateway, 3, "key-3", Some("/other"), None);

    let mut killed =
        crate::usecase::terminal_surface::lifecycle_usecase::kill_by_worktree(&gateway, "/repo");
    killed.sort_unstable();

    assert_eq!(killed, vec![1, 2]);
    assert!(gateway.snapshot(1).is_none());
    assert!(gateway.snapshot(2).is_none());
    assert!(gateway.snapshot(3).is_some());
}

#[test]
fn test_ターミナル画面終了_画面削除後の終了通知を拒否する() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    insert_test_session(&gateway, 1, "key-1", Some("/repo"), None);
    gateway.remove_surface(1);

    assert!(gateway.registry.lock().mark_exited(1, Some(0)).is_none());
}

#[tokio::test]
async fn test_定期保存の期限切れ_子を回収して保留データと保存枠を返す() {
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
    let background = Arc::new(BackgroundCheckpoint {
        store: store.clone(),
        session_key: "deadline".into(),
        registry: Arc::new(Mutex::new(TerminalSurfaceRegistry::default())),
        runtime_generation: 1,
        terminal_surface: Arc::new(Mutex::new(NativeTerminalEmulator::new(
            80,
            24,
            TERMINAL_SURFACE_SCROLLBACK_ROWS,
        ))),
        journal: journal.clone(),
        io: Arc::new(tokio::sync::Mutex::new(())),
    });
    let attempt = background.clone();
    // When
    super::super::super::shared::background_worker::background_worker_tests::assert_expired_releases(Box::pin(async move { attempt.flush().await?; Ok(None) })).await;
    // Then
    assert!(background.io.try_lock().is_ok());
    let pending = journal.lock().take_pending();
    assert!(pending.base.is_some());
    assert_eq!(pending.records.len(), 1);
    journal.lock().restore_failed(pending);
    background.flush().await.unwrap();
    let loaded = store.load("deadline").unwrap().unwrap();
    assert_eq!(loaded.sequence, 1);
    assert!(loaded.replay.contains("kept-output"));
}

struct FlowControlledSink {
    hub: Arc<super::super::event_hub::TerminalSurfaceEventHub>,
    waiting: mpsc::Sender<std::thread::ThreadId>,
    events: mpsc::Sender<TerminalSurfaceEvent>,
}

impl TerminalSurfaceEventSink for FlowControlledSink {
    fn wait_output(&self, session_key: &str) {
        self.waiting.send(std::thread::current().id()).unwrap();
        self.hub.wait_output(session_key);
    }

    fn publish(&self, event: TerminalSurfaceEvent) {
        self.hub.publish(event.clone());
        self.events.send(event).unwrap();
    }
}

struct ObservedReader {
    read: mpsc::Sender<()>,
    data: std::io::Cursor<Vec<u8>>,
}

impl std::io::Read for ObservedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.read.send(()).unwrap();
        self.data.read(buf)
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

#[test]
fn test_流量停止_実出力readerとprocessorが履歴の低水位まで停止し再開する() {
    use crate::domain::terminal_surface::gateway::TerminalSurfaceEventSource;
    // Given
    let hub = Arc::new(super::super::event_hub::TerminalSurfaceEventHub::with_flags(8, true));
    let (waiting, waits) = mpsc::channel();
    let (events, received) = mpsc::channel();
    let sink = Arc::new(FlowControlledSink {
        hub: hub.clone(),
        waiting,
        events,
    });
    let (mut context, _) = journal_output_context(false, Arc::new(Default::default()));
    context.event_sink = Some(sink);
    let drained = context.output_drained.clone();
    let (read, reads) = mpsc::channel();
    let output = NativePtyOutput::from_parts(
        Box::new(ObservedReader {
            read,
            data: std::io::Cursor::new(b"resumed".to_vec()),
        }),
        Box::new(MockKiller {
            killed: Arc::new(Default::default()),
        }),
    );
    hub.subscribe_output("journal-key", "client", 100_001);
    // When
    spawn_output_reader(output, context);
    let first = waits.recv_timeout(Duration::from_secs(1));
    let second = waits.recv_timeout(Duration::from_secs(1));
    let read_while_paused = reads.recv_timeout(Duration::from_millis(50));
    let output_while_paused = received.try_recv();
    hub.processed_output("journal-key", "client", 95_001);
    let read_at_low_watermark = reads.recv_timeout(Duration::from_millis(50));
    hub.processed_output("journal-key", "client", 1);
    // Then
    reads.recv_timeout(Duration::from_secs(1)).unwrap();
    let output = received.recv_timeout(Duration::from_secs(1)).unwrap();
    let exit = received.recv_timeout(Duration::from_secs(1)).unwrap();
    let (done, changed) = &*drained;
    let mut done = done.lock();
    if !*done {
        changed.wait_for(&mut done, Duration::from_secs(1));
    }
    assert!(*done);
    assert_ne!(first.unwrap(), second.unwrap());
    assert!(matches!(
        read_while_paused,
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(matches!(
        output_while_paused,
        Err(mpsc::TryRecvError::Empty)
    ));
    assert!(matches!(
        read_at_low_watermark,
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(
        matches!(output, TerminalSurfaceEvent::Output { data, .. } if data.as_ref() == "resumed")
    );
    assert!(matches!(
        exit,
        TerminalSurfaceEvent::Exit {
            exit_code: Some(0),
            ..
        }
    ));
}

#[test]
fn test_流量停止_実processorの出力で高水位を超えると後続出力を低水位まで止める() {
    use crate::domain::terminal_surface::gateway::TerminalSurfaceEventSource;
    // Given
    let hub = Arc::new(super::super::event_hub::TerminalSurfaceEventHub::with_flags(8, true));
    let (waiting, waits) = mpsc::channel();
    let (events, received) = mpsc::channel();
    let sink = Arc::new(FlowControlledSink {
        hub: hub.clone(),
        waiting,
        events,
    });
    let (mut context, _) = journal_output_context(false, Arc::new(Default::default()));
    context.event_sink = Some(sink);
    hub.subscribe_output("journal-key", "client", 0);
    let (send, commands) = mpsc::sync_channel(4);
    let worker = std::thread::spawn(move || run_output_processor(commands, context));
    // When
    waits.recv_timeout(Duration::from_secs(1)).unwrap();
    let units = 7 * crate::infrastructure::terminal::output_batcher::OUTPUT_BATCH_MAX_CODE_UNITS;
    send.send(TerminalOutputCommand::Data {
        data: "x".repeat(units),
        input_traces: vec![],
    })
    .unwrap();
    let mut published = 0;
    while published < units {
        let TerminalSurfaceEvent::Output { data, .. } =
            received.recv_timeout(Duration::from_secs(1)).unwrap()
        else {
            panic!("output");
        };
        published += data.len();
    }
    let stopped = waits.recv_timeout(Duration::from_secs(1));
    send.send(TerminalOutputCommand::Data {
        data: "next".into(),
        input_traces: vec![],
    })
    .unwrap();
    send.send(TerminalOutputCommand::Exit(Some(0))).unwrap();
    let premature = received.recv_timeout(Duration::from_millis(50));
    hub.processed_output("journal-key", "client", units - 4_999);
    let resumed = received.recv_timeout(Duration::from_secs(1));
    worker.join().unwrap();
    // Then
    stopped.unwrap();
    assert!(matches!(premature, Err(mpsc::RecvTimeoutError::Timeout)));
    assert!(
        matches!(resumed.unwrap(), TerminalSurfaceEvent::Output { data, .. } if data.as_ref() == "next")
    );
    assert!(matches!(
        received.recv_timeout(Duration::from_secs(1)).unwrap(),
        TerminalSurfaceEvent::Exit {
            exit_code: Some(0),
            ..
        }
    ));
}

#[test]
fn test_ターミナル削除_画面削除後のruntime削除の有無によらず入力attachmentを解放する() {
    for remove_runtime_after_surface in [false, true] {
        // Given
        let gateway = TerminalSurfaceRuntimeGateway::default();
        insert_test_session(&gateway, 1, "key", Some("/repo"), None);
        gateway.activate_input_attachment("key", "input");
        gateway
            .write_attached("key", "input", 1, "pending")
            .unwrap();
        gateway.activate_input_attachment("other", "other-input");

        // When
        assert!(gateway.remove_surface(1).is_some());
        if remove_runtime_after_surface {
            gateway.remove_runtime(1);
        }

        // Then
        let mut ingress = gateway.input_ingress.lock();
        assert_eq!(
            ingress.admit("key", "input", 0, "stale".into()),
            Err(TerminalSurfaceInputIngressError::StaleAttachment)
        );
        assert!(ingress
            .admit("other", "other-input", 0, "active".into())
            .is_ok());
    }
}

#[tokio::test]
async fn test_ターミナル再作成_継続購読へ終了とsnapshotを届け入力連番と処理済み通知を保つ() {
    assert_terminal_recreation(false).await;
}

#[tokio::test]
async fn test_ターミナル再作成_送り待ちが空でも購読と入力と処理済み通知を保つ() {
    assert_terminal_recreation(true).await;
}

async fn assert_terminal_recreation(drain_exit: bool) {
    // Given
    use crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub;
    use crate::domain::state_subscription::{Event, SubscriptionTarget};
    use crate::usecase::state_subscription::{
        StateSubscriptionEvent, StateSubscriptionUsecase, StateValue,
    };
    use crate::usecase::terminal_surface::application::{
        TerminalSurfaceApplication, TerminalSurfaceStreamItem,
    };
    use futures_util::StreamExt;
    let hub = Arc::new(TerminalSurfaceEventHub::with_flags(256, true));
    let gateway = Arc::new(TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(
        crate::usecase::work_queue::shared().clone(),
        std::path::PathBuf::new(),
        hub.clone(),
        false,
    ));
    let owner = workspace_owner("/repo");
    let key = owner.stable_key();
    let target = SubscriptionTarget::Terminal(owner.clone()).to_string();
    let (_, old_written) = insert_test_session_with_resizer(
        &gateway,
        1,
        &key,
        Some("/repo"),
        None,
        Box::new(MockResizer { rows: 24, cols: 80 }),
    );
    gateway.insert_surface(TerminalSurface::new(1, owner.clone(), None));
    let terminal = Arc::new(TerminalSurfaceApplication::new(
        std::sync::Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
        gateway.clone(),
        hub.clone(),
    ));
    let subscriptions = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    )
    .with_terminal(terminal.clone());
    let stream = subscriptions.open("client".into()).unwrap();
    tokio::pin!(stream);
    stream.next().await;
    subscriptions
        .start_terminal("client", &target, None, "input")
        .await
        .unwrap();
    stream.next().await;
    stream.next().await;
    terminal
        .write_attached(&owner, "input", 0, None, "old")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while old_written.lock().len() < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(&*old_written.lock(), b"old");
    // When
    hub.publish(TerminalSurfaceEvent::Exit {
        session_key: key.clone(),
        runtime_generation: 1,
        exit_code: Some(7),
        sequence: 0,
    });
    if drain_exit {
        let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap();
        assert!(
            matches!(next, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(value.as_ref(), StateValue::Terminal(TerminalSurfaceStreamItem::Exit { exit_code: Some(7), .. })))
        );
    }
    gateway.remove_surface(1).unwrap();
    gateway.remove_runtime(1);
    let (_, new_written) = insert_test_session_with_resizer(
        &gateway,
        2,
        &key,
        Some("/repo"),
        None,
        Box::new(MockResizer { rows: 24, cols: 80 }),
    );
    gateway.insert_surface(TerminalSurface::new(2, owner.clone(), None));
    // Then
    if !drain_exit {
        let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap();
        assert!(
            matches!(next, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(value.as_ref(), StateValue::Terminal(TerminalSurfaceStreamItem::Exit { exit_code: Some(7), .. })))
        );
    }
    let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap();
    assert!(
        matches!(next, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if matches!(value.as_ref(), StateValue::Terminal(TerminalSurfaceStreamItem::Snapshot(surface)) if surface.runtime_generation.value() == 2))
    );
    terminal
        .write_attached(&owner, "input", 1, None, "new")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while new_written.lock().len() < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(&*new_written.lock(), b"new");
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: key.clone(),
        sequence: 1,
        data: "x".repeat(100_001).into(),
    });
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
    ));
    let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap();
    assert!(
        matches!(next, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(value.as_ref(), StateValue::Terminal(TerminalSurfaceStreamItem::Output { sequence: 1, data, .. }) if data.len() == 100_001))
    );
    let (sent, received) = tokio::sync::oneshot::channel();
    let waiting_hub = hub.clone();
    let waiter = tokio::task::spawn_blocking(move || {
        waiting_hub.wait_output(&key);
        sent.send(()).unwrap();
    });
    tokio::pin!(received);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut received)
            .await
            .is_err()
    );
    for _ in 0..20 {
        subscriptions
            .terminal_processed("client", &target, 5000)
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(2), received)
        .await
        .unwrap()
        .unwrap();
    waiter.await.unwrap();
    subscriptions.stop("client", &target).unwrap();
    assert!(terminal
        .write_attached(&owner, "input", 2, None, "stale")
        .is_err());
    assert!(subscriptions
        .terminal_processed("client", &target, 5000)
        .is_err());
}
