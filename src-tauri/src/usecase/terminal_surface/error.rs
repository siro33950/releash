use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceGatewayError, TerminalSurfaceInputUnavailableCause,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UsecaseError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    InvalidOperation(String),
    #[error("{0}")]
    Technical(crate::domain::failure::TechnicalFailure),
    #[error("Terminal input attachment is no longer active")]
    StaleAttachment,
    #[error("Terminal Surface owner identity collision")]
    OwnerConflict,
}

impl UsecaseError {
    pub(crate) fn technical_failure(&self) -> Option<&crate::domain::failure::TechnicalFailure> {
        match self {
            Self::Technical(failure) => Some(failure),
            Self::NotFound(_)
            | Self::InvalidOperation(_)
            | Self::StaleAttachment
            | Self::OwnerConflict => None,
        }
    }
}

impl From<TerminalSurfaceGatewayError> for UsecaseError {
    fn from(value: TerminalSurfaceGatewayError) -> Self {
        match value {
            TerminalSurfaceGatewayError::NotFound(message) => Self::NotFound(message),
            TerminalSurfaceGatewayError::InvalidOperation(message) => {
                Self::InvalidOperation(message)
            }
            TerminalSurfaceGatewayError::Technical(failure) => Self::Technical(failure),
            TerminalSurfaceGatewayError::InputUnavailable(
                TerminalSurfaceInputUnavailableCause::StaleAttachment,
            ) => Self::StaleAttachment,
            TerminalSurfaceGatewayError::InputUnavailable(cause) => {
                Self::InvalidOperation(cause.internal_cause().into())
            }
        }
    }
}

#[cfg(test)]
#[path = "error_test.rs"]
mod error_tests;
