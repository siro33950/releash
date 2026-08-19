use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::adaptor::gateway::local_event_store::writer::PreparedNodeEvent;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::adaptor::gateway::workflow::fact_log;
use crate::domain::agent_session::aggregates::{
    AgentSession, AgentSessionLifecycle, AgentSessionLifecycleEvent,
    AgentSessionRemovalAuthorization, AgentSessionTreeParent,
};
use crate::domain::agent_session::repository::{
    AgentSessionRepository, AgentSessionRepositoryError, VersionedAgentSession,
};
use crate::domain::agent_session::{
    AgentSessionHistoryGatewayError, AgentSessionOwnershipQuery, ProviderSessionOwnership,
    ProviderSessionOwnershipEvent,
};
use crate::domain::local_event::{
    AgentSessionRemovalMutation, CommitBatchError, CommitBatchResult, CommitIdentity,
    CommitOperationKind, ExpectedStreamHead, IdempotencyBinding, LoadStreamRequest,
    LocalAtomicBatch, LocalDomainEvent, LocalEventQuery, LocalEventQueryResult,
    LocalEventTransactionRepository, LocalStateMutation, ProviderSessionOwnershipProjectionRecord,
    Revision, RevisionGuard, SessionProjectionMutation, SessionProjectionRecord, StreamId,
    StreamVersion, UncommittedDomainEvent,
};
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleEvent, ScopedProviderLifecycleEvent,
};
use crate::domain::workflow::{
    ExecutionOrigin, NodeFact, NodeFactMeta, NodeFactRecord, NodeKindName, ProcessExitedFact,
    SessionAttachedFact, SessionRootFact, StartedFact, TreeRootFact,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::provider_lifecycle::ProviderSessionStartTransaction;

/// 統一 Node 事実ログを正とする AgentSession repository。
///
/// session の永続状態（生成・provider 参照・lifecycle・archive）は実行木の
/// 事実（node_events）として記録され、読み出しは行の走査による導出。
/// provider session の所有権（ownership）と provider lifecycle は従来の
/// commit 機構のまま（実世界突合の材料としてスコープ外で存続）。
pub(crate) struct LocalAgentSessionRepository {
    repository: Arc<dyn LocalEventTransactionRepository>,
    installation_id: String,
    store: Arc<LocalEventStore>,
}

/// session の事実が載る木上の位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionLocation {
    pub(crate) tree_id: String,
    pub(crate) node_execution_id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) node_name: String,
    pub(crate) attempt: u32,
}

impl SessionLocation {
    fn meta(&self) -> NodeFactMeta {
        NodeFactMeta {
            tree_id: self.tree_id.clone(),
            node_execution_id: self.node_execution_id.clone(),
            parent_id: self.parent_id.clone(),
            node_name: self.node_name.clone(),
            kind: NodeKindName::Session,
            attempt: self.attempt,
        }
    }
}

impl LocalAgentSessionRepository {
    pub(crate) fn new(store: Arc<LocalEventStore>) -> Self {
        let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
        let installation_id = store.installation_id().to_string();
        Self {
            repository,
            installation_id,
            store,
        }
    }

    fn locate(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionLocation>, AgentSessionRepositoryError> {
        locate_session(
            &fact_log::FactLogReadBackend::Live(Arc::clone(&self.store)),
            session_id,
        )
    }

    fn session_event_rows(
        &self,
        location: &SessionLocation,
        session_id: &str,
        session: &AgentSession,
        events: &[AgentSessionLifecycleEvent],
        persisted_lifecycle: Option<AgentSessionLifecycle>,
    ) -> Result<Vec<PreparedNodeEvent>, AgentSessionRepositoryError> {
        let meta = location.meta();
        let timestamp_ms = now_ms();
        let mut rows = Vec::new();
        for event in events {
            let fact = match event {
                AgentSessionLifecycleEvent::Created { .. } => {
                    // 生成の事実は create 経路が root started として追記済み。
                    continue;
                }
                AgentSessionLifecycleEvent::ProviderSessionAssociated {
                    provider_session_id,
                    transcript_ref,
                } => NodeFact::SessionAttached(SessionAttachedFact {
                    session_id: session_id.to_string(),
                    provider_session_id: Some(provider_session_id.clone()),
                    transcript_ref: transcript_ref.clone(),
                    initial_instruction_admitted: session.initial_instruction_admitted(),
                }),
                AgentSessionLifecycleEvent::InitialInstructionAdmitted => {
                    NodeFact::SessionAttached(SessionAttachedFact {
                        session_id: session_id.to_string(),
                        provider_session_id: session.provider_session_id().map(str::to_string),
                        transcript_ref: session.transcript_ref().map(str::to_string),
                        initial_instruction_admitted: true,
                    })
                }
                AgentSessionLifecycleEvent::LifecycleChanged {
                    lifecycle,
                    last_exit_abnormal,
                } => match lifecycle {
                    AgentSessionLifecycle::Paused => NodeFact::ProcessExited(ProcessExitedFact {
                        exit_code: if *last_exit_abnormal {
                            session.last_exit_code()
                        } else {
                            Some(0)
                        },
                        result_summary: None,
                        failure_reason: last_exit_abnormal
                            .then(|| "provider process exited abnormally".to_string()),
                        failure_kind: None,
                    }),
                    AgentSessionLifecycle::Open => Self::lifecycle_open_fact(
                        persisted_lifecycle.unwrap_or(AgentSessionLifecycle::Paused),
                    ),
                    AgentSessionLifecycle::Archived => NodeFact::ArchiveRequested,
                },
            };
            let pending = fact_log::pending_single_fact(&meta, &fact, timestamp_ms)
                .map_err(|_| AgentSessionRepositoryError::Corrupt)?;
            rows.push(PreparedNodeEvent {
                row: pending.row,
                timestamp_ms: pending.timestamp_ms,
                expect_tree_absent: false,
            });
        }
        Ok(rows)
    }

    /// archive からの復帰は restore_requested として記録する（LifecycleChanged
    /// Open の既定は resume_requested のため、呼び出し前の lifecycle で分岐する）。
    fn lifecycle_open_fact(previous: AgentSessionLifecycle) -> NodeFact {
        match previous {
            AgentSessionLifecycle::Archived => NodeFact::RestoreRequested,
            _ => NodeFact::ResumeRequested,
        }
    }

    async fn commit_provider_batch(
        &self,
        session_id: &str,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        ownership: Option<OwnershipClaim>,
        node_events: Vec<PreparedNodeEvent>,
        caller_request_id: &str,
        operation: &str,
    ) -> Result<(), AgentSessionRepositoryError> {
        if lifecycle_events.is_empty() && ownership.is_none() && node_events.is_empty() {
            return Ok(());
        }
        let occurred_at_ms = now_ms();
        let mut events = Vec::new();
        let mut expected_heads = Vec::new();
        let mut mutations = Vec::new();
        if let Some(claim) = ownership {
            expected_heads.push(ExpectedStreamHead {
                stream_id: claim.stream.clone(),
                expected: stream_version(claim.previous_revision)?,
            });
            events.extend(
                claim
                    .events
                    .into_iter()
                    .map(|event| UncommittedDomainEvent {
                        stream_id: claim.stream.clone(),
                        event: LocalDomainEvent::ProviderSessionOwnership(event),
                        occurred_at_ms,
                    }),
            );
            mutations.push(LocalStateMutation::SessionProjection(
                SessionProjectionMutation {
                    session_id: claim.key,
                    projection: SessionProjectionRecord::ProviderSessionOwnership(claim.record),
                    expected: if claim.previous_revision == 0 {
                        RevisionGuard::Absent
                    } else {
                        RevisionGuard::Expected(revision(claim.previous_revision)?)
                    },
                    revision: revision(claim.next_revision)?,
                },
            ));
        }
        for scoped in lifecycle_events {
            let (scope, event) = scoped.into_parts();
            if scope.agent_session_id() != session_id {
                return Err(AgentSessionRepositoryError::InvalidRequest);
            }
            let stream_id = StreamId::provider_lifecycle(scope.agent_session_id())
                .map_err(|_| AgentSessionRepositoryError::InvalidRequest)?;
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
                    .map_err(|_| AgentSessionRepositoryError::Unavailable)?
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
            &node_events,
        )?;
        let batch = LocalAtomicBatch {
            commit_id,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::UserMutation,
                idempotency_key: format!("agent-session.{operation}.{caller_request_id}"),
                payload_hash,
            },
            expected_heads,
            events,
            state_mutations: mutations,
        };
        match self
            .store
            .commit_batch_with_node_events(batch, node_events)
            .await
        {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => Ok(()),
            Err(
                CommitBatchError::PayloadConflict | CommitBatchError::StreamHeadConflict { .. },
            ) => Err(AgentSessionRepositoryError::Conflict),
            Err(
                CommitBatchError::StorageUnavailable { .. }
                | CommitBatchError::OutcomeUnknown { .. },
            ) => Err(AgentSessionRepositoryError::Unavailable),
            Err(
                CommitBatchError::CapacityExceeded
                | CommitBatchError::SequenceExhausted
                | CommitBatchError::Corrupt { .. },
            ) => Err(AgentSessionRepositoryError::Corrupt),
        }
    }

    async fn claim_ownership(
        &self,
        session: &AgentSession,
        provider_session_id: &str,
    ) -> Result<OwnershipClaim, AgentSessionRepositoryError> {
        let key = ownership_storage_key(session.provider(), provider_session_id);
        let (mut ownership, previous_revision) = self
            .load_ownership(&key, session.provider(), provider_session_id)
            .await?;
        if ownership.agent_session_id() == Some(session.id()) {
            return Err(AgentSessionRepositoryError::InvalidRequest);
        }
        ownership.claim(session.id()).map_err(|owned| {
            AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                agent_session_id: owned.agent_session_id,
            }
        })?;
        let events = ownership.take_uncommitted_events();
        let next_revision = previous_revision
            .checked_add(
                u64::try_from(events.len()).map_err(|_| AgentSessionRepositoryError::Corrupt)?,
            )
            .ok_or(AgentSessionRepositoryError::Corrupt)?;
        Ok(OwnershipClaim {
            key,
            stream: ownership_stream(session.provider(), provider_session_id)?,
            record: ProviderSessionOwnershipProjectionRecord {
                provider: provider_record(session.provider()),
                provider_session_id: provider_session_id.to_string(),
                agent_session_id: Some(session.id().to_string()),
            },
            events,
            previous_revision,
            next_revision,
        })
    }
}

struct OwnershipClaim {
    key: String,
    stream: StreamId,
    record: ProviderSessionOwnershipProjectionRecord,
    events: Vec<ProviderSessionOwnershipEvent>,
    previous_revision: u64,
    next_revision: u64,
}

#[async_trait::async_trait]
impl AgentSessionRepository for LocalAgentSessionRepository {
    async fn create(
        &self,
        session: AgentSession,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        self.create_with_lifecycle_events(session, Vec::new(), caller_request_id)
            .await
    }

    async fn create_with_lifecycle_events(
        &self,
        mut session: AgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        if caller_request_id.trim().is_empty() {
            return Err(AgentSessionRepositoryError::InvalidRequest);
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
            return Err(AgentSessionRepositoryError::InvalidRequest);
        }
        if session.tree_parent().is_none() {
            if let Some(location) = self.locate(session.id())? {
                let expected_id = crate::domain::agent_session::launch_resource_id(
                    "agent-session",
                    caller_request_id,
                )
                .ok_or(AgentSessionRepositoryError::InvalidRequest)?;
                if session.id() != expected_id {
                    return Err(AgentSessionRepositoryError::Conflict);
                }
                let records = fact_log::read_tree_records(&self.store, &location.tree_id)
                    .map_err(|_| AgentSessionRepositoryError::Unavailable)?;
                let existing = derive_session(session.id(), &location, &records)?;
                if existing.session().workspace() != session.workspace()
                    || existing.session().worktree_path() != session.worktree_path()
                    || existing.session().provider() != session.provider()
                {
                    return Err(AgentSessionRepositoryError::Conflict);
                }
                let rearm_request_id = format!(
                    "{caller_request_id}.rearm.{}",
                    lifecycle_binding_identity(&lifecycle_events)
                );
                self.commit_provider_batch(
                    session.id(),
                    lifecycle_events,
                    None,
                    Vec::new(),
                    &rearm_request_id,
                    "rearm",
                )
                .await?;
                return Ok(existing);
            }
        }
        if session.tree_parent().is_some() {
            let lifecycle_stream = StreamId::provider_lifecycle(session.id())
                .map_err(|_| AgentSessionRepositoryError::InvalidRequest)?;
            let persisted = self
                .repository
                .load_stream(LoadStreamRequest {
                    stream_id: lifecycle_stream,
                    after: None,
                    limit: 1,
                })
                .await
                .map_err(|_| AgentSessionRepositoryError::Unavailable)?
                .head
                .value()
                > 0;
            if persisted {
                let rearm_request_id = format!(
                    "{caller_request_id}.rearm.{}",
                    lifecycle_binding_identity(&lifecycle_events)
                );
                self.commit_provider_batch(
                    session.id(),
                    lifecycle_events,
                    None,
                    Vec::new(),
                    &rearm_request_id,
                    "rearm",
                )
                .await?;
                return Ok(VersionedAgentSession::restored(session, 1));
            }
        }
        let initial_instruction_admitted = domain_events.iter().any(|event| {
            matches!(
                event,
                AgentSessionLifecycleEvent::InitialInstructionAdmitted
            )
        });
        let mut node_events = Vec::new();
        if session.tree_parent().is_none() {
            let meta = NodeFactMeta {
                tree_id: session.id().to_string(),
                node_execution_id: session.id().to_string(),
                parent_id: None,
                node_name: "session".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
            };
            let fact = NodeFact::Started(StartedFact {
                parent: None,
                root: Some(TreeRootFact::Session(SessionRootFact {
                    workspace_identity: session.workspace().as_str().to_string(),
                    worktree_path: session.worktree_path().to_string(),
                    session: crate::domain::workflow::SessionSpec {
                        provider: session.provider(),
                        model: None,
                        permission: None,
                        facets: Default::default(),
                    },
                    created_from: ExecutionOrigin::DesktopUi,
                })),
            });
            let pending = fact_log::pending_single_fact(&meta, &fact, now_ms())
                .map_err(|_| AgentSessionRepositoryError::Corrupt)?;
            node_events.push(PreparedNodeEvent {
                row: pending.row,
                timestamp_ms: pending.timestamp_ms,
                expect_tree_absent: true,
            });
        }
        if initial_instruction_admitted && session.tree_parent().is_none() {
            let location = SessionLocation {
                tree_id: session.id().to_string(),
                node_execution_id: session.id().to_string(),
                parent_id: None,
                node_name: "session".to_string(),
                attempt: 1,
            };
            node_events.extend(self.session_event_rows(
                &location,
                session.id(),
                &session,
                &[AgentSessionLifecycleEvent::InitialInstructionAdmitted],
                None,
            )?);
        }
        self.commit_provider_batch(
            session.id(),
            lifecycle_events,
            None,
            node_events,
            caller_request_id,
            "create",
        )
        .await?;
        Ok(VersionedAgentSession::restored(session, 1))
    }

    async fn find(
        &self,
        session_id: &str,
    ) -> Result<Option<VersionedAgentSession>, AgentSessionRepositoryError> {
        if session_id.trim().is_empty() {
            return Err(AgentSessionRepositoryError::InvalidRequest);
        }
        let Some(location) = self.locate(session_id)? else {
            return Ok(None);
        };
        let records = fact_log::read_tree_records(&self.store, &location.tree_id)
            .map_err(|_| AgentSessionRepositoryError::Unavailable)?;
        derive_session(session_id, &location, &records).map(Some)
    }

    async fn save(
        &self,
        session: VersionedAgentSession,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        self.commit_session_started(session, Vec::new(), caller_request_id)
            .await
    }

    async fn remove(
        &self,
        session: VersionedAgentSession,
        _authorization: AgentSessionRemovalAuthorization,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionRepositoryError> {
        if caller_request_id.trim().is_empty() || !session.session().uncommitted_events().is_empty()
        {
            return Err(AgentSessionRepositoryError::InvalidRequest);
        }
        let session = session.into_session();
        if session.tree_parent().is_some() {
            return Err(AgentSessionRepositoryError::InvalidRequest);
        }
        let tree_id = session.id().to_string();
        let mut removal = AgentSessionRemovalMutation {
            node_event_tree_id: tree_id,
            ownership_projection_id: None,
            ownership_stream: None,
            ownership_expected: None,
        };
        // 所有権の解放（provider session を他 session が取り込めるように）。
        if let Some(provider_session_id) = session.provider_session_id() {
            let ownership_key = ownership_storage_key(session.provider(), provider_session_id);
            let (ownership, ownership_revision) = self
                .load_ownership(&ownership_key, session.provider(), provider_session_id)
                .await?;
            if ownership.agent_session_id() == Some(session.id()) {
                removal.ownership_projection_id = Some(ownership_key);
                removal.ownership_stream =
                    Some(ownership_stream(session.provider(), provider_session_id)?);
                removal.ownership_expected = Some(revision(ownership_revision)?);
            }
        }
        let mutations = vec![LocalStateMutation::AgentSessionRemoval(removal)];
        let (commit_id, payload_hash) = commit_identity(
            self.repository.as_ref(),
            caller_request_id,
            &[],
            &mutations,
            &[],
        )?;
        let batch = LocalAtomicBatch {
            commit_id,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::UserMutation,
                idempotency_key: format!("agent-session.remove.{caller_request_id}"),
                payload_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: mutations,
        };
        match self.repository.commit_batch(batch).await {
            Ok(_) => Ok(()),
            Err(
                CommitBatchError::StorageUnavailable { .. }
                | CommitBatchError::OutcomeUnknown { .. },
            ) => Err(AgentSessionRepositoryError::Unavailable),
            Err(_) => Err(AgentSessionRepositoryError::Conflict),
        }
    }
}

#[async_trait::async_trait]
impl ProviderSessionStartTransaction for LocalAgentSessionRepository {
    async fn commit_session_started(
        &self,
        session: VersionedAgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        if caller_request_id.trim().is_empty() {
            return Err(AgentSessionRepositoryError::InvalidRequest);
        }
        let previous_revision = session.revision();
        let mut session = session.into_session();
        let pending = session.take_uncommitted_events();
        if (pending.is_empty() && lifecycle_events.is_empty())
            || pending
                .iter()
                .any(|event| matches!(event, AgentSessionLifecycleEvent::Created { .. }))
        {
            return Err(AgentSessionRepositoryError::InvalidRequest);
        }
        let Some(location) = self.locate(session.id())? else {
            return Err(AgentSessionRepositoryError::Conflict);
        };
        // Open への遷移は archive からの復帰かどうかで restore / resume を書き分ける。
        // 直前状態は永続化済みの事実列から導出する（集約はすでに遷移後）。
        let persisted_lifecycle = if pending.iter().any(|event| {
            matches!(
                event,
                AgentSessionLifecycleEvent::LifecycleChanged {
                    lifecycle: AgentSessionLifecycle::Open,
                    ..
                }
            )
        }) {
            let records = fact_log::read_tree_records(&self.store, &location.tree_id)
                .map_err(|_| AgentSessionRepositoryError::Unavailable)?;
            Some(
                derive_session(session.id(), &location, &records)?
                    .session()
                    .lifecycle(),
            )
        } else {
            None
        };
        // provider session の新規関連付けは所有権の CAS を先に通す。
        let claimed_provider_session_id = pending.iter().find_map(|event| match event {
            AgentSessionLifecycleEvent::ProviderSessionAssociated {
                provider_session_id,
                ..
            } => Some(provider_session_id.clone()),
            _ => None,
        });
        let ownership = match &claimed_provider_session_id {
            Some(provider_session_id) => {
                Some(self.claim_ownership(&session, provider_session_id).await?)
            }
            None => None,
        };
        let commit_result = self
            .commit_provider_batch(
                session.id(),
                lifecycle_events,
                ownership,
                self.session_event_rows(
                    &location,
                    session.id(),
                    &session,
                    &pending,
                    persisted_lifecycle,
                )?,
                caller_request_id,
                "save",
            )
            .await;
        if let Err(error) = commit_result {
            // CAS 敗北は勝者を読み直し、所有者付きの決定的な拒否として返す。
            if matches!(error, AgentSessionRepositoryError::Conflict) {
                if let Some(provider_session_id) = &claimed_provider_session_id {
                    let (current, _) = self
                        .load_ownership(
                            &ownership_storage_key(session.provider(), provider_session_id),
                            session.provider(),
                            provider_session_id,
                        )
                        .await?;
                    if let Some(owner) = current.agent_session_id() {
                        if owner != session.id() {
                            return Err(AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                                agent_session_id: owner.to_string(),
                            });
                        }
                    }
                }
            }
            return Err(error);
        }
        let revision_value = previous_revision.saturating_add(
            u64::try_from(pending.len()).map_err(|_| AgentSessionRepositoryError::Corrupt)?,
        );
        Ok(VersionedAgentSession::restored(session, revision_value))
    }
}

#[async_trait::async_trait]
impl AgentSessionOwnershipQuery for LocalAgentSessionRepository {
    async fn is_owned(
        &self,
        provider: ProviderKind,
        provider_session_id: &str,
    ) -> Result<bool, AgentSessionHistoryGatewayError> {
        if provider_session_id.trim().is_empty() {
            return Err(AgentSessionHistoryGatewayError::InvalidRequest);
        }
        self.load_ownership(
            &ownership_storage_key(provider, provider_session_id),
            provider,
            provider_session_id,
        )
        .await
        .map(|(ownership, _)| ownership.agent_session_id().is_some())
        .map_err(|error| match error {
            AgentSessionRepositoryError::InvalidRequest => {
                AgentSessionHistoryGatewayError::InvalidRequest
            }
            AgentSessionRepositoryError::Corrupt => AgentSessionHistoryGatewayError::Corrupt,
            AgentSessionRepositoryError::Conflict
            | AgentSessionRepositoryError::ProviderSessionAlreadyOwned { .. }
            | AgentSessionRepositoryError::Unavailable => {
                AgentSessionHistoryGatewayError::Unavailable
            }
        })
    }
}

impl LocalAgentSessionRepository {
    async fn load_ownership(
        &self,
        storage_key: &str,
        provider: ProviderKind,
        provider_session_id: &str,
    ) -> Result<(ProviderSessionOwnership, u64), AgentSessionRepositoryError> {
        let result = self
            .repository
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: storage_key.to_string(),
            })
            .await
            .map_err(|_| AgentSessionRepositoryError::Unavailable)?;
        let LocalEventQueryResult::SessionProjectionByIdentity(view) = result else {
            return Err(AgentSessionRepositoryError::Corrupt);
        };
        let Some(view) = view else {
            return ProviderSessionOwnership::restore(provider, provider_session_id, None)
                .map(|ownership| (ownership, 0))
                .map_err(|_| AgentSessionRepositoryError::Corrupt);
        };
        let SessionProjectionRecord::ProviderSessionOwnership(projection) = view.projection else {
            return Err(AgentSessionRepositoryError::Corrupt);
        };
        if projection.provider != provider_record(provider)
            || projection.provider_session_id != provider_session_id
        {
            return Err(AgentSessionRepositoryError::Corrupt);
        }
        let ownership = ProviderSessionOwnership::restore(
            provider,
            provider_session_id,
            projection.agent_session_id.as_deref(),
        )
        .map_err(|_| AgentSessionRepositoryError::Corrupt)?;
        let revision = u64::try_from(view.revision.value())
            .map_err(|_| AgentSessionRepositoryError::Corrupt)?;
        Ok((ownership, revision))
    }
}

/// session の位置解決: 単独 session は自分の木の root、workflow の子は
/// attach された node。
pub(crate) fn locate_session(
    backend: &fact_log::FactLogReadBackend,
    session_id: &str,
) -> Result<Option<SessionLocation>, AgentSessionRepositoryError> {
    let records = fact_log::read_tree_records_from(backend, session_id)
        .map_err(|_| AgentSessionRepositoryError::Unavailable)?;
    if let Some(first) = records.first() {
        if matches!(
            &first.fact,
            NodeFact::Started(started) if matches!(started.root, Some(TreeRootFact::Session(_)))
        ) {
            return Ok(Some(SessionLocation {
                tree_id: first.meta.tree_id.clone(),
                node_execution_id: first.meta.node_execution_id.clone(),
                parent_id: None,
                node_name: first.meta.node_name.clone(),
                attempt: first.meta.attempt,
            }));
        }
    }
    let Some((tree_id, node_execution_id)) = fact_log::find_session_attachment(backend, session_id)
        .map_err(|_| AgentSessionRepositoryError::Unavailable)?
    else {
        return Ok(None);
    };
    let records = fact_log::read_tree_records_from(backend, &tree_id)
        .map_err(|_| AgentSessionRepositoryError::Unavailable)?;
    let Some(row) = records
        .iter()
        .find(|record| record.meta.node_execution_id == node_execution_id)
    else {
        return Ok(None);
    };
    Ok(Some(SessionLocation {
        tree_id,
        node_execution_id,
        parent_id: row.meta.parent_id.clone(),
        node_name: row.meta.node_name.clone(),
        attempt: row.meta.attempt,
    }))
}

/// 事実行列から AgentSession を導出する。
pub(crate) fn derive_session(
    session_id: &str,
    location: &SessionLocation,
    records: &[NodeFactRecord],
) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
    let root = records
        .first()
        .and_then(|record| match &record.fact {
            NodeFact::Started(started) => started.root.as_ref(),
            _ => None,
        })
        .ok_or(AgentSessionRepositoryError::Corrupt)?;
    let (workspace_identity, worktree_path, provider, tree_parent) = match root {
        TreeRootFact::Session(session_root) => (
            session_root.workspace_identity.clone(),
            session_root.worktree_path.clone(),
            session_root.session.provider,
            None,
        ),
        TreeRootFact::Workflow(workflow_root) => {
            let provider = workflow_root
                .definition
                .node_by_name(&location.node_name)
                .and_then(|node| match &node.kind {
                    crate::domain::workflow::NodeKind::Session(spec) => Some(spec.provider),
                    _ => None,
                })
                .ok_or(AgentSessionRepositoryError::Corrupt)?;
            let tree_parent = AgentSessionTreeParent::new(
                location.tree_id.clone(),
                location.node_execution_id.clone(),
            )
            .map_err(|_| AgentSessionRepositoryError::Corrupt)?;
            (
                // workflow 木は workspace を持たない（workspace = 正規化 worktree）。
                WorkspaceIdentity::new(&workflow_root.worktree_path)
                    .as_str()
                    .to_string(),
                workflow_root.worktree_path.clone(),
                provider,
                Some(tree_parent),
            )
        }
    };

    // 状態導出の規則は domain（fact_replay）が所有する。
    let view = crate::domain::workflow::services::fact_replay::derive_session_facts(
        records,
        &location.node_execution_id,
        session_id,
    );
    let lifecycle = if view.archived {
        AgentSessionLifecycle::Archived
    } else if view.exited {
        AgentSessionLifecycle::Paused
    } else {
        AgentSessionLifecycle::Open
    };

    let mut session = AgentSession::create(
        session_id,
        WorkspaceIdentity::new(&workspace_identity),
        worktree_path.clone(),
        provider,
        tree_parent,
    )
    .map_err(|_| AgentSessionRepositoryError::Corrupt)?;
    session.take_uncommitted_events();
    if let Some(provider_session_id) = view.provider_session_id {
        session
            .associate_provider_session(provider_session_id, view.transcript_ref.as_deref())
            .map_err(|_| AgentSessionRepositoryError::Corrupt)?;
        session.take_uncommitted_events();
    }
    if view.initial_instruction_admitted {
        let _ = session.admit_initial_instruction();
        session.take_uncommitted_events();
    }
    session.restore_derived_lifecycle(lifecycle, view.last_exit_abnormal);
    let revision = records.last().map(|record| record.seq).unwrap_or(0);
    let revision = u64::try_from(revision).map_err(|_| AgentSessionRepositoryError::Corrupt)?;
    Ok(VersionedAgentSession::restored(session, revision))
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
) -> Result<StreamId, AgentSessionRepositoryError> {
    let digest = Sha256::digest(provider_session_id.as_bytes());
    StreamId::provider_session_ownership(provider_label(provider), &hex::encode(digest))
        .map_err(|_| AgentSessionRepositoryError::Corrupt)
}

fn provider_record(
    provider: ProviderKind,
) -> crate::domain::local_event::AgentSessionProviderRecord {
    match provider {
        ProviderKind::Claude => crate::domain::local_event::AgentSessionProviderRecord::Claude,
        ProviderKind::Codex => crate::domain::local_event::AgentSessionProviderRecord::Codex,
    }
}

fn provider_label(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Claude => "claude",
        ProviderKind::Codex => "codex",
    }
}

fn revision(value: u64) -> Result<Revision, AgentSessionRepositoryError> {
    let value = i64::try_from(value).map_err(|_| AgentSessionRepositoryError::Corrupt)?;
    Revision::new(value).map_err(|_| AgentSessionRepositoryError::Corrupt)
}

fn stream_version(value: u64) -> Result<StreamVersion, AgentSessionRepositoryError> {
    let value = i64::try_from(value).map_err(|_| AgentSessionRepositoryError::Corrupt)?;
    StreamVersion::new(value).map_err(|_| AgentSessionRepositoryError::Corrupt)
}

fn lifecycle_binding_identity(events: &[ScopedProviderLifecycleEvent]) -> String {
    let mut material = Vec::new();
    for scoped in events {
        let (_, event) = scoped.clone().into_parts();
        let binding_id = match event {
            ProviderLifecycleEvent::BindingArmed { binding_id, .. }
            | ProviderLifecycleEvent::SessionAssociated { binding_id, .. }
            | ProviderLifecycleEvent::TranscriptAssociated { binding_id, .. }
            | ProviderLifecycleEvent::StopObserved { binding_id }
            | ProviderLifecycleEvent::StopFailed { binding_id, .. }
            | ProviderLifecycleEvent::LifecycleUnavailable { binding_id, .. }
            | ProviderLifecycleEvent::BindingExpired { binding_id } => binding_id,
        };
        material.extend_from_slice(&(binding_id.len() as u64).to_be_bytes());
        material.extend_from_slice(binding_id.as_bytes());
    }
    hex::encode(Sha256::digest(material))
}

fn commit_identity(
    repository: &dyn LocalEventTransactionRepository,
    caller_request_id: &str,
    events: &[UncommittedDomainEvent],
    mutations: &[LocalStateMutation],
    node_events: &[PreparedNodeEvent],
) -> Result<(CommitIdentity, [u8; 32]), AgentSessionRepositoryError> {
    let mut canonical = repository
        .canonical_event_batch_identity_v1(events)
        .map_err(|_| AgentSessionRepositoryError::Corrupt)?;
    for mutation in mutations {
        let identity = repository
            .canonical_mutation_identity_v1(mutation)
            .map_err(|_| AgentSessionRepositoryError::Corrupt)?;
        canonical.extend_from_slice(&(identity.len() as u64).to_be_bytes());
        canonical.extend_from_slice(&identity);
    }
    for event in node_events {
        for value in [
            event.row.tree_id.as_bytes(),
            event.row.node_execution_id.as_bytes(),
            event
                .row
                .parent_id
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
            event.row.node_name.as_bytes(),
            event.row.kind.as_bytes(),
            event.row.event_type.as_bytes(),
            event.row.detail.as_bytes(),
        ] {
            canonical.extend_from_slice(&(value.len() as u64).to_be_bytes());
            canonical.extend_from_slice(value);
        }
        canonical.extend_from_slice(&event.row.attempt.to_be_bytes());
        canonical.push(u8::from(event.expect_tree_absent));
    }
    let payload_hash: [u8; 32] = Sha256::digest(&canonical).into();
    let identity: [u8; 32] = Sha256::digest(
        [
            b"agent-session/v1\0".as_slice(),
            caller_request_id.as_bytes(),
            b"\0",
            canonical.as_slice(),
        ]
        .concat(),
    )
    .into();
    let commit_id = CommitIdentity::parse(&hex::encode(identity))
        .map_err(|_| AgentSessionRepositoryError::Corrupt)?;
    Ok((commit_id, payload_hash))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}
