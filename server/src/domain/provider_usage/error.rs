#[derive(Debug)]
pub enum UsageError {
    Invalid,
    Technical(crate::domain::failure::StorageFailure),
}
impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid => f.write_str("Invalid provider usage"),
            Self::Technical(error) => write!(f, "Provider usage source failed: {error}"),
        }
    }
}
impl std::error::Error for UsageError {}
impl From<crate::domain::failure::TechnicalFailure> for UsageError {
    fn from(error: crate::domain::failure::TechnicalFailure) -> Self {
        Self::Technical(crate::domain::failure::StorageFailure {
            nature: error.nature,
            source: crate::domain::failure::StorageFailureSource::Technical(error),
            context: None,
        })
    }
}
