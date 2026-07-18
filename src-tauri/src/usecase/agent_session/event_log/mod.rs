//! Durable event log and projector for agent sessions.
//!
//! Connection points for adjacent work:
//! - issues-1213: this projector writes the same read model that the existing
//!   split session layout (`meta.json` + `messages/{seq}.json` + `index.json`)
//!   stores, so the storage format stays compatible.
//! - issues-1214: live-only deltas are kept out of `AgentSessionEvent`, leaving
//!   a clear boundary for the runtime seq-delta streaming protocol. Live
//!   buffers stay in `usecase::agent_session::runtime`; only durable parts and
//!   terminal live blocks are appended here.
//! - issues-1217: runtime integration is owned by
//!   `usecase::agent_session::runtime`; this pure module can move behind a
//!   later runtime/stream/persist split without changing the event vocabulary.

mod events;
mod finalization;
mod log;
mod part_events;
mod projector;

pub use events::{
    AgentSessionEvent, BackendSessionRecoveryReason, GoalReactivationOutcome, InterruptReason,
    PermissionDecision, PromptInput, TurnStopReason, TurnTokenUsage,
};
pub(crate) use finalization::{
    finalize_turn, latest_unresolved_permission_request, UnresolvedPermissionRequest,
};
pub use log::TurnEventLog;
pub(crate) use part_events::append_part_events;
pub use part_events::PartEventMode;
#[cfg(test)]
pub(crate) use projector::apply_event_to_queue_pause;
#[cfg(test)]
pub(crate) use projector::session_error_message;
pub(crate) use projector::SessionReadModel;
pub use projector::{
    latest_turn_interruption, AgentTurnFailureSignal, BackendSessionRecoveryProjection,
    WorkflowTurnCompleteInput,
};

#[cfg(test)]
mod tests;
