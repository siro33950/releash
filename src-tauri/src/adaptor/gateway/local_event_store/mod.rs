//! Permanent SQLite local event store gateway.
//!
//! Implements `domain::local_event::LocalEventTransactionRepository` on
//! bundled SQLite: schema version 1, the single-writer worker with a bounded
//! two-lane queue, the bounded reader pool, canonical CBOR payload codec,
//! MAC-protected cursors, and the `authority-v1.json` cutover pointer.

pub(crate) mod agent_session_codec;
pub(crate) mod authority;
pub(crate) mod canonical_cbor;
pub(crate) mod clock;
pub(crate) mod commit;
pub(crate) mod connection;
pub(crate) mod cursor;
pub(crate) mod envelope;
pub(crate) mod fault;
pub(crate) mod hmac_sha256;
pub(crate) mod migration;
pub(crate) mod projection_record_codec;
pub(crate) mod read_only;
pub(crate) mod reader;
pub(crate) mod schema;
pub(crate) mod state_record_codec;
pub(crate) mod store;
pub(crate) mod workflow_codec;
pub(crate) mod writer;

#[cfg(test)]
mod tests;

pub(crate) use store::{LocalEventStore, LocalEventStoreConfig};
