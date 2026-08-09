use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::domain::local_event::{
    CommitBatchError, CommitBatchResult, CommitIdentity, CommitOperationKind, CommitResolution,
    ExpectedStreamHead, IdempotencyBinding, LoadStreamRequest, LocalAtomicBatch, LocalDomainEvent,
    LocalEventTransactionRepository, StreamId, UncommittedDomainEvent,
};
use crate::domain::provider_lifecycle::{
    ProviderLifecycleEventRepository, ProviderLifecycleRepositoryError,
    ScopedProviderLifecycleEvent,
};

pub(crate) struct LocalProviderLifecycleEventRepository {
    repository: Arc<dyn LocalEventTransactionRepository>,
    installation_id: String,
    pending: Mutex<HashMap<[u8; 32], PreparedCommit>>,
}

#[derive(Clone)]
struct PreparedCommit {
    identity: String,
    commit_id: CommitIdentity,
    payload_hash: [u8; 32],
    stream_ids: Vec<StreamId>,
    events: Vec<UncommittedDomainEvent>,
}

impl LocalProviderLifecycleEventRepository {
    pub(crate) fn new(
        repository: Arc<dyn LocalEventTransactionRepository>,
        installation_id: String,
    ) -> Self {
        Self {
            repository,
            installation_id,
            pending: Mutex::new(HashMap::new()),
        }
    }

    async fn append_events(
        &self,
        scoped_events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleRepositoryError> {
        if scoped_events.is_empty() {
            return Ok(());
        }
        let semantic_events = uncommitted_events(&scoped_events, 0)?;
        let semantic_identity = self
            .repository
            .canonical_event_batch_identity_v1(&semantic_events)
            .map_err(|_| ProviderLifecycleRepositoryError::Corrupt)?;
        let semantic_key: [u8; 32] = Sha256::digest(&semantic_identity).into();
        let mut prepared = match self.take_pending(&semantic_key)? {
            Some(pending) => match self.resolve_bounded(&pending.commit_id).await {
                Ok(CommitResolution::Committed(_)) => return Ok(()),
                Ok(CommitResolution::NotCommitted) => pending,
                Err(error) => {
                    self.restore_pending(semantic_key, pending)?;
                    return Err(error);
                }
            },
            None => self.prepare_commit(&scoped_events)?,
        };

        for _ in 0..4 {
            let expected_heads = self.load_expected_heads(&prepared.stream_ids).await?;
            let batch = LocalAtomicBatch {
                commit_id: prepared.commit_id.clone(),
                idempotency: IdempotencyBinding {
                    installation_id: self.installation_id.clone(),
                    operation_kind: CommitOperationKind::SessionLifecycle,
                    idempotency_key: format!("provider-lifecycle.{}", prepared.identity),
                    payload_hash: prepared.payload_hash,
                },
                expected_heads,
                events: prepared.events.clone(),
                state_mutations: Vec::new(),
            };
            match self.repository.commit_batch(batch).await {
                Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {
                    return Ok(())
                }
                Err(CommitBatchError::StreamHeadConflict { .. }) => continue,
                Err(CommitBatchError::OutcomeUnknown { identity }) => {
                    if identity != prepared.commit_id {
                        return Err(ProviderLifecycleRepositoryError::Corrupt);
                    }
                    match self.resolve_bounded(&identity).await {
                        Ok(CommitResolution::Committed(_)) => return Ok(()),
                        Ok(CommitResolution::NotCommitted) => continue,
                        Err(error) => {
                            prepared.commit_id = identity;
                            self.restore_pending(semantic_key, prepared)?;
                            return Err(error);
                        }
                    }
                }
                Err(CommitBatchError::StorageUnavailable { .. })
                | Err(CommitBatchError::EffectAdmissionBlocked) => {
                    return Err(ProviderLifecycleRepositoryError::StorageUnavailable)
                }
                Err(
                    CommitBatchError::PayloadConflict
                    | CommitBatchError::CapacityExceeded
                    | CommitBatchError::SequenceExhausted
                    | CommitBatchError::Corrupt { .. },
                ) => return Err(ProviderLifecycleRepositoryError::Corrupt),
            }
        }
        Err(ProviderLifecycleRepositoryError::StorageUnavailable)
    }

    fn prepare_commit(
        &self,
        scoped_events: &[ScopedProviderLifecycleEvent],
    ) -> Result<PreparedCommit, ProviderLifecycleRepositoryError> {
        let events = uncommitted_events(scoped_events, now_ms())?;
        let mut stream_ids = Vec::new();
        let mut seen_streams = HashSet::new();
        for event in &events {
            let stream_id = event.stream_id.clone();
            if seen_streams.insert(stream_id.clone()) {
                stream_ids.push(stream_id);
            }
        }
        let canonical = self
            .repository
            .canonical_event_batch_identity_v1(&events)
            .map_err(|_| ProviderLifecycleRepositoryError::Corrupt)?;
        let payload_hash: [u8; 32] = Sha256::digest(&canonical).into();
        let commit_hash: [u8; 32] =
            Sha256::digest([b"provider-lifecycle/v1\0".as_slice(), canonical.as_slice()].concat())
                .into();
        let identity = hex::encode(commit_hash);
        let commit_id = CommitIdentity::parse(&identity)
            .map_err(|_| ProviderLifecycleRepositoryError::Corrupt)?;
        Ok(PreparedCommit {
            identity,
            commit_id,
            payload_hash,
            stream_ids,
            events,
        })
    }

    async fn load_expected_heads(
        &self,
        stream_ids: &[StreamId],
    ) -> Result<Vec<ExpectedStreamHead>, ProviderLifecycleRepositoryError> {
        let mut expected_heads = Vec::with_capacity(stream_ids.len());
        for stream_id in stream_ids {
            let head = self
                .repository
                .load_stream(LoadStreamRequest {
                    stream_id: stream_id.clone(),
                    after: None,
                    limit: 1,
                })
                .await
                .map_err(|_| ProviderLifecycleRepositoryError::StorageUnavailable)?
                .head;
            expected_heads.push(ExpectedStreamHead {
                stream_id: stream_id.clone(),
                expected: head,
            });
        }
        Ok(expected_heads)
    }

    async fn resolve_bounded(
        &self,
        identity: &CommitIdentity,
    ) -> Result<CommitResolution, ProviderLifecycleRepositoryError> {
        let mut retry_delay = Duration::from_millis(10);
        for attempt in 0..4 {
            match self.repository.resolve_commit(identity.clone()).await {
                Ok(resolution) => return Ok(resolution),
                Err(_) if attempt < 3 => {
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = retry_delay.saturating_mul(2);
                }
                Err(_) => return Err(ProviderLifecycleRepositoryError::StorageUnavailable),
            }
        }
        unreachable!("bounded resolution loop always returns")
    }

    fn take_pending(
        &self,
        key: &[u8; 32],
    ) -> Result<Option<PreparedCommit>, ProviderLifecycleRepositoryError> {
        self.pending
            .lock()
            .map_err(|_| ProviderLifecycleRepositoryError::Corrupt)
            .map(|mut pending| pending.remove(key))
    }

    fn restore_pending(
        &self,
        key: [u8; 32],
        commit: PreparedCommit,
    ) -> Result<(), ProviderLifecycleRepositoryError> {
        self.pending
            .lock()
            .map_err(|_| ProviderLifecycleRepositoryError::Corrupt)?
            .insert(key, commit);
        Ok(())
    }
}

fn uncommitted_events(
    scoped_events: &[ScopedProviderLifecycleEvent],
    occurred_at_ms: i64,
) -> Result<Vec<UncommittedDomainEvent>, ProviderLifecycleRepositoryError> {
    scoped_events
        .iter()
        .cloned()
        .map(|scoped| {
            let (scope, event) = scoped.into_parts();
            let stream_id = StreamId::provider_lifecycle(scope.agent_session_id())
                .map_err(|_| ProviderLifecycleRepositoryError::InvalidInput)?;
            Ok(UncommittedDomainEvent {
                stream_id,
                event: LocalDomainEvent::ProviderLifecycle(event),
                occurred_at_ms,
            })
        })
        .collect()
}

#[async_trait::async_trait]
impl ProviderLifecycleEventRepository for LocalProviderLifecycleEventRepository {
    async fn append(
        &self,
        events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleRepositoryError> {
        self.append_events(events).await
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}
