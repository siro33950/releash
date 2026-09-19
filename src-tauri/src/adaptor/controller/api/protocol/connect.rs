use super::client as wire;
pub(crate) use crate::adaptor::protocol::connect::{rpc, to_rpc, to_wire};

pub(crate) fn command_error(error: wire::CommandError) -> connectrpc::ConnectError {
    command_error_with_code(error, connectrpc::ErrorCode::FailedPrecondition)
}

pub(crate) fn command_error_with_code(
    error: wire::CommandError,
    code: connectrpc::ErrorCode,
) -> connectrpc::ConnectError {
    match to_rpc::<rpc::CommandError>(&error) {
        Ok(detail) => connectrpc::ConnectError::new(code, "Command failed").with_detail(
            connectrpc::ErrorDetail::from_message("releash.client.v1.CommandError", &detail),
        ),
        Err(error) => error,
    }
}
