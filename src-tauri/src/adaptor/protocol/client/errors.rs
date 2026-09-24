use super as wire;
impl From<crate::other::AppError> for wire::CommandError {
    fn from(value: crate::other::AppError) -> Self {
        match value {
            crate::other::AppError::Classified { error, .. } => (*error).into(),
            crate::other::AppError::Internal(value) => Self {
                variant: Some(wire::command_error::Variant::Message(wire::ResultString {
                    value: Some(value),
                })),
            },
            crate::other::AppError::Coded { code, message, .. } => Self {
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
    pub(crate) kind: crate::domain::failure::FailureKind,
    pub(crate) detail: wire::CommandError,
}
impl From<crate::other::AppError> for CommandFailure {
    fn from(error: crate::other::AppError) -> Self {
        use crate::domain::failure::ClassifiedFailure;
        Self {
            kind: error.failure_kind(),
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
            kind: crate::domain::failure::FailureKind::StateRequired,
            detail: error.into(),
        }
    }
}

impl From<crate::domain::external_editor::EditorError> for CommandFailure {
    fn from(error: crate::domain::external_editor::EditorError) -> Self {
        crate::other::AppError::from_failure(error).into()
    }
}
