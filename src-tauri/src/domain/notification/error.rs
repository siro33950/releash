#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    #[error("{0}")]
    Message(String),
}
