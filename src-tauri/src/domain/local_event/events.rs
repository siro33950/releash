//! Closed domain-event union persisted by the local event store.
//!
//! `LocalDomainEvent` wraps the whole agent-session and workflow domain event
//! enums so variant additions in those modules never require changes here.

use crate::domain::agent_session::ProviderSessionOwnershipEvent;
use crate::domain::local_event::identifiers::{
    CommitIdentity, EventId, GlobalSequence, StreamId, StreamSequence, StreamVersion,
};
use crate::domain::provider_lifecycle::{ProviderHookHealthEvent, ProviderLifecycleEvent};

/// Closed sum of every domain event the store can persist.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalDomainEvent {
    ProviderSessionOwnership(ProviderSessionOwnershipEvent),
    ProviderLifecycle(ProviderLifecycleEvent),
    ProviderHookHealth(ProviderHookHealthEvent),
}

/// One event a batch wants to append to a stream, before commit assigns
/// sequences and identity.
#[derive(Debug, Clone, PartialEq)]
pub struct UncommittedDomainEvent {
    pub stream_id: StreamId,
    pub event: LocalDomainEvent,
    /// Milliseconds since the Unix epoch at which the fact occurred.
    pub occurred_at_ms: i64,
}

/// A loaded event body. Unknown stored types are surfaced without meaning;
/// readers that require the meaning must fail closed instead of guessing.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadedDomainEvent {
    Known(Box<LocalDomainEvent>),
    /// The store preserved the raw envelope; the payload stays gateway-side.
    Unknown {
        event_type: String,
        payload_version: i64,
    },
}

/// One committed event returned from `load_stream`.
#[derive(Debug, Clone, PartialEq)]
pub struct CommittedDomainEvent {
    pub event_id: EventId,
    pub commit_id: CommitIdentity,
    pub stream_id: StreamId,
    pub stream_sequence: StreamSequence,
    pub global_sequence: GlobalSequence,
    pub occurred_at_ms: i64,
    pub event: LoadedDomainEvent,
}

/// Bounded read request over one stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadStreamRequest {
    pub stream_id: StreamId,
    /// Return events strictly after this stream sequence; `None` from start.
    pub after: Option<StreamSequence>,
    /// Maximum entries in the page. The store also applies its byte bound.
    pub limit: usize,
}

/// Bounded page of committed events plus the head observed in the same
/// snapshot; no partial page is ever produced.
#[derive(Debug, Clone, PartialEq)]
pub struct DomainEventPage {
    pub events: Vec<CommittedDomainEvent>,
    pub head: StreamVersion,
    /// Cursor for the next page; `None` when the page reached the head.
    pub next_after: Option<StreamSequence>,
}
