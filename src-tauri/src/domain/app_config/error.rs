#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppConfigError {
    Repository(String),
    InvalidInput(String),
}

impl std::fmt::Display for AppConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Repository(msg) | Self::InvalidInput(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for AppConfigError {}

impl crate::domain::failure::ClassifiedFailure for AppConfigError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind as F;
        match self {
            Self::Repository(_) => F::Internal,
            Self::InvalidInput(_) => F::InvalidInput,
        }
    }
}

#[cfg(test)]
#[path = "error_test.rs"]
mod error_tests;
