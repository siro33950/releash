#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum NotificationError {
    #[error("{0}")]
    Message(String),
}
