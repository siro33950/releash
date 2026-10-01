use crate::domain::failure::TechnicalFailure;
use crate::domain::failure::TechnicalFailureNature;
use crate::usecase::failure::WorkFailure;

pub(crate) fn failure(error: std::io::Error) -> WorkFailure {
    WorkFailure::from(TechnicalFailure::from(&error))
}

pub(crate) fn nature(error: &std::io::Error) -> TechnicalFailureNature {
    use std::io::ErrorKind as E;
    match error.kind() {
        E::Interrupted
        | E::WouldBlock
        | E::ConnectionReset
        | E::ConnectionAborted
        | E::NotConnected => TechnicalFailureNature::Transient,
        E::TimedOut => TechnicalFailureNature::TimedOut,
        _ => TechnicalFailureNature::Other,
    }
}

impl From<&std::io::Error> for crate::domain::failure::TechnicalFailure {
    fn from(error: &std::io::Error) -> Self {
        Self {
            nature: nature(error),
            message: error.to_string(),
        }
    }
}

impl From<std::io::Error> for crate::domain::failure::TechnicalFailure {
    fn from(error: std::io::Error) -> Self {
        (&error).into()
    }
}

#[cfg(test)]
#[path = "background_io_test.rs"]
mod background_io_tests;
