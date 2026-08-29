use crate::domain::terminal_surface::gateway::TerminalSurfaceGatewayError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UsecaseError {
    #[error("{0}")]
    Gateway(String),
    #[error("Terminal Surface owner identity collision")]
    OwnerConflict,
    #[error("{error}")]
    PtySpawn { error: String },
    #[error("{error}")]
    OtherSpawnFailure { error: String },
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
