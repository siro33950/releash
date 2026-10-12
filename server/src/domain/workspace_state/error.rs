#[derive(Debug)]
pub enum WorkspaceStateError {
    InvalidLayout,
    VersionConflict,
    Technical(crate::domain::failure::TechnicalFailure),
}

impl std::fmt::Display for WorkspaceStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLayout => f.write_str("Invalid pane layout"),
            Self::VersionConflict => f.write_str("Workspace state version conflict"),
            Self::Technical(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for WorkspaceStateError {}
