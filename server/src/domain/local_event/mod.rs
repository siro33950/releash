//! Permanent local event store domain.
//!
//! Owns the closed vocabulary of the single mutation authority spanning
//! agent sessions and workflows: store identities, the atomic batch, the
//! closed state-mutation family, the closed query sum, and the
//! `LocalEventTransactionRepository` port. No serde, rusqlite, filesystem,
//! Tauri, or WebSocket dependency is allowed here.

pub mod batch;
pub mod events;
pub mod failure;
pub mod identifiers;
pub mod mutation;
pub mod query;
pub mod record;
#[allow(clippy::module_inception)]
pub mod repository;

pub use batch::{
    CommitBatchError, CommitBatchResult, CommitOperationKind, CommitResolution, CommittedBatch,
    CommittedStreamHead, IdempotencyBinding, LocalAtomicBatch,
};
pub use events::{
    CommittedDomainEvent, DomainEventPage, LoadStreamRequest, LoadedDomainEvent, LocalDomainEvent,
    UncommittedDomainEvent,
};
pub use failure::{SafeOperationFailure, SessionOperationFailureKind};
pub use identifiers::{
    CommitIdentity, EventId, ExpectedStreamHead, GlobalSequence, Revision, StreamId,
    StreamSequence, StreamVersion,
};
pub use mutation::{
    AgentSessionRemovalMutation, LocalStateMutation, RevisionGuard, SessionProjectionMutation,
};
pub use query::{
    CanonicalRuntimeOwnerView, LocalEventQuery, LocalEventQueryError, LocalEventQueryResult,
    SessionProjectionView,
};
pub use record::{
    AgentSessionProviderRecord, ProviderHookHealthProjectionRecord,
    ProviderSessionOwnershipProjectionRecord, SessionProjectionRecord,
    WorkflowExecutionMetadataRecord,
};
pub use repository::LocalEventTransactionRepository;
