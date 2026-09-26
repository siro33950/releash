use crate::domain::failure::{Failure, TechnicalFailureNature};
use crate::usecase::work_queue::WorkFailure;

pub(crate) fn failure(error: std::io::Error) -> WorkFailure {
    use std::io::ErrorKind as E;
    let nature = match error.kind() {
        E::Interrupted
        | E::WouldBlock
        | E::ConnectionReset
        | E::ConnectionAborted
        | E::NotConnected => TechnicalFailureNature::Transient,
        E::TimedOut => TechnicalFailureNature::TimedOut,
        _ => TechnicalFailureNature::Other,
    };
    WorkFailure {
        kind: Failure::Technical(nature),
        message: error.to_string(),
    }
}

#[cfg(test)]
#[path = "background_io_test.rs"]
mod background_io_tests;
