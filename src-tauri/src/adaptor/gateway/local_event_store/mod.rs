//! Permanent fixed-path SQLite local event store gateway.

pub(crate) mod agent_session_codec;
pub(crate) mod canonical_cbor;
pub(crate) mod clock;
pub(crate) mod commit;
pub(crate) mod connection;
pub(crate) mod cursor;
pub(crate) mod envelope;
pub(crate) mod fault;
pub(crate) mod hmac_sha256;
pub(crate) mod indexed_projection_codec;
pub(crate) mod layout;
pub(crate) mod projection_record_codec;
pub(crate) mod read_only;
pub(crate) mod reader;
pub(crate) mod schema;
pub(crate) mod state_record_codec;
pub(crate) mod store;
pub(crate) mod workflow_codec;
pub(crate) mod workspace_query_migration;
pub(crate) mod writer;

#[cfg(test)]
mod tests;

pub(crate) use store::{LocalEventStore, LocalEventStoreConfig};
