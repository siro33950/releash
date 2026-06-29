use crate::domain::pty_session::{
    entities::PtySession, entities::PtySessionSnapshot, entities::PtySpawnReservation,
    entities::PtySpawnReservationError, PtyEvictReason,
};
use crate::usecase::pty_session::dto::FoundPtySession;
use crate::usecase::pty_session::error::UsecaseError;

pub struct PtyBackendSpawnRequest {
    pub pty_id: u64,
    pub rows: u16,
    pub cols: u16,
    pub cwd: Option<String>,
    pub exec_command: Option<String>,
}

pub(crate) trait PtySessionReadGateway {
    fn find_by_session_key(&self, session_key: &str) -> Option<FoundPtySession>;
    fn list_snapshots(&self) -> Vec<PtySessionSnapshot>;
}

pub(crate) trait PtySessionGateway: PtySessionReadGateway {
    type AppContext: ?Sized;
    type Runtime;

    fn next_pty_id(&self) -> u64;
    fn spawn_backend(
        &self,
        app: &Self::AppContext,
        request: PtyBackendSpawnRequest,
    ) -> Result<Self::Runtime, UsecaseError>;
    fn insert_session(&self, session: PtySession, runtime: Self::Runtime);
    fn start_output_reader(&self, app: &Self::AppContext, pty_id: u64) -> Result<(), UsecaseError>;
    fn snapshot(&self, pty_id: u64) -> Option<PtySessionSnapshot>;
    fn select_kill_targets_by_worktree(&self, worktree_path: &str) -> Vec<u64>;
    fn select_gc_targets(&self, worktree_path: &str, keep_session_keys: &[String]) -> Vec<u64>;
    fn remove_session(&self, pty_id: u64) -> Option<PtySessionSnapshot>;
    fn now_ms(&self) -> u64;
    fn record_activity(&self, pty_id: u64, now_ms: u64) -> bool;
    fn pin_session_key(&self, session_key: &str);
    fn unpin_session_key_if_unused(&self, session_key: &str);
    fn register_active_terminal(&self, worktree_path: &str, session_key: &str, active_token: &str);
    fn unregister_active_terminal(
        &self,
        worktree_path: &str,
        session_key: &str,
        active_token: &str,
    );
    fn reserve_spawn_slot(
        &self,
        worktree_path: Option<&str>,
        now_ms: u64,
    ) -> Result<PtySpawnReservation, PtySpawnReservationError>;
    fn complete_spawn_slot(&self, reservation: &PtySpawnReservation);
    fn rollback_spawn_slot(&self, reservation: &PtySpawnReservation);
    fn select_idle_timed_out(&self, now_ms: u64) -> Vec<u64>;
    fn snapshot_if_idle_evictable(&self, pty_id: u64, now_ms: u64) -> Option<PtySessionSnapshot>;
    fn emit_evicted(
        &self,
        app: &Self::AppContext,
        snapshot: &PtySessionSnapshot,
        reason: PtyEvictReason,
    );

    fn write(&self, pty_id: u64, data: &str) -> Result<(), UsecaseError>;
    fn resize(&self, pty_id: u64, rows: u16, cols: u16) -> Result<(), UsecaseError>;
    fn kill_runtime(&self, pty_id: u64) -> Result<(), UsecaseError>;
    fn remove_runtime(&self, pty_id: u64);
    fn remove_if_exited(&self, pty_id: u64);
}
