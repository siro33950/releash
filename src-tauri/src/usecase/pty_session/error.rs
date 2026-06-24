use crate::domain::pty_session::entities::PtySpawnReservationError;

#[derive(Debug, thiserror::Error)]
pub enum UsecaseError {
    #[error("{0}")]
    Gateway(String),
    #[error("{0}")]
    CapReached(String),
}

impl From<String> for UsecaseError {
    fn from(value: String) -> Self {
        Self::Gateway(value)
    }
}

impl From<PtySpawnReservationError> for UsecaseError {
    fn from(value: PtySpawnReservationError) -> Self {
        match value {
            PtySpawnReservationError::WorktreeCapReached(worktree_path) => {
                Self::CapReached(format!("PTY cap reached for worktree {worktree_path}"))
            }
            PtySpawnReservationError::TotalCapReached => {
                Self::CapReached("PTY total cap reached".to_string())
            }
        }
    }
}
