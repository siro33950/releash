#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum AppConfigError {
    #[error("{0}")]
    Repository(String),
    #[error("{0}")]
    InvalidInput(String),
}
