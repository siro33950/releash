use super::envelope::DecodedStoredEvent;
use super::reader::{storage_unavailable, QueryContext};
use crate::domain::local_event::*;
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

pub(super) fn latest(
    connection: &Connection,
    context: &QueryContext,
    stream: &StreamId,
) -> Result<(StreamVersion, Option<LocalDomainEvent>), LocalEventQueryError> {
    let row = connection.query_row("SELECT stream_sequence, event_type, payload_version, payload, payload_sha256 FROM events WHERE stream_id = ?1 ORDER BY stream_sequence DESC LIMIT 1", [stream.as_str()], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, Vec<u8>>(3)?, row.get::<_, Vec<u8>>(4)?))).optional().map_err(|error| storage_unavailable(&error))?;
    let Some((sequence, kind, version, payload, hash)) = row else {
        return Ok((StreamVersion::new(0).expect("empty head"), None));
    };
    let corrupt = || LocalEventQueryError::Corrupt {
        correlation_id: uuid::Uuid::new_v4().to_string(),
    };
    if Sha256::digest(&payload).as_slice() != hash {
        return Err(corrupt());
    }
    let DecodedStoredEvent::Known(event) = context
        .registry
        .decode(&kind, version, &payload)
        .map_err(|_| corrupt())?
    else {
        return Err(LocalEventQueryError::IncompatibleStoredEvent {
            correlation_id: uuid::Uuid::new_v4().to_string(),
        });
    };
    Ok((
        StreamVersion::new(sequence).map_err(|_| corrupt())?,
        Some(*event),
    ))
}

pub(super) fn states(
    connection: &Connection,
) -> Result<Vec<(String, StreamVersion)>, LocalEventQueryError> {
    let mut statement = connection.prepare("SELECT stream_id, stream_sequence FROM events e WHERE stream_id LIKE 'workspace-state:%' AND event_type = 'workspace.state_changed' AND NOT EXISTS (SELECT 1 FROM events next WHERE next.stream_id = e.stream_id AND next.stream_sequence > e.stream_sequence)").map_err(|error| storage_unavailable(&error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| storage_unavailable(&error))?;
    rows.map(|row| {
        let (stream, version) = row.map_err(|error| storage_unavailable(&error))?;
        Ok((
            stream,
            StreamVersion::new(version).map_err(|_| LocalEventQueryError::Corrupt {
                correlation_id: uuid::Uuid::new_v4().to_string(),
            })?,
        ))
    })
    .collect()
}
