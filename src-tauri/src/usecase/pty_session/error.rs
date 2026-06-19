#[derive(Debug, thiserror::Error)]
pub enum UsecaseError {
    #[error("{0}")]
    Gateway(String),
}

impl From<String> for UsecaseError {
    fn from(value: String) -> Self {
        Self::Gateway(value)
    }
}
