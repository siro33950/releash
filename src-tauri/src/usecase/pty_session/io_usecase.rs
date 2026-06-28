use crate::usecase::pty_session::error::UsecaseError;
use crate::usecase::pty_session::ports::PtySessionGateway;

use crate::domain::shell::join_quoted_paths;

pub fn write(
    manager: &impl PtySessionGateway,
    pty_id: u64,
    data: &str,
) -> Result<(), UsecaseError> {
    manager.write(pty_id, data)
}

pub fn write_paths(
    manager: &impl PtySessionGateway,
    pty_id: u64,
    paths: &[String],
) -> Result<(), UsecaseError> {
    if paths.is_empty() {
        return Ok(());
    }
    let data = join_quoted_paths(paths);
    manager.write(pty_id, &data)
}

pub fn resize(
    manager: &impl PtySessionGateway,
    pty_id: u64,
    rows: u16,
    cols: u16,
) -> Result<(), UsecaseError> {
    manager.resize(pty_id, rows, cols)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pty_session::entities::{
        PtySession, PtySessionSnapshot, PtySpawnReservation, PtySpawnReservationError,
    };
    use crate::domain::pty_session::PtyEvictReason;
    use crate::usecase::pty_session::dto::FoundPtySession;
    use crate::usecase::pty_session::ports::{PtyBackendSpawnRequest, PtySessionReadGateway};
    use parking_lot::Mutex;

    struct FakePtyGateway {
        writes: Mutex<Vec<(u64, String)>>,
    }

    impl FakePtyGateway {
        fn new() -> Self {
            Self {
                writes: Mutex::new(Vec::new()),
            }
        }
    }

    impl PtySessionReadGateway for FakePtyGateway {
        fn find_by_session_key(&self, _session_key: &str) -> Option<FoundPtySession> {
            None
        }

        fn list_snapshots(&self) -> Vec<PtySessionSnapshot> {
            Vec::new()
        }
    }

    impl PtySessionGateway for FakePtyGateway {
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

        fn snapshot(&self, _pty_id: u64) -> Option<PtySessionSnapshot> {
            None
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

        fn remove_session(&self, _pty_id: u64) -> Option<PtySessionSnapshot> {
            None
        }

        fn now_ms(&self) -> u64 {
            0
        }

        fn record_activity(&self, _pty_id: u64, _now_ms: u64) -> bool {
            false
        }

        fn pin_session_key(&self, _session_key: &str) {}

        fn unpin_session_key_if_unused(&self, _session_key: &str) {}

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
            Vec::new()
        }

        fn snapshot_if_idle_evictable(
            &self,
            _pty_id: u64,
            _now_ms: u64,
        ) -> Option<PtySessionSnapshot> {
            None
        }

        fn emit_evicted(
            &self,
            _app: &Self::AppContext,
            _snapshot: &PtySessionSnapshot,
            _reason: PtyEvictReason,
        ) {
        }

        fn write(&self, pty_id: u64, data: &str) -> Result<(), UsecaseError> {
            self.writes.lock().push((pty_id, data.to_string()));
            Ok(())
        }

        fn resize(&self, _pty_id: u64, _rows: u16, _cols: u16) -> Result<(), UsecaseError> {
            Ok(())
        }

        fn get_pty_size(&self, _pty_id: u64) -> Result<(u16, u16), UsecaseError> {
            Ok((24, 80))
        }

        fn kill_runtime(&self, _pty_id: u64) -> Result<(), UsecaseError> {
            Ok(())
        }

        fn remove_runtime(&self, _pty_id: u64) {}

        fn remove_if_exited(&self, _pty_id: u64) {}
    }

    #[test]
    fn write_paths_quotes_and_joins_before_writing() {
        let gateway = FakePtyGateway::new();

        write_paths(
            &gateway,
            42,
            &[
                "/tmp/a.txt".to_string(),
                "/tmp/my file.txt".to_string(),
                "/tmp/it's.txt".to_string(),
            ],
        )
        .unwrap();

        assert_eq!(
            *gateway.writes.lock(),
            vec![(
                42,
                "/tmp/a.txt '/tmp/my file.txt' '/tmp/it'\\''s.txt'".to_string()
            )]
        );
    }

    #[test]
    fn write_paths_empty_is_noop() {
        let gateway = FakePtyGateway::new();

        write_paths(&gateway, 42, &[]).unwrap();

        assert!(gateway.writes.lock().is_empty());
    }
}
