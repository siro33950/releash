use crate::domain::pty_session::{entities::PtySession, entities::PtySessionSnapshot};
use crate::usecase::pty_session::dto::FoundPtySession;
use crate::usecase::pty_session::error::UsecaseError;

pub struct PtyBackendSpawnRequest {
    pub pty_id: u64,
    pub rows: u16,
    pub cols: u16,
    pub cwd: Option<String>,
    pub exec_command: Option<String>,
}

pub(crate) trait PtySessionGateway {
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
    fn find_by_session_key(&self, session_key: &str) -> Option<FoundPtySession>;
    fn snapshot(&self, pty_id: u64) -> Option<PtySessionSnapshot>;
    fn list_snapshots(&self) -> Vec<PtySessionSnapshot>;
    fn select_kill_targets_by_worktree(&self, worktree_path: &str) -> Vec<u64>;
    fn select_gc_targets(&self, worktree_path: &str, keep_session_keys: &[String]) -> Vec<u64>;
    fn remove_session(&self, pty_id: u64) -> Option<PtySessionSnapshot>;

    fn write(&self, pty_id: u64, data: &str) -> Result<(), UsecaseError>;
    fn resize(&self, pty_id: u64, rows: u16, cols: u16) -> Result<(), UsecaseError>;
    #[allow(dead_code)]
    fn get_pty_size(&self, pty_id: u64) -> Result<(u16, u16), UsecaseError>;
    fn kill_runtime(&self, pty_id: u64) -> Result<(), UsecaseError>;
    fn remove_if_exited(&self, pty_id: u64);
}
