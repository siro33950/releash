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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalEventQueryError {
    Technical(crate::domain::failure::TechnicalFailure),
    InvalidRequest,
    CanonicalWriterRequired,
    QueryBusy,
    ResponseTooLarge,
    /// A stored event required for meaning could not be decoded.
    IncompatibleStoredEvent {
        correlation_id: String,
    },
    StorageUnavailable {
        failure: SafeOperationFailure,
    },
    StorageAccessRequired {
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
            Self::Technical(error) => std::fmt::Display::fmt(error, f),
            Self::CanonicalWriterRequired => write!(
                f,
                "Commit resolution requires the canonical writer authority."
            ),
            Self::InvalidRequest => write!(f, "invalid request"),
            Self::QueryBusy => write!(f, "query busy"),
            Self::ResponseTooLarge => write!(f, "response too large"),
            Self::IncompatibleStoredEvent { correlation_id } => {
                write!(
                    f,
                    "incompatible stored event (correlation_id={correlation_id})"
                )
            }
            Self::StorageUnavailable { failure } | Self::StorageAccessRequired { failure } => {
                write!(f, "storage unavailable: {failure}")
            }
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
