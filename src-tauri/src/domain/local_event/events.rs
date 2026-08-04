//! Closed domain-event union persisted by the local event store.
//!
//! `LocalDomainEvent` wraps the whole agent-session and workflow domain event
//! enums so variant additions in those modules never require changes here.
//! The application stream owns its own minimal event vocabulary below.

#![allow(dead_code)] // Closed persisted event vocabulary retains compatibility accessors.

use crate::domain::agent_session::events::AgentSessionDomainEvent;
use crate::domain::local_event::identifiers::{
    CommitIdentity, EventId, GlobalSequence, StreamId, StreamSequence, StreamVersion,
};
use crate::domain::provider_lifecycle::ProviderLifecycleEvent;
use crate::domain::workflow::events::WorkflowDomainEvent;

/// Shutdown / quit intent fixed by the first accepted quit request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitIntent {
    Exit { code: i64 },
    Restart { code: i64 },
}

/// Closed shutdown phases from the issues-1499 design "Public closed types".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationShutdownPhase {
    Prepared,
    Activated,
    Quiescing,
    Completed,
    Failed,
    Cancelled,
    ReconciliationRequired,
}

/// Minimal application-stream event vocabulary owned by this module.
/// The shutdown coordinator appends here; it must not invent a parallel
/// event enum.
#[derive(Debug, Clone, PartialEq)]
pub enum ApplicationDomainEvent {
    ApplicationQuitAccepted {
        quit_operation_id: String,
        intent: QuitIntent,
        at_ms: i64,
    },
    ShutdownPhaseAdvanced {
        shutdown_id: String,
        phase: ApplicationShutdownPhase,
        at_ms: i64,
    },
    ShutdownDetailsCompacted {
        shutdown_id: String,
        at_ms: i64,
    },
}

/// Closed sum of every domain event the store can persist.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalDomainEvent {
    AgentSession(AgentSessionDomainEvent),
    Workflow(WorkflowDomainEvent),
    ProviderLifecycle(ProviderLifecycleEvent),
    Application(ApplicationDomainEvent),
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
