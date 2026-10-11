#[derive(Debug)]
pub enum WorkspaceStateError {
    Message(String),
    VersionConflict,
}

impl std::fmt::Display for WorkspaceStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(msg) => f.write_str(msg),
            Self::VersionConflict => f.write_str("Workspace state version conflict"),
        }
    }
}

impl std::error::Error for WorkspaceStateError {}
