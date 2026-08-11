use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use super::*;
use crate::domain::terminal_surface::entities::{
    TerminalSurfaceRegistry, TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError,
};
use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceGatewayError, TerminalSurfaceRepository,
};
use crate::domain::terminal_surface::{
    TerminalProcessLaunch, TerminalSurfaceLifecycleConfig, TerminalSurfaceOwner,
};
use crate::domain::workspace_tree::WorkspaceIdentity;

struct MockGateway {
    registry: Mutex<TerminalSurfaceRegistry>,
    fail_spawn: AtomicBool,
    fail_load_checkpoint: AtomicBool,
    fail_start_reader: AtomicBool,
    fail_kill_runtime: AtomicBool,
    spawn_count: Mutex<usize>,
    spawned_sizes: Mutex<Vec<(u16, u16)>>,
    spawned_processes: Mutex<Vec<Option<TerminalProcessLaunch>>>,
    checkpoint: Mutex<Option<TerminalSurfaceCheckpoint>>,
    written_inputs: Mutex<Vec<String>>,
    killed: Mutex<Vec<u64>>,
    block_spawn: AtomicBool,
    spawn_started: (Mutex<bool>, Condvar),
    release_spawn: (Mutex<bool>, Condvar),
    reserve_attempts: (Mutex<usize>, Condvar),
    spawn_resolved: Condvar,
}

impl MockGateway {
    fn new() -> Self {
        Self {
            registry: Mutex::new(TerminalSurfaceRegistry::with_config(
                TerminalSurfaceLifecycleConfig {
                    per_worktree_cap: 2,
                    max_panes_total: 3,
                },
            )),
            fail_spawn: AtomicBool::new(false),
            fail_load_checkpoint: AtomicBool::new(false),
            fail_start_reader: AtomicBool::new(false),
            fail_kill_runtime: AtomicBool::new(false),
            spawn_count: Mutex::new(0),
            spawned_sizes: Mutex::new(Vec::new()),
            spawned_processes: Mutex::new(Vec::new()),
            checkpoint: Mutex::new(None),
            written_inputs: Mutex::new(Vec::new()),
            killed: Mutex::new(Vec::new()),
            block_spawn: AtomicBool::new(false),
            spawn_started: (Mutex::new(false), Condvar::new()),
            release_spawn: (Mutex::new(false), Condvar::new()),
            reserve_attempts: (Mutex::new(0), Condvar::new()),
            spawn_resolved: Condvar::new(),
        }
    }

    fn insert_session(&self, owner_id: &str, worktree_path: &str) -> u64 {
        let mut registry = self.registry.lock().unwrap();
        let runtime_generation = registry.next_runtime_generation();
        registry.insert(TerminalSurface::new(
            runtime_generation,
            TerminalSurfaceOwner::session(WorkspaceIdentity::new(worktree_path), owner_id).unwrap(),
            None,
        ));
        runtime_generation
    }

    fn mark_exited(&self, runtime_generation: u64) {
        self.registry
            .lock()
            .unwrap()
            .mark_exited(runtime_generation, Some(0));
    }
}

fn workspace_owner(path: &str) -> TerminalSurfaceOwner {
    TerminalSurfaceOwner::workspace(WorkspaceIdentity::new(path)).unwrap()
}

impl TerminalSurfaceRepository for MockGateway {
    fn find_summary_by_session_key(&self, session_key: &str) -> Option<TerminalSurfaceSummary> {
        self.registry
            .lock()
            .unwrap()
            .find_by_session_key(session_key)
            .map(TerminalSurface::summary)
    }

    fn list_summaries(
        &self,
    ) -> Vec<crate::domain::terminal_surface::entities::TerminalSurfaceSummary> {
        self.registry.lock().unwrap().list_summaries()
    }
}

impl TerminalSurfaceGateway for MockGateway {
    fn next_runtime_generation(&self) -> u64 {
        self.registry.lock().unwrap().next_runtime_generation()
    }

    fn load_terminal_checkpoint(
        &self,
        _session_key: &str,
    ) -> Result<Option<TerminalSurfaceCheckpoint>, TerminalSurfaceGatewayError> {
        if self.fail_load_checkpoint.load(Ordering::SeqCst) {
            return Err(TerminalSurfaceGatewayError::new("checkpoint load failed"));
        }
        Ok(self.checkpoint.lock().unwrap().clone())
    }

    fn delete_terminal_checkpoint(
        &self,
        _session_key: &str,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        *self.checkpoint.lock().unwrap() = None;
        Ok(())
    }

    fn spawn_runtime(
        &self,
        request: TerminalRuntimeSpawnRequest,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        *self.spawn_count.lock().unwrap() += 1;
        self.spawned_sizes
            .lock()
            .unwrap()
            .push((request.rows, request.cols));
        self.spawned_processes.lock().unwrap().push(request.process);
        if self.fail_spawn.load(Ordering::SeqCst) {
            return Err(TerminalSurfaceGatewayError::new("spawn failed"));
        }
        if self.block_spawn.load(Ordering::SeqCst) {
            let (started, changed) = &self.spawn_started;
            *started.lock().unwrap() = true;
            changed.notify_all();
            let (released, changed) = &self.release_spawn;
            let _guard = changed
                .wait_while(released.lock().unwrap(), |released| !*released)
                .unwrap();
        }
        Ok(())
    }

    fn insert_surface(&self, surface: TerminalSurface) {
        self.registry.lock().unwrap().insert(surface);
    }

    fn start_output_reader(
        &self,
        _runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        if self.fail_start_reader.load(Ordering::SeqCst) {
            return Err(TerminalSurfaceGatewayError::new("reader failed"));
        }
        Ok(())
    }

    fn snapshot(&self, runtime_generation: u64) -> Option<TerminalSurface> {
        self.registry
            .lock()
            .unwrap()
            .get(runtime_generation)
            .cloned()
    }

    fn select_kill_targets_by_worktree(&self, worktree_path: &str) -> Vec<u64> {
        self.registry
            .lock()
            .unwrap()
            .select_kill_targets_by_worktree(worktree_path)
    }

    fn remove_surface(&self, runtime_generation: u64) -> Option<TerminalSurface> {
        self.registry.lock().unwrap().remove(runtime_generation)
    }

    fn reserve_spawn_slot(
        &self,
        session_key: &str,
        worktree_path: Option<&str>,
    ) -> Result<TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError> {
        let (attempts, changed) = &self.reserve_attempts;
        *attempts.lock().unwrap() += 1;
        changed.notify_all();
        self.registry
            .lock()
            .unwrap()
            .reserve_spawn_slot(session_key, worktree_path)
    }

    fn complete_spawn_slot(&self, reservation: &TerminalSurfaceSpawnReservation) {
        self.registry
            .lock()
            .unwrap()
            .complete_spawn_slot(reservation);
        self.spawn_resolved.notify_all();
    }

    fn rollback_spawn_slot(&self, reservation: &TerminalSurfaceSpawnReservation) {
        self.registry
            .lock()
            .unwrap()
            .rollback_spawn_slot(reservation);
        self.spawn_resolved.notify_all();
    }

    fn activate_input_attachment(&self, _session_key: &str, _attachment_id: &str) {}

    fn deactivate_input_attachment(&self, _session_key: &str, _attachment_id: &str) {}

    fn write_attached(
        &self,
        session_key: &str,
        _attachment_id: &str,
        _sequence: u64,
        data: &str,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        self.write(session_key, data)
    }

    fn wait_for_spawn_resolution(&self, session_key: &str) -> Option<TerminalSurfaceSummary> {
        let mut registry = self.registry.lock().unwrap();
        loop {
            if let Some(surface) = registry.find_by_session_key(session_key) {
                return Some(surface.summary());
            }
            if !registry.is_spawn_reserved(session_key) {
                return None;
            }
            registry = self.spawn_resolved.wait(registry).unwrap();
        }
    }

    fn write(&self, _session_key: &str, data: &str) -> Result<(), TerminalSurfaceGatewayError> {
        self.written_inputs.lock().unwrap().push(data.to_string());
        Ok(())
    }

    fn resize(
        &self,
        _session_key: &str,
        _rows: u16,
        _cols: u16,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        Ok(())
    }

    fn request_runtime_stop(
        &self,
        runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        self.killed.lock().unwrap().push(runtime_generation);
        if self.fail_kill_runtime.load(Ordering::SeqCst) {
            return Err(TerminalSurfaceGatewayError::new("kill failed"));
        }
        Ok(())
    }

    fn remove_runtime(&self, _runtime_generation: u64) {}
}

#[test]
fn test_ターミナル画面取得または生成_上限未到達なら新規生成する() {
    let gateway = MockGateway::new();

    let result = get_or_spawn(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        Some("dev".to_string()),
    )
    .unwrap();

    assert!(result.is_new);
    assert_eq!(*gateway.spawn_count.lock().unwrap(), 1);
}

#[test]
fn test_ターミナル画面生成_新規ptyだけに起動コマンドを一度入力する() {
    let gateway = MockGateway::new();

    let result = get_or_spawn_with_startup(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        None,
        Some("  cargo test  ".to_string()),
    )
    .unwrap();

    assert!(result.is_new);
    assert_eq!(
        gateway.written_inputs.lock().unwrap().as_slice(),
        ["cargo test\n"]
    );
}

#[test]
fn test_agent_session_terminal生成_providerをstructured_root_processとして渡す() {
    let gateway = MockGateway::new();
    let process = TerminalProcessLaunch::new(
        "/usr/local/bin/codex",
        vec!["resume".to_string(), "provider-session-1".to_string()],
        vec![(
            "RELEASH_AGENT_SESSION_ID".to_string(),
            "agent-1".to_string(),
        )],
    )
    .unwrap();

    let result = get_or_spawn_with_process(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-1").unwrap(),
        Some("Codex".to_string()),
        process.clone(),
    )
    .unwrap();

    assert!(result.is_new);
    assert_eq!(
        gateway.spawned_processes.lock().unwrap().as_slice(),
        [Some(process)]
    );
    assert!(gateway.written_inputs.lock().unwrap().is_empty());
}

#[test]
fn test_agent_session_terminal再開_終了済みruntimeを新しいprocessへ置換する() {
    let gateway = MockGateway::new();
    let old_generation = gateway.insert_session("agent-1", "/repo");
    gateway.mark_exited(old_generation);
    let process = TerminalProcessLaunch::new(
        "/usr/local/bin/codex",
        vec!["resume".to_string(), "provider-session-1".to_string()],
        vec![],
    )
    .unwrap();

    let result = get_or_spawn_with_process(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-1").unwrap(),
        Some("Codex".to_string()),
        process,
    )
    .unwrap();

    assert!(result.is_new);
    assert_ne!(result.surface.runtime_generation.value(), old_generation);
    assert_eq!(*gateway.spawn_count.lock().unwrap(), 1);
    assert!(gateway.snapshot(old_generation).is_none());
}

#[test]
fn test_agent_session_terminal_archiveは停止してcheckpointを保持しdeleteは破棄する() {
    let gateway = MockGateway::new();
    *gateway.checkpoint.lock().unwrap() = Some(TerminalSurfaceCheckpoint {
        replay: "last frame".to_string(),
        sequence: 7,
        cols: 80,
        rows: 24,
    });
    let owner = TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-1").unwrap();
    let generation = gateway.insert_session("agent-1", "/repo");

    crate::usecase::terminal_surface::lifecycle_usecase::stop_preserving_checkpoint(
        &gateway, &owner,
    )
    .unwrap();

    assert_eq!(*gateway.killed.lock().unwrap(), vec![generation]);
    assert!(gateway.snapshot(generation).is_none());
    assert_eq!(
        gateway
            .checkpoint
            .lock()
            .unwrap()
            .as_ref()
            .map(|checkpoint| checkpoint.replay.as_str()),
        Some("last frame")
    );

    let regenerated = gateway.insert_session("agent-1", "/repo");
    crate::usecase::terminal_surface::lifecycle_usecase::kill(&gateway, &owner).unwrap();
    assert!(gateway.snapshot(regenerated).is_none());
    assert!(gateway.checkpoint.lock().unwrap().is_none());
}

#[test]
fn test_ターミナル画面_取得または生成_同一所有者の生成中に再接続すると同じ画面へ合流する() {
    let gateway = Arc::new(MockGateway::new());
    gateway.block_spawn.store(true, Ordering::SeqCst);

    let first = std::thread::spawn({
        let gateway = Arc::clone(&gateway);
        move || {
            get_or_spawn(
                gateway.as_ref(),
                24,
                80,
                Some("/repo".to_string()),
                workspace_owner("/repo"),
                None,
            )
        }
    });
    let (started, changed) = &gateway.spawn_started;
    let _guard = changed
        .wait_while(started.lock().unwrap(), |started| !*started)
        .unwrap();

    let second = std::thread::spawn({
        let gateway = Arc::clone(&gateway);
        move || {
            get_or_spawn(
                gateway.as_ref(),
                24,
                80,
                Some("/repo".to_string()),
                workspace_owner("/repo"),
                None,
            )
        }
    });
    let (attempts, changed) = &gateway.reserve_attempts;
    let _guard = changed
        .wait_while(attempts.lock().unwrap(), |attempts| *attempts < 2)
        .unwrap();
    let (released, changed) = &gateway.release_spawn;
    *released.lock().unwrap() = true;
    changed.notify_all();

    let first = first.join().unwrap().unwrap();
    let second = second.join().unwrap().unwrap();
    assert_eq!(
        first.surface.runtime_generation,
        second.surface.runtime_generation
    );
    assert!(first.is_new);
    assert!(!second.is_new);
    assert_eq!(*gateway.spawn_count.lock().unwrap(), 1);
}

#[test]
fn test_ターミナル画面取得または生成_通信文脈を要求しない() {
    let gateway = MockGateway::new();

    let result = get_or_spawn(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        None,
    )
    .unwrap();

    assert!(result.is_new);
}

#[test]
fn test_ターミナル画面取得または生成_上限到達時は既存画面を保持してエラーにする() {
    let gateway = MockGateway::new();
    let first = gateway.insert_session("key-1", "/repo");
    let second = gateway.insert_session("key-2", "/repo");

    let result = get_or_spawn(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        None,
    );

    assert!(matches!(result, Err(UsecaseError::CapReached(_))));
    assert_eq!(*gateway.spawn_count.lock().unwrap(), 0);
    assert!(gateway.killed.lock().unwrap().is_empty());
    assert!(gateway.snapshot(first).is_some());
    assert!(gateway.snapshot(second).is_some());
}

#[test]
fn test_ターミナル画面生成_実行環境生成失敗時に予約を解除する() {
    let gateway = MockGateway::new();
    gateway.fail_spawn.store(true, Ordering::SeqCst);

    let result = get_or_spawn(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        None,
    );

    assert!(matches!(result, Err(UsecaseError::Gateway(_))));
    gateway.fail_spawn.store(false, Ordering::SeqCst);

    let retry = get_or_spawn(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        None,
    )
    .unwrap();

    assert!(retry.is_new);
    assert_eq!(*gateway.spawn_count.lock().unwrap(), 2);
}

#[test]
fn test_ターミナル画面生成_復元点読込失敗時に予約を解除して同一所有者が再試行できる() {
    let gateway = MockGateway::new();
    gateway.fail_load_checkpoint.store(true, Ordering::SeqCst);

    let result = get_or_spawn(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        None,
    );

    assert!(matches!(
        result,
        Err(UsecaseError::Gateway(message)) if message == "checkpoint load failed"
    ));
    assert!(!gateway
        .registry
        .lock()
        .unwrap()
        .is_spawn_reserved(&workspace_owner("/repo").stable_key()));
    gateway.fail_load_checkpoint.store(false, Ordering::SeqCst);

    let retry = get_or_spawn(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        None,
    )
    .unwrap();

    assert!(retry.is_new);
    assert_eq!(*gateway.spawn_count.lock().unwrap(), 1);
}

#[test]
fn test_ターミナル画面生成_出力読取開始失敗時に新規画面を片付ける() {
    let gateway = MockGateway::new();
    gateway.fail_start_reader.store(true, Ordering::SeqCst);

    let result = get_or_spawn(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        None,
    );

    assert!(matches!(result, Err(UsecaseError::Gateway(_))));
    assert_eq!(*gateway.spawn_count.lock().unwrap(), 1);
    assert_eq!(*gateway.killed.lock().unwrap(), vec![1]);
    assert!(gateway.snapshot(1).is_none());
}

#[test]
fn test_ターミナル画面生成_出力読取開始失敗時は復元点も破棄して再試行の起動コマンドを実行する() {
    let gateway = MockGateway::new();
    *gateway.checkpoint.lock().unwrap() = Some(TerminalSurfaceCheckpoint {
        replay: "stale".to_string(),
        sequence: 7,
        cols: 80,
        rows: 24,
    });
    gateway.fail_start_reader.store(true, Ordering::SeqCst);

    let failed = get_or_spawn_with_startup(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        None,
        Some("cargo test".to_string()),
    );
    assert!(failed.is_err());
    assert!(gateway.checkpoint.lock().unwrap().is_none());

    gateway.fail_start_reader.store(false, Ordering::SeqCst);
    let retried = get_or_spawn_with_startup(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        None,
        Some("cargo test".to_string()),
    )
    .unwrap();

    assert!(!retried.restored_from_checkpoint);
    assert_eq!(
        gateway.written_inputs.lock().unwrap().as_slice(),
        ["cargo test\n"]
    );
}

#[test]
fn test_ターミナル画面明示終了_復元点も破棄して再生成の起動コマンドを実行する() {
    let gateway = MockGateway::new();
    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "session-1").unwrap();
    gateway.insert_session("session-1", "/repo");
    *gateway.checkpoint.lock().unwrap() = Some(TerminalSurfaceCheckpoint {
        replay: "stale".to_string(),
        sequence: 7,
        cols: 80,
        rows: 24,
    });

    crate::usecase::terminal_surface::lifecycle_usecase::kill(&gateway, &owner).unwrap();
    assert!(gateway.checkpoint.lock().unwrap().is_none());

    let regenerated = get_or_spawn_with_startup(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        owner,
        None,
        Some("cargo test".to_string()),
    )
    .unwrap();

    assert!(!regenerated.restored_from_checkpoint);
    assert_eq!(
        gateway.written_inputs.lock().unwrap().as_slice(),
        ["cargo test\n"]
    );
}

#[test]
fn test_ターミナル画面生成_後始末終了失敗時は明示再試行まで操作子を保持する() {
    let gateway = MockGateway::new();
    gateway.fail_start_reader.store(true, Ordering::SeqCst);
    gateway.fail_kill_runtime.store(true, Ordering::SeqCst);

    let result = get_or_spawn(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "failed-spawn").unwrap(),
        None,
    );

    assert!(matches!(
        result,
        Err(UsecaseError::Gateway(message)) if message == "reader failed"
    ));
    assert_eq!(*gateway.spawn_count.lock().unwrap(), 1);
    assert_eq!(*gateway.killed.lock().unwrap(), vec![1]);
    assert!(gateway.snapshot(1).is_some());

    gateway.fail_kill_runtime.store(false, Ordering::SeqCst);
    crate::usecase::terminal_surface::lifecycle_usecase::kill_runtime_generation(&gateway, 1)
        .unwrap();

    assert_eq!(*gateway.killed.lock().unwrap(), vec![1, 1]);
    assert!(gateway.snapshot(1).is_none());
}

#[test]
fn test_ターミナル画面取得または生成_既存所有者なら生成せず画面を返す() {
    let gateway = MockGateway::new();
    let runtime_generation = gateway.insert_session("session-1", "/repo");

    let result = get_or_spawn(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "session-1").unwrap(),
        None,
    )
    .unwrap();

    assert!(!result.is_new);
    assert_eq!(
        result.surface.runtime_generation.value(),
        runtime_generation
    );
    assert_eq!(*gateway.spawn_count.lock().unwrap(), 0);
}

#[test]
fn test_ターミナル画面取得または生成_既存所有者確認で復元点本体を読み出さない() {
    let gateway = MockGateway::new();
    gateway.insert_session("session-1", "/repo");

    let result = get_or_spawn(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "session-1").unwrap(),
        None,
    )
    .unwrap();

    assert!(!result.is_new);
    let _: TerminalSurfaceSummary = result.surface;
}

#[test]
fn test_ターミナル画面取得または生成_別ワークスペースの同一所有者識別子を隔離して生成する() {
    let gateway = MockGateway::new();
    let runtime_generation = gateway.insert_session("session-1", "/repo");

    let result = get_or_spawn(
        &gateway,
        24,
        80,
        Some("/other".to_string()),
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/other"), "session-1").unwrap(),
        None,
    )
    .unwrap();

    assert!(result.is_new);
    assert_ne!(
        result.surface.runtime_generation.value(),
        runtime_generation
    );
    assert_eq!(*gateway.spawn_count.lock().unwrap(), 1);
}

#[test]
fn test_ターミナル画面_再起動復元_復元点寸法で新規ptyを開始する() {
    let gateway = MockGateway::new();
    *gateway.checkpoint.lock().unwrap() = Some(TerminalSurfaceCheckpoint {
        replay: "checkpoint".to_string(),
        sequence: 12,
        cols: 111,
        rows: 37,
    });

    let result = get_or_spawn_with_startup(
        &gateway,
        24,
        80,
        Some("/repo".to_string()),
        workspace_owner("/repo"),
        None,
        Some("must-not-run".to_string()),
    )
    .unwrap();

    assert!(result.restored_from_checkpoint);
    assert_eq!(*gateway.spawned_sizes.lock().unwrap(), vec![(37, 111)]);
    assert!(gateway.written_inputs.lock().unwrap().is_empty());
}
