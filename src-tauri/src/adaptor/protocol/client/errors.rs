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
impl From<crate::usecase::application_startup::ApplicationUnavailable> for wire::CommandError {
    fn from(value: crate::usecase::application_startup::ApplicationUnavailable) -> Self {
        Self { variant: Some(wire::command_error::Variant::Application(Box::new(match value { crate::usecase::application_startup::ApplicationUnavailable::ApplicationUnavailable => wire::ApplicationError { r#type: Some("application_unavailable".into()), message: None, correlation_id: None } }))) }
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
pub(crate) struct CommandFailure {
    pub(crate) kind: connectrpc::ErrorCode,
    pub(crate) detail: wire::CommandError,
}
impl From<crate::adaptor::presenter::error::AppError> for CommandFailure {
    fn from(error: crate::adaptor::presenter::error::AppError) -> Self {
        Self {
            kind: error.connect_code(),
            detail: error.into(),
        }
    }
}
impl From<crate::adaptor::controller::terminal_surface::TerminalCommandError> for CommandFailure {
    fn from(error: crate::adaptor::controller::terminal_surface::TerminalCommandError) -> Self {
        Self {
            kind: error.kind,
            detail: error.into(),
        }
    }
}
#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
impl wire::ClientValue for CommandFailure {
    fn into_json(self) -> Result<serde_json::Value, String> {
        self.detail.into_json()
    }
}

impl From<crate::usecase::application_startup::ApplicationUnavailable> for CommandFailure {
    fn from(error: crate::usecase::application_startup::ApplicationUnavailable) -> Self {
        Self {
            kind: error.connect_code(),
            detail: error.into(),
        }
    }
}

impl From<crate::domain::external_editor::EditorError> for CommandFailure {
    fn from(error: crate::domain::external_editor::EditorError) -> Self {
        crate::adaptor::presenter::error::AppError::from_failure(error).into()
    }
}
