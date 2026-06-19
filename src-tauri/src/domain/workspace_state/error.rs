#[derive(Debug, thiserror::Error)]
pub enum WorkspaceStateError {
    #[error("{0}")]
    Message(String),
}
