use super as wire;
impl From<String> for wire::CommandError {
    fn from(value: String) -> Self {
        Self {
            variant: Some(wire::command_error::Variant::Message(wire::ResultString {
                value: Some(value),
            })),
        }
    }
}
impl From<crate::other::AppError> for wire::CommandError {
    fn from(value: crate::other::AppError) -> Self {
        match value {
            crate::other::AppError::Internal(value) => value.into(),
            crate::other::AppError::Coded { code, message } => Self {
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
