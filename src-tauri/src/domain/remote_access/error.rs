#[derive(Debug, thiserror::Error)]
pub enum RemoteAccessError {
    #[error("{0}")]
    Message(String),
}
