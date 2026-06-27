//! Durable event log and projector for agent sessions.
//!
//! Connection points for adjacent work:
//! - issues-1213: this projector writes the same read model that the existing
//!   split session layout (`meta.json` + `messages/{seq}.json` + `index.json`)
//!   stores, so the storage format stays compatible.
//! - issues-1214: live-only deltas are kept out of `AgentSessionEvent`, leaving
//!   a clear boundary for a future seq-delta streaming protocol. The current
//!   bridge integration keeps live-only SDK deltas inside the legacy streaming
//!   accumulator; only durable parts and terminal live blocks are appended here.
//! - issues-1217: runtime integration remains in `bridge_common.rs`; this
//!   pure module can move behind a later runtime/stream/persist split without
//!   changing the event vocabulary.

mod events;
mod finalization;
mod log;
mod part_events;
mod projector;

pub use events::{
    human_parts_from_content_images, AgentSessionEvent, InterruptReason, PermissionDecision,
    PromptInput, TurnStopReason, TurnTokenUsage,
};
pub use log::TurnEventLog;
pub use part_events::PartEventMode;
#[cfg(test)]
pub use projector::project;
pub use projector::{AgentTurnFailureSignal, WorkflowTurnCompleteInput};

#[cfg(test)]
mod tests;
