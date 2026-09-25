use crate::domain::local_event::failure::SafeOperationFailure;
use crate::domain::local_event::identifiers::Revision;
use crate::domain::local_event::record::SessionProjectionRecord;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalEventQuery {
    SessionProjectionByIdentity { session_id: String },
    CanonicalRuntimeOwnerSnapshot { limit: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionProjectionView {
    pub session_id: String,
    pub projection: SessionProjectionRecord,
    pub revision: Revision,
}

/// Lightweight runtime-ownership facts extracted from one canonical SQLite
/// statement. Inactive sessions remain present so a live PID can be resolved
/// to its canonical worktree without returning the full projection body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalRuntimeOwnerView {
    AgentSession { worktree_path: String, active: bool },
    ActiveWorkflow { worktree_path: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum LocalEventQueryResult {
    SessionProjectionByIdentity(Option<SessionProjectionView>),
    CanonicalRuntimeOwnerSnapshot(Vec<CanonicalRuntimeOwnerView>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LocalEventQueryError {
    Stopped(crate::domain::operation_context::OperationStopped),
    InvalidRequest,
    QueryBusy,
    DeadlineExceeded,
    ResponseTooLarge,
    /// A stored event required for meaning could not be decoded.
    IncompatibleStoredEvent {
        correlation_id: String,
    },
    StorageUnavailable {
        failure: SafeOperationFailure,
    },
    Corrupt {
        correlation_id: String,
    },
    Internal {
        correlation_id: String,
    },
}

impl fmt::Display for LocalEventQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped(error) => std::fmt::Display::fmt(error, f),
            Self::InvalidRequest => write!(f, "invalid request"),
            Self::QueryBusy => write!(f, "query busy"),
            Self::DeadlineExceeded => write!(f, "deadline exceeded"),
            Self::ResponseTooLarge => write!(f, "response too large"),
            Self::IncompatibleStoredEvent { correlation_id } => {
                write!(
                    f,
                    "incompatible stored event (correlation_id={correlation_id})"
                )
            }
            Self::StorageUnavailable { failure } => write!(f, "storage unavailable: {failure}"),
            Self::Corrupt { correlation_id } => {
                write!(f, "store corrupt (correlation_id={correlation_id})")
            }
            Self::Internal { correlation_id } => {
                write!(f, "internal error (correlation_id={correlation_id})")
            }
        }
    }
}

impl std::error::Error for LocalEventQueryError {}

impl crate::domain::failure::ClassifiedFailure for LocalEventQueryError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind;
        match self {
            Self::Stopped(error) => crate::domain::failure::ClassifiedFailure::failure_kind(error),
            Self::InvalidRequest => FailureKind::InvalidInput,
            Self::QueryBusy => FailureKind::Temporary,
            Self::DeadlineExceeded => FailureKind::Expired,
            Self::ResponseTooLarge => FailureKind::Capacity,
            Self::IncompatibleStoredEvent { .. } => FailureKind::StateRequired,
            Self::StorageUnavailable { failure } => failure.failure_kind(),
            Self::Corrupt { .. } => FailureKind::Corrupt,
            Self::Internal { .. } => FailureKind::Internal,
        }
    }
}

#[cfg(test)]
#[path = "query_test.rs"]
mod query_tests;

impl From<crate::domain::operation_context::OperationStopped> for LocalEventQueryError {
    fn from(error: crate::domain::operation_context::OperationStopped) -> Self {
        match error {
            crate::domain::operation_context::OperationStopped::Expired => Self::DeadlineExceeded,
            _ => Self::Stopped(error),
        }
    }
}
