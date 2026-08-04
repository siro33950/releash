use super::*;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex as StdMutex};
use std::time::Duration;

fn workspace_owner(path: &str) -> TerminalSurfaceOwner {
    TerminalSurfaceOwner::workspace(WorkspaceIdentity::new(path))
}

fn session_owner(path: &str, session_id: &str) -> TerminalSurfaceOwner {
    TerminalSurfaceOwner::session(WorkspaceIdentity::new(path), session_id)
}

struct BlockingFirstEventSink {
    first_started: Arc<(StdMutex<bool>, Condvar)>,
    release_first: Arc<(StdMutex<bool>, Condvar)>,
    sequences: StdMutex<Vec<u64>>,
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
                        data: "first".to_string(),
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
    let store = TerminalCheckpointFileStore::new(data_dir.path());
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
    let app = tauri::test::mock_builder()
        .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
            data_dir.path().to_path_buf(),
        ))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let gateway = TerminalSurfaceRuntimeGatewayFor::new(app.handle().clone());

    let result = crate::usecase::terminal_surface::spawn_usecase::get_or_spawn(
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

fn insert_test_session_with_resizer<R: tauri::Runtime>(
    gateway: &TerminalSurfaceRuntimeGatewayFor<R>,
    runtime_generation: u64,
    session_key: &str,
    worktree_path: Option<&str>,
    label: Option<&str>,
    resizer: Box<dyn NativePtyResizer + Send>,
) -> Arc<std::sync::atomic::AtomicBool> {
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
        TerminalSurfaceOwner::session(workspace, session_key),
        label.map(str::to_string),
    );
    session.worktree_path = worktree_path.map(str::to_string);
    let checkpoint = terminal_surface.lock().snapshot(1);
    assert!(session.apply_checkpoint(runtime_generation, to_domain_checkpoint(&checkpoint)));
    gateway.runtimes.lock().insert(
        runtime_generation,
        AttachedTerminalRuntime {
            native_pty: NativePtyRuntime::from_parts(
                Box::new(MockWriter(written)),
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
        },
    );
    gateway.insert_surface(session);
    killed
}

fn insert_test_session<R: tauri::Runtime>(
    gateway: &TerminalSurfaceRuntimeGatewayFor<R>,
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
}

#[test]
fn test_ターミナル画面入力_存在するptyへ書き込む() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    insert_test_session(&gateway, 1, "key", Some("/repo"), None);
    assert!(gateway.write("key", "hello").is_ok());
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
            let sequence = registry.lock().record_output(1)?;
            Some((
                sequence,
                TerminalSurfaceEvent::Output {
                    session_key: "key".to_string(),
                    data: "output-after-resize".to_string(),
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
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let first_started = Arc::new((StdMutex::new(false), Condvar::new()));
    let release_first = Arc::new((StdMutex::new(false), Condvar::new()));
    let sink = Arc::new(BlockingSessionSink {
        blocked_session_key: "surface-a".to_string(),
        started: Arc::clone(&first_started),
        release: Arc::clone(&release_first),
    });
    let gateway = Arc::new(TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(
        app.handle().clone(),
        sink,
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
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let captured = Arc::new(CapturedTerminalOutput::default());
    let gateway = TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(
        app.handle().clone(),
        captured.clone(),
    );
    insert_test_session(&gateway, 1, "key", Some("/repo"), None);

    gateway.resize("key", 30, 100).unwrap();

    assert_eq!(*captured.resizes.lock().unwrap(), vec![(100, 30, 2)]);
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
fn test_ターミナル画面整理_指定セッションキーを保持する() {
    let gateway = TerminalSurfaceRuntimeGateway::default();
    insert_test_session(&gateway, 1, "key-1", Some("/repo"), Some("dev"));
    insert_test_session(&gateway, 2, "key-2", Some("/repo"), Some("test"));
    insert_test_session(&gateway, 3, "key-3", Some("/other"), None);

    let killed = crate::usecase::terminal_surface::lifecycle_usecase::gc_by_worktree(
        &gateway,
        "/repo",
        &[String::from("key-1")],
    );

    assert_eq!(killed, vec![2]);
    assert!(gateway.snapshot(1).is_some());
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
