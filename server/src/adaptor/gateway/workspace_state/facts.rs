use std::future::Future;
use std::sync::Arc;

use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

use super::repository_impl::StoredWorkspaceState;
use crate::adaptor::gateway::local_event_store::canonical_cbor::CborValue;
use crate::adaptor::gateway::local_event_store::envelope::{
    DecodedStoredEvent, EventCodecError, EventCodecRegistry, LocalEventPayloadCodec,
};
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::domain::local_event::*;
use crate::domain::workspace_state::WorkspaceStateError;

pub(crate) struct UIStateCodec {
    pub group: bool,
}

impl LocalEventPayloadCodec for UIStateCodec {
    fn event_type(&self) -> &'static str {
        if self.group {
            "workspace.repository_group_changed"
        } else {
            "workspace.state_changed"
        }
    }
    fn payload_version(&self) -> i64 {
        1
    }
    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(
            (self.group, event),
            (true, LocalDomainEvent::RepositoryGroupChanged(_))
                | (false, LocalDomainEvent::WorkspaceStateChanged(_))
        )
    }
    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        let raw = match event {
            LocalDomainEvent::WorkspaceStateChanged(state) if !self.group => {
                serde_json::to_string(&StoredWorkspaceState::from(state.as_ref()))
            }
            LocalDomainEvent::RepositoryGroupChanged(collapsed) if self.group => {
                serde_json::to_string(collapsed)
            }
            _ => return Err(self.malformed()),
        };
        raw.map(CborValue::Text).map_err(|_| self.malformed())
    }
    fn decode(
        &self,
        version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if version != 1 {
            return Ok(None);
        }
        let CborValue::Text(raw) = value else {
            return Err(self.malformed());
        };
        if self.group {
            serde_json::from_str(raw)
                .map(LocalDomainEvent::RepositoryGroupChanged)
                .map(Some)
                .map_err(|_| self.malformed())
        } else {
            let stored: StoredWorkspaceState =
                serde_json::from_str(raw).map_err(|_| self.malformed())?;
            let state: crate::domain::workspace_state::WorkspaceState =
                stored.try_into().map_err(|_| self.malformed())?;
            Ok(Some(LocalDomainEvent::WorkspaceStateChanged(Box::new(
                state,
            ))))
        }
    }
}

impl UIStateCodec {
    fn malformed(&self) -> EventCodecError {
        EventCodecError::MalformedPayload {
            event_type: self.event_type().into(),
        }
    }
}

pub(crate) struct WorkspaceStateInvalidationCodec;

impl LocalEventPayloadCodec for WorkspaceStateInvalidationCodec {
    fn event_type(&self) -> &'static str {
        "workspace.state_invalidated"
    }
    fn payload_version(&self) -> i64 {
        1
    }
    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::WorkspaceStateInvalidated)
    }
    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        if self.handles(event) {
            Ok(CborValue::Null)
        } else {
            Err(EventCodecError::MalformedPayload {
                event_type: self.event_type().into(),
            })
        }
    }
    fn decode(
        &self,
        version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if version != 1 {
            return Ok(None);
        }
        if *value == CborValue::Null {
            Ok(Some(LocalDomainEvent::WorkspaceStateInvalidated))
        } else {
            Err(EventCodecError::MalformedPayload {
                event_type: self.event_type().into(),
            })
        }
    }
}

pub(crate) fn stream(key: &str, group: bool) -> StreamId {
    let namespace = if group {
        "repository-group"
    } else {
        "workspace-state"
    };
    StreamId::parse(&format!(
        "{namespace}:{}",
        hex::encode(Sha256::digest(key.as_bytes()))
    ))
    .expect("hashed UI state identity")
}

pub(crate) fn run<T: Send>(
    operation: impl Future<Output = Result<T, WorkspaceStateError>> + Send,
) -> Result<T, WorkspaceStateError> {
    let context = crate::common::operation_context::current();
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(error)?;
                runtime.block_on(crate::common::operation_context::scope(context, operation))
            })
            .join()
            .map_err(|_| error("UI state worker panicked"))?
    })
}

pub(crate) fn error(value: impl std::fmt::Display) -> WorkspaceStateError {
    WorkspaceStateError::Message(value.to_string())
}

pub(crate) async fn latest(
    store: &LocalEventStore,
    stream: &StreamId,
) -> Result<(StreamVersion, Option<LocalDomainEvent>), WorkspaceStateError> {
    let id = stream.as_str().to_string();
    let row = store.submit_query(move |connection| {
        connection.query_row(
            "SELECT stream_sequence, event_type, payload_version, payload, payload_sha256 FROM events WHERE stream_id = ?1 ORDER BY stream_sequence DESC LIMIT 1",
            [id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, Vec<u8>>(3)?, row.get::<_, Vec<u8>>(4)?)),
        ).optional().map_err(|error| crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error))
    }).await.map_err(error)?;
    let Some((sequence, kind, version, payload, hash)) = row else {
        return Ok((StreamVersion::new(0).expect("empty head"), None));
    };
    if Sha256::digest(&payload).as_slice() != hash {
        return Err(error("Corrupt UI state payload"));
    }
    let DecodedStoredEvent::Known(event) = EventCodecRegistry::new()
        .decode(&kind, version, &payload)
        .map_err(error)?
    else {
        return Err(error("Incompatible stored UI state event"));
    };
    Ok((StreamVersion::new(sequence).map_err(error)?, Some(*event)))
}

pub(crate) async fn append(
    store: &Arc<LocalEventStore>,
    stream: StreamId,
    event: LocalDomainEvent,
) -> Result<(), WorkspaceStateError> {
    let (head, _) = latest(store, &stream).await?;
    let identity = uuid::Uuid::new_v4().to_string();
    let payload = EventCodecRegistry::new().encode(&event).map_err(error)?;
    let batch = LocalAtomicBatch {
        commit_id: CommitIdentity::parse(&identity).map_err(error)?,
        idempotency: IdempotencyBinding {
            installation_id: store.installation_id().into(),
            operation_kind: CommitOperationKind::UserMutation,
            idempotency_key: identity,
            payload_hash: Sha256::digest(&payload.payload).into(),
        },
        expected_heads: vec![ExpectedStreamHead {
            stream_id: stream.clone(),
            expected: head,
        }],
        events: vec![UncommittedDomainEvent {
            stream_id: stream,
            event,
            occurred_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(error)?
                .as_millis() as i64,
        }],
        state_mutations: vec![],
    };
    store.commit_batch(batch).await.map(|_| ()).map_err(error)
}

#[cfg(test)]
#[path = "facts_test.rs"]
mod facts_tests;
