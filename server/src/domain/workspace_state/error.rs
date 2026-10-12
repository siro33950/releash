#[derive(Debug)]
pub enum WorkspaceStateError {
    Message(String),
    VersionConflict,
    Technical(crate::domain::failure::TechnicalFailure),
}

impl std::fmt::Display for WorkspaceStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(msg) => f.write_str(msg),
            Self::VersionConflict => f.write_str("Workspace state version conflict"),
            Self::Technical(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for WorkspaceStateError {}
