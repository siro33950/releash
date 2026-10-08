use super as wire;
use crate::adaptor::presenter::connect::ConnectFailure;
impl From<crate::adaptor::presenter::error::AppError> for wire::CommandError {
    fn from(value: crate::adaptor::presenter::error::AppError) -> Self {
        match value {
            crate::adaptor::presenter::error::AppError::Presented { error, .. } => (*error).into(),
            crate::adaptor::presenter::error::AppError::Internal(value) => Self {
                variant: Some(wire::command_error::Variant::Message(wire::ResultString {
                    value: Some(value),
                })),
            },
            crate::adaptor::presenter::error::AppError::Coded { code, message, .. } => Self {
                variant: Some(wire::command_error::Variant::Coded(wire::CodedError {
                    code: Some(code),
                    message: Some(message),
                })),
            },
        }
    }
}
impl From<crate::adaptor::controller::terminal_surface::TerminalCommandError>
    for wire::CommandError
{
    fn from(value: crate::adaptor::controller::terminal_surface::TerminalCommandError) -> Self {
        Self {
            variant: Some(wire::command_error::Variant::Coded(wire::CodedError {
                code: Some(value.code),
                message: Some(value.message),
            })),
        }
    }
}

#[cfg(test)]
#[path = "errors_test.rs"]
mod errors_tests;

#[derive(Debug)]
pub struct CommandFailure {
    pub(crate) kind: connectrpc::ErrorCode,
    pub(crate) message: Option<String>,
    pub detail: wire::CommandError,
}
impl From<crate::adaptor::presenter::error::AppError> for CommandFailure {
    fn from(error: crate::adaptor::presenter::error::AppError) -> Self {
        Self {
            kind: error.connect_code(),
            message: error.cause().map(str::to_string),
            detail: error.into(),
        }
    }
}
impl From<crate::adaptor::controller::terminal_surface::TerminalCommandError> for CommandFailure {
    fn from(error: crate::adaptor::controller::terminal_surface::TerminalCommandError) -> Self {
        Self {
            kind: error.kind,
            message: error.cause.clone(),
            detail: error.into(),
        }
    }
}
#[cfg(any(test, feature = "test-support"))]
impl wire::ClientValue for CommandFailure {
    fn into_json(self) -> Result<serde_json::Value, String> {
        self.detail.into_json()
    }
}

impl From<crate::domain::external_editor::EditorError> for CommandFailure {
    fn from(error: crate::domain::external_editor::EditorError) -> Self {
        crate::adaptor::presenter::error::AppError::from_failure(error).into()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl From<wire::CommandError> for CommandFailure {
    fn from(detail: wire::CommandError) -> Self {
        Self {
            message: None,
            kind: connectrpc::ErrorCode::Internal,
            detail,
        }
    }
}
