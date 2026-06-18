#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum RemoteAccessError {
    #[error("{0}")]
    Message(String),
}
