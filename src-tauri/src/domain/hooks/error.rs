#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum HooksError {
    #[error("{0}")]
    Message(String),
}
