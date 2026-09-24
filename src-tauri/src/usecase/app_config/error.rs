use crate::domain::app_config::AppConfigError;

#[derive(Debug, thiserror::Error)]
pub enum UsecaseError {
    #[error("{0}")]
    InvalidInput(String),
    #[error(transparent)]
    AppConfig(#[from] AppConfigError),
}

impl From<UsecaseError> for String {
    fn from(value: UsecaseError) -> Self {
        value.to_string()
    }
}

impl crate::domain::failure::ClassifiedFailure for UsecaseError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind as F;
        match self {
            Self::AppConfig(error) => error.failure_kind(),
            Self::InvalidInput(_) => F::InvalidInput,
        }
    }
}

#[cfg(test)]
#[path = "error_test.rs"]
mod error_tests;
