use super::repository_impl::StoredWorkspaceState;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::domain::local_event::*;
use crate::domain::workspace_state::WorkspaceStateError;
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub(crate) fn workspace_stream(key: &str) -> StreamId {
    hashed_stream("workspace-state", key)
}
pub(crate) fn repository_group_stream(key: &str) -> StreamId {
    hashed_stream("repository-group", key)
}
fn hashed_stream(namespace: &str, key: &str) -> StreamId {
    StreamId::parse(&format!(
        "{namespace}:{}",
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
    let payload = match &event {
        LocalDomainEvent::WorkspaceStateChanged(state) => {
            serde_json::to_vec(&StoredWorkspaceState::from(state.as_ref()))
        }
        LocalDomainEvent::RepositoryGroupChanged(collapsed) => serde_json::to_vec(collapsed),
        LocalDomainEvent::WorkspaceStateInvalidated => Ok(b"null".to_vec()),
        _ => return Err(error("Unexpected UI state event")),
    }
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
    store
        .commit_batch(batch)
        .await
        .map(|_| ())
        .map_err(|failure| match failure {
            CommitBatchError::StreamHeadConflict { .. } => WorkspaceStateError::VersionConflict,
            failure => error(failure),
        })
}
