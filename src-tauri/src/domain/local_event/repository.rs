//! `LocalEventTransactionRepository`: the single mutation authority port.

use crate::domain::local_event::batch::{
    CommitBatchError, CommitBatchResult, CommitResolution, LocalAtomicBatch,
};
use crate::domain::local_event::events::{
    DomainEventPage, LoadStreamRequest, UncommittedDomainEvent,
};
use crate::domain::local_event::identifiers::CommitIdentity;
use crate::domain::local_event::mutation::LocalStateMutation;
use crate::domain::local_event::query::{
    LocalEventQuery, LocalEventQueryError, LocalEventQueryResult,
};

/// The only mutation authority across agent sessions and workflows.
///
/// `commit_batch` is the single mutation entry point. Query methods perform
/// snapshot reads only and never repair, migrate, or rebuild projections
/// implicitly. SQL, row IDs, WAL, and serialization never leak through this
/// port.
#[async_trait::async_trait]
pub trait LocalEventTransactionRepository: Send + Sync {
    /// Stable replay identity for a state mutation.
    ///
    /// The default covers domain-owned non-projection families. Persistence
    /// gateways override this for projection rows because their historical
    /// identity includes the exact Stored V1 representation.
    fn canonical_mutation_identity_v1(
        &self,
        mutation: &LocalStateMutation,
    ) -> Result<Vec<u8>, String> {
        mutation.canonical_identity_v1().map_err(str::to_string)
    }

    /// Stable identity of the exact event envelopes that the gateway will
    /// persist for an atomic owner batch.
    ///
    /// Event type/version and canonical payload bytes are persistence
    /// concerns, so domain/usecase callers cannot provide a fallback. Writable
    /// and read-only SQLite gateways override this through the same registry
    /// used by commit preparation.
    fn canonical_event_batch_identity_v1(
        &self,
        _events: &[UncommittedDomainEvent],
    ) -> Result<Vec<u8>, String> {
        Err("canonical event identity is unavailable".to_string())
    }

    async fn commit_batch(
        &self,
        batch: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError>;

    /// Resolve an `OutcomeUnknown` commit identity to Committed or proven
    /// absence. Absence is only proof after writer exclusion and WAL
    /// recovery, which the store guarantees before answering.
    async fn resolve_commit(
        &self,
        identity: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError>;

    async fn load_stream(
        &self,
        request: LoadStreamRequest,
    ) -> Result<DomainEventPage, LocalEventQueryError>;

    async fn query(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError>;

    /// Synchronous facade for established synchronous application ports.
    ///
    /// Implementations dispatch onto an existing bounded reader pool. A call
    /// must not create a thread, async runtime, or database connection.
    fn query_blocking(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError>;
}
