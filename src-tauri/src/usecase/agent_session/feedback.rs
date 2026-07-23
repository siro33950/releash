//! Durable, session-scoped operation feedback (issues-1499 R-011/B-041..B-046).
//!
//! Feedback is represented by pending obligations in the permanent store so it
//! remains queryable even when the legacy session projection is unreadable. The
//! owner/prefix index gives each session an independent, cursor-protected page.

use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::domain::local_event::{
    AgentSessionNoticeOperationRecord, CommitBatchError, CommitBatchResult, CommitIdentity,
    CommitOperationKind, CommitResolution, FeedbackActionRecord, IdempotencyBinding,
    LocalAtomicBatch, LocalEventQuery, LocalEventQueryError, LocalEventQueryResult,
    LocalEventTransactionRepository, LocalStateMutation, ObligationMutation, ObligationRecord,
    ObligationStateRecord, PendingIndexEntry, PendingPartition, QueryCursor,
    RecoveryActionMutation, RecoveryAttemptRecord, RecoveryResultRecord, Revision, RevisionGuard,
    SafeOperationFailure, SessionOperationFailureKind,
};
use crate::usecase::agent_session::notice_state::AgentSessionNoticeOperation;

pub(crate) const MAX_FEEDBACK_PAGE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeedbackAction {
    Dismiss,
    RetryResolution,
}

impl FeedbackAction {
    fn label(self) -> &'static str {
        match self {
            Self::Dismiss => "dismiss",
            Self::RetryResolution => "retry_resolution",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeedbackActionIdentity {
    pub action: FeedbackAction,
    pub action_id: String,
    pub origin_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionFeedbackEntry {
    pub feedback_id: String,
    pub attempt_id: String,
    pub session_id: String,
    pub operation: AgentSessionNoticeOperation,
    pub revision: u64,
    pub actions: Vec<FeedbackAction>,
    pub action_identities: Vec<FeedbackActionIdentity>,
    pub failure: SafeOperationFailure,
    pub(crate) resolution_identity: Option<String>,
}

impl SessionFeedbackEntry {
    pub(crate) fn action_identity(&self, action: FeedbackAction) -> Option<&str> {
        self.action_identities
            .iter()
            .find(|identity| identity.action == action)
            .map(|identity| identity.action_id.as_str())
    }
}

/// A durable capacity slot for one exact operation attempt. Reservations use
/// a non-feedback ordered key, so they count against the 512-entry feedback
/// bound without becoming visible as unresolved failures before the attempt
/// actually fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionFeedbackReservation {
    pub feedback_id: String,
    pub attempt_id: String,
    pub session_id: String,
    pub operation: AgentSessionNoticeOperation,
    process_instance_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionFeedbackPage {
    pub entries: Vec<SessionFeedbackEntry>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FeedbackRetryOutcome {
    Resolved,
    Failed(Box<SessionFeedbackEntry>),
}

/// Executes only a previously reserved exact resolution identity. Admission
/// and result persistence stay in this usecase; implementations must provide
/// same-identity idempotency/readback and must not reconstruct an effect from
/// current session state.
#[async_trait::async_trait]
pub(crate) trait FeedbackResolutionPort: Send + Sync {
    async fn retry_exact_resolution(
        &self,
        resolution_identity: &str,
    ) -> Result<(), SafeOperationFailure>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FeedbackError {
    InvalidRequest,
    ShutdownInProgress,
    NotFound,
    RevisionConflict { current_revision: u64 },
    CapacityExceeded,
    CursorMismatch,
    CursorExpired,
    QueryBusy,
    DeadlineExceeded,
    ResponseTooLarge,
    StorageUnavailable { failure: SafeOperationFailure },
    OutcomeUnknown { feedback_id: String },
    Internal { correlation_id: String },
}

pub(crate) struct SessionFeedbackUsecase {
    repository: Arc<dyn LocalEventTransactionRepository>,
    generation_id: String,
    process_instance_id: String,
    resolution_port: Option<Arc<dyn FeedbackResolutionPort>>,
}

impl SessionFeedbackUsecase {
    pub(crate) fn new(
        repository: Arc<dyn LocalEventTransactionRepository>,
        generation_id: String,
    ) -> Self {
        Self {
            repository,
            generation_id,
            process_instance_id: uuid::Uuid::new_v4().to_string(),
            resolution_port: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_resolution_port(
        mut self,
        resolution_port: Arc<dyn FeedbackResolutionPort>,
    ) -> Self {
        self.resolution_port = Some(resolution_port);
        self
    }

    /// Reserve one of the globally bounded feedback slots before a read or
    /// mutation attempt performs any effect. The identity is deterministic,
    /// so retrying the exact attempt replays this reservation.
    pub(crate) async fn reserve_attempt(
        &self,
        session_id: &str,
        operation: AgentSessionNoticeOperation,
        attempt_id: &str,
    ) -> Result<SessionFeedbackReservation, FeedbackError> {
        if !valid_identity_component(session_id) || !valid_identity_component(attempt_id) {
            return Err(FeedbackError::InvalidRequest);
        }
        let feedback_id = feedback_identity(session_id, attempt_id);
        let record = encode_reservation(
            &feedback_id,
            attempt_id,
            session_id,
            operation,
            &self.process_instance_id,
        )?;
        let payload_hash = feedback_obligation_hash(&record);
        let commit_hash: [u8; 32] = Sha256::digest(
            format!(
                "feedback-reserve/v1\0{feedback_id}\0{}",
                hex::encode(payload_hash)
            )
            .as_bytes(),
        )
        .into();
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex::encode(commit_hash))
                .map_err(|_| internal("reserve-identity"))?,
            idempotency: IdempotencyBinding {
                generation_id: self.generation_id.clone(),
                operation_kind: CommitOperationKind::Recovery,
                idempotency_key: format!("feedback.reserve.{feedback_id}"),
                payload_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: feedback_id.clone(),
                record,
                pending: Some(PendingIndexEntry {
                    ordered_key: format!("feedback-slot:{session_id}:{feedback_id}"),
                    owner: session_id.to_string(),
                    partition: PendingPartition::Owner,
                    shutdown_plan: None,
                }),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).expect("zero revision"),
            })],
        };
        self.commit_resolved(batch, &feedback_id).await?;
        Ok(SessionFeedbackReservation {
            feedback_id,
            attempt_id: attempt_id.to_string(),
            session_id: session_id.to_string(),
            operation,
            process_instance_id: self.process_instance_id.clone(),
        })
    }

    /// Turn the exact reserved slot into visible unresolved feedback. The
    /// reservation and its 512-entry capacity are retained under the same
    /// identity; no post-effect capacity race is possible.
    pub(crate) async fn materialize_failure(
        &self,
        reservation: &SessionFeedbackReservation,
        failure: SafeOperationFailure,
        resolution_identity: Option<String>,
    ) -> Result<SessionFeedbackEntry, FeedbackError> {
        if failure.correlation_id.is_empty()
            || !valid_resolution_identity(resolution_identity.as_deref())
        {
            return Err(FeedbackError::InvalidRequest);
        }
        let record = encode_record(
            &reservation.feedback_id,
            &reservation.attempt_id,
            &reservation.session_id,
            reservation.operation,
            &failure,
            resolution_identity.as_deref(),
        )?;
        let payload_hash = feedback_obligation_hash(&record);
        let commit_hash: [u8; 32] = Sha256::digest(
            format!(
                "feedback-materialize/v1\0{}\0{}",
                reservation.feedback_id,
                hex::encode(payload_hash)
            )
            .as_bytes(),
        )
        .into();
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex::encode(commit_hash))
                .map_err(|_| internal("materialize-identity"))?,
            idempotency: IdempotencyBinding {
                generation_id: self.generation_id.clone(),
                operation_kind: CommitOperationKind::Recovery,
                idempotency_key: format!("feedback.materialize.{}", reservation.feedback_id),
                payload_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: reservation.feedback_id.clone(),
                record,
                pending: Some(PendingIndexEntry {
                    ordered_key: format!(
                        "feedback:{}:{}",
                        reservation.session_id, reservation.feedback_id
                    ),
                    owner: reservation.session_id.clone(),
                    partition: PendingPartition::Owner,
                    shutdown_plan: None,
                }),
                expected: RevisionGuard::Expected(
                    Revision::new(0).expect("zero reservation revision"),
                ),
                revision: Revision::new(1).expect("materialized revision"),
            })],
        };
        self.commit_resolved(batch, &reservation.feedback_id)
            .await?;
        let actions = actions(resolution_identity.is_some());
        Ok(SessionFeedbackEntry {
            feedback_id: reservation.feedback_id.clone(),
            attempt_id: reservation.attempt_id.clone(),
            session_id: reservation.session_id.clone(),
            operation: reservation.operation,
            revision: 1,
            action_identities: action_identities(&reservation.feedback_id, 1, &actions),
            actions,
            failure,
            resolution_identity,
        })
    }

    /// Settle an exact successful attempt without inspecting or clearing any
    /// other feedback identity.
    pub(crate) async fn complete_success(
        &self,
        reservation: &SessionFeedbackReservation,
    ) -> Result<(), FeedbackError> {
        let record = encode_reservation(
            &reservation.feedback_id,
            &reservation.attempt_id,
            &reservation.session_id,
            reservation.operation,
            &reservation.process_instance_id,
        )?;
        let payload_hash = feedback_obligation_hash(&record);
        let commit_hash: [u8; 32] =
            Sha256::digest(format!("feedback-success/v1\0{}", reservation.feedback_id).as_bytes())
                .into();
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex::encode(commit_hash))
                .map_err(|_| internal("success-identity"))?,
            idempotency: IdempotencyBinding {
                generation_id: self.generation_id.clone(),
                operation_kind: CommitOperationKind::Recovery,
                idempotency_key: format!("feedback.success.{}", reservation.feedback_id),
                payload_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: reservation.feedback_id.clone(),
                record,
                pending: None,
                expected: RevisionGuard::Expected(
                    Revision::new(0).expect("zero reservation revision"),
                ),
                revision: Revision::new(1).expect("completed revision"),
            })],
        };
        self.commit_resolved(batch, &reservation.feedback_id).await
    }

    /// Convert reservations left by an earlier process into visible
    /// outcome-unknown feedback. Current-process reservations are untouched,
    /// so startup reconciliation can safely overlap with new commands.
    pub(crate) async fn recover_abandoned_reservations(&self) -> Result<usize, FeedbackError> {
        let mut cursor = None;
        let mut abandoned = Vec::new();
        loop {
            let result = self
                .repository
                .query(LocalEventQuery::PendingRecoveryPage {
                    limit: MAX_FEEDBACK_PAGE,
                    partition: None,
                    owner: None,
                    ordered_key_prefix: Some("feedback-slot:".to_string()),
                    shutdown_plan: None,
                    cursor: cursor.map(QueryCursor::from_opaque),
                })
                .await
                .map_err(map_query_error)?;
            let LocalEventQueryResult::PendingRecoveryPage(page) = result else {
                return Err(internal("reservation-page-shape"));
            };
            for stored in page.entries {
                let reservation = decode_reservation(&stored.record)?;
                if stored.obligation_id != reservation.feedback_id || stored.revision.value() != 0 {
                    return Err(internal("reservation-record-mismatch"));
                }
                if reservation.process_instance_id != self.process_instance_id {
                    abandoned.push(reservation);
                }
            }
            cursor = page.next_cursor.map(|value| value.as_str().to_string());
            if cursor.is_none() {
                break;
            }
        }

        for reservation in &abandoned {
            let failure = SafeOperationFailure::new(
                SessionOperationFailureKind::OutcomeUnknown,
                true,
                "The session operation was interrupted before its result was saved.",
                format!("abandoned-{}", reservation.feedback_id),
            )
            .with_detail("Retry the operation or dismiss this feedback.");
            self.materialize_failure(reservation, failure, None).await?;
        }
        Ok(abandoned.len())
    }

    #[cfg(test)]
    pub(crate) async fn record_failure(
        &self,
        session_id: &str,
        operation: AgentSessionNoticeOperation,
        failure: SafeOperationFailure,
        _retry_resolution_available: bool,
    ) -> Result<SessionFeedbackEntry, FeedbackError> {
        // A legacy boolean cannot prove an exact effect identity. Keep it for
        // source compatibility, but never turn it into a retry capability.
        self.record_failure_with_resolution(session_id, operation, failure, None)
            .await
    }

    #[cfg(test)]
    pub(crate) async fn record_failure_with_resolution(
        &self,
        session_id: &str,
        operation: AgentSessionNoticeOperation,
        failure: SafeOperationFailure,
        resolution_identity: Option<String>,
    ) -> Result<SessionFeedbackEntry, FeedbackError> {
        if session_id.is_empty() || session_id.len() > 128 || failure.correlation_id.is_empty() {
            return Err(FeedbackError::InvalidRequest);
        }
        if !valid_resolution_identity(resolution_identity.as_deref()) {
            return Err(FeedbackError::InvalidRequest);
        }
        let attempt_id = failure.correlation_id.clone();
        let feedback_id = feedback_identity(session_id, &attempt_id);
        let record = encode_record(
            &feedback_id,
            &attempt_id,
            session_id,
            operation,
            &failure,
            resolution_identity.as_deref(),
        )?;
        let payload_hash = feedback_obligation_hash(&record);
        let commit_hash: [u8; 32] = Sha256::digest(
            format!(
                "feedback-create/v1\0{feedback_id}\0{}",
                hex::encode(payload_hash)
            )
            .as_bytes(),
        )
        .into();
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex::encode(commit_hash))
                .map_err(|_| internal("create-identity"))?,
            idempotency: IdempotencyBinding {
                generation_id: self.generation_id.clone(),
                operation_kind: CommitOperationKind::Recovery,
                idempotency_key: format!("feedback.create.{feedback_id}"),
                payload_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: feedback_id.clone(),
                record,
                pending: Some(PendingIndexEntry {
                    ordered_key: format!("feedback:{session_id}:{feedback_id}"),
                    owner: session_id.to_string(),
                    partition: PendingPartition::Owner,
                    shutdown_plan: None,
                }),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).expect("zero revision"),
            })],
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {
                let actions = actions(resolution_identity.is_some());
                let action_identities = action_identities(&feedback_id, 0, &actions);
                Ok(SessionFeedbackEntry {
                    feedback_id,
                    attempt_id,
                    session_id: session_id.to_string(),
                    operation,
                    revision: 0,
                    action_identities,
                    actions,
                    failure,
                    resolution_identity,
                })
            }
            Err(error) => Err(map_commit_error(error, &feedback_id)),
        }
    }

    pub(crate) async fn list(
        &self,
        session_id: &str,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<SessionFeedbackPage, FeedbackError> {
        if session_id.is_empty()
            || session_id.len() > 128
            || limit == 0
            || limit > MAX_FEEDBACK_PAGE
        {
            return Err(FeedbackError::InvalidRequest);
        }
        let result = self
            .repository
            .query(LocalEventQuery::PendingRecoveryPage {
                limit,
                partition: None,
                owner: Some(session_id.to_string()),
                ordered_key_prefix: Some(format!("feedback:{session_id}:")),
                shutdown_plan: None,
                cursor: cursor.map(QueryCursor::from_opaque),
            })
            .await
            .map_err(map_query_error)?;
        let LocalEventQueryResult::PendingRecoveryPage(page) = result else {
            return Err(internal("list-shape"));
        };
        let mut entries = Vec::with_capacity(page.entries.len());
        for stored in page.entries {
            let entry = decode_record(&stored.record, stored.revision.value() as u64)?;
            if entry.feedback_id != stored.obligation_id || entry.session_id != session_id {
                return Err(internal("list-owner"));
            }
            entries.push(entry);
        }
        Ok(SessionFeedbackPage {
            entries,
            next_cursor: page.next_cursor.map(|cursor| cursor.as_str().to_string()),
        })
    }

    pub(crate) async fn dismiss(
        &self,
        session_id: &str,
        feedback_id: &str,
        expected_revision: u64,
        action_id: &str,
    ) -> Result<(), FeedbackError> {
        let expected_revision_i64 =
            i64::try_from(expected_revision).map_err(|_| FeedbackError::InvalidRequest)?;
        let (current, pending) = self.load_stored_owned(session_id, feedback_id).await?;
        if !pending || current.revision != expected_revision {
            return Err(FeedbackError::RevisionConflict {
                current_revision: current.revision,
            });
        }
        if current.action_identity(FeedbackAction::Dismiss) != Some(action_id) {
            return Err(FeedbackError::InvalidRequest);
        }
        let revision =
            Revision::new(expected_revision_i64).map_err(|_| FeedbackError::InvalidRequest)?;
        let next = revision.next().ok_or(FeedbackError::InvalidRequest)?;
        let record = encode_record(
            feedback_id,
            &current.attempt_id,
            session_id,
            current.operation,
            &current.failure,
            current.resolution_identity.as_deref(),
        )?;
        let payload_hash = feedback_obligation_hash(&record);
        let commit_hash: [u8; 32] = Sha256::digest(
            format!("feedback-dismiss/v1\0{feedback_id}\0{}", next.value()).as_bytes(),
        )
        .into();
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex::encode(commit_hash))
                .map_err(|_| internal("dismiss-identity"))?,
            idempotency: IdempotencyBinding {
                generation_id: self.generation_id.clone(),
                operation_kind: CommitOperationKind::Recovery,
                idempotency_key: format!("feedback.dismiss.{feedback_id}.{}", next.value()),
                payload_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: feedback_id.to_string(),
                record,
                pending: None,
                expected: RevisionGuard::Expected(revision),
                revision: next,
            })],
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => Ok(()),
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                let (current, _) = self.load_stored_owned(session_id, feedback_id).await?;
                Err(FeedbackError::RevisionConflict {
                    current_revision: current.revision,
                })
            }
            Err(error) => Err(map_commit_error(error, feedback_id)),
        }
    }

    pub(crate) async fn retry_resolution(
        &self,
        session_id: &str,
        feedback_id: &str,
        expected_revision: u64,
        action_id: &str,
    ) -> Result<FeedbackRetryOutcome, FeedbackError> {
        let expected_revision_i64 =
            i64::try_from(expected_revision).map_err(|_| FeedbackError::InvalidRequest)?;
        let (current, pending) = self.load_stored_owned(session_id, feedback_id).await?;
        if !pending || current.revision != expected_revision {
            return Err(FeedbackError::RevisionConflict {
                current_revision: current.revision,
            });
        }
        if current.action_identity(FeedbackAction::RetryResolution) != Some(action_id) {
            return Err(FeedbackError::InvalidRequest);
        }
        let resolution_identity = current
            .resolution_identity
            .as_deref()
            .ok_or(FeedbackError::InvalidRequest)?;
        let port = self
            .resolution_port
            .as_ref()
            .ok_or(FeedbackError::InvalidRequest)?;

        // The controller preflight is advisory. Reserve the exact retry under
        // the writer's shutdown gate before any external I/O so a concurrent
        // quit returns ShutdownInProgress with zero resolution-port calls.
        // The deterministic action row also makes a response-loss retry reuse
        // the same external effect identity.
        let retry_binding: [u8; 32] = Sha256::digest(
            format!(
                "feedback-retry-binding/v1\0{session_id}\0{feedback_id}\0{expected_revision}\0{resolution_identity}"
            )
            .as_bytes(),
        )
        .into();
        let retry_action_id = format!("feedback-retry-{}", hex::encode(retry_binding));
        let retry_attempt = RecoveryAttemptRecord::FeedbackRetry {
            feedback_id: feedback_id.to_string(),
            origin_revision: expected_revision,
            resolution_identity: resolution_identity.to_string(),
            state: ObligationStateRecord::EffectReserved,
        };
        let reserve_commit_hash: [u8; 32] =
            Sha256::digest(format!("feedback-retry-reserve/v1\0{retry_action_id}").as_bytes())
                .into();
        let reserve = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex::encode(reserve_commit_hash))
                .map_err(|_| internal("retry-reserve-identity"))?,
            idempotency: IdempotencyBinding {
                generation_id: self.generation_id.clone(),
                operation_kind: CommitOperationKind::Recovery,
                idempotency_key: format!(
                    "feedback.retry.{feedback_id}.{expected_revision}.reserve"
                ),
                payload_hash: retry_binding,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![LocalStateMutation::RecoveryAction(RecoveryActionMutation {
                action_id: retry_action_id.clone(),
                binding_hash: retry_binding,
                attempt: retry_attempt.clone(),
                completed: None,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).expect("zero revision"),
            })],
        };
        match self.repository.commit_batch(reserve).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {}
            Err(error) => return Err(map_commit_error(error, feedback_id)),
        }

        // The obligation above was durably reserved before this external
        // effect. No repository transaction or session lock is held here.
        let result = port.retry_exact_resolution(resolution_identity).await;
        let revision =
            Revision::new(expected_revision_i64).map_err(|_| FeedbackError::InvalidRequest)?;
        let next = revision.next().ok_or(FeedbackError::InvalidRequest)?;
        let (record, pending, outcome) = match result {
            Ok(()) => {
                let record = encode_record(
                    feedback_id,
                    &current.attempt_id,
                    session_id,
                    current.operation,
                    &current.failure,
                    current.resolution_identity.as_deref(),
                )?;
                (record, None, FeedbackRetryOutcome::Resolved)
            }
            Err(failure) => {
                let record = encode_record(
                    feedback_id,
                    &current.attempt_id,
                    session_id,
                    current.operation,
                    &failure,
                    current.resolution_identity.as_deref(),
                )?;
                let actions = current.actions.clone();
                let entry = SessionFeedbackEntry {
                    failure,
                    revision: next.value() as u64,
                    action_identities: action_identities(
                        feedback_id,
                        next.value() as u64,
                        &actions,
                    ),
                    actions,
                    ..current.clone()
                };
                let pending = PendingIndexEntry {
                    ordered_key: format!("feedback:{session_id}:{feedback_id}"),
                    owner: session_id.to_string(),
                    partition: PendingPartition::Owner,
                    shutdown_plan: None,
                };
                (
                    record,
                    Some(pending),
                    FeedbackRetryOutcome::Failed(Box::new(entry)),
                )
            }
        };
        let resolved = matches!(&outcome, FeedbackRetryOutcome::Resolved);
        let completed = RecoveryResultRecord::FeedbackRetry {
            feedback_id: feedback_id.to_string(),
            resource_revision: next.value() as u64,
            resolved,
        };
        let mut payload_hasher = Sha256::new();
        payload_hasher.update(feedback_obligation_hash(&record));
        payload_hasher.update(retry_binding);
        payload_hasher.update(next.value().to_le_bytes());
        payload_hasher.update([u8::from(resolved)]);
        let payload_hash = payload_hasher.finalize().into();
        let commit_hash: [u8; 32] = Sha256::digest(
            format!("feedback-retry/v1\0{feedback_id}\0{}", next.value()).as_bytes(),
        )
        .into();
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex::encode(commit_hash))
                .map_err(|_| internal("retry-identity"))?,
            idempotency: IdempotencyBinding {
                generation_id: self.generation_id.clone(),
                operation_kind: CommitOperationKind::OperationProgress,
                idempotency_key: format!("feedback.retry.{feedback_id}.{}", next.value()),
                payload_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![
                LocalStateMutation::RecoveryAction(RecoveryActionMutation {
                    action_id: retry_action_id,
                    binding_hash: retry_binding,
                    attempt: retry_attempt,
                    completed: Some(completed),
                    expected: RevisionGuard::Expected(Revision::new(0).expect("zero revision")),
                    revision: Revision::new(1).expect("revision one"),
                }),
                LocalStateMutation::Obligation(ObligationMutation {
                    obligation_id: feedback_id.to_string(),
                    record,
                    pending,
                    expected: RevisionGuard::Expected(revision),
                    revision: next,
                }),
            ],
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => Ok(outcome),
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                let (saved, saved_pending) =
                    self.load_stored_owned(session_id, feedback_id).await?;
                if saved.revision == next.value() as u64 {
                    if saved_pending {
                        Ok(FeedbackRetryOutcome::Failed(Box::new(saved)))
                    } else {
                        Ok(FeedbackRetryOutcome::Resolved)
                    }
                } else {
                    Err(FeedbackError::RevisionConflict {
                        current_revision: saved.revision,
                    })
                }
            }
            Err(error) => Err(map_commit_error(error, feedback_id)),
        }
    }

    async fn load_stored_owned(
        &self,
        session_id: &str,
        feedback_id: &str,
    ) -> Result<(SessionFeedbackEntry, bool), FeedbackError> {
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: feedback_id.to_string(),
            })
            .await
            .map_err(map_query_error)?;
        let LocalEventQueryResult::ObligationByIdentity(Some(stored)) = result else {
            return Err(FeedbackError::NotFound);
        };
        let pending = stored.pending.is_some();
        let entry = decode_record(&stored.record, stored.revision.value() as u64)?;
        if entry.session_id != session_id || entry.feedback_id != feedback_id {
            return Err(FeedbackError::NotFound);
        }
        Ok((entry, pending))
    }

    async fn commit_resolved(
        &self,
        batch: LocalAtomicBatch,
        feedback_id: &str,
    ) -> Result<(), FeedbackError> {
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => Ok(()),
            Err(CommitBatchError::OutcomeUnknown { identity }) => {
                match self
                    .repository
                    .resolve_commit(identity)
                    .await
                    .map_err(map_query_error)?
                {
                    CommitResolution::Committed(_) => Ok(()),
                    CommitResolution::NotCommitted => Err(FeedbackError::OutcomeUnknown {
                        feedback_id: feedback_id.to_string(),
                    }),
                }
            }
            Err(error) => Err(map_commit_error(error, feedback_id)),
        }
    }
}

fn actions(retry: bool) -> Vec<FeedbackAction> {
    let mut actions = vec![FeedbackAction::Dismiss];
    if retry {
        actions.push(FeedbackAction::RetryResolution);
    }
    actions
}

fn action_identities(
    feedback_id: &str,
    origin_revision: u64,
    actions: &[FeedbackAction],
) -> Vec<FeedbackActionIdentity> {
    actions
        .iter()
        .copied()
        .map(|action| FeedbackActionIdentity {
            action,
            action_id: feedback_action_identity(feedback_id, origin_revision, action),
            origin_revision,
        })
        .collect()
}

fn feedback_action_identity(
    feedback_id: &str,
    origin_revision: u64,
    action: FeedbackAction,
) -> String {
    let digest: [u8; 32] = Sha256::digest(
        format!(
            "feedback-action/v1\0{feedback_id}\0{origin_revision}\0{}",
            action.label()
        )
        .as_bytes(),
    )
    .into();
    format!("feedback-action-{}", hex::encode(digest))
}

fn valid_identity_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}

fn valid_resolution_identity(identity: Option<&str>) -> bool {
    identity.map(valid_identity_component).unwrap_or(true)
}

fn feedback_identity(session_id: &str, correlation_id: &str) -> String {
    let digest: [u8; 32] =
        Sha256::digest(format!("feedback/v1\0{session_id}\0{correlation_id}").as_bytes()).into();
    format!("feedback-{}", hex::encode(digest))
}

fn operation_record(operation: AgentSessionNoticeOperation) -> AgentSessionNoticeOperationRecord {
    match operation {
        AgentSessionNoticeOperation::Send => AgentSessionNoticeOperationRecord::Send,
        AgentSessionNoticeOperation::LoadSession => AgentSessionNoticeOperationRecord::LoadSession,
        AgentSessionNoticeOperation::LoadOlder => AgentSessionNoticeOperationRecord::LoadOlder,
        AgentSessionNoticeOperation::CancelQueue => AgentSessionNoticeOperationRecord::CancelQueue,
        AgentSessionNoticeOperation::ResumeQueue => AgentSessionNoticeOperationRecord::ResumeQueue,
        AgentSessionNoticeOperation::CloseSession => {
            AgentSessionNoticeOperationRecord::CloseSession
        }
        AgentSessionNoticeOperation::RestoreSession => {
            AgentSessionNoticeOperationRecord::RestoreSession
        }
        AgentSessionNoticeOperation::ArchiveSession => {
            AgentSessionNoticeOperationRecord::ArchiveSession
        }
        AgentSessionNoticeOperation::ForkSession => AgentSessionNoticeOperationRecord::ForkSession,
        AgentSessionNoticeOperation::SetTitle => AgentSessionNoticeOperationRecord::SetTitle,
        AgentSessionNoticeOperation::RespondPermission => {
            AgentSessionNoticeOperationRecord::RespondPermission
        }
        AgentSessionNoticeOperation::SetBackend => AgentSessionNoticeOperationRecord::SetBackend,
    }
}

fn notice_operation(operation: AgentSessionNoticeOperationRecord) -> AgentSessionNoticeOperation {
    match operation {
        AgentSessionNoticeOperationRecord::Send => AgentSessionNoticeOperation::Send,
        AgentSessionNoticeOperationRecord::LoadSession => AgentSessionNoticeOperation::LoadSession,
        AgentSessionNoticeOperationRecord::LoadOlder => AgentSessionNoticeOperation::LoadOlder,
        AgentSessionNoticeOperationRecord::CancelQueue => AgentSessionNoticeOperation::CancelQueue,
        AgentSessionNoticeOperationRecord::ResumeQueue => AgentSessionNoticeOperation::ResumeQueue,
        AgentSessionNoticeOperationRecord::CloseSession => {
            AgentSessionNoticeOperation::CloseSession
        }
        AgentSessionNoticeOperationRecord::RestoreSession => {
            AgentSessionNoticeOperation::RestoreSession
        }
        AgentSessionNoticeOperationRecord::ArchiveSession => {
            AgentSessionNoticeOperation::ArchiveSession
        }
        AgentSessionNoticeOperationRecord::ForkSession => AgentSessionNoticeOperation::ForkSession,
        AgentSessionNoticeOperationRecord::SetTitle => AgentSessionNoticeOperation::SetTitle,
        AgentSessionNoticeOperationRecord::RespondPermission => {
            AgentSessionNoticeOperation::RespondPermission
        }
        AgentSessionNoticeOperationRecord::SetBackend => AgentSessionNoticeOperation::SetBackend,
    }
}

fn update_hash_field(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

fn feedback_obligation_hash(record: &ObligationRecord) -> [u8; 32] {
    let mut hasher = Sha256::new();
    match record {
        ObligationRecord::FeedbackReservation {
            feedback_id,
            attempt_id,
            session_id,
            operation,
            process_instance_id,
        } => {
            hasher.update(b"session-feedback-reservation/v1\0");
            for value in [feedback_id, attempt_id, session_id, process_instance_id] {
                update_hash_field(&mut hasher, value);
            }
            update_hash_field(&mut hasher, notice_operation(*operation).label());
        }
        ObligationRecord::Feedback {
            feedback_id,
            attempt_id,
            session_id,
            operation,
            actions,
            resolution_identity,
            failure,
        } => {
            hasher.update(b"session-feedback/v1\0");
            for value in [feedback_id, attempt_id, session_id] {
                update_hash_field(&mut hasher, value);
            }
            update_hash_field(&mut hasher, notice_operation(*operation).label());
            for action in actions {
                update_hash_field(
                    &mut hasher,
                    match action {
                        FeedbackActionRecord::Dismiss => "dismiss",
                        FeedbackActionRecord::RetryResolution => "retry_resolution",
                    },
                );
            }
            match resolution_identity {
                Some(value) => {
                    hasher.update([1]);
                    update_hash_field(&mut hasher, value);
                }
                None => hasher.update([0]),
            }
            update_hash_field(&mut hasher, failure_kind_label_for_hash(failure.kind));
            hasher.update([u8::from(failure.retryable)]);
            update_hash_field(&mut hasher, failure.label.value());
            match &failure.detail {
                Some(value) => {
                    hasher.update([1]);
                    update_hash_field(&mut hasher, value.value());
                }
                None => hasher.update([0]),
            }
            update_hash_field(&mut hasher, &failure.correlation_id);
        }
        _ => hasher.update(b"invalid-feedback-obligation"),
    }
    hasher.finalize().into()
}

fn failure_kind_label_for_hash(kind: SessionOperationFailureKind) -> &'static str {
    match kind {
        SessionOperationFailureKind::StorageUnavailable => "storage_unavailable",
        SessionOperationFailureKind::StorageCorrupt => "storage_corrupt",
        SessionOperationFailureKind::MigrationBlocked => "migration_blocked",
        SessionOperationFailureKind::PersistFailure => "persist_failure",
        SessionOperationFailureKind::ProtocolIncompatible => "protocol_incompatible",
        SessionOperationFailureKind::ProviderUnavailable => "provider_unavailable",
        SessionOperationFailureKind::ExternalEffectFailed => "external_effect_failed",
        SessionOperationFailureKind::OutcomeUnknown => "outcome_unknown",
        SessionOperationFailureKind::DeadlineExceeded => "deadline_exceeded",
        SessionOperationFailureKind::CapacityExceeded => "capacity_exceeded",
        SessionOperationFailureKind::StopCapacityExceeded => "stop_capacity_exceeded",
        SessionOperationFailureKind::ShutdownAuthorityMismatch => "shutdown_authority_mismatch",
        SessionOperationFailureKind::TargetRevisionChanged => "target_revision_changed",
        SessionOperationFailureKind::OwnerRevisionChanged => "owner_revision_changed",
        SessionOperationFailureKind::RuntimeGenerationChanged => "runtime_generation_changed",
        SessionOperationFailureKind::InvalidEffectIntent => "invalid_effect_intent",
        SessionOperationFailureKind::PreviousShutdownReconciliationRequired => {
            "previous_shutdown_reconciliation_required"
        }
        SessionOperationFailureKind::PreviousShutdownCompactionPending => {
            "previous_shutdown_compaction_pending"
        }
        SessionOperationFailureKind::Internal => "internal",
    }
}

fn encode_reservation(
    feedback_id: &str,
    attempt_id: &str,
    session_id: &str,
    operation: AgentSessionNoticeOperation,
    process_instance_id: &str,
) -> Result<ObligationRecord, FeedbackError> {
    Ok(ObligationRecord::FeedbackReservation {
        feedback_id: feedback_id.to_string(),
        attempt_id: attempt_id.to_string(),
        session_id: session_id.to_string(),
        operation: operation_record(operation),
        process_instance_id: process_instance_id.to_string(),
    })
}

fn decode_reservation(
    record: &ObligationRecord,
) -> Result<SessionFeedbackReservation, FeedbackError> {
    let ObligationRecord::FeedbackReservation {
        feedback_id,
        attempt_id,
        session_id,
        operation,
        process_instance_id,
    } = record
    else {
        return Err(internal("reservation-schema"));
    };
    Ok(SessionFeedbackReservation {
        feedback_id: feedback_id.clone(),
        attempt_id: attempt_id.clone(),
        session_id: session_id.clone(),
        operation: notice_operation(*operation),
        process_instance_id: process_instance_id.clone(),
    })
}

fn encode_record(
    feedback_id: &str,
    attempt_id: &str,
    session_id: &str,
    operation: AgentSessionNoticeOperation,
    failure: &SafeOperationFailure,
    resolution_identity: Option<&str>,
) -> Result<ObligationRecord, FeedbackError> {
    let mut stored_actions = vec![FeedbackActionRecord::Dismiss];
    if resolution_identity.is_some() {
        stored_actions.push(FeedbackActionRecord::RetryResolution);
    }
    Ok(ObligationRecord::Feedback {
        feedback_id: feedback_id.to_string(),
        attempt_id: attempt_id.to_string(),
        session_id: session_id.to_string(),
        operation: operation_record(operation),
        actions: stored_actions,
        resolution_identity: resolution_identity.map(str::to_string),
        failure: failure.clone(),
    })
}

fn decode_record(
    record: &ObligationRecord,
    revision: u64,
) -> Result<SessionFeedbackEntry, FeedbackError> {
    let ObligationRecord::Feedback {
        feedback_id,
        attempt_id,
        session_id,
        operation,
        actions: stored_actions,
        resolution_identity,
        failure,
    } = record
    else {
        return Err(internal("record-schema"));
    };
    let retry = stored_actions.contains(&FeedbackActionRecord::RetryResolution);
    if !stored_actions.contains(&FeedbackActionRecord::Dismiss)
        || retry != resolution_identity.is_some()
    {
        return Err(internal("record-actions"));
    }
    let actions = actions(retry);
    Ok(SessionFeedbackEntry {
        action_identities: action_identities(feedback_id, revision, &actions),
        feedback_id: feedback_id.clone(),
        attempt_id: attempt_id.clone(),
        session_id: session_id.clone(),
        operation: notice_operation(*operation),
        revision,
        actions,
        failure: failure.clone(),
        resolution_identity: resolution_identity.clone(),
    })
}

fn map_query_error(error: LocalEventQueryError) -> FeedbackError {
    match error {
        LocalEventQueryError::NotFound => FeedbackError::NotFound,
        LocalEventQueryError::CursorMismatch | LocalEventQueryError::SnapshotMismatch => {
            FeedbackError::CursorMismatch
        }
        LocalEventQueryError::CursorExpired => FeedbackError::CursorExpired,
        LocalEventQueryError::QueryBusy => FeedbackError::QueryBusy,
        LocalEventQueryError::DeadlineExceeded => FeedbackError::DeadlineExceeded,
        LocalEventQueryError::ResponseTooLarge => FeedbackError::ResponseTooLarge,
        LocalEventQueryError::StorageUnavailable { failure } => {
            FeedbackError::StorageUnavailable { failure }
        }
        LocalEventQueryError::Corrupt { correlation_id }
        | LocalEventQueryError::Internal { correlation_id }
        | LocalEventQueryError::IncompatibleStoredEvent { correlation_id }
        | LocalEventQueryError::ReplayRequired { correlation_id } => {
            FeedbackError::Internal { correlation_id }
        }
        _ => FeedbackError::InvalidRequest,
    }
}

fn map_commit_error(error: CommitBatchError, feedback_id: &str) -> FeedbackError {
    match error {
        CommitBatchError::CapacityExceeded => FeedbackError::CapacityExceeded,
        CommitBatchError::OutcomeUnknown { .. } => FeedbackError::OutcomeUnknown {
            feedback_id: feedback_id.to_string(),
        },
        CommitBatchError::StorageUnavailable { failure } if failure.is_shutdown_in_progress() => {
            FeedbackError::ShutdownInProgress
        }
        CommitBatchError::StorageUnavailable { failure } => {
            FeedbackError::StorageUnavailable { failure }
        }
        CommitBatchError::Corrupt { correlation_id } => FeedbackError::Internal { correlation_id },
        CommitBatchError::PayloadConflict | CommitBatchError::StreamHeadConflict { .. } => {
            FeedbackError::InvalidRequest
        }
        CommitBatchError::SequenceExhausted => FeedbackError::CapacityExceeded,
    }
}

fn internal(label: &str) -> FeedbackError {
    FeedbackError::Internal {
        correlation_id: format!("feedback-{label}-{}", uuid::Uuid::new_v4()),
    }
}
