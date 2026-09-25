//! Atomic commit batch and its result vocabulary.

use std::fmt;

use crate::domain::local_event::events::UncommittedDomainEvent;
use crate::domain::local_event::failure::SafeOperationFailure;
use crate::domain::local_event::identifiers::{
    CommitIdentity, ExpectedStreamHead, GlobalSequence, StreamId, StreamVersion,
};
use crate::domain::local_event::mutation::LocalStateMutation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommitOperationKind {
    UserMutation,
    Projection,
    Workflow,
}

impl CommitOperationKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::UserMutation => "user_mutation",
            Self::Projection => "projection",
            Self::Workflow => "workflow",
        }
    }

    pub fn is_critical(self) -> bool {
        matches!(self, Self::Workflow)
    }
}

/// Idempotency binding of a batch: `(generation, operation_kind, key)` must
/// be unique, and retries with the same key must carry the same canonical
/// payload hash to converge on the saved result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyBinding {
    pub installation_id: String,
    pub operation_kind: CommitOperationKind,
    pub idempotency_key: String,
    /// SHA-256 over the caller's canonical exact payload.
    pub payload_hash: [u8; 32],
}

/// The only mutation entry point of the store.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalAtomicBatch {
    pub commit_id: CommitIdentity,
    pub idempotency: IdempotencyBinding,
    /// Every stream this batch changes, exactly once each.
    pub expected_heads: Vec<ExpectedStreamHead>,
    pub events: Vec<UncommittedDomainEvent>,
    pub state_mutations: Vec<LocalStateMutation>,
}

/// Head of one stream after the commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedStreamHead {
    pub stream_id: StreamId,
    pub head: StreamVersion,
}

/// Durable proof of a sealed commit, reconstructable from the store by the
/// same commit identity at any later time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedBatch {
    pub commit_id: CommitIdentity,
    /// Inclusive global sequence range; `None` for a batch with zero events.
    pub sequence_range: Option<(GlobalSequence, GlobalSequence)>,
    pub stream_heads: Vec<CommittedStreamHead>,
    pub event_count: i64,
    pub mutation_count: i64,
    /// Integrity hash over the sealed commit summary (not a public identity).
    pub result_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommitBatchResult {
    /// This call performed the commit.
    Committed(CommittedBatch),
    /// The same idempotency binding was already committed earlier.
    Replayed(CommittedBatch),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommitBatchError {
    Technical(crate::domain::failure::TechnicalFailure),
    /// Same idempotency key or unique record key with a different canonical
    /// payload / content binding.
    PayloadConflict,
    /// An expected stream head or mutation revision guard did not match.
    StreamHeadConflict {
        current: StreamVersion,
    },
    /// The node fact head changed before append.
    TreeHeadConflict,
    /// The writer queue is temporarily full.
    QueueBusy,
    /// A node fact append reply was lost; resolve from the fact log.
    AppendOutcomeUnknown,
    /// Batch bounds exceeded before writer admission.
    CapacityExceeded,
    /// A sequence / revision would pass `i64::MAX`.
    SequenceExhausted,
    /// The store rolled back before SQLite COMMIT; nothing changed.
    StorageUnavailable {
        failure: SafeOperationFailure,
    },
    /// COMMIT was started but the result could not be confirmed. Resolve with
    /// `resolve_commit` or a retry of the same batch; never a new identity.
    OutcomeUnknown {
        identity: CommitIdentity,
    },
    /// The store detected inconsistent durable state.
    Corrupt {
        correlation_id: String,
    },
}

impl fmt::Display for CommitBatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Technical(error) => std::fmt::Display::fmt(error, f),
            Self::PayloadConflict => write!(f, "same key bound to a different payload"),
            Self::StreamHeadConflict { current } => {
                write!(
                    f,
                    "expected head/revision mismatch (current={})",
                    current.value()
                )
            }
            Self::TreeHeadConflict => write!(f, "node event tree changed before append"),
            Self::QueueBusy => write!(f, "write queue is full"),
            Self::AppendOutcomeUnknown => write!(f, "node event write outcome is unknown"),
            Self::CapacityExceeded => write!(f, "batch capacity exceeded"),
            Self::SequenceExhausted => write!(f, "sequence space exhausted"),
            Self::StorageUnavailable { failure } => write!(f, "storage unavailable: {failure}"),
            Self::OutcomeUnknown { identity } => {
                write!(f, "commit outcome unknown for {}", identity.as_str())
            }
            Self::Corrupt { correlation_id } => {
                write!(f, "store corrupt (correlation_id={correlation_id})")
            }
        }
    }
}

impl std::error::Error for CommitBatchError {}

/// Resolution of a commit identity after the store finished WAL recovery and
/// holds the exclusive writer lock; only then is absence proof of no commit.
#[derive(Debug, Clone, PartialEq)]
pub enum CommitResolution {
    Committed(CommittedBatch),
    NotCommitted,
}

impl crate::domain::failure::ClassifiedFailure for CommitBatchError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind;
        match self {
            Self::Technical(error) => {
                crate::domain::failure::ClassifiedFailure::failure_kind(error)
            }
            Self::PayloadConflict => FailureKind::StateRequired,
            Self::QueueBusy => FailureKind::Temporary,
            Self::StreamHeadConflict { .. }
            | Self::OutcomeUnknown { .. }
            | Self::TreeHeadConflict
            | Self::AppendOutcomeUnknown => FailureKind::RestartRequired,
            Self::CapacityExceeded | Self::SequenceExhausted => FailureKind::Capacity,
            Self::StorageUnavailable { failure } => failure.failure_kind(),
            Self::Corrupt { .. } => FailureKind::Corrupt,
        }
    }
}

#[cfg(test)]
#[path = "batch_test.rs"]
mod batch_tests;
