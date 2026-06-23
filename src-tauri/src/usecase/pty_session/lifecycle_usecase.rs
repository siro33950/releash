use crate::domain::pty_session::entities::PtySessionSnapshot;
use crate::domain::pty_session::PtyEvictReason;
use crate::usecase::pty_session::error::UsecaseError;
use crate::usecase::pty_session::ports::PtySessionGateway;

pub fn kill(manager: &impl PtySessionGateway, pty_id: u64) -> Result<(), UsecaseError> {
    remove_and_kill(manager, pty_id)
}

pub fn evict<G: PtySessionGateway>(
    manager: &G,
    app: &G::AppContext,
    pty_id: u64,
    reason: PtyEvictReason,
    now_ms: u64,
) -> Result<Option<PtySessionSnapshot>, UsecaseError> {
    let Some(snapshot) = manager.snapshot_if_idle_evictable(pty_id, now_ms) else {
        return Ok(None);
    };
    if !snapshot.exited {
        manager.kill_runtime(pty_id)?;
    }
    manager.remove_session(pty_id);
    manager.emit_evicted(app, &snapshot, reason);
    Ok(Some(snapshot))
}

fn remove_and_kill(manager: &impl PtySessionGateway, pty_id: u64) -> Result<(), UsecaseError> {
    let snapshot = manager
        .snapshot(pty_id)
        .ok_or_else(|| UsecaseError::Gateway(format!("PTY {} not found", pty_id)))?;
    if !snapshot.exited {
        manager.kill_runtime(pty_id)?;
    }
    manager.remove_session(pty_id);
    Ok(())
}

pub fn kill_by_worktree(manager: &impl PtySessionGateway, worktree_path: &str) -> Vec<u64> {
    let targets = manager.select_kill_targets_by_worktree(worktree_path);
    kill_targets(manager, targets)
}

pub fn gc_by_worktree(
    manager: &impl PtySessionGateway,
    worktree_path: &str,
    keep_session_keys: &[String],
) -> Vec<u64> {
    let targets = manager.select_gc_targets(worktree_path, keep_session_keys);
    kill_targets(manager, targets)
}

pub fn remove_if_exited(manager: &impl PtySessionGateway, pty_id: u64) {
    manager.remove_if_exited(pty_id);
}

pub fn register_active_terminal(
    manager: &impl PtySessionGateway,
    worktree_path: &str,
    session_key: &str,
    active_token: &str,
) {
    manager.register_active_terminal(worktree_path, session_key, active_token);
}

pub fn unregister_active_terminal(
    manager: &impl PtySessionGateway,
    worktree_path: &str,
    session_key: &str,
    active_token: &str,
) {
    manager.unregister_active_terminal(worktree_path, session_key, active_token);
}

pub fn sweep_idle<G: PtySessionGateway>(manager: &G, app: &G::AppContext, now_ms: u64) -> Vec<u64> {
    let targets = manager.select_idle_timed_out(now_ms);
    let mut evicted = Vec::with_capacity(targets.len());
    for pty_id in targets {
        match evict(manager, app, pty_id, PtyEvictReason::Idle, now_ms) {
            Ok(Some(_)) => evicted.push(pty_id),
            Ok(None) => {}
            Err(e) => log::error!("Failed to evict idle PTY {}: {}", pty_id, e),
        }
    }
    evicted
}

fn kill_targets(manager: &impl PtySessionGateway, targets: Vec<u64>) -> Vec<u64> {
    let mut killed = Vec::with_capacity(targets.len());
    for pty_id in targets {
        match remove_and_kill(manager, pty_id) {
            Ok(()) => killed.push(pty_id),
            Err(e) => log::error!("Failed to kill PTY {}: {}", pty_id, e),
        }
    }
    killed
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::domain::pty_session::entities::{
        PtySession, PtySessionSnapshot, PtySpawnReservation, PtySpawnReservationError,
    };
    use crate::domain::pty_session::PtyKind;
    use crate::usecase::pty_session::dto::FoundPtySession;
    use crate::usecase::pty_session::ports::{PtyBackendSpawnRequest, PtySessionReadGateway};

    struct MockGateway {
        snapshots: Mutex<Vec<PtySessionSnapshot>>,
        idle_targets: Vec<u64>,
        idle_evictable: Mutex<bool>,
        fail_kill: Mutex<bool>,
        killed: Mutex<Vec<u64>>,
        removed: Mutex<Vec<u64>>,
        emitted: Mutex<Vec<(u64, PtyEvictReason)>>,
    }

    impl MockGateway {
        fn new(snapshot: PtySessionSnapshot) -> Self {
            Self {
                snapshots: Mutex::new(vec![snapshot]),
                idle_targets: vec![1],
                idle_evictable: Mutex::new(true),
                fail_kill: Mutex::new(false),
                killed: Mutex::new(Vec::new()),
                removed: Mutex::new(Vec::new()),
                emitted: Mutex::new(Vec::new()),
            }
        }
    }

    impl PtySessionReadGateway for MockGateway {
        fn find_by_session_key(&self, _session_key: &str) -> Option<FoundPtySession> {
            None
        }

        fn list_snapshots(&self) -> Vec<PtySessionSnapshot> {
            self.snapshots.lock().unwrap().clone()
        }
    }

    impl PtySessionGateway for MockGateway {
        type AppContext = ();
        type Runtime = ();

        fn next_pty_id(&self) -> u64 {
            1
        }

        fn spawn_backend(
            &self,
            _app: &Self::AppContext,
            _request: PtyBackendSpawnRequest,
        ) -> Result<Self::Runtime, UsecaseError> {
            Ok(())
        }

        fn insert_session(&self, _session: PtySession, _runtime: Self::Runtime) {}

        fn start_output_reader(
            &self,
            _app: &Self::AppContext,
            _pty_id: u64,
        ) -> Result<(), UsecaseError> {
            Ok(())
        }

        fn snapshot(&self, pty_id: u64) -> Option<PtySessionSnapshot> {
            self.snapshots
                .lock()
                .unwrap()
                .iter()
                .find(|snapshot| snapshot.pty_id == pty_id)
                .cloned()
        }

        fn select_kill_targets_by_worktree(&self, _worktree_path: &str) -> Vec<u64> {
            Vec::new()
        }

        fn select_gc_targets(
            &self,
            _worktree_path: &str,
            _keep_session_keys: &[String],
        ) -> Vec<u64> {
            Vec::new()
        }

        fn remove_session(&self, pty_id: u64) -> Option<PtySessionSnapshot> {
            self.removed.lock().unwrap().push(pty_id);
            let mut snapshots = self.snapshots.lock().unwrap();
            let index = snapshots
                .iter()
                .position(|snapshot| snapshot.pty_id == pty_id)?;
            Some(snapshots.remove(index))
        }

        fn now_ms(&self) -> u64 {
            200
        }

        fn record_activity(&self, _pty_id: u64, _now_ms: u64) -> bool {
            true
        }

        fn pin_session_key(&self, _session_key: &str) {}

        fn register_active_terminal(
            &self,
            _worktree_path: &str,
            _session_key: &str,
            _active_token: &str,
        ) {
        }

        fn unregister_active_terminal(
            &self,
            _worktree_path: &str,
            _session_key: &str,
            _active_token: &str,
        ) {
        }

        fn reserve_spawn_slot(
            &self,
            _worktree_path: Option<&str>,
            _now_ms: u64,
        ) -> Result<PtySpawnReservation, PtySpawnReservationError> {
            Ok(PtySpawnReservation {
                worktree_path: None,
                evict_targets: Vec::new(),
            })
        }

        fn complete_spawn_slot(&self, _reservation: &PtySpawnReservation) {}

        fn rollback_spawn_slot(&self, _reservation: &PtySpawnReservation) {}

        fn select_idle_timed_out(&self, _now_ms: u64) -> Vec<u64> {
            *self.idle_evictable.lock().unwrap() = false;
            self.idle_targets.clone()
        }

        fn snapshot_if_idle_evictable(
            &self,
            pty_id: u64,
            _now_ms: u64,
        ) -> Option<PtySessionSnapshot> {
            if !*self.idle_evictable.lock().unwrap() {
                return None;
            }
            self.snapshot(pty_id)
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
            if *self.fail_kill.lock().unwrap() {
                return Err(UsecaseError::Gateway("kill failed".to_string()));
            }
            Ok(())
        }

        fn remove_runtime(&self, _pty_id: u64) {}

        fn remove_if_exited(&self, _pty_id: u64) {}
    }

    fn snapshot() -> PtySessionSnapshot {
        PtySessionSnapshot {
            pty_id: 1,
            session_key: "key-1".to_string(),
            worktree_path: Some("/repo".to_string()),
            label: None,
            kind: PtyKind::Terminal,
            exited: false,
            exit_code: None,
        }
    }

    #[test]
    fn sweep_idle_revalidates_target_before_kill() {
        let gateway = MockGateway::new(snapshot());

        let evicted = sweep_idle(&gateway, &(), 200);

        assert!(evicted.is_empty());
        assert!(gateway.killed.lock().unwrap().is_empty());
        assert!(gateway.removed.lock().unwrap().is_empty());
        assert!(gateway.emitted.lock().unwrap().is_empty());
        assert!(gateway.snapshot(1).is_some());
    }

    #[test]
    fn evict_removes_idle_target_then_kills_runtime_and_emits() {
        let gateway = MockGateway::new(snapshot());

        let removed = evict(&gateway, &(), 1, PtyEvictReason::Idle, 200)
            .unwrap()
            .unwrap();

        assert_eq!(removed.pty_id, 1);
        assert_eq!(*gateway.removed.lock().unwrap(), vec![1]);
        assert_eq!(*gateway.killed.lock().unwrap(), vec![1]);
        assert_eq!(
            *gateway.emitted.lock().unwrap(),
            vec![(1, PtyEvictReason::Idle)]
        );
        assert!(gateway.snapshot(1).is_none());
    }

    #[test]
    fn evict_keeps_handles_when_kill_runtime_fails() {
        let gateway = MockGateway::new(snapshot());
        *gateway.fail_kill.lock().unwrap() = true;

        let result = evict(&gateway, &(), 1, PtyEvictReason::Idle, 200);

        assert!(matches!(result, Err(UsecaseError::Gateway(_))));
        assert_eq!(*gateway.killed.lock().unwrap(), vec![1]);
        assert!(gateway.removed.lock().unwrap().is_empty());
        assert!(gateway.emitted.lock().unwrap().is_empty());
        assert!(gateway.snapshot(1).is_some());

        *gateway.fail_kill.lock().unwrap() = false;
        let retry = evict(&gateway, &(), 1, PtyEvictReason::Idle, 200)
            .unwrap()
            .unwrap();

        assert_eq!(retry.pty_id, 1);
        assert_eq!(*gateway.removed.lock().unwrap(), vec![1]);
        assert_eq!(
            *gateway.emitted.lock().unwrap(),
            vec![(1, PtyEvictReason::Idle)]
        );
    }
}
