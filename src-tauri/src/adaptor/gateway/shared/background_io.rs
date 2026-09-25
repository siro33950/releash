use crate::domain::failure::FailureKind;
use crate::usecase::work_queue::WorkFailure;

pub(crate) fn failure(error: std::io::Error) -> WorkFailure {
    use std::io::ErrorKind as E;
    let kind = match error.kind() {
        E::Interrupted
        | E::WouldBlock
        | E::ConnectionReset
        | E::ConnectionAborted
        | E::NotConnected => FailureKind::Temporary,
        E::PermissionDenied => FailureKind::Permission,
        E::NotFound => FailureKind::Missing,
        E::InvalidInput => FailureKind::InvalidInput,
        E::InvalidData => FailureKind::Corrupt,
        E::TimedOut => FailureKind::Expired,
        E::StorageFull | E::OutOfMemory => FailureKind::Capacity,
        E::Unsupported => FailureKind::Unsupported,
        _ => FailureKind::Internal,
    };
    WorkFailure {
        kind,
        message: error.to_string(),
    }
}

#[cfg(test)]
#[path = "background_io_test.rs"]
mod background_io_tests;
