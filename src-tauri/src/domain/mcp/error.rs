#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum McpError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Gateway(String),
}
