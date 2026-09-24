include!(concat!(env!("OUT_DIR"), "/connect/mod.rs"));
pub use releash::client::v1 as rpc;

pub(crate) fn to_wire<T: prost::Message + Default>(
    value: &impl buffa::Message,
) -> Result<T, connectrpc::ConnectError> {
    T::decode(buffa::Message::encode_to_vec(value).as_slice()).map_err(|error| {
        protocol_error(
            crate::domain::failure::FailureKind::InvalidInput,
            error.to_string(),
        )
    })
}

pub(crate) fn to_rpc<T: buffa::Message>(
    value: &impl prost::Message,
) -> Result<T, connectrpc::ConnectError> {
    T::decode_from_slice(&prost::Message::encode_to_vec(value)).map_err(|error| {
        protocol_error(
            crate::domain::failure::FailureKind::Internal,
            error.to_string(),
        )
    })
}

fn code(kind: crate::domain::failure::FailureKind) -> connectrpc::ErrorCode {
    use crate::domain::failure::FailureKind as F;
    use connectrpc::ErrorCode as C;
    match kind {
        F::Temporary => C::Unavailable,
        F::RestartRequired => C::Aborted,
        F::StateRequired => C::FailedPrecondition,
        F::InvalidInput => C::InvalidArgument,
        F::Expired => C::DeadlineExceeded,
        F::Missing => C::NotFound,
        F::AlreadyPresent => C::AlreadyExists,
        F::Permission => C::PermissionDenied,
        F::Capacity => C::ResourceExhausted,
        F::Unsupported => C::Unimplemented,
        F::Internal => C::Internal,
        F::Corrupt => C::DataLoss,
        F::Cancelled => C::Canceled,
        F::Unknown => C::Unknown,
        F::OutsideRange => C::OutOfRange,
        F::AuthenticationRequired => C::Unauthenticated,
    }
}

fn protocol_error(
    kind: crate::domain::failure::FailureKind,
    message: impl Into<String>,
) -> connectrpc::ConnectError {
    connectrpc::ConnectError::new(code(kind), message.into())
}

pub(crate) fn classified_error(
    error: impl crate::domain::failure::ClassifiedFailure + std::fmt::Display,
) -> connectrpc::ConnectError {
    protocol_error(error.failure_kind(), error.to_string())
}

pub(crate) fn command_error(error: super::client::CommandFailure) -> connectrpc::ConnectError {
    match to_rpc::<rpc::CommandError>(&error.detail) {
        Ok(detail) => protocol_error(error.kind, "Command failed").with_detail(
            connectrpc::ErrorDetail::from_message("releash.client.v1.CommandError", &detail),
        ),
        Err(error) => error,
    }
}

#[cfg(test)]
#[path = "connect_test.rs"]
mod tests;

impl crate::domain::failure::ClassifiedFailure for connectrpc::ConnectError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind as F;
        use connectrpc::ErrorCode as C;
        match self.code {
            C::Unavailable => F::Temporary,
            C::Aborted => F::RestartRequired,
            C::FailedPrecondition => F::StateRequired,
            C::InvalidArgument => F::InvalidInput,
            C::DeadlineExceeded => F::Expired,
            C::NotFound => F::Missing,
            C::AlreadyExists => F::AlreadyPresent,
            C::PermissionDenied => F::Permission,
            C::ResourceExhausted => F::Capacity,
            C::Unimplemented => F::Unsupported,
            C::Internal => F::Internal,
            C::DataLoss => F::Corrupt,
            C::Canceled => F::Cancelled,
            C::Unknown => F::Unknown,
            C::OutOfRange => F::OutsideRange,
            C::Unauthenticated => F::AuthenticationRequired,
            _ => F::Unknown,
        }
    }
}
