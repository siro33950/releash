use crate::domain::pty_session::{entities::PtySession, PtyEvictReason, PtyKind};
use crate::usecase::pty_session::dto::{pty_kind_to_wire, GetOrSpawnPtyResult};
use crate::usecase::pty_session::error::UsecaseError;
use crate::usecase::pty_session::ports::{PtyBackendSpawnRequest, PtySessionGateway};

#[allow(clippy::too_many_arguments)]
pub fn spawn<G: PtySessionGateway>(
    manager: &G,
    app: &G::AppContext,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    worktree_path: Option<String>,
    label: Option<String>,
    kind: PtyKind,
) -> Result<(u64, String), UsecaseError> {
    let reservation = manager
        .reserve_spawn_slot(worktree_path.as_deref(), manager.now_ms())
        .map_err(UsecaseError::from)?;

    let pty_id = manager.next_pty_id();
    let session_key = uuid::Uuid::new_v4().to_string();
    let runtime = match manager.spawn_backend(
        app,
        PtyBackendSpawnRequest {
            pty_id,
            rows,
            cols,
            cwd,
            exec_command: None,
        },
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            manager.rollback_spawn_slot(&reservation);
            return Err(error);
        }
    };

    manager.insert_session(
        PtySession::new(pty_id, session_key.clone(), worktree_path, label, kind),
        runtime,
    );
    manager.record_activity(pty_id, manager.now_ms());
    manager.pin_session_key(&session_key);
    if let Err(error) = manager.start_output_reader(app, pty_id) {
        cleanup_failed_spawn(manager, pty_id);
        manager.rollback_spawn_slot(&reservation);
        return Err(error);
    }

    for target in reservation.evict_targets.clone() {
        match crate::usecase::pty_session::lifecycle_usecase::evict(
            manager,
            app,
            target,
            PtyEvictReason::CapExceeded,
            manager.now_ms(),
        ) {
            Ok(Some(_)) => {}
            Ok(None) => {
                cleanup_failed_spawn(manager, pty_id);
                manager.rollback_spawn_slot(&reservation);
                return Err(UsecaseError::CapReached(
                    "PTY cap reached; eviction target is no longer idle".to_string(),
                ));
            }
            Err(error) => {
                cleanup_failed_spawn(manager, pty_id);
                manager.rollback_spawn_slot(&reservation);
                return Err(error);
            }
        }
    }

    manager.complete_spawn_slot(&reservation);

    Ok((pty_id, session_key))
}

fn cleanup_failed_spawn(manager: &impl PtySessionGateway, pty_id: u64) {
    if manager.snapshot(pty_id).is_some() {
        let _ = manager.kill_runtime(pty_id);
        manager.remove_session(pty_id);
    } else {
        manager.remove_runtime(pty_id);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn get_or_spawn<G: PtySessionGateway>(
    manager: &G,
    app: &G::AppContext,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    session_key: Option<String>,
    worktree_path: String,
    label: Option<String>,
    kind: PtyKind,
) -> Result<GetOrSpawnPtyResult, UsecaseError> {
    if let Some(key) = &session_key {
        if let Some(found) = manager.find_by_session_key(key) {
            if found.snapshot.worktree_path.as_deref() != Some(worktree_path.as_str()) {
                return Err(UsecaseError::Gateway(format!(
                    "PTY session {} not found for worktree {}",
                    key, worktree_path
                )));
            }
            manager.record_activity(found.snapshot.pty_id, manager.now_ms());
            manager.pin_session_key(&found.snapshot.session_key);
            return Ok(GetOrSpawnPtyResult {
                pty_id: found.snapshot.pty_id,
                session_key: found.snapshot.session_key,
                buffered_output: found.buffered_output,
                buffered_output_sequence: found.buffered_output_sequence,
                is_new: false,
                is_exited: found.snapshot.exited,
                exit_code: found.snapshot.exit_code,
                label: found.snapshot.label,
                kind: pty_kind_to_wire(found.snapshot.kind).to_string(),
            });
        }
    }

    let (pty_id, new_session_key) = spawn(
        manager,
        app,
        rows,
        cols,
        cwd,
        Some(worktree_path),
        label.clone(),
        kind,
    )?;

    Ok(GetOrSpawnPtyResult {
        pty_id,
        session_key: new_session_key,
        buffered_output: String::new(),
        buffered_output_sequence: 0,
        is_new: true,
        is_exited: false,
        exit_code: None,
        label,
        kind: pty_kind_to_wire(kind).to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    use super::*;
    use crate::domain::pty_session::entities::{
        PtySessionRegistry, PtySessionSnapshot, PtySpawnReservation, PtySpawnReservationError,
    };
    use crate::domain::pty_session::PtyLifecycleConfig;
    use crate::usecase::pty_session::dto::FoundPtySession;
    use crate::usecase::pty_session::ports::PtySessionReadGateway;

    struct MockGateway {
        registry: Mutex<PtySessionRegistry>,
        now_ms: AtomicU64,
        fail_spawn: AtomicBool,
        fail_start_reader: AtomicBool,
        pin_reserved_after_reserve: AtomicBool,
        spawn_count: Mutex<usize>,
        killed: Mutex<Vec<u64>>,
        emitted: Mutex<Vec<(u64, PtyEvictReason)>>,
    }

    impl MockGateway {
        fn new() -> Self {
            Self {
                registry: Mutex::new(PtySessionRegistry::with_config(PtyLifecycleConfig {
                    per_worktree_cap: 2,
                    max_panes_total: 3,
                    idle_timeout: Duration::from_millis(100),
                    output_buffer_cap: 64 * 1024,
                    sweep_interval: Duration::from_secs(60),
                })),
                now_ms: AtomicU64::new(200),
                fail_spawn: AtomicBool::new(false),
                fail_start_reader: AtomicBool::new(false),
                pin_reserved_after_reserve: AtomicBool::new(false),
                spawn_count: Mutex::new(0),
                killed: Mutex::new(Vec::new()),
                emitted: Mutex::new(Vec::new()),
            }
        }

        fn insert_session_with_activity(
            &self,
            session_key: &str,
            worktree_path: &str,
            activity_ms: u64,
        ) -> u64 {
            let mut registry = self.registry.lock().unwrap();
            let pty_id = registry.next_pty_id();
            registry.insert(PtySession::new(
                pty_id,
                session_key.to_string(),
                Some(worktree_path.to_string()),
                None,
                PtyKind::Terminal,
            ));
            registry.record_activity(pty_id, activity_ms);
            pty_id
        }

        fn is_pinned(&self, pty_id: u64) -> bool {
            self.registry.lock().unwrap().is_pinned(pty_id)
        }
    }

    impl PtySessionReadGateway for MockGateway {
        fn find_by_session_key(&self, session_key: &str) -> Option<FoundPtySession> {
            let snapshot = self
                .registry
                .lock()
                .unwrap()
                .find_by_session_key(session_key)
                .map(PtySession::snapshot)?;
            Some(FoundPtySession {
                snapshot,
                buffered_output: "buffered".to_string(),
                buffered_output_sequence: 9,
            })
        }

        fn list_snapshots(&self) -> Vec<PtySessionSnapshot> {
            self.registry.lock().unwrap().list_snapshots()
        }
    }

    impl PtySessionGateway for MockGateway {
        type AppContext = ();
        type Runtime = ();

        fn next_pty_id(&self) -> u64 {
            self.registry.lock().unwrap().next_pty_id()
        }

        fn spawn_backend(
            &self,
            _app: &Self::AppContext,
            _request: PtyBackendSpawnRequest,
        ) -> Result<Self::Runtime, UsecaseError> {
            *self.spawn_count.lock().unwrap() += 1;
            if self.fail_spawn.load(Ordering::SeqCst) {
                return Err(UsecaseError::Gateway("spawn failed".to_string()));
            }
            Ok(())
        }

        fn insert_session(&self, session: PtySession, _runtime: Self::Runtime) {
            self.registry.lock().unwrap().insert(session);
        }

        fn start_output_reader(
            &self,
            _app: &Self::AppContext,
            _pty_id: u64,
        ) -> Result<(), UsecaseError> {
            if self.fail_start_reader.load(Ordering::SeqCst) {
                return Err(UsecaseError::Gateway("reader failed".to_string()));
            }
            Ok(())
        }

        fn snapshot(&self, pty_id: u64) -> Option<PtySessionSnapshot> {
            self.registry
                .lock()
                .unwrap()
                .get(pty_id)
                .map(PtySession::snapshot)
        }

        fn select_kill_targets_by_worktree(&self, worktree_path: &str) -> Vec<u64> {
            self.registry
                .lock()
                .unwrap()
                .select_kill_targets_by_worktree(worktree_path)
        }

        fn select_gc_targets(&self, worktree_path: &str, keep_session_keys: &[String]) -> Vec<u64> {
            self.registry
                .lock()
                .unwrap()
                .select_gc_targets(worktree_path, keep_session_keys)
        }

        fn remove_session(&self, pty_id: u64) -> Option<PtySessionSnapshot> {
            self.registry
                .lock()
                .unwrap()
                .remove(pty_id)
                .map(|session| session.snapshot())
        }

        fn now_ms(&self) -> u64 {
            self.now_ms.load(Ordering::SeqCst)
        }

        fn record_activity(&self, pty_id: u64, now_ms: u64) -> bool {
            self.registry
                .lock()
                .unwrap()
                .record_activity(pty_id, now_ms)
        }

        fn pin_session_key(&self, session_key: &str) {
            self.registry.lock().unwrap().pin_session_key(session_key);
        }

        fn register_active_terminal(
            &self,
            worktree_path: &str,
            session_key: &str,
            active_token: &str,
        ) {
            self.registry.lock().unwrap().register_active_terminal(
                worktree_path,
                session_key,
                active_token,
            );
        }

        fn unregister_active_terminal(
            &self,
            worktree_path: &str,
            session_key: &str,
            active_token: &str,
        ) {
            self.registry.lock().unwrap().unregister_active_terminal(
                worktree_path,
                session_key,
                active_token,
            );
        }

        fn reserve_spawn_slot(
            &self,
            worktree_path: Option<&str>,
            now_ms: u64,
        ) -> Result<PtySpawnReservation, PtySpawnReservationError> {
            let mut registry = self.registry.lock().unwrap();
            let reservation = registry.reserve_spawn_slot(worktree_path, now_ms)?;
            if self.pin_reserved_after_reserve.load(Ordering::SeqCst) {
                for target in &reservation.evict_targets {
                    if let Some(session_key) = registry
                        .get(*target)
                        .map(|session| session.session_key.clone())
                    {
                        registry.pin_session_key(&session_key);
                    }
                }
            }
            Ok(reservation)
        }

        fn complete_spawn_slot(&self, reservation: &PtySpawnReservation) {
            self.registry
                .lock()
                .unwrap()
                .complete_spawn_slot(reservation);
        }

        fn rollback_spawn_slot(&self, reservation: &PtySpawnReservation) {
            self.registry
                .lock()
                .unwrap()
                .rollback_spawn_slot(reservation);
        }

        fn select_idle_timed_out(&self, now_ms: u64) -> Vec<u64> {
            self.registry.lock().unwrap().select_idle_timed_out(now_ms)
        }

        fn snapshot_if_idle_evictable(
            &self,
            pty_id: u64,
            now_ms: u64,
        ) -> Option<PtySessionSnapshot> {
            self.registry
                .lock()
                .unwrap()
                .snapshot_if_idle_evictable(pty_id, now_ms)
        }

        fn emit_evicted(
            &self,
            _app: &Self::AppContext,
            snapshot: &PtySessionSnapshot,
            reason: PtyEvictReason,
        ) {
            self.emitted.lock().unwrap().push((snapshot.pty_id, reason));
        }

        fn write(&self, _pty_id: u64, _data: &str) -> Result<(), UsecaseError> {
            Ok(())
        }

        fn resize(&self, _pty_id: u64, _rows: u16, _cols: u16) -> Result<(), UsecaseError> {
            Ok(())
        }

        fn get_pty_size(&self, _pty_id: u64) -> Result<(u16, u16), UsecaseError> {
            Ok((80, 24))
        }

        fn kill_runtime(&self, pty_id: u64) -> Result<(), UsecaseError> {
            self.killed.lock().unwrap().push(pty_id);
            Ok(())
        }

        fn remove_runtime(&self, _pty_id: u64) {}

        fn remove_if_exited(&self, pty_id: u64) {
            if self
                .snapshot(pty_id)
                .is_some_and(|snapshot| snapshot.exited)
            {
                self.remove_session(pty_id);
            }
        }
    }

    #[test]
    fn get_or_spawn_spawns_when_caps_are_not_reached() {
        let gateway = MockGateway::new();

        let result = get_or_spawn(
            &gateway,
            &(),
            24,
            80,
            Some("/repo".to_string()),
            None,
            "/repo".to_string(),
            Some("dev".to_string()),
            PtyKind::Terminal,
        )
        .unwrap();

        assert!(result.is_new);
        assert_eq!(*gateway.spawn_count.lock().unwrap(), 1);
        assert!(gateway.is_pinned(result.pty_id));
    }

    #[test]
    fn cap_reached_evicts_oldest_idle_worktree_session_before_spawn() {
        let gateway = MockGateway::new();
        let first = gateway.insert_session_with_activity("key-1", "/repo", 0);
        let second = gateway.insert_session_with_activity("key-2", "/repo", 50);

        let result = get_or_spawn(
            &gateway,
            &(),
            24,
            80,
            Some("/repo".to_string()),
            None,
            "/repo".to_string(),
            None,
            PtyKind::Terminal,
        )
        .unwrap();

        assert!(result.is_new);
        assert_eq!(*gateway.spawn_count.lock().unwrap(), 1);
        assert_eq!(*gateway.killed.lock().unwrap(), vec![first]);
        assert_eq!(
            *gateway.emitted.lock().unwrap(),
            vec![(first, PtyEvictReason::CapExceeded)]
        );
        assert!(gateway.snapshot(first).is_none());
        assert!(gateway.snapshot(second).is_some());
    }

    #[test]
    fn cap_reached_without_idle_candidate_returns_cap_reached() {
        let gateway = MockGateway::new();
        gateway.insert_session_with_activity("key-1", "/repo", 150);
        gateway.insert_session_with_activity("key-2", "/repo", 160);

        let result = get_or_spawn(
            &gateway,
            &(),
            24,
            80,
            Some("/repo".to_string()),
            None,
            "/repo".to_string(),
            None,
            PtyKind::Terminal,
        );

        assert!(matches!(result, Err(UsecaseError::CapReached(_))));
        assert_eq!(*gateway.spawn_count.lock().unwrap(), 0);
        assert!(gateway.killed.lock().unwrap().is_empty());
    }

    #[test]
    fn cap_reached_does_not_evict_reserved_target_that_became_pinned() {
        let gateway = MockGateway::new();
        let first = gateway.insert_session_with_activity("key-1", "/repo", 0);
        let second = gateway.insert_session_with_activity("key-2", "/repo", 50);
        gateway
            .pin_reserved_after_reserve
            .store(true, Ordering::SeqCst);

        let result = get_or_spawn(
            &gateway,
            &(),
            24,
            80,
            Some("/repo".to_string()),
            None,
            "/repo".to_string(),
            None,
            PtyKind::Terminal,
        );

        assert!(matches!(result, Err(UsecaseError::CapReached(_))));
        assert_eq!(*gateway.spawn_count.lock().unwrap(), 1);
        assert_eq!(*gateway.killed.lock().unwrap(), vec![3]);
        assert!(gateway.emitted.lock().unwrap().is_empty());
        assert!(gateway.snapshot(first).is_some());
        assert!(gateway.snapshot(second).is_some());
        assert!(gateway.is_pinned(first));
        assert!(gateway.snapshot(3).is_none());
    }

    #[test]
    fn spawn_failure_rolls_back_reserved_slot() {
        let gateway = MockGateway::new();
        gateway.fail_spawn.store(true, Ordering::SeqCst);

        let result = get_or_spawn(
            &gateway,
            &(),
            24,
            80,
            Some("/repo".to_string()),
            None,
            "/repo".to_string(),
            None,
            PtyKind::Terminal,
        );

        assert!(matches!(result, Err(UsecaseError::Gateway(_))));
        gateway.fail_spawn.store(false, Ordering::SeqCst);

        let retry = get_or_spawn(
            &gateway,
            &(),
            24,
            80,
            Some("/repo".to_string()),
            None,
            "/repo".to_string(),
            None,
            PtyKind::Terminal,
        )
        .unwrap();

        assert!(retry.is_new);
        assert_eq!(*gateway.spawn_count.lock().unwrap(), 2);
    }

    #[test]
    fn cap_reached_spawn_failure_preserves_reserved_evict_target() {
        let gateway = MockGateway::new();
        let first = gateway.insert_session_with_activity("key-1", "/repo", 0);
        let second = gateway.insert_session_with_activity("key-2", "/repo", 50);
        gateway.fail_spawn.store(true, Ordering::SeqCst);

        let result = get_or_spawn(
            &gateway,
            &(),
            24,
            80,
            Some("/repo".to_string()),
            None,
            "/repo".to_string(),
            None,
            PtyKind::Terminal,
        );

        assert!(matches!(result, Err(UsecaseError::Gateway(_))));
        assert_eq!(*gateway.spawn_count.lock().unwrap(), 1);
        assert!(gateway.killed.lock().unwrap().is_empty());
        assert!(gateway.emitted.lock().unwrap().is_empty());
        assert!(gateway.snapshot(first).is_some());
        assert!(gateway.snapshot(second).is_some());
    }

    #[test]
    fn start_reader_failure_cleans_up_new_session_and_preserves_evict_target() {
        let gateway = MockGateway::new();
        let first = gateway.insert_session_with_activity("key-1", "/repo", 0);
        let second = gateway.insert_session_with_activity("key-2", "/repo", 50);
        gateway.fail_start_reader.store(true, Ordering::SeqCst);

        let result = get_or_spawn(
            &gateway,
            &(),
            24,
            80,
            Some("/repo".to_string()),
            None,
            "/repo".to_string(),
            None,
            PtyKind::Terminal,
        );

        assert!(matches!(result, Err(UsecaseError::Gateway(_))));
        assert_eq!(*gateway.spawn_count.lock().unwrap(), 1);
        assert_eq!(*gateway.killed.lock().unwrap(), vec![3]);
        assert!(gateway.emitted.lock().unwrap().is_empty());
        assert!(gateway.snapshot(first).is_some());
        assert!(gateway.snapshot(second).is_some());
        assert!(gateway.snapshot(3).is_none());
    }

    #[test]
    fn existing_session_key_replays_buffer_and_does_not_spawn() {
        let gateway = MockGateway::new();
        let pty_id = gateway.insert_session_with_activity("key-1", "/repo", 0);

        let result = get_or_spawn(
            &gateway,
            &(),
            24,
            80,
            Some("/repo".to_string()),
            Some("key-1".to_string()),
            "/repo".to_string(),
            None,
            PtyKind::Terminal,
        )
        .unwrap();

        assert!(!result.is_new);
        assert_eq!(result.pty_id, pty_id);
        assert_eq!(result.buffered_output, "buffered");
        assert_eq!(result.buffered_output_sequence, 9);
        assert_eq!(*gateway.spawn_count.lock().unwrap(), 0);
        assert!(gateway.is_pinned(pty_id));
    }

    #[test]
    fn existing_session_key_for_other_worktree_is_rejected_without_replay_or_spawn() {
        let gateway = MockGateway::new();
        let pty_id = gateway.insert_session_with_activity("key-1", "/repo", 0);

        let result = get_or_spawn(
            &gateway,
            &(),
            24,
            80,
            Some("/other".to_string()),
            Some("key-1".to_string()),
            "/other".to_string(),
            None,
            PtyKind::Terminal,
        );

        assert!(matches!(result, Err(UsecaseError::Gateway(_))));
        assert_eq!(*gateway.spawn_count.lock().unwrap(), 0);
        assert!(!gateway.is_pinned(pty_id));
    }
}
