use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::domain::local_event::*;
use crate::domain::workspace_state::WorkspaceStateError;
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub(crate) fn workspace_stream(key: &str) -> StreamId {
    hashed_stream(
        crate::domain::local_event::identifiers::WORKSPACE_STATE_PREFIX,
        key,
    )
}
pub(crate) fn repository_group_stream(key: &str) -> StreamId {
    hashed_stream(
        crate::domain::local_event::identifiers::REPOSITORY_GROUP_PREFIX,
        key,
    )
}
fn hashed_stream(namespace: &str, key: &str) -> StreamId {
    StreamId::parse(&format!(
        "{namespace}{}",
        hex::encode(Sha256::digest(key.as_bytes()))
    ))
    .expect("hashed UI state identity")
}
pub(crate) fn error(value: impl std::fmt::Display) -> WorkspaceStateError {
    WorkspaceStateError::Message(value.to_string())
}

pub(crate) async fn append(
    store: &Arc<LocalEventStore>,
    stream: StreamId,
    event: LocalDomainEvent,
    head: StreamVersion,
) -> Result<(), WorkspaceStateError> {
    let identity = uuid::Uuid::new_v4().to_string();
    let events = vec![UncommittedDomainEvent {
        stream_id: stream.clone(),
        event,
        occurred_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(error)?
            .as_millis() as i64,
    }];
    let payload = store
        .canonical_event_batch_identity_v1(&events)
        .map_err(error)?;
    let batch = LocalAtomicBatch {
        commit_id: CommitIdentity::parse(&identity).map_err(error)?,
        idempotency: IdempotencyBinding {
            installation_id: store.installation_id().into(),
            operation_kind: CommitOperationKind::UserMutation,
            idempotency_key: identity,
            payload_hash: Sha256::digest(&payload).into(),
        },
        expected_heads: vec![ExpectedStreamHead {
            stream_id: stream.clone(),
            expected: head,
        }],
        events,
        state_mutations: vec![],
    };
    store
        .commit_batch(batch)
        .await
        .map(|_| ())
        .map_err(|failure| match failure {
            CommitBatchError::StreamHeadConflict { .. } => WorkspaceStateError::VersionConflict,
            failure => error(failure),
        })
}

pub(crate) async fn latest(
    store: &LocalEventStore,
    stream: StreamId,
) -> Result<(StreamVersion, Option<LocalDomainEvent>), LocalEventQueryError> {
    let result = store
        .query(LocalEventQuery::StreamHead {
            stream_id: stream.clone(),
        })
        .await?;
    let LocalEventQueryResult::StreamHead(head) = result else {
        return Err(LocalEventQueryError::InvalidRequest);
    };
    if head.value() == 0 {
        return Ok((head, None));
    }
    let page = store
        .load_stream(LoadStreamRequest {
            stream_id: stream.clone(),
            after: if head.value() == 1 {
                None
            } else {
                Some(
                    StreamSequence::new(head.value() - 1)
                        .map_err(|_| LocalEventQueryError::InvalidRequest)?,
                )
            },
            limit: 1,
        })
        .await?;
    let Some(event) = page.events.into_iter().next() else {
        return Err(crate::adaptor::gateway::local_event_store::reader::corrupt(
            &format!(
                "stream={} head={} missing event",
                stream.as_str(),
                head.value()
            ),
        ));
    };
    match event.event {
        LoadedDomainEvent::Known(event) => Ok((head, Some(*event))),
        LoadedDomainEvent::Unknown {
            event_type,
            payload_version,
        } => Err(
            crate::adaptor::gateway::local_event_store::reader::incompatible(&format!(
                "stream={} sequence={} event_type={} payload_version={}",
                stream.as_str(),
                head.value(),
                event_type,
                payload_version
            )),
        ),
    }
}
