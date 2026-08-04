use crate::domain::terminal_surface::entities::TerminalSurfaceSpawnReservationError;
use crate::domain::terminal_surface::gateway::TerminalSurfaceGatewayError;

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

impl From<TerminalSurfaceGatewayError> for UsecaseError {
    fn from(value: TerminalSurfaceGatewayError) -> Self {
        Self::Gateway(value.message().to_string())
    }
}

impl From<TerminalSurfaceSpawnReservationError> for UsecaseError {
    fn from(value: TerminalSurfaceSpawnReservationError) -> Self {
        match value {
            TerminalSurfaceSpawnReservationError::OwnerOccupied(session_key) => Self::Gateway(
                format!("Terminal Surface owner is already being created: {session_key}"),
            ),
            TerminalSurfaceSpawnReservationError::WorktreeCapReached(worktree_path) => {
                Self::CapReached(format!("PTY cap reached for worktree {worktree_path}"))
            }
            TerminalSurfaceSpawnReservationError::TotalCapReached => {
                Self::CapReached("PTY total cap reached".to_string())
            }
        }
    }
}
