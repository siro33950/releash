use super::client as wire;
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
impl From<crate::adaptor::protocol::application_operation_v1::ApplicationCommandErrorWire<'_>>
    for wire::CommandError
{
    fn from(
        value: crate::adaptor::protocol::application_operation_v1::ApplicationCommandErrorWire<'_>,
    ) -> Self {
        Self {
            variant: Some(wire::command_error::Variant::Application(Box::new(
                wire::ApplicationError {
                    r#type: Some(value.error_type.into()),
                    message: Some(value.message.into()),
                    correlation_id: value.correlation_id.map(Into::into),
                    failure: value
                        .failure
                        .cloned()
                        .map(|failure| failure.try_into().expect("safe operation failure fields")),
                },
            ))),
        }
    }
}

impl From<crate::adaptor::protocol::application_operation_v1::OperationApplicationErrorDtoV1>
    for wire::CommandError
{
    fn from(
        value: crate::adaptor::protocol::application_operation_v1::OperationApplicationErrorDtoV1,
    ) -> Self {
        value.wire().into()
    }
}
impl From<crate::adaptor::protocol::application_operation_v1::ApplicationQuitErrorDtoV1>
    for wire::CommandError
{
    fn from(
        value: crate::adaptor::protocol::application_operation_v1::ApplicationQuitErrorDtoV1,
    ) -> Self {
        value.wire().into()
    }
}
impl From<crate::adaptor::protocol::application_operation_v1::ApplicationQuitLookupErrorDtoV1>
    for wire::CommandError
{
    fn from(
        value: crate::adaptor::protocol::application_operation_v1::ApplicationQuitLookupErrorDtoV1,
    ) -> Self {
        value.wire().into()
    }
}
impl From<crate::adaptor::protocol::application_operation_v1::CurrentShutdownErrorDtoV1>
    for wire::CommandError
{
    fn from(
        value: crate::adaptor::protocol::application_operation_v1::CurrentShutdownErrorDtoV1,
    ) -> Self {
        value.wire().into()
    }
}
impl From<crate::adaptor::protocol::application_operation_v1::ShutdownPlanQueryErrorDtoV1>
    for wire::CommandError
{
    fn from(
        value: crate::adaptor::protocol::application_operation_v1::ShutdownPlanQueryErrorDtoV1,
    ) -> Self {
        value.wire().into()
    }
}
impl From<crate::adaptor::protocol::application_operation_v1::ShutdownDetailsMutationErrorDtoV1>
    for wire::CommandError
{
    fn from(
        value: crate::adaptor::protocol::application_operation_v1::ShutdownDetailsMutationErrorDtoV1,
    ) -> Self {
        value.wire().into()
    }
}
impl From<crate::adaptor::protocol::application_operation_v1::RecoveryActionCommandErrorDtoV1>
    for wire::CommandError
{
    fn from(
        value: crate::adaptor::protocol::application_operation_v1::RecoveryActionCommandErrorDtoV1,
    ) -> Self {
        value.wire().into()
    }
}
impl From<crate::usecase::application_startup::ApplicationUnavailable> for wire::CommandError {
    fn from(value: crate::usecase::application_startup::ApplicationUnavailable) -> Self {
        Self { variant: Some(wire::command_error::Variant::Application(Box::new(match value { crate::usecase::application_startup::ApplicationUnavailable::ApplicationUnavailable => wire::ApplicationError { r#type: Some("application_unavailable".into()), message: None, correlation_id: None, failure: None } }))) }
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
