#[derive(Debug)]
pub enum WorkspaceStateError {
    Message(String),
}

impl std::fmt::Display for WorkspaceStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for WorkspaceStateError {}
