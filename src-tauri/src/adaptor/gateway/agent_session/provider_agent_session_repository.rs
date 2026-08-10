use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::domain::agent_session::aggregates::{
    AgentSession, AgentSessionLifecycle, AgentSessionLifecycleEvent, AgentSessionOrigin,
    AgentSessionRemovalAuthorization,
};
use crate::domain::agent_session::repository::{
    ProviderAgentSessionRepository, ProviderAgentSessionRepositoryError,
    VersionedProviderAgentSession,
};
use crate::domain::agent_session::services::ProviderSessionOwnership;
use crate::domain::agent_session::{
    ProviderAgentSessionHistoryGatewayError, ProviderAgentSessionOwnershipQuery,
};
use crate::domain::local_event::{
    CommitBatchError, CommitBatchResult, CommitIdentity, CommitOperationKind, ExpectedStreamHead,
    IdempotencyBinding, LoadStreamRequest, LocalAtomicBatch, LocalDomainEvent, LocalEventQuery,
    LocalEventQueryResult, LocalEventTransactionRepository, LocalStateMutation,
    ProviderAgentSessionLifecycleRecord, ProviderAgentSessionOriginRecord,
    ProviderAgentSessionProjectionRecord, ProviderAgentSessionProviderRecord,
    ProviderAgentSessionRemovalMutation, ProviderSessionOwnershipProjectionRecord, Revision,
    RevisionGuard, SessionProjectionMutation, SessionProjectionRecord,
    SessionProjectionRemovalMutation, StreamId, StreamVersion, UncommittedDomainEvent,
};
use crate::domain::provider_lifecycle::{ProviderKind, ScopedProviderLifecycleEvent};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::provider_lifecycle::ProviderSessionStartTransaction;

const STORAGE_PREFIX: &str = "provider-agent-session:";

pub(crate) struct LocalProviderAgentSessionRepository {
    repository: Arc<dyn LocalEventTransactionRepository>,
    installation_id: String,
}

impl LocalProviderAgentSessionRepository {
    pub(crate) fn new(
        repository: Arc<dyn LocalEventTransactionRepository>,
        installation_id: String,
    ) -> Self {
        Self {
            repository,
            installation_id,
        }
    }

    async fn create_atomic(
        &self,
        mut session: AgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        if caller_request_id.trim().is_empty() {
            return Err(ProviderAgentSessionRepositoryError::InvalidRequest);
        }
        let domain_events = session.take_uncommitted_events();
        if !matches!(
            domain_events.as_slice(),
            [AgentSessionLifecycleEvent::Created { .. }]
                | [
                    AgentSessionLifecycleEvent::Created { .. },
                    AgentSessionLifecycleEvent::InitialInstructionAdmitted
                ]
        ) {
            return Err(ProviderAgentSessionRepositoryError::InvalidRequest);
        }
        let revision_value = u64::try_from(domain_events.len())
            .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?;
        let revision = Revision::new(
            i64::try_from(revision_value)
                .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?,
        )
        .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?;
        let session_stream = StreamId::agent_session(session.id())
            .map_err(|_| ProviderAgentSessionRepositoryError::InvalidRequest)?;
        let occurred_at_ms = now_ms();
        let mut events = domain_events
            .into_iter()
            .map(|event| UncommittedDomainEvent {
                stream_id: session_stream.clone(),
                event: LocalDomainEvent::AgentSessionLifecycle(event),
                occurred_at_ms,
            })
            .collect::<Vec<_>>();
        let mut expected_heads = vec![ExpectedStreamHead {
            stream_id: session_stream,
            expected: StreamVersion::zero(),
        }];
        for scoped in lifecycle_events {
            let (scope, event) = scoped.into_parts();
            if scope.agent_session_id() != session.id() {
                return Err(ProviderAgentSessionRepositoryError::InvalidRequest);
            }
            let stream_id = StreamId::provider_lifecycle(scope.agent_session_id())
                .map_err(|_| ProviderAgentSessionRepositoryError::InvalidRequest)?;
            if !expected_heads
                .iter()
                .any(|expected| expected.stream_id == stream_id)
            {
                expected_heads.push(ExpectedStreamHead {
                    stream_id: stream_id.clone(),
                    expected: StreamVersion::zero(),
                });
            }
            events.push(UncommittedDomainEvent {
                stream_id,
                event: LocalDomainEvent::ProviderLifecycle(event),
                occurred_at_ms,
            });
        }
        let mutation = LocalStateMutation::SessionProjection(SessionProjectionMutation {
            session_id: storage_key(session.id()),
            projection: SessionProjectionRecord::ProviderAgentSession(projection(
                &session,
                session.initial_instruction_admitted(),
            )),
            expected: RevisionGuard::Absent,
            revision,
        });
        let (commit_id, payload_hash) = commit_identity(
            self.repository.as_ref(),
            caller_request_id,
            &events,
            std::slice::from_ref(&mutation),
        )?;
        let batch = LocalAtomicBatch {
            commit_id,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::SessionLifecycle,
                idempotency_key: format!("provider-agent-session.create.{caller_request_id}"),
                payload_hash,
            },
            expected_heads,
            events,
            state_mutations: vec![mutation],
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => Ok(
                VersionedProviderAgentSession::restored(session, revision_value),
            ),
            Err(
                CommitBatchError::PayloadConflict | CommitBatchError::StreamHeadConflict { .. },
            ) => Err(ProviderAgentSessionRepositoryError::AlreadyExists),
            Err(
                CommitBatchError::StorageUnavailable { .. }
                | CommitBatchError::OutcomeUnknown { .. }
                | CommitBatchError::EffectAdmissionBlocked,
            ) => Err(ProviderAgentSessionRepositoryError::Unavailable),
            Err(
                CommitBatchError::CapacityExceeded
                | CommitBatchError::SequenceExhausted
                | CommitBatchError::Corrupt { .. },
            ) => Err(ProviderAgentSessionRepositoryError::Corrupt),
        }
    }

    async fn save_with_lifecycle_events(
        &self,
        session: VersionedProviderAgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        if caller_request_id.trim().is_empty() {
            return Err(ProviderAgentSessionRepositoryError::InvalidRequest);
        }
        let previous_revision = session.revision();
        let mut session = session.into_session();
        let pending = session.uncommitted_events().to_vec();
        if (pending.is_empty() && lifecycle_events.is_empty())
            || pending
                .iter()
                .any(|event| matches!(event, AgentSessionLifecycleEvent::Created { .. }))
        {
            return Err(ProviderAgentSessionRepositoryError::InvalidRequest);
        }
        let revision_value = previous_revision
            .checked_add(
                u64::try_from(pending.len())
                    .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?,
            )
            .ok_or(ProviderAgentSessionRepositoryError::Corrupt)?;
        let association = pending.iter().find_map(|event| match event {
            AgentSessionLifecycleEvent::ProviderSessionAssociated {
                provider_session_id,
                ..
            } => Some(provider_session_id.clone()),
            _ => None,
        });
        let occurred_at_ms = now_ms();
        let mut events = Vec::new();
        let mut expected_heads = Vec::new();
        let mut mutations = Vec::new();

        if !pending.is_empty() {
            let session_stream = StreamId::agent_session(session.id())
                .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?;
            events.extend(pending.iter().cloned().map(|event| UncommittedDomainEvent {
                stream_id: session_stream.clone(),
                event: LocalDomainEvent::AgentSessionLifecycle(event),
                occurred_at_ms,
            }));
            expected_heads.push(ExpectedStreamHead {
                stream_id: session_stream,
                expected: stream_version(previous_revision)?,
            });
            mutations.push(LocalStateMutation::SessionProjection(
                SessionProjectionMutation {
                    session_id: storage_key(session.id()),
                    projection: SessionProjectionRecord::ProviderAgentSession(projection(
                        &session,
                        session.initial_instruction_admitted(),
                    )),
                    expected: RevisionGuard::Expected(revision(previous_revision)?),
                    revision: revision(revision_value)?,
                },
            ));
        }

        if let Some(provider_session_id) = association.as_deref() {
            let ownership_key = ownership_storage_key(session.provider(), provider_session_id);
            let (mut ownership, ownership_revision) = self
                .load_ownership(&ownership_key, session.provider(), provider_session_id)
                .await?;
            ownership.claim(session.id()).map_err(|owned| {
                ProviderAgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                    agent_session_id: owned.agent_session_id,
                }
            })?;
            let ownership_events = ownership.take_uncommitted_events();
            let ownership_next = ownership_revision
                .checked_add(
                    u64::try_from(ownership_events.len())
                        .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?,
                )
                .ok_or(ProviderAgentSessionRepositoryError::Corrupt)?;
            let ownership_stream = ownership_stream(session.provider(), provider_session_id)?;
            expected_heads.push(ExpectedStreamHead {
                stream_id: ownership_stream.clone(),
                expected: stream_version(ownership_revision)?,
            });
            events.extend(
                ownership_events
                    .into_iter()
                    .map(|event| UncommittedDomainEvent {
                        stream_id: ownership_stream.clone(),
                        event: LocalDomainEvent::ProviderSessionOwnership(event),
                        occurred_at_ms,
                    }),
            );
            mutations.push(LocalStateMutation::SessionProjection(
                SessionProjectionMutation {
                    session_id: ownership_key,
                    projection: SessionProjectionRecord::ProviderSessionOwnership(
                        ProviderSessionOwnershipProjectionRecord {
                            provider: provider_record(session.provider()),
                            provider_session_id: provider_session_id.to_string(),
                            agent_session_id: Some(session.id().to_string()),
                        },
                    ),
                    expected: if ownership_revision == 0 {
                        RevisionGuard::Absent
                    } else {
                        RevisionGuard::Expected(revision(ownership_revision)?)
                    },
                    revision: revision(ownership_next)?,
                },
            ));
        }

        for scoped in lifecycle_events {
            let (scope, event) = scoped.into_parts();
            let stream_id = StreamId::provider_lifecycle(scope.agent_session_id())
                .map_err(|_| ProviderAgentSessionRepositoryError::InvalidRequest)?;
            if !expected_heads
                .iter()
                .any(|expected| expected.stream_id == stream_id)
            {
                let head = self
                    .repository
                    .load_stream(LoadStreamRequest {
                        stream_id: stream_id.clone(),
                        after: None,
                        limit: 1,
                    })
                    .await
                    .map_err(|_| ProviderAgentSessionRepositoryError::Unavailable)?
                    .head;
                expected_heads.push(ExpectedStreamHead {
                    stream_id: stream_id.clone(),
                    expected: head,
                });
            }
            events.push(UncommittedDomainEvent {
                stream_id,
                event: LocalDomainEvent::ProviderLifecycle(event),
                occurred_at_ms,
            });
        }

        let (commit_id, payload_hash) = commit_identity(
            self.repository.as_ref(),
            caller_request_id,
            &events,
            &mutations,
        )?;
        let batch = LocalAtomicBatch {
            commit_id,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::SessionLifecycle,
                idempotency_key: format!("provider-agent-session.save.{caller_request_id}"),
                payload_hash,
            },
            expected_heads,
            events,
            state_mutations: mutations,
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {
                session.take_uncommitted_events();
                Ok(VersionedProviderAgentSession::restored(
                    session,
                    revision_value,
                ))
            }
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                if let Some(provider_session_id) = association.as_deref() {
                    let ownership_key =
                        ownership_storage_key(session.provider(), provider_session_id);
                    let (ownership, _) = self
                        .load_ownership(&ownership_key, session.provider(), provider_session_id)
                        .await?;
                    if let Some(owner) = ownership.agent_session_id() {
                        if owner != session.id() {
                            return Err(
                                ProviderAgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                                    agent_session_id: owner.to_string(),
                                },
                            );
                        }
                    }
                }
                Err(ProviderAgentSessionRepositoryError::Conflict)
            }
            Err(CommitBatchError::PayloadConflict) => {
                Err(ProviderAgentSessionRepositoryError::Conflict)
            }
            Err(
                CommitBatchError::StorageUnavailable { .. }
                | CommitBatchError::OutcomeUnknown { .. }
                | CommitBatchError::EffectAdmissionBlocked,
            ) => Err(ProviderAgentSessionRepositoryError::Unavailable),
            Err(
                CommitBatchError::CapacityExceeded
                | CommitBatchError::SequenceExhausted
                | CommitBatchError::Corrupt { .. },
            ) => Err(ProviderAgentSessionRepositoryError::Corrupt),
        }
    }
}

#[async_trait::async_trait]
impl ProviderAgentSessionRepository for LocalProviderAgentSessionRepository {
    async fn create(
        &self,
        session: AgentSession,
        caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        self.create_atomic(session, Vec::new(), caller_request_id)
            .await
    }

    async fn create_with_lifecycle_events(
        &self,
        session: AgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        self.create_atomic(session, lifecycle_events, caller_request_id)
            .await
    }

    async fn find(
        &self,
        session_id: &str,
    ) -> Result<Option<VersionedProviderAgentSession>, ProviderAgentSessionRepositoryError> {
        if session_id.trim().is_empty() {
            return Err(ProviderAgentSessionRepositoryError::InvalidRequest);
        }
        let result = self
            .repository
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: storage_key(session_id),
            })
            .await
            .map_err(|_| ProviderAgentSessionRepositoryError::Unavailable)?;
        let LocalEventQueryResult::SessionProjectionByIdentity(view) = result else {
            return Err(ProviderAgentSessionRepositoryError::Corrupt);
        };
        let Some(view) = view else {
            return Ok(None);
        };
        let SessionProjectionRecord::ProviderAgentSession(projection) = view.projection else {
            return Err(ProviderAgentSessionRepositoryError::Corrupt);
        };
        let session = restore(projection)?;
        let revision = u64::try_from(view.revision.value())
            .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?;
        Ok(Some(VersionedProviderAgentSession::restored(
            session, revision,
        )))
    }

    async fn save(
        &self,
        session: VersionedProviderAgentSession,
        caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        self.save_with_lifecycle_events(session, Vec::new(), caller_request_id)
            .await
    }

    async fn remove(
        &self,
        session: VersionedProviderAgentSession,
        authorization: AgentSessionRemovalAuthorization,
        caller_request_id: &str,
    ) -> Result<(), ProviderAgentSessionRepositoryError> {
        if caller_request_id.trim().is_empty() || !session.session().uncommitted_events().is_empty()
        {
            return Err(ProviderAgentSessionRepositoryError::InvalidRequest);
        }
        let previous_revision = session.revision();
        let session = session.into_session();
        let session_stream = StreamId::agent_session(session.id())
            .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?;
        let occurred_at_ms = now_ms();
        let events = vec![UncommittedDomainEvent {
            stream_id: session_stream.clone(),
            event: LocalDomainEvent::AgentSessionLifecycle(authorization.tombstone_event()),
            occurred_at_ms,
        }];
        let expected_heads = vec![ExpectedStreamHead {
            stream_id: session_stream.clone(),
            expected: stream_version(previous_revision)?,
        }];
        let mut mutations = vec![LocalStateMutation::SessionProjectionRemoval(
            SessionProjectionRemovalMutation {
                session_id: storage_key(session.id()),
                expected: RevisionGuard::Expected(revision(previous_revision)?),
            },
        )];
        let (ownership_projection_id, ownership_stream, ownership_expected) =
            if let Some(provider_session_id) = session.provider_session_id() {
                let ownership_key = ownership_storage_key(session.provider(), provider_session_id);
                let (ownership, ownership_revision) = self
                    .load_ownership(&ownership_key, session.provider(), provider_session_id)
                    .await?;
                if ownership.agent_session_id() != Some(session.id()) {
                    return Err(ProviderAgentSessionRepositoryError::Corrupt);
                }
                (
                    Some(ownership_key),
                    Some(ownership_stream(session.provider(), provider_session_id)?),
                    Some(revision(ownership_revision)?),
                )
            } else {
                (None, None, None)
            };
        let retained_tombstone_sequence = previous_revision
            .checked_add(1)
            .ok_or(ProviderAgentSessionRepositoryError::Corrupt)?;
        mutations.push(LocalStateMutation::ProviderAgentSessionRemoval(
            ProviderAgentSessionRemovalMutation {
                agent_session_stream: session_stream,
                retained_tombstone_sequence: stream_version(retained_tombstone_sequence)?,
                ownership_projection_id,
                ownership_stream,
                ownership_expected,
            },
        ));

        let (commit_id, payload_hash) = commit_identity(
            self.repository.as_ref(),
            caller_request_id,
            &events,
            &mutations,
        )?;
        let batch = LocalAtomicBatch {
            commit_id,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::SessionLifecycle,
                idempotency_key: format!("provider-agent-session.remove.{caller_request_id}"),
                payload_hash,
            },
            expected_heads,
            events,
            state_mutations: mutations,
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => Ok(()),
            Err(
                CommitBatchError::PayloadConflict | CommitBatchError::StreamHeadConflict { .. },
            ) => Err(ProviderAgentSessionRepositoryError::Conflict),
            Err(
                CommitBatchError::StorageUnavailable { .. }
                | CommitBatchError::OutcomeUnknown { .. }
                | CommitBatchError::EffectAdmissionBlocked,
            ) => Err(ProviderAgentSessionRepositoryError::Unavailable),
            Err(
                CommitBatchError::CapacityExceeded
                | CommitBatchError::SequenceExhausted
                | CommitBatchError::Corrupt { .. },
            ) => Err(ProviderAgentSessionRepositoryError::Corrupt),
        }
    }
}

#[async_trait::async_trait]
impl ProviderSessionStartTransaction for LocalProviderAgentSessionRepository {
    async fn commit_session_started(
        &self,
        session: VersionedProviderAgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        self.save_with_lifecycle_events(session, lifecycle_events, caller_request_id)
            .await
    }
}

#[async_trait::async_trait]
impl ProviderAgentSessionOwnershipQuery for LocalProviderAgentSessionRepository {
    async fn is_owned(
        &self,
        provider: ProviderKind,
        provider_session_id: &str,
    ) -> Result<bool, ProviderAgentSessionHistoryGatewayError> {
        if provider_session_id.trim().is_empty() {
            return Err(ProviderAgentSessionHistoryGatewayError::InvalidRequest);
        }
        self.load_ownership(
            &ownership_storage_key(provider, provider_session_id),
            provider,
            provider_session_id,
        )
        .await
        .map(|(ownership, _)| ownership.agent_session_id().is_some())
        .map_err(|error| match error {
            ProviderAgentSessionRepositoryError::InvalidRequest => {
                ProviderAgentSessionHistoryGatewayError::InvalidRequest
            }
            ProviderAgentSessionRepositoryError::Corrupt => {
                ProviderAgentSessionHistoryGatewayError::Corrupt
            }
            ProviderAgentSessionRepositoryError::AlreadyExists
            | ProviderAgentSessionRepositoryError::Conflict
            | ProviderAgentSessionRepositoryError::ProviderSessionAlreadyOwned { .. }
            | ProviderAgentSessionRepositoryError::Unavailable => {
                ProviderAgentSessionHistoryGatewayError::Unavailable
            }
        })
    }
}

impl LocalProviderAgentSessionRepository {
    async fn load_ownership(
        &self,
        storage_key: &str,
        provider: ProviderKind,
        provider_session_id: &str,
    ) -> Result<(ProviderSessionOwnership, u64), ProviderAgentSessionRepositoryError> {
        let result = self
            .repository
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: storage_key.to_string(),
            })
            .await
            .map_err(|_| ProviderAgentSessionRepositoryError::Unavailable)?;
        let LocalEventQueryResult::SessionProjectionByIdentity(view) = result else {
            return Err(ProviderAgentSessionRepositoryError::Corrupt);
        };
        let Some(view) = view else {
            return ProviderSessionOwnership::restore(provider, provider_session_id, None)
                .map(|ownership| (ownership, 0))
                .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt);
        };
        let SessionProjectionRecord::ProviderSessionOwnership(projection) = view.projection else {
            return Err(ProviderAgentSessionRepositoryError::Corrupt);
        };
        if projection.provider != provider_record(provider)
            || projection.provider_session_id != provider_session_id
        {
            return Err(ProviderAgentSessionRepositoryError::Corrupt);
        }
        let ownership = ProviderSessionOwnership::restore(
            provider,
            provider_session_id,
            projection.agent_session_id.as_deref(),
        )
        .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?;
        let revision = u64::try_from(view.revision.value())
            .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?;
        Ok((ownership, revision))
    }
}

fn storage_key(session_id: &str) -> String {
    format!("{STORAGE_PREFIX}{session_id}")
}

fn ownership_storage_key(provider: ProviderKind, provider_session_id: &str) -> String {
    let digest = Sha256::digest(provider_session_id.as_bytes());
    format!(
        "provider-session-ownership:{}:{}",
        provider_label(provider),
        hex::encode(digest)
    )
}

fn ownership_stream(
    provider: ProviderKind,
    provider_session_id: &str,
) -> Result<StreamId, ProviderAgentSessionRepositoryError> {
    let digest = Sha256::digest(provider_session_id.as_bytes());
    StreamId::provider_session_ownership(provider_label(provider), &hex::encode(digest))
        .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)
}

fn provider_label(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Claude => "claude",
        ProviderKind::Codex => "codex",
    }
}

fn revision(value: u64) -> Result<Revision, ProviderAgentSessionRepositoryError> {
    let value = i64::try_from(value).map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?;
    Revision::new(value).map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)
}

fn stream_version(value: u64) -> Result<StreamVersion, ProviderAgentSessionRepositoryError> {
    let value = i64::try_from(value).map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?;
    StreamVersion::new(value).map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)
}

fn projection(
    session: &AgentSession,
    initial_instruction_admitted: bool,
) -> ProviderAgentSessionProjectionRecord {
    ProviderAgentSessionProjectionRecord {
        id: session.id().to_string(),
        workspace_identity: session.workspace().as_str().to_string(),
        worktree_path: session.worktree_path().to_string(),
        provider: provider_record(session.provider()),
        origin: origin_record(session.origin()),
        lifecycle: lifecycle_record(session.lifecycle()),
        provider_session_id: session.provider_session_id().map(str::to_string),
        transcript_ref: session.transcript_ref().map(str::to_string),
        initial_instruction_admitted,
        last_exit_abnormal: session.last_exit_abnormal(),
    }
}

fn restore(
    projection: ProviderAgentSessionProjectionRecord,
) -> Result<AgentSession, ProviderAgentSessionRepositoryError> {
    let origin = match projection.origin {
        ProviderAgentSessionOriginRecord::Standalone => AgentSessionOrigin::Standalone,
        ProviderAgentSessionOriginRecord::WorkflowNode {
            workflow_execution_id,
            node_execution_id,
        } => AgentSessionOrigin::workflow_node(workflow_execution_id, node_execution_id)
            .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?,
    };
    let mut events = vec![AgentSessionLifecycleEvent::Created {
        id: projection.id,
        workspace: WorkspaceIdentity::new(projection.workspace_identity),
        worktree_path: projection.worktree_path,
        provider: provider_kind(projection.provider),
        origin,
    }];
    if let Some(provider_session_id) = projection.provider_session_id {
        events.push(AgentSessionLifecycleEvent::ProviderSessionAssociated {
            provider_session_id,
            transcript_ref: projection.transcript_ref,
        });
    } else if projection.transcript_ref.is_some() {
        return Err(ProviderAgentSessionRepositoryError::Corrupt);
    }
    if projection.lifecycle != ProviderAgentSessionLifecycleRecord::Open {
        events.push(AgentSessionLifecycleEvent::LifecycleChanged {
            lifecycle: match projection.lifecycle {
                ProviderAgentSessionLifecycleRecord::Open => unreachable!(),
                ProviderAgentSessionLifecycleRecord::Paused => AgentSessionLifecycle::Paused,
                ProviderAgentSessionLifecycleRecord::Archived => AgentSessionLifecycle::Archived,
            },
            last_exit_abnormal: projection.last_exit_abnormal,
        });
    }
    if projection.initial_instruction_admitted {
        events.push(AgentSessionLifecycleEvent::InitialInstructionAdmitted);
    }
    AgentSession::rehydrate(&events).map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)
}

fn provider_record(provider: ProviderKind) -> ProviderAgentSessionProviderRecord {
    match provider {
        ProviderKind::Claude => ProviderAgentSessionProviderRecord::Claude,
        ProviderKind::Codex => ProviderAgentSessionProviderRecord::Codex,
    }
}

fn provider_kind(provider: ProviderAgentSessionProviderRecord) -> ProviderKind {
    match provider {
        ProviderAgentSessionProviderRecord::Claude => ProviderKind::Claude,
        ProviderAgentSessionProviderRecord::Codex => ProviderKind::Codex,
    }
}

fn origin_record(origin: &AgentSessionOrigin) -> ProviderAgentSessionOriginRecord {
    match origin {
        AgentSessionOrigin::Standalone => ProviderAgentSessionOriginRecord::Standalone,
        AgentSessionOrigin::WorkflowNode {
            workflow_execution_id,
            node_execution_id,
        } => ProviderAgentSessionOriginRecord::WorkflowNode {
            workflow_execution_id: workflow_execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
        },
    }
}

fn lifecycle_record(lifecycle: AgentSessionLifecycle) -> ProviderAgentSessionLifecycleRecord {
    match lifecycle {
        AgentSessionLifecycle::Open => ProviderAgentSessionLifecycleRecord::Open,
        AgentSessionLifecycle::Paused => ProviderAgentSessionLifecycleRecord::Paused,
        AgentSessionLifecycle::Archived => ProviderAgentSessionLifecycleRecord::Archived,
    }
}

fn commit_identity(
    repository: &dyn LocalEventTransactionRepository,
    caller_request_id: &str,
    events: &[UncommittedDomainEvent],
    mutations: &[LocalStateMutation],
) -> Result<(CommitIdentity, [u8; 32]), ProviderAgentSessionRepositoryError> {
    let mut canonical = repository
        .canonical_event_batch_identity_v1(events)
        .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?;
    for mutation in mutations {
        let identity = repository
            .canonical_mutation_identity_v1(mutation)
            .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?;
        canonical.extend_from_slice(&(identity.len() as u64).to_be_bytes());
        canonical.extend_from_slice(&identity);
    }
    let payload_hash: [u8; 32] = Sha256::digest(&canonical).into();
    let identity: [u8; 32] = Sha256::digest(
        [
            b"provider-agent-session/v1\0".as_slice(),
            caller_request_id.as_bytes(),
            b"\0",
            canonical.as_slice(),
        ]
        .concat(),
    )
    .into();
    let commit_id = CommitIdentity::parse(&hex::encode(identity))
        .map_err(|_| ProviderAgentSessionRepositoryError::Corrupt)?;
    Ok((commit_id, payload_hash))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}
