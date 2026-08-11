use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::domain::local_event::{
    AgentSessionProviderRecord, CommitBatchError, CommitBatchResult, CommitIdentity,
    CommitOperationKind, ExpectedStreamHead, IdempotencyBinding, LoadStreamRequest,
    LocalAtomicBatch, LocalDomainEvent, LocalEventQuery, LocalEventQueryResult,
    LocalEventTransactionRepository, LocalStateMutation, ProviderHookHealthProjectionRecord,
    Revision, RevisionGuard, SessionProjectionMutation, SessionProjectionRecord, StreamId,
    UncommittedDomainEvent,
};
use crate::domain::provider_lifecycle::{
    ProviderHookHealth, ProviderHookHealthEvent, ProviderHookHealthRepository,
    ProviderHookHealthRepositoryError, ProviderKind, ProviderLifecycleUnavailableReason,
    VersionedProviderHookHealth,
};

pub(crate) struct LocalProviderHookHealthRepository {
    repository: Arc<dyn LocalEventTransactionRepository>,
    installation_id: String,
}

impl LocalProviderHookHealthRepository {
    pub(crate) fn new(
        repository: Arc<dyn LocalEventTransactionRepository>,
        installation_id: String,
    ) -> Self {
        Self {
            repository,
            installation_id,
        }
    }
}

#[async_trait::async_trait]
impl ProviderHookHealthRepository for LocalProviderHookHealthRepository {
    async fn load(
        &self,
        provider: ProviderKind,
    ) -> Result<VersionedProviderHookHealth, ProviderHookHealthRepositoryError> {
        let result = self
            .repository
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: storage_key(provider),
            })
            .await
            .map_err(|_| ProviderHookHealthRepositoryError::StorageUnavailable)?;
        let LocalEventQueryResult::SessionProjectionByIdentity(view) = result else {
            return Err(ProviderHookHealthRepositoryError::Corrupt);
        };
        let Some(view) = view else {
            return Ok(VersionedProviderHookHealth::restored(
                ProviderHookHealth::new(provider),
                0,
            ));
        };
        let SessionProjectionRecord::ProviderHookHealth(projection) = view.projection else {
            return Err(ProviderHookHealthRepositoryError::Corrupt);
        };
        if projection.provider != provider_record(provider) {
            return Err(ProviderHookHealthRepositoryError::Corrupt);
        }
        let latest_launch_id = projection.latest_launch_id;
        let mut events = vec![ProviderHookHealthEvent::LaunchObserved {
            provider,
            launch_id: latest_launch_id.clone(),
        }];
        if projection.latest_launch_session_started {
            events.push(ProviderHookHealthEvent::SessionStartedObserved {
                provider,
                launch_id: latest_launch_id,
            });
        }
        match (projection.warning_launch_id, projection.warning_reason) {
            (None, None) => {}
            (Some(launch_id), Some(reason)) => {
                events.push(ProviderHookHealthEvent::WarningRecorded {
                    provider,
                    launch_id,
                    reason: parse_reason(&reason)?,
                });
            }
            _ => return Err(ProviderHookHealthRepositoryError::Corrupt),
        }
        let health = ProviderHookHealth::rehydrate(provider, &events)
            .ok_or(ProviderHookHealthRepositoryError::Corrupt)?;
        let revision = u64::try_from(view.revision.value())
            .map_err(|_| ProviderHookHealthRepositoryError::Corrupt)?;
        Ok(VersionedProviderHookHealth::restored(health, revision))
    }

    async fn save(
        &self,
        health: VersionedProviderHookHealth,
        caller_request_id: &str,
    ) -> Result<VersionedProviderHookHealth, ProviderHookHealthRepositoryError> {
        if caller_request_id.trim().is_empty() {
            return Err(ProviderHookHealthRepositoryError::InvalidInput);
        }
        let previous_revision = health.revision();
        let mut health = health.into_health();
        let pending = health.take_uncommitted_events();
        if pending.is_empty() {
            return Err(ProviderHookHealthRepositoryError::InvalidInput);
        }
        let next_revision = previous_revision
            .checked_add(
                u64::try_from(pending.len())
                    .map_err(|_| ProviderHookHealthRepositoryError::Corrupt)?,
            )
            .ok_or(ProviderHookHealthRepositoryError::Corrupt)?;
        let stream_id = StreamId::application();
        let head = self
            .repository
            .load_stream(LoadStreamRequest {
                stream_id: stream_id.clone(),
                after: None,
                limit: 1,
            })
            .await
            .map_err(|_| ProviderHookHealthRepositoryError::StorageUnavailable)?
            .head;
        let occurred_at_ms = now_ms();
        let events = pending
            .into_iter()
            .map(|event| UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: LocalDomainEvent::ProviderHookHealth(event),
                occurred_at_ms,
            })
            .collect::<Vec<_>>();
        let (warning_launch_id, warning_reason) = health
            .warning()
            .map(|(launch_id, reason)| {
                (
                    Some(launch_id.to_string()),
                    Some(reason_label(reason).to_string()),
                )
            })
            .unwrap_or((None, None));
        let mutation = LocalStateMutation::SessionProjection(SessionProjectionMutation {
            session_id: storage_key(health.provider()),
            projection: SessionProjectionRecord::ProviderHookHealth(
                ProviderHookHealthProjectionRecord {
                    provider: provider_record(health.provider()),
                    latest_launch_id: health
                        .latest_launch_id()
                        .ok_or(ProviderHookHealthRepositoryError::Corrupt)?
                        .to_string(),
                    latest_launch_session_started: health.latest_launch_session_started(),
                    warning_launch_id,
                    warning_reason,
                },
            ),
            expected: if previous_revision == 0 {
                RevisionGuard::Absent
            } else {
                RevisionGuard::Expected(revision(previous_revision)?)
            },
            revision: revision(next_revision)?,
        });
        let canonical_events = self
            .repository
            .canonical_event_batch_identity_v1(&events)
            .map_err(|_| ProviderHookHealthRepositoryError::Corrupt)?;
        let canonical_mutation = self
            .repository
            .canonical_mutation_identity_v1(&mutation)
            .map_err(|_| ProviderHookHealthRepositoryError::Corrupt)?;
        let payload_hash: [u8; 32] =
            Sha256::digest([canonical_events.as_slice(), canonical_mutation.as_slice()].concat())
                .into();
        let identity = Sha256::digest(
            [
                b"provider-hook-health/v1\0".as_slice(),
                caller_request_id.as_bytes(),
                payload_hash.as_slice(),
            ]
            .concat(),
        );
        let commit_id = CommitIdentity::parse(&hex::encode(identity))
            .map_err(|_| ProviderHookHealthRepositoryError::Corrupt)?;
        let batch = LocalAtomicBatch {
            commit_id,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::Projection,
                idempotency_key: format!("provider-hook-health.{caller_request_id}"),
                payload_hash,
            },
            expected_heads: vec![ExpectedStreamHead {
                stream_id,
                expected: head,
            }],
            events,
            state_mutations: vec![mutation],
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {
                Ok(VersionedProviderHookHealth::restored(health, next_revision))
            }
            Err(
                CommitBatchError::PayloadConflict | CommitBatchError::StreamHeadConflict { .. },
            ) => Err(ProviderHookHealthRepositoryError::Conflict),
            Err(
                CommitBatchError::StorageUnavailable { .. }
                | CommitBatchError::OutcomeUnknown { .. },
            ) => Err(ProviderHookHealthRepositoryError::StorageUnavailable),
            Err(
                CommitBatchError::CapacityExceeded
                | CommitBatchError::SequenceExhausted
                | CommitBatchError::Corrupt { .. },
            ) => Err(ProviderHookHealthRepositoryError::Corrupt),
        }
    }
}

fn storage_key(provider: ProviderKind) -> String {
    format!("provider-hook-health:{}", provider_label(provider))
}

fn provider_label(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Claude => "claude",
        ProviderKind::Codex => "codex",
    }
}

fn provider_record(provider: ProviderKind) -> AgentSessionProviderRecord {
    match provider {
        ProviderKind::Claude => AgentSessionProviderRecord::Claude,
        ProviderKind::Codex => AgentSessionProviderRecord::Codex,
    }
}

fn reason_label(reason: ProviderLifecycleUnavailableReason) -> &'static str {
    match reason {
        ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded => {
            "session_start_deadline_exceeded"
        }
        ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed => {
            "codex_hook_delivery_unconfirmed"
        }
        ProviderLifecycleUnavailableReason::ProviderHookConfigurationRejected => {
            "provider_hook_configuration_rejected"
        }
        ProviderLifecycleUnavailableReason::LocalApiUnavailable => "local_api_unavailable",
    }
}

fn parse_reason(
    reason: &str,
) -> Result<ProviderLifecycleUnavailableReason, ProviderHookHealthRepositoryError> {
    match reason {
        "session_start_deadline_exceeded" => {
            Ok(ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded)
        }
        "codex_hook_delivery_unconfirmed" => {
            Ok(ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed)
        }
        "provider_hook_configuration_rejected" => {
            Ok(ProviderLifecycleUnavailableReason::ProviderHookConfigurationRejected)
        }
        "local_api_unavailable" => Ok(ProviderLifecycleUnavailableReason::LocalApiUnavailable),
        _ => Err(ProviderHookHealthRepositoryError::Corrupt),
    }
}

fn revision(value: u64) -> Result<Revision, ProviderHookHealthRepositoryError> {
    let value = i64::try_from(value).map_err(|_| ProviderHookHealthRepositoryError::Corrupt)?;
    Revision::new(value).map_err(|_| ProviderHookHealthRepositoryError::Corrupt)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}
