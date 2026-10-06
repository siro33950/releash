use super::*;
use crate::adaptor::gateway::terminal_surface::test_helpers::*;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::terminal_surface::output::TerminalSurfaceOutputControl;
use crate::usecase::terminal_surface::test_helpers::workspace_owner;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex as StdMutex};
use std::time::Duration;

fn session_owner(path: &str, session_id: &str) -> TerminalSurfaceOwner {
    TerminalSurfaceOwner::session(WorkspaceIdentity::new(path), session_id).unwrap()
}

#[test]
fn test_出力順序区間_削除済み世代ではcallbackを実行しない() {
    // Given
    let gateway = TerminalSurfaceRuntimeGatewayFor::default();
    gateway.insert_surface(TerminalSurface::new(1, workspace_owner("/repo"), None));
    let mut visited = false;

    // When
    assert!(gateway.with_output_order(1, &mut || visited = true));
    assert!(visited);
    visited = false;
    gateway.remove_surface(1).unwrap();
    let entered = gateway.with_output_order(1, &mut || visited = true);

    // Then
    assert!(!visited);
    assert!(!entered);
}

struct BlockingFirstEventSink {
    first_started: Arc<(StdMutex<bool>, Condvar)>,
    release_first: Arc<(StdMutex<bool>, Condvar)>,
    sequences: StdMutex<Vec<u64>>,
}

#[derive(Default)]
struct RecordingEventSink {
    events: StdMutex<Vec<TerminalSurfaceOutputEvent>>,
}

#[derive(Default)]
struct SummarySink {
    removed: StdMutex<Vec<u64>>,
}

#[test]
fn test_ターミナル状態通知_削除時に世代を伝える() {
    // Given
    let sink = Arc::new(SummarySink::default());
    let gateway = TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(
        Arc::new(|_| {}),
        std::path::PathBuf::new(),
        sink.clone(),
        false,
    );
    let workspace = TerminalSurface::new(1, workspace_owner("/repo"), None);
    let mut session = TerminalSurface::new(2, session_owner("/repo", "session"), None);
    session
        .record_output(session.runtime_generation, std::time::Instant::now())
        .unwrap();

    // When
    gateway.insert_surface(workspace.clone());
    gateway.insert_surface(session.clone());
    gateway.remove_surface(1).unwrap();
    gateway.remove_surface(2).unwrap();

    // Then
    assert_eq!(*sink.removed.lock().unwrap(), vec![1, 2]);
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
                    TerminalSurfaceOutputEvent::Output {
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
                    TerminalSurfaceOutputEvent::Resize {
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

#[test]
fn test_ターミナル画面_実行環境_初期状態では画面を持たない() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    assert!(gateway.list_summaries().is_empty());
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
        &crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub::new(),
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

struct BlockingResizer {
    started: Arc<(StdMutex<bool>, Condvar)>,
    release: Arc<(StdMutex<bool>, Condvar)>,
}

impl NativePtyResizer for BlockingResizer {
    fn resize(
        &mut self,
        _rows: u16,
        _cols: u16,
    ) -> Result<(), crate::infrastructure::terminal::native_pty::NativePtyError> {
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
    let terminal_surface = Arc::new(Mutex::new(NativeTerminalEmulator::new(
        80,
        24,
        TERMINAL_SURFACE_SCROLLBACK_ROWS,
    )));
    let session_key = uuid::Uuid::new_v4().to_string();
    let scheduler = CheckpointScheduler {
        dirty: Arc::new(move |_| {
            flush_calls.fetch_add(1, Ordering::SeqCst);
        }),
        session_key: session_key.clone(),
        flush: Arc::new(|| Ok(())),
        background: Arc::new(BackgroundCheckpoint {
            store: TerminalCheckpointFileStore::new(
                std::env::temp_dir().as_path(),
                TERMINAL_SURFACE_SCROLLBACK_ROWS,
            ),
            session_key,
            registry: Arc::clone(&registry),
            runtime_generation: 1,
            terminal_surface: Arc::clone(&terminal_surface),
            journal: Arc::clone(&journal),
            io: Arc::new(Mutex::new(())),
        }),
    };
    let context = TerminalOutputReaderContext {
        event_sink: None,
        event_order: Arc::new(TerminalSurfaceEventOrder::default()),
        registry,
        runtime_generation: 1,
        terminal_surface,
        checkpoint_scheduler: Some(scheduler),
        session_key: "journal-key".to_string(),
        output_drained: Arc::new((Mutex::new(false), parking_lot::Condvar::new())),
        checkpoint_journal: Some(Arc::clone(&journal)),
        journal_enabled,
        first_provider_byte_started_at: Instant::now(),
    };
    (context, journal)
}

#[test]
fn test_ターミナル出力journal_有効時はrecordとmark_dirtyを実行する() {
    let flush_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (context, journal) = journal_output_context(true, Arc::clone(&flush_calls));

    publish_terminal_output(&context, "journal-data".to_string());

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

    publish_terminal_output(&context, "journal-data".to_string());

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
                TerminalSurfaceOutputEvent::Output {
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

struct FlowControlledSink {
    hub: Arc<crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub>,
    waiting: mpsc::Sender<std::thread::ThreadId>,
    events: mpsc::Sender<TerminalSurfaceOutputEvent>,
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

#[test]
fn test_流量停止_実出力readerとprocessorが履歴の低水位まで停止し再開する() {
    use crate::usecase::terminal_surface::output::TerminalSurfaceOutputControl;
    // Given
    let hub = Arc::new(
        crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub::with_flags(8, true),
    );
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
        matches!(output, TerminalSurfaceOutputEvent::Output { data, .. } if data.as_ref() == "resumed")
    );
    assert!(matches!(
        exit,
        TerminalSurfaceOutputEvent::Exit {
            exit_code: Some(0),
            ..
        }
    ));
}

#[test]
fn test_流量停止_実processorの出力で高水位を超えると後続出力を低水位まで止める() {
    use crate::usecase::terminal_surface::output::TerminalSurfaceOutputControl;
    // Given
    let hub = Arc::new(
        crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub::with_flags(8, true),
    );
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
    })
    .unwrap();
    let mut published = 0;
    while published < units {
        let TerminalSurfaceOutputEvent::Output { data, .. } =
            received.recv_timeout(Duration::from_secs(1)).unwrap()
        else {
            panic!("output");
        };
        published += data.len();
    }
    let stopped = waits.recv_timeout(Duration::from_secs(1));
    send.send(TerminalOutputCommand::Data {
        data: "next".into(),
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
        matches!(resumed.unwrap(), TerminalSurfaceOutputEvent::Output { data, .. } if data.as_ref() == "next")
    );
    assert!(matches!(
        received.recv_timeout(Duration::from_secs(1)).unwrap(),
        TerminalSurfaceOutputEvent::Exit {
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
    use crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub;
    use crate::test_support::state_subscription::{Event, StateSubscriptionEvent};
    use crate::usecase::state_subscription::StateSubscriptionUsecase;
    use crate::usecase::state_subscription::SubscriptionTarget;
    use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
    use futures_util::StreamExt;
    let hub = Arc::new(TerminalSurfaceEventHub::with_flags(256, true));
    let gateway = Arc::new(TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(
        Arc::new(|_| {}),
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
    hub.initialize(crate::test_support::state_subscription::registration(
        &key, "/repo", None, 1, 0,
    ))
    .unwrap();
    let terminal = Arc::new(TerminalSurfaceApplication::new(
        std::sync::Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
        gateway.clone(),
        Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
        hub.clone(),
    ));
    let subscriptions = StateSubscriptionUsecase::new(
        vec![],
        crate::test_support::state_subscription::read_driver(),
    );
    let subscriptions = subscriptions.with_terminal(terminal.clone());
    let stream = subscriptions.open("client".into()).unwrap();
    tokio::pin!(stream);
    stream.next().await;
    subscriptions
        .deps()
        .start_subscription(
            "client",
            &crate::usecase::state_subscription::SubscriptionTarget::parse(&target).unwrap(),
            "input",
            None,
        )
        .await
        .unwrap();
    stream.next().await;
    stream.next().await;
    terminal.write_attached(&owner, "input", 0, "old").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while old_written.lock().len() < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(&*old_written.lock(), b"old");
    // When
    hub.publish(TerminalSurfaceOutputEvent::Exit {
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
            matches!(next, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(crate::test_support::state_subscription::terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Exit(exit) if exit.exit_code == Some(7)))
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
    hub.initialize(crate::test_support::state_subscription::registration(
        &key, "/repo", None, 2, 0,
    ))
    .unwrap();
    // Then
    if !drain_exit {
        let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap();
        assert!(
            matches!(next, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(crate::test_support::state_subscription::terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Exit(exit) if exit.exit_code == Some(7)))
        );
    }
    let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap();
    assert!(
        matches!(next, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, value))) if version.epoch.ends_with(":2") && matches!(crate::test_support::state_subscription::terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Snapshot(surface) if surface.session_key == key))
    );
    terminal.write_attached(&owner, "input", 1, "new").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while new_written.lock().len() < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(&*new_written.lock(), b"new");
    hub.publish(TerminalSurfaceOutputEvent::Output {
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
        matches!(next, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(crate::test_support::state_subscription::terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Output(output) if output.sequence == 1 && output.data.len() == 100_001))
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
        crate::test_support::state_subscription::terminal_processed(
            &subscriptions,
            "client",
            &target,
            5000,
        )
        .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(2), received)
        .await
        .unwrap()
        .unwrap();
    waiter.await.unwrap();
    subscriptions
        .terminal
        .stop_delivery(
            "client",
            &crate::usecase::state_subscription::SubscriptionTarget::parse(&target).unwrap(),
            "input",
            &subscriptions
                .usecase
                .test_presenter()
                .unwrap()
                .delivery("input")
                .unwrap()
                .2,
        )
        .unwrap();
    assert!(terminal
        .write_attached(&owner, "input", 2, "stale")
        .is_err());
    assert!(crate::test_support::state_subscription::terminal_processed(
        &subscriptions,
        "client",
        &target,
        5000
    )
    .is_err());
}

struct FailingResizer(std::io::ErrorKind);

impl NativePtyResizer for FailingResizer {
    fn resize(
        &mut self,
        _: u16,
        _: u16,
    ) -> Result<(), crate::infrastructure::terminal::native_pty::NativePtyError> {
        Err(
            crate::infrastructure::terminal::native_pty::NativePtyError {
                kind: self.0,
                message: "resize source failure".into(),
            },
        )
    }
}

#[derive(Debug)]
struct FailingKiller(std::io::ErrorKind);

#[test]
fn test_pty外部失敗_resizeで性質と元のメッセージを保持する() {
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    // Given
    for (kind, nature) in [
        (
            std::io::ErrorKind::WouldBlock,
            TechnicalFailureNature::Transient,
        ),
        (
            std::io::ErrorKind::TimedOut,
            TechnicalFailureNature::TimedOut,
        ),
        (
            std::io::ErrorKind::PermissionDenied,
            TechnicalFailureNature::Other,
        ),
    ] {
        let gateway = TerminalSurfaceRuntimeGatewayFor::default();
        insert_test_session_with_resizer(
            &gateway,
            1,
            "key",
            Some("/repo"),
            None,
            Box::new(FailingResizer(kind)),
        );
        // When
        let error = gateway.resize("key", 30, 100).unwrap_err();
        // Then
        assert_eq!(
            error,
            TerminalSurfaceGatewayError::Technical(TechnicalFailure {
                nature,
                message: "resize source failure".into(),
            })
        );
    }
}

#[test]
fn test_pty外部失敗_killで性質と元のメッセージを保持する() {
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    // Given
    for (kind, nature) in [
        (
            std::io::ErrorKind::WouldBlock,
            TechnicalFailureNature::Transient,
        ),
        (
            std::io::ErrorKind::TimedOut,
            TechnicalFailureNature::TimedOut,
        ),
        (
            std::io::ErrorKind::PermissionDenied,
            TechnicalFailureNature::Other,
        ),
    ] {
        let gateway = TerminalSurfaceRuntimeGatewayFor::default();
        insert_test_session(&gateway, 1, "key", Some("/repo"), None);
        gateway.runtimes.lock().get_mut(&1).unwrap().native_pty = NativePtyRuntime::from_parts(
            Box::new(MockWriter(Arc::new(Mutex::new(Vec::new())))),
            Box::new(FailingKiller(kind)),
            Box::new(MockResizer { rows: 24, cols: 80 }),
        );
        // When
        let error = gateway.request_runtime_stop(1).unwrap_err();
        // Then
        assert_eq!(
            error,
            TerminalSurfaceGatewayError::Technical(TechnicalFailure {
                nature,
                message: "Failed to kill PTY: kill source failure".into(),
            })
        );
    }
}

#[tokio::test]
async fn test_terminal対象なし_購読開始とsnapshot読取と配信でnot_foundを保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::adaptor::presenter::state_subscription::{PublishedState, StateSubscriptionEvent};
    use crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub;
    use crate::infrastructure::state_subscription::Event;
    use crate::usecase::state_subscription::{
        StateReadFailure, StateSubscriptionUsecase, SubscriptionTarget,
    };
    use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
    use crate::usecase::terminal_surface::error::UsecaseError;
    use crate::usecase::terminal_surface::output::TerminalSurfaceStateSink;
    use crate::usecase::terminal_surface::subscription::TerminalSubscriptionOutput;
    use futures_util::StreamExt;
    // Given
    let hub = Arc::new(TerminalSurfaceEventHub::with_flags(256, true));
    let gateway = Arc::new(TerminalSurfaceRuntimeGatewayFor::default());
    let terminal = Arc::new(TerminalSurfaceApplication::new(
        Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway), gateway,
        Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())), hub,
    ));
    let subscriptions = StateSubscriptionUsecase::new(
        vec![],
        crate::test_support::state_subscription::read_driver(),
    )
    .with_terminal(terminal.clone());
    let presenter = subscriptions.test_presenter().unwrap();
    let mut stream = Box::pin(subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::Terminal(workspace_owner("/repo"));
    // When
    let start = subscriptions
        .deps()
        .start_subscription("client", &target, "input", None)
        .await
        .unwrap_err();
    let snapshot = subscriptions
        .terminal
        .refresh_terminal(&target)
        .await
        .unwrap_err();
    // Then
    for error in [&start, &snapshot] {
        assert!(
            matches!(&error.source, StateReadFailure::Terminal(inner) if matches!(**inner, UsecaseError::NotFound(_)))
        );
        assert_eq!(error.connect_code(), connectrpc::ErrorCode::NotFound);
    }
    presenter
        .initialize(&crate::test_support::state_subscription::registration(
            "key", "/repo", None, 1, 0,
        ))
        .unwrap();
    presenter.publish_failure(&target, snapshot).unwrap();
    let delivery = subscriptions
        .usecase
        .test_presenter()
        .unwrap()
        .reserve_delivery("client", "input", &target.to_string(), None)
        .unwrap();
    crate::usecase::state_subscription::StateSubscriptionDelivery::start(&delivery).unwrap();
    let event = stream.next().await.unwrap();
    assert!(
        matches!(event, StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))
        if matches!(&*value, PublishedState::Failure(failure) if failure.code == connectrpc::ErrorCode::NotFound.grpc_code() as i32))
    );
}

#[test]
fn test_checkpoint保存_技術的な失敗の性質とメッセージを保持する() {
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    // Given / When / Then
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        let failure = TechnicalFailure {
            nature,
            message: "source failure".into(),
        };
        assert_eq!(
            checkpoint_work_failure(failure.clone().into()),
            TerminalSurfaceGatewayError::Technical(failure)
        );
    }
}

impl TerminalSurfaceEventSink for RecordingEventSink {
    fn publish(&self, event: TerminalSurfaceOutputEvent) {
        self.events.lock().unwrap().push(event);
    }
}

impl TerminalSurfaceEventSink for SummarySink {
    fn remove(&self, runtime_generation: u64) -> bool {
        self.removed.lock().unwrap().push(runtime_generation);
        false
    }

    fn publish(&self, _: TerminalSurfaceOutputEvent) {}
}

impl TerminalSurfaceEventSink for BlockingFirstEventSink {
    fn publish(&self, event: TerminalSurfaceOutputEvent) {
        let sequence = match event {
            TerminalSurfaceOutputEvent::Output { sequence, .. }
            | TerminalSurfaceOutputEvent::Resize { sequence, .. }
            | TerminalSurfaceOutputEvent::Exit { sequence, .. } => sequence,
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

impl TerminalSurfaceEventSink for FlowControlledSink {
    fn wait_output(&self, session_key: &str) {
        self.waiting.send(std::thread::current().id()).unwrap();
        self.hub.wait_output(session_key);
    }

    fn publish(&self, event: TerminalSurfaceOutputEvent) {
        self.hub.publish(event.clone());
        self.events.send(event).unwrap();
    }
}

impl portable_pty::ChildKiller for FailingKiller {
    fn kill(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::new(self.0, "kill source failure"))
    }
    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        Box::new(Self(self.0))
    }
}
