#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum ExternalEditorError {
    #[error("{0}")]
    Message(String),
}
