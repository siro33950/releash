//! Session lifecycle command contract (R-014, B-053..B-056, B-095,
//! B-101..B-103).
//!
//! One Tauri-only command accepts `Close | ArchiveOpen | ArchiveClosed |
//! SwitchBackend { backend_id }` against an exact payload
//! `session_id / expected_session_revision / action`. Acceptance commits the
//! caller binding, the backend-issued operation record, the required
//! terminal / queue-pause facts, and the close obligation in one batch; the
//! runtime effect runs afterwards under a 10-second deadline and resolves to
//! `Completed` or `ReconciliationRequired`.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::domain::agent_session::events::{
    AgentSessionDomainEvent, InterruptReason as EventInterruptReason, ObligationKind,
    ObligationState, SessionLifecycleKind,
};
use crate::domain::local_event::{
    CallerOperationKey, CommitBatchError, CommitBatchResult, CommitIdentity, CommitOperationKind,
    CommitResolution, ExpectedStreamHead, IdempotencyBinding, LoadStreamRequest, LocalAtomicBatch,
    LocalDomainEvent, LocalEventQuery, LocalEventQueryError, LocalEventQueryResult,
    LocalEventTransactionRepository, LocalStateMutation, ObligationMutation, ObligationRecord,
    ObligationStateRecord, OperationBindingMutation, OperationKind, OperationReceiptRecord,
    OperationRecordMutation, OperationStatusRecord, OperationStatusValue, PendingIndexEntry,
    PendingPartition, RecordAuthentication, Revision, RevisionGuard, SafeOperationFailure,
    SessionLifecycleRecordAction, SessionOperationFailureKind, StreamId, StreamVersion,
    TerminalInterruptReasonRecord, TerminalRecordMutation, TerminalResultRecord,
    UncommittedDomainEvent,
};

use super::identity::{constant_time_eq_32, validate_operation_identity};
use super::ports::{
    OperationBindingAuthority, RecoveryEffectResult, SessionCloseRecoveryReadbackPort,
    SessionCloseRecoveryReadbackRequest, SessionLifecycleEffect, SessionLifecycleGate,
    SessionLifecycleState,
};
use super::record::hex_encode;
use super::send::principal_material;

pub(crate) const LIFECYCLE_DEADLINE: Duration = Duration::from_secs(10);

/// Closed action set of the session lifecycle command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLifecycleAction {
    Close,
    ArchiveOpen,
    ArchiveClosed,
    SwitchBackend { backend_id: String },
}

impl SessionLifecycleAction {
    fn label(&self) -> String {
        match self {
            Self::Close => "close".to_string(),
            Self::ArchiveOpen => "archive_open".to_string(),
            Self::ArchiveClosed => "archive_closed".to_string(),
            Self::SwitchBackend { backend_id } => format!("switch_backend:{backend_id}"),
        }
    }

    fn binding_fields(&self) -> (&'static str, Option<&str>) {
        match self {
            Self::Close => ("close", None),
            Self::ArchiveOpen => ("archive-open", None),
            Self::ArchiveClosed => ("archive-closed", None),
            Self::SwitchBackend { backend_id } => ("switch-backend", Some(backend_id.as_str())),
        }
    }

    fn kind(&self) -> SessionLifecycleKind {
        match self {
            Self::Close => SessionLifecycleKind::Close,
            Self::ArchiveOpen | Self::ArchiveClosed => SessionLifecycleKind::Archive,
            Self::SwitchBackend { .. } => SessionLifecycleKind::BackendSwitch,
        }
    }
}

/// Exact caller payload of one lifecycle request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLifecycleRequest {
    pub principal: String,
    pub request_id: String,
    pub session_id: String,
    pub expected_session_revision: i64,
    pub action: SessionLifecycleAction,
}

/// Immutable receipt of an accepted lifecycle operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLifecycleReceipt {
    /// Backend-issued opaque operation identity (not the caller request ID).
    pub operation_id: String,
    pub session_id: String,
    pub action: SessionLifecycleAction,
    /// The session revision fixed at first acceptance; joins never change it.
    pub first_accepted_revision: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionLifecycleOperationState {
    Accepted,
    Completed,
    ReconciliationRequired { failure: SafeOperationFailure },
}

/// Pre-acceptance rejections: zero session change, zero external effect.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionLifecycleRejection {
    Busy,
    PendingOperation,
    RevisionConflict { current_revision: i64 },
    InvalidState,
    Failed { failure: SafeOperationFailure },
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionLifecycleCommandResult {
    Accepted {
        receipt: SessionLifecycleReceipt,
        state: SessionLifecycleOperationState,
    },
    Rejected(SessionLifecycleRejection),
    /// The acceptance commit result is unknown; the caller resolves the same
    /// request identity, never a new one.
    OutcomeUnknown {
        request_id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionLifecycleOperationError {
    InvalidRequest,
    PayloadConflict,
    ShutdownInProgress,
    NotFound,
    QueryBusy,
    DeadlineExceeded,
    StorageUnavailable { failure: SafeOperationFailure },
    Internal { correlation_id: String },
}

#[derive(Debug, Clone)]
struct PendingLifecycleEntry {
    operation_id: String,
    action_label: String,
}

enum LifecycleObligationSlot {
    Pending,
    Available {
        expected: RevisionGuard,
        revision: Revision,
    },
}

enum JoinBindingDisposition {
    Saved,
    Rejected(SafeOperationFailure),
    OutcomeUnknown,
}

struct StoredLifecycleRecord {
    receipt: SessionLifecycleReceipt,
    state: SessionLifecycleOperationState,
    commit_operation_kind: CommitOperationKind,
    principal_mac: [u8; 32],
    binding_hmac: [u8; 32],
    revision: Revision,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

fn correlation(context: &str) -> String {
    format!("slc-{context}-{:x}", now_ms())
}

fn lookup_query_error(error: LocalEventQueryError) -> SessionLifecycleOperationError {
    match error {
        LocalEventQueryError::InvalidRequest => SessionLifecycleOperationError::InvalidRequest,
        LocalEventQueryError::NotFound => SessionLifecycleOperationError::NotFound,
        LocalEventQueryError::QueryBusy => SessionLifecycleOperationError::QueryBusy,
        LocalEventQueryError::DeadlineExceeded => SessionLifecycleOperationError::DeadlineExceeded,
        LocalEventQueryError::StorageUnavailable { failure } => {
            SessionLifecycleOperationError::StorageUnavailable { failure }
        }
        LocalEventQueryError::Corrupt { correlation_id }
        | LocalEventQueryError::IncompatibleStoredEvent { correlation_id }
        | LocalEventQueryError::ReplayRequired { correlation_id }
        | LocalEventQueryError::Internal { correlation_id } => {
            SessionLifecycleOperationError::Internal { correlation_id }
        }
        other => SessionLifecycleOperationError::Internal {
            correlation_id: correlation(&format!("get-{other:?}")),
        },
    }
}

fn storage_rejection(context: &str) -> SessionLifecycleRejection {
    SessionLifecycleRejection::Failed {
        failure: SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            true,
            "The local event store is unavailable.",
            correlation(context),
        ),
    }
}

fn persistence_reconciliation(context: &str) -> SessionLifecycleOperationState {
    SessionLifecycleOperationState::ReconciliationRequired {
        failure: SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            true,
            "The lifecycle effect completed, but its durable result requires reconciliation.",
            correlation(context),
        ),
    }
}

/// `Accepted` is an internal effect-reservation marker, not a durable proof
/// that the external lifecycle effect is still running. After a process loss
/// the same row also represents an effect whose result could not be saved, so
/// replay and point-query surfaces must expose the conservative state.
fn externally_visible_state(
    operation_id: &str,
    state: SessionLifecycleOperationState,
) -> SessionLifecycleOperationState {
    match state {
        SessionLifecycleOperationState::Accepted => {
            SessionLifecycleOperationState::ReconciliationRequired {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "The lifecycle effect completed, but its durable result requires reconciliation.",
                    format!("slc-accepted-effect-unconfirmed-{operation_id}"),
                ),
            }
        }
        state => state,
    }
}

fn action_record(action: &SessionLifecycleAction) -> SessionLifecycleRecordAction {
    match action {
        SessionLifecycleAction::Close => SessionLifecycleRecordAction::Close,
        SessionLifecycleAction::ArchiveOpen => SessionLifecycleRecordAction::ArchiveOpen,
        SessionLifecycleAction::ArchiveClosed => SessionLifecycleRecordAction::ArchiveClosed,
        SessionLifecycleAction::SwitchBackend { backend_id } => {
            SessionLifecycleRecordAction::SwitchBackend {
                backend_id: backend_id.clone(),
            }
        }
    }
}

fn lifecycle_action(action: SessionLifecycleRecordAction) -> SessionLifecycleAction {
    match action {
        SessionLifecycleRecordAction::Close => SessionLifecycleAction::Close,
        SessionLifecycleRecordAction::ArchiveOpen => SessionLifecycleAction::ArchiveOpen,
        SessionLifecycleRecordAction::ArchiveClosed => SessionLifecycleAction::ArchiveClosed,
        SessionLifecycleRecordAction::SwitchBackend { backend_id } => {
            SessionLifecycleAction::SwitchBackend { backend_id }
        }
    }
}

fn status_record(state: &SessionLifecycleOperationState) -> OperationStatusRecord {
    let value = match state {
        SessionLifecycleOperationState::Accepted => OperationStatusValue::Accepted,
        SessionLifecycleOperationState::Completed => OperationStatusValue::Completed,
        SessionLifecycleOperationState::ReconciliationRequired { failure } => {
            OperationStatusValue::ReconciliationRequired {
                failure: failure.clone(),
            }
        }
    };
    OperationStatusRecord {
        kind: OperationKind::SessionLifecycle,
        value,
    }
}

fn receipt_record(
    receipt: &SessionLifecycleReceipt,
    commit_operation_kind: CommitOperationKind,
    principal_mac: &[u8; 32],
    binding_hmac: &[u8; 32],
) -> OperationReceiptRecord {
    OperationReceiptRecord::SessionLifecycle {
        operation_id: receipt.operation_id.clone(),
        session_id: receipt.session_id.clone(),
        action: action_record(&receipt.action),
        first_accepted_revision: receipt.first_accepted_revision,
        commit_operation_kind,
        authentication: RecordAuthentication {
            principal_mac: *principal_mac,
            binding_hmac: *binding_hmac,
        },
    }
}

fn status_identity_material(state: &SessionLifecycleOperationState) -> Vec<u8> {
    match state {
        SessionLifecycleOperationState::Accepted => b"accepted".to_vec(),
        SessionLifecycleOperationState::Completed => b"completed".to_vec(),
        SessionLifecycleOperationState::ReconciliationRequired { failure } => {
            format!("reconciliation_required\0{}", failure.correlation_id).into_bytes()
        }
    }
}

fn decode_lifecycle_record(
    receipt: OperationReceiptRecord,
    status: OperationStatusRecord,
    revision: Revision,
) -> Option<StoredLifecycleRecord> {
    let (
        operation_id,
        session_id,
        action,
        first_accepted_revision,
        commit_operation_kind,
        authentication,
    ) = match receipt {
        OperationReceiptRecord::SessionLifecycle {
            operation_id,
            session_id,
            action,
            first_accepted_revision,
            commit_operation_kind,
            authentication,
        } => (
            operation_id,
            session_id,
            action,
            first_accepted_revision,
            commit_operation_kind,
            authentication,
        ),
        OperationReceiptRecord::Send { .. }
        | OperationReceiptRecord::PermissionResponse { .. }
        | OperationReceiptRecord::Stop { .. }
        | OperationReceiptRecord::ApplicationQuit { .. } => return None,
    };
    if status.kind != OperationKind::SessionLifecycle {
        return None;
    }
    let state = match status.value {
        OperationStatusValue::Accepted => SessionLifecycleOperationState::Accepted,
        OperationStatusValue::Completed => SessionLifecycleOperationState::Completed,
        OperationStatusValue::ReconciliationRequired { failure } => {
            SessionLifecycleOperationState::ReconciliationRequired { failure }
        }
        OperationStatusValue::AwaitingProviderStart { .. }
        | OperationStatusValue::AwaitingProviderResponse { .. }
        | OperationStatusValue::Queued { .. }
        | OperationStatusValue::ProviderStartReserved { .. }
        | OperationStatusValue::Running { .. }
        | OperationStatusValue::PermissionCompleted { .. }
        | OperationStatusValue::StopCompleted { .. }
        | OperationStatusValue::Preparing
        | OperationStatusValue::Activated
        | OperationStatusValue::ExitPending
        | OperationStatusValue::Exited
        | OperationStatusValue::OutcomeUnknown { .. }
        | OperationStatusValue::FailedBeforeActivation { .. }
        | OperationStatusValue::Failed { .. }
        | OperationStatusValue::Terminal { .. } => return None,
    };
    match commit_operation_kind {
        CommitOperationKind::SessionLifecycle | CommitOperationKind::ShutdownTarget => {}
        CommitOperationKind::Send
        | CommitOperationKind::PermissionResponse
        | CommitOperationKind::Stop
        | CommitOperationKind::ApplicationQuit
        | CommitOperationKind::Recovery
        | CommitOperationKind::UserMutation
        | CommitOperationKind::OperationProgress
        | CommitOperationKind::Projection
        | CommitOperationKind::Workflow => return None,
    }
    Some(StoredLifecycleRecord {
        receipt: SessionLifecycleReceipt {
            operation_id,
            session_id,
            action: lifecycle_action(action),
            first_accepted_revision,
        },
        state,
        commit_operation_kind,
        principal_mac: authentication.principal_mac,
        binding_hmac: authentication.binding_hmac,
        revision,
    })
}

fn lifecycle_obligation(
    record: &ObligationRecord,
) -> Option<(
    &str,
    &str,
    &str,
    &SessionLifecycleRecordAction,
    ObligationStateRecord,
)> {
    match record {
        ObligationRecord::RecoveryTransition { original, .. }
        | ObligationRecord::Observed { original, .. } => lifecycle_obligation(original),
        ObligationRecord::SessionClose {
            obligation_id,
            operation_id,
            session_id,
            action,
            state,
        } => Some((obligation_id, operation_id, session_id, action, *state)),
        ObligationRecord::Send { .. }
        | ObligationRecord::PermissionResponse { .. }
        | ObligationRecord::StopInterrupt { .. }
        | ObligationRecord::BackendSessionRecovery { .. }
        | ObligationRecord::WorkflowShutdown { .. }
        | ObligationRecord::WorkflowTurnCompletion { .. }
        | ObligationRecord::RecoveryPublication { .. }
        | ObligationRecord::ProviderEstablish { .. }
        | ObligationRecord::TurnExecution { .. }
        | ObligationRecord::TerminalCommit { .. }
        | ObligationRecord::RecoveryReserved { .. }
        | ObligationRecord::RecoveryCompleted { .. }
        | ObligationRecord::FeedbackReservation { .. }
        | ObligationRecord::Feedback { .. }
        | ObligationRecord::WorkflowExecution { .. } => None,
    }
}

fn lifecycle_obligation_record(
    obligation_id: &str,
    receipt: &SessionLifecycleReceipt,
    state: ObligationStateRecord,
) -> ObligationRecord {
    ObligationRecord::SessionClose {
        obligation_id: obligation_id.to_string(),
        operation_id: receipt.operation_id.clone(),
        session_id: receipt.session_id.clone(),
        action: action_record(&receipt.action),
        state,
    }
}

pub struct SessionLifecycleOperationUsecase {
    repository: Arc<dyn LocalEventTransactionRepository>,
    authority: Arc<dyn OperationBindingAuthority>,
    gate: Arc<dyn SessionLifecycleGate>,
    installation_id: String,
}

impl SessionLifecycleOperationUsecase {
    pub fn new(
        repository: Arc<dyn LocalEventTransactionRepository>,
        authority: Arc<dyn OperationBindingAuthority>,
        gate: Arc<dyn SessionLifecycleGate>,
        installation_id: String,
    ) -> Self {
        Self {
            repository,
            authority,
            gate,
            installation_id,
        }
    }

    fn operation_id_for(&self, principal: &str, request_id: &str) -> String {
        hex_encode(
            &self
                .authority
                .mac(format!("lifecycle-op\0{principal}\0{request_id}").as_bytes()),
        )
    }

    async fn lookup_record(
        &self,
        operation_id: &str,
    ) -> Result<Option<StoredLifecycleRecord>, LocalEventQueryError> {
        let result = self
            .repository
            .query(LocalEventQuery::OperationByIdentity {
                kind: OperationKind::SessionLifecycle,
                operation_id: operation_id.to_string(),
            })
            .await?;
        let LocalEventQueryResult::OperationByIdentity(view) = result else {
            return Err(LocalEventQueryError::Internal {
                correlation_id: correlation("lookup-shape"),
            });
        };
        Ok(view.and_then(|view| {
            decode_lifecycle_record(view.receipt, view.latest_status, view.revision)
        }))
    }

    fn caller_key(&self, request: &SessionLifecycleRequest) -> CallerOperationKey {
        CallerOperationKey {
            principal: request.principal.clone(),
            installation_id: self.installation_id.clone(),
            kind: OperationKind::SessionLifecycle,
            caller_request_id: request.request_id.clone(),
        }
    }

    async fn lookup_binding(
        &self,
        key: &CallerOperationKey,
    ) -> Result<Option<crate::domain::local_event::OperationBindingView>, LocalEventQueryError>
    {
        let result = self
            .repository
            .query(LocalEventQuery::OperationBindingByIdentity { key: key.clone() })
            .await?;
        let LocalEventQueryResult::OperationBindingByIdentity(binding) = result else {
            return Err(LocalEventQueryError::Internal {
                correlation_id: correlation("binding-shape"),
            });
        };
        Ok(binding)
    }

    async fn current_stream_head(&self, stream_id: &StreamId) -> Result<i64, LocalEventQueryError> {
        let page = self
            .repository
            .load_stream(LoadStreamRequest {
                stream_id: stream_id.clone(),
                after: None,
                limit: 1,
            })
            .await?;
        Ok(page.head.value())
    }

    fn obligation_id(&self, session_id: &str) -> String {
        format!(
            "session-lifecycle-target-{}",
            hex_encode(
                &self
                    .authority
                    .digest(format!("session-lifecycle-target/v1\0{session_id}").as_bytes(),)
            )
        )
    }

    async fn pending_entry(
        &self,
        session_id: &str,
    ) -> Result<Option<PendingLifecycleEntry>, LocalEventQueryError> {
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: self.obligation_id(session_id),
            })
            .await?;
        let LocalEventQueryResult::ObligationByIdentity(value) = result else {
            return Err(LocalEventQueryError::Internal {
                correlation_id: correlation("pending-shape"),
            });
        };
        let Some(value) = value.filter(|value| value.pending.is_some()) else {
            return Ok(None);
        };
        let Some((obligation_id, operation_id, stored_session_id, action, _)) =
            lifecycle_obligation(&value.record)
        else {
            return Err(LocalEventQueryError::Corrupt {
                correlation_id: correlation("pending-record"),
            });
        };
        if obligation_id != self.obligation_id(session_id) || stored_session_id != session_id {
            return Err(LocalEventQueryError::Corrupt {
                correlation_id: correlation("pending-integrity"),
            });
        }
        Ok(Some(PendingLifecycleEntry {
            operation_id: operation_id.to_string(),
            action_label: lifecycle_action(action.clone()).label(),
        }))
    }

    async fn obligation_slot(
        &self,
        session_id: &str,
    ) -> Result<LifecycleObligationSlot, LocalEventQueryError> {
        let obligation_id = self.obligation_id(session_id);
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.clone(),
            })
            .await?;
        let LocalEventQueryResult::ObligationByIdentity(value) = result else {
            return Err(LocalEventQueryError::Internal {
                correlation_id: correlation("obligation-slot-shape"),
            });
        };
        let Some(value) = value else {
            return Ok(LifecycleObligationSlot::Available {
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).expect("zero revision"),
            });
        };
        if value.pending.is_some() {
            return Ok(LifecycleObligationSlot::Pending);
        }
        let Some((stored_obligation_id, _, stored_session_id, _, _)) =
            lifecycle_obligation(&value.record)
        else {
            return Err(LocalEventQueryError::Corrupt {
                correlation_id: correlation("obligation-slot-record"),
            });
        };
        if stored_obligation_id != obligation_id || stored_session_id != session_id {
            return Err(LocalEventQueryError::Corrupt {
                correlation_id: correlation("obligation-slot-integrity"),
            });
        }
        let revision = value
            .revision
            .next()
            .ok_or_else(|| LocalEventQueryError::Internal {
                correlation_id: correlation("obligation-slot-revision"),
            })?;
        Ok(LifecycleObligationSlot::Available {
            expected: RevisionGuard::Expected(value.revision),
            revision,
        })
    }

    async fn effect_is_reserved(
        &self,
        operation_id: &str,
        obligation_id: &str,
    ) -> Result<bool, SessionLifecycleOperationError> {
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.to_string(),
            })
            .await
            .map_err(lookup_query_error)?;
        let LocalEventQueryResult::ObligationByIdentity(Some(obligation)) = result else {
            return Ok(false);
        };
        if obligation.pending.is_none() {
            return Ok(false);
        }
        let Some((stored_obligation_id, stored_operation_id, _, _, state)) =
            lifecycle_obligation(&obligation.record)
        else {
            return Err(SessionLifecycleOperationError::Internal {
                correlation_id: correlation("effect-reservation-record"),
            });
        };
        Ok(stored_obligation_id == obligation_id
            && stored_operation_id == operation_id
            && state == ObligationStateRecord::EffectReserved)
    }

    /// Record a caller binding for a request joining an existing operation.
    /// The first accepted revision guard is never changed by a join.
    async fn record_join_binding(
        &self,
        request: &SessionLifecycleRequest,
        operation_id: &str,
        binding_hmac: [u8; 32],
        commit_operation_kind: CommitOperationKind,
    ) -> Result<JoinBindingDisposition, SessionLifecycleOperationError> {
        let digest = self
            .authority
            .digest(format!("slc-join\0{}\0{}", request.principal, request.request_id).as_bytes());
        let commit_id = CommitIdentity::parse(&hex_encode(&digest)).map_err(|_| {
            SessionLifecycleOperationError::Internal {
                correlation_id: correlation("join-identity"),
            }
        })?;
        let batch = LocalAtomicBatch {
            commit_id: commit_id.clone(),
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: commit_operation_kind,
                idempotency_key: format!(
                    "slc.join.{}",
                    hex_encode(
                        &self.authority.digest(
                            format!(
                                "slc-join-owner/v1\0{}\0{}\0{}",
                                request.principal, self.installation_id, request.request_id
                            )
                            .as_bytes(),
                        )
                    )
                ),
                payload_hash: binding_hmac,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![LocalStateMutation::OperationBinding(
                OperationBindingMutation {
                    key: self.caller_key(request),
                    operation_id: operation_id.to_string(),
                    binding_hmac,
                },
            )],
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {
                Ok(JoinBindingDisposition::Saved)
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                match self.repository.resolve_commit(commit_id).await {
                    Ok(CommitResolution::Committed(_)) => Ok(JoinBindingDisposition::Saved),
                    Ok(CommitResolution::NotCommitted) => {
                        Ok(JoinBindingDisposition::Rejected(SafeOperationFailure::new(
                            SessionOperationFailureKind::PersistFailure,
                            true,
                            "The lifecycle join was not committed.",
                            correlation("join-not-committed"),
                        )))
                    }
                    Err(_) => Ok(JoinBindingDisposition::OutcomeUnknown),
                }
            }
            Err(CommitBatchError::StorageUnavailable { failure })
                if failure.is_shutdown_in_progress() =>
            {
                Err(SessionLifecycleOperationError::ShutdownInProgress)
            }
            Err(CommitBatchError::StorageUnavailable { failure }) => {
                Ok(JoinBindingDisposition::Rejected(failure))
            }
            Err(CommitBatchError::PayloadConflict) => {
                let key = self.caller_key(request);
                let saved = self.lookup_binding(&key).await.map_err(|_| {
                    SessionLifecycleOperationError::Internal {
                        correlation_id: correlation("join-conflict-readback"),
                    }
                })?;
                if saved.as_ref().is_some_and(|saved| {
                    saved.operation_id == operation_id
                        && constant_time_eq_32(&saved.binding_hmac, &binding_hmac)
                }) {
                    Ok(JoinBindingDisposition::Saved)
                } else {
                    Err(SessionLifecycleOperationError::PayloadConflict)
                }
            }
            Err(CommitBatchError::CapacityExceeded | CommitBatchError::SequenceExhausted) => {
                Ok(JoinBindingDisposition::Rejected(SafeOperationFailure::new(
                    SessionOperationFailureKind::CapacityExceeded,
                    true,
                    "The lifecycle join capacity is exhausted.",
                    correlation("join-capacity"),
                )))
            }
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                Ok(JoinBindingDisposition::Rejected(SafeOperationFailure::new(
                    SessionOperationFailureKind::PersistFailure,
                    true,
                    "The lifecycle join changed concurrently.",
                    correlation("join-conflict"),
                )))
            }
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                Err(SessionLifecycleOperationError::Internal { correlation_id })
            }
        }
    }

    async fn join_pending_operation(
        &self,
        request: &SessionLifecycleRequest,
    ) -> Result<Option<SessionLifecycleCommandResult>, SessionLifecycleOperationError> {
        let entry = self.pending_entry(&request.session_id).await.map_err(|_| {
            SessionLifecycleOperationError::Internal {
                correlation_id: correlation("pending"),
            }
        })?;
        let Some(entry) = entry else {
            return Ok(None);
        };
        if entry.action_label != request.action.label() {
            return Ok(Some(SessionLifecycleCommandResult::Rejected(
                SessionLifecycleRejection::PendingOperation,
            )));
        }
        let record = self.lookup_record(&entry.operation_id).await.map_err(|_| {
            SessionLifecycleOperationError::Internal {
                correlation_id: correlation("pending-operation"),
            }
        })?;
        let Some(record) = record else {
            return Ok(Some(SessionLifecycleCommandResult::Rejected(
                SessionLifecycleRejection::PendingOperation,
            )));
        };
        let principal_mac = self.authority.mac(&principal_material(&request.principal));
        if !constant_time_eq_32(&record.principal_mac, &principal_mac) {
            return Err(SessionLifecycleOperationError::NotFound);
        }
        // A joining caller binds its own request identity to the already
        // accepted backend operation.  Reusing the prospective operation ID
        // derived for this caller would create two distinct canonical
        // bindings for the same lifecycle flight.
        let (binding_action, binding_backend) = request.action.binding_fields();
        let binding_hmac = self.authority.mac(&super::binding::session_lifecycle(
            super::binding::SessionLifecycleBinding {
                principal: &request.principal,
                installation_id: &self.installation_id,
                request_id: &request.request_id,
                backend_operation_id: &entry.operation_id,
                session_id: &request.session_id,
                expected_revision: request.expected_session_revision as u64,
                action: binding_action,
                backend_id: binding_backend,
            },
        ));
        match self
            .record_join_binding(
                request,
                &entry.operation_id,
                binding_hmac,
                record.commit_operation_kind,
            )
            .await?
        {
            JoinBindingDisposition::Saved => {}
            JoinBindingDisposition::Rejected(failure) => {
                return Ok(Some(SessionLifecycleCommandResult::Rejected(
                    SessionLifecycleRejection::Failed { failure },
                )))
            }
            JoinBindingDisposition::OutcomeUnknown => {
                return Ok(Some(SessionLifecycleCommandResult::OutcomeUnknown {
                    request_id: request.request_id.clone(),
                }))
            }
        }
        Ok(Some(SessionLifecycleCommandResult::Accepted {
            receipt: record.receipt,
            state: externally_visible_state(&entry.operation_id, record.state),
        }))
    }

    /// A rejection computed from a mutable snapshot is final only after a
    /// point lookup proves that this exact caller operation did not win a
    /// concurrent acceptance race.
    async fn converge_or_reject(
        &self,
        request: &SessionLifecycleRequest,
        operation_id: &str,
        principal_mac: &[u8; 32],
        binding_hmac: &[u8; 32],
        rejection: SessionLifecycleRejection,
    ) -> Result<SessionLifecycleCommandResult, SessionLifecycleOperationError> {
        match self.lookup_record(operation_id).await {
            Ok(Some(record)) => {
                if !constant_time_eq_32(&record.principal_mac, principal_mac) {
                    return Err(SessionLifecycleOperationError::NotFound);
                }
                if !constant_time_eq_32(&record.binding_hmac, binding_hmac) {
                    return Err(SessionLifecycleOperationError::PayloadConflict);
                }
                Ok(SessionLifecycleCommandResult::Accepted {
                    receipt: record.receipt,
                    state: externally_visible_state(operation_id, record.state),
                })
            }
            Ok(None) => Ok(SessionLifecycleCommandResult::Rejected(rejection)),
            Err(_) => Ok(SessionLifecycleCommandResult::OutcomeUnknown {
                request_id: request.request_id.clone(),
            }),
        }
    }

    /// Accept, join, reject, or replay one session lifecycle request. The
    /// command resolves within the 10-second deadline to `Completed`,
    /// `ReconciliationRequired`, or a pre-acceptance rejection.
    pub async fn request(
        &self,
        request: SessionLifecycleRequest,
    ) -> Result<SessionLifecycleCommandResult, SessionLifecycleOperationError> {
        self.request_with_commit_operation_kind(request, CommitOperationKind::SessionLifecycle)
            .await
    }

    /// Internal close command for a target already frozen into the current
    /// application-shutdown plan. Public callers cannot select this admission
    /// lane; their lifecycle commits remain closed once shutdown is current.
    pub(crate) async fn request_shutdown_target(
        &self,
        request: SessionLifecycleRequest,
    ) -> Result<SessionLifecycleCommandResult, SessionLifecycleOperationError> {
        if !request.principal.starts_with("shutdown:")
            || !matches!(&request.action, SessionLifecycleAction::Close)
        {
            return Err(SessionLifecycleOperationError::InvalidRequest);
        }
        self.request_with_commit_operation_kind(request, CommitOperationKind::ShutdownTarget)
            .await
    }

    async fn request_with_commit_operation_kind(
        &self,
        request: SessionLifecycleRequest,
        commit_operation_kind: CommitOperationKind,
    ) -> Result<SessionLifecycleCommandResult, SessionLifecycleOperationError> {
        if validate_operation_identity(&request.request_id).is_err()
            || request.expected_session_revision < 0
        {
            return Err(SessionLifecycleOperationError::InvalidRequest);
        }
        let Ok(stream_id) = StreamId::agent_session(&request.session_id) else {
            return Err(SessionLifecycleOperationError::InvalidRequest);
        };
        let caller_key = self.caller_key(&request);
        let saved_binding = match self.lookup_binding(&caller_key).await {
            Ok(binding) => binding,
            Err(_) => {
                return Ok(SessionLifecycleCommandResult::OutcomeUnknown {
                    request_id: request.request_id,
                })
            }
        };
        if let Some(saved_binding) = saved_binding {
            let (binding_action, binding_backend) = request.action.binding_fields();
            let requested_binding = self.authority.mac(&super::binding::session_lifecycle(
                super::binding::SessionLifecycleBinding {
                    principal: &request.principal,
                    installation_id: &self.installation_id,
                    request_id: &request.request_id,
                    backend_operation_id: &saved_binding.operation_id,
                    session_id: &request.session_id,
                    expected_revision: request.expected_session_revision as u64,
                    action: binding_action,
                    backend_id: binding_backend,
                },
            ));
            if !constant_time_eq_32(&saved_binding.binding_hmac, &requested_binding) {
                return Err(SessionLifecycleOperationError::PayloadConflict);
            }
            let record = match self.lookup_record(&saved_binding.operation_id).await {
                Ok(Some(record)) => record,
                Ok(None) | Err(_) => {
                    return Ok(SessionLifecycleCommandResult::OutcomeUnknown {
                        request_id: request.request_id,
                    })
                }
            };
            let principal_mac = self.authority.mac(&principal_material(&request.principal));
            if !constant_time_eq_32(&record.principal_mac, &principal_mac) {
                return Err(SessionLifecycleOperationError::NotFound);
            }
            return Ok(SessionLifecycleCommandResult::Accepted {
                receipt: record.receipt,
                state: externally_visible_state(&saved_binding.operation_id, record.state),
            });
        }

        let operation_id = self.operation_id_for(&request.principal, &request.request_id);
        let principal_mac = self.authority.mac(&principal_material(&request.principal));
        let (binding_action, binding_backend) = request.action.binding_fields();
        let binding_hmac = self.authority.mac(&super::binding::session_lifecycle(
            super::binding::SessionLifecycleBinding {
                principal: &request.principal,
                installation_id: &self.installation_id,
                request_id: &request.request_id,
                backend_operation_id: &operation_id,
                session_id: &request.session_id,
                expected_revision: request.expected_session_revision as u64,
                action: binding_action,
                backend_id: binding_backend,
            },
        ));

        // An operation record and its caller binding are one acceptance
        // transaction. Never treat an unbound record as a replay or create a
        // replacement identity around it.
        match self.lookup_record(&operation_id).await {
            Ok(Some(_)) => {
                return Err(SessionLifecycleOperationError::Internal {
                    correlation_id: correlation("unbound-operation"),
                })
            }
            Ok(None) => {}
            Err(_) => {
                return Ok(SessionLifecycleCommandResult::OutcomeUnknown {
                    request_id: request.request_id,
                });
            }
        }

        // Join a live slot, or reserve the exact CAS needed to reuse a
        // completed per-session obligation row. The following session
        // projection/head guards close a concurrent slot transition.
        let obligation_slot = match self.obligation_slot(&request.session_id).await {
            Err(_) => {
                return self
                    .converge_or_reject(
                        &request,
                        &operation_id,
                        &principal_mac,
                        &binding_hmac,
                        storage_rejection("obligation-slot"),
                    )
                    .await;
            }
            Ok(slot) => match slot {
                LifecycleObligationSlot::Pending => {
                    if let Some(result) = self.join_pending_operation(&request).await? {
                        return Ok(result);
                    }
                    return self
                        .converge_or_reject(
                            &request,
                            &operation_id,
                            &principal_mac,
                            &binding_hmac,
                            SessionLifecycleRejection::PendingOperation,
                        )
                        .await;
                }
                LifecycleObligationSlot::Available { expected, revision } => (expected, revision),
            },
        };

        // Admission against the current bounded snapshot.
        let snapshot = match self.gate.session_snapshot(&request.session_id).await {
            Ok(snapshot) => snapshot,
            Err(failure) => {
                return self
                    .converge_or_reject(
                        &request,
                        &operation_id,
                        &principal_mac,
                        &binding_hmac,
                        SessionLifecycleRejection::Failed { failure },
                    )
                    .await;
            }
        };
        if snapshot.session_revision != request.expected_session_revision {
            return self
                .converge_or_reject(
                    &request,
                    &operation_id,
                    &principal_mac,
                    &binding_hmac,
                    SessionLifecycleRejection::RevisionConflict {
                        current_revision: snapshot.session_revision,
                    },
                )
                .await;
        }
        let active_turn_id = match (&request.action, &snapshot.lifecycle) {
            (
                SessionLifecycleAction::Close | SessionLifecycleAction::ArchiveOpen,
                SessionLifecycleState::Open { active_turn_id, .. },
            ) => *active_turn_id,
            (SessionLifecycleAction::ArchiveClosed, SessionLifecycleState::Closed) => None,
            (
                SessionLifecycleAction::SwitchBackend { .. },
                SessionLifecycleState::Open {
                    idle,
                    active_turn_id,
                },
            ) => {
                if active_turn_id.is_some() || !*idle {
                    return self
                        .converge_or_reject(
                            &request,
                            &operation_id,
                            &principal_mac,
                            &binding_hmac,
                            SessionLifecycleRejection::Busy,
                        )
                        .await;
                }
                if snapshot.has_pending_permission
                    || snapshot.has_pending_recovery
                    || snapshot.has_pending_provider_operation
                {
                    return self
                        .converge_or_reject(
                            &request,
                            &operation_id,
                            &principal_mac,
                            &binding_hmac,
                            SessionLifecycleRejection::InvalidState,
                        )
                        .await;
                }
                None
            }
            _ => {
                return self
                    .converge_or_reject(
                        &request,
                        &operation_id,
                        &principal_mac,
                        &binding_hmac,
                        SessionLifecycleRejection::InvalidState,
                    )
                    .await;
            }
        };
        let requires_runtime_effect = snapshot.has_runtime
            && !matches!(request.action, SessionLifecycleAction::ArchiveClosed);
        let accepted_state = if requires_runtime_effect {
            SessionLifecycleOperationState::Accepted
        } else {
            SessionLifecycleOperationState::Completed
        };

        let head = match self.current_stream_head(&stream_id).await {
            Ok(head) => head,
            Err(_) => {
                return self
                    .converge_or_reject(
                        &request,
                        &operation_id,
                        &principal_mac,
                        &binding_hmac,
                        storage_rejection("head"),
                    )
                    .await;
            }
        };

        let receipt = SessionLifecycleReceipt {
            operation_id: operation_id.clone(),
            session_id: request.session_id.clone(),
            action: request.action.clone(),
            first_accepted_revision: request.expected_session_revision,
        };
        let at = now_ms();
        let mut events = vec![UncommittedDomainEvent {
            stream_id: stream_id.clone(),
            event: LocalDomainEvent::AgentSession(
                AgentSessionDomainEvent::SessionLifecycleOperationAccepted {
                    operation_id: operation_id.clone(),
                    kind: request.action.kind(),
                    at: at as f64,
                },
            ),
            occurred_at_ms: at,
        }];
        let mut mutations: Vec<LocalStateMutation> = vec![
            LocalStateMutation::OperationBinding(OperationBindingMutation {
                key: CallerOperationKey {
                    principal: request.principal.clone(),
                    installation_id: self.installation_id.clone(),
                    kind: OperationKind::SessionLifecycle,
                    caller_request_id: request.request_id.clone(),
                },
                operation_id: operation_id.clone(),
                binding_hmac,
            }),
            LocalStateMutation::SessionLifecycleOperation(OperationRecordMutation {
                kind: OperationKind::SessionLifecycle,
                operation_id: operation_id.clone(),
                receipt: receipt_record(
                    &receipt,
                    commit_operation_kind,
                    &principal_mac,
                    &binding_hmac,
                ),
                latest_status: status_record(&accepted_state),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).expect("zero revision"),
            }),
        ];

        // Active normal close / open archive terminate the running turn with
        // a SessionClosed terminal; Idle close / archive and closed archive
        // add no synthetic terminal.
        let terminal_candidate = if let Some(turn_id) = active_turn_id {
            events.push(UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: LocalDomainEvent::AgentSession(AgentSessionDomainEvent::TurnInterrupted {
                    turn_id,
                    reason: EventInterruptReason::SessionClosed,
                    exit_code: -1,
                    error: None,
                }),
                occurred_at_ms: at,
            });
            let terminal_result = TerminalResultRecord::SessionClosed {
                operation_id: operation_id.clone(),
                reason: TerminalInterruptReasonRecord::SessionClosed,
                result: crate::domain::agent_session::entities::TurnResult::Interrupted {
                    reason: crate::domain::agent_session::entities::InterruptReason::SessionClosed,
                    error: None,
                },
            };
            let participant_digest = self.authority.digest(
                format!(
                    "session-closed-terminal-semantic/v1\0{}\0{}\0{turn_id}",
                    operation_id, request.session_id
                )
                .as_bytes(),
            );
            let terminal = TerminalRecordMutation {
                session_id: request.session_id.clone(),
                turn_id: turn_id.to_string(),
                terminal_identity: operation_id.clone(),
                result: terminal_result,
                participant_digest,
            };
            mutations.push(LocalStateMutation::TerminalRecord(terminal.clone()));
            Some(terminal)
        } else {
            None
        };
        if matches!(
            request.action,
            SessionLifecycleAction::Close | SessionLifecycleAction::ArchiveOpen
        ) {
            events.push(UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: LocalDomainEvent::AgentSession(AgentSessionDomainEvent::SessionClosed {
                    at: at as f64,
                }),
                occurred_at_ms: at,
            });
        }
        // Queue pause: content-preserving pause for close / open archive /
        // backend switch; a closed-session archive leaves the queue as-is.
        let pauses_queue = !matches!(request.action, SessionLifecycleAction::ArchiveClosed);
        if pauses_queue && !snapshot.queue_paused {
            events.push(UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueuePaused {
                    at: at as f64,
                }),
                occurred_at_ms: at,
            });
        }
        // A runtime close obligation exists only when there is a concrete
        // live runtime to close. Closed archive and lifecycle mutations for a
        // runtime-free session complete in the acceptance batch with no fake
        // pending work or empty external effect.
        let obligation_id = self.obligation_id(&request.session_id);
        if requires_runtime_effect {
            events.push(UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: LocalDomainEvent::AgentSession(
                    AgentSessionDomainEvent::ObligationRecorded {
                        obligation_id: obligation_id.clone(),
                        kind: ObligationKind::SessionClose,
                        state: ObligationState::EffectReserved,
                        at: at as f64,
                    },
                ),
                occurred_at_ms: at,
            });
            mutations.push(LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: obligation_id.clone(),
                record: lifecycle_obligation_record(
                    &obligation_id,
                    &receipt,
                    ObligationStateRecord::EffectReserved,
                ),
                pending: Some(PendingIndexEntry {
                    ordered_key: format!("{at:020}-{obligation_id}"),
                    owner: request.session_id.clone(),
                    partition: PendingPartition::Owner,
                    shutdown_plan: None,
                }),
                expected: obligation_slot.0,
                revision: obligation_slot.1,
            }));
        }

        let projection_events = events
            .iter()
            .filter_map(|event| match &event.event {
                LocalDomainEvent::AgentSession(event) => Some(event.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let projection_mutations = match self
            .gate
            .acceptance_state_mutations(&request.session_id, &request.action, &projection_events)
            .await
        {
            Ok(mutations) => mutations,
            Err(failure) => {
                return self
                    .converge_or_reject(
                        &request,
                        &operation_id,
                        &principal_mac,
                        &binding_hmac,
                        SessionLifecycleRejection::Failed { failure },
                    )
                    .await;
            }
        };
        mutations.extend(projection_mutations);
        if let Some(terminal) = terminal_candidate.as_ref() {
            let participants = match self.gate.terminal_participants(terminal).await {
                Ok(participants) => participants,
                Err(failure) => {
                    return self
                        .converge_or_reject(
                            &request,
                            &operation_id,
                            &principal_mac,
                            &binding_hmac,
                            SessionLifecycleRejection::Failed { failure },
                        )
                        .await;
                }
            };
            for event in participants.events {
                events.push(UncommittedDomainEvent {
                    stream_id: stream_id.clone(),
                    event: LocalDomainEvent::AgentSession(event),
                    occurred_at_ms: at,
                });
            }
            mutations.extend(participants.mutations);
        }

        let commit_digest = self
            .authority
            .digest(format!("slc-commit\0{operation_id}").as_bytes());
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex_encode(&commit_digest)).map_err(|_| {
                SessionLifecycleOperationError::Internal {
                    correlation_id: correlation("commit-id"),
                }
            })?,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: commit_operation_kind,
                idempotency_key: operation_id.clone(),
                payload_hash: binding_hmac,
            },
            expected_heads: vec![ExpectedStreamHead {
                stream_id: stream_id.clone(),
                expected: StreamVersion::new(head).map_err(|_| {
                    SessionLifecycleOperationError::Internal {
                        correlation_id: correlation("head"),
                    }
                })?,
            }],
            events,
            state_mutations: mutations,
        };

        let committed_fresh = match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_)) => true,
            Ok(CommitBatchResult::Replayed(_)) => false,
            Err(CommitBatchError::PayloadConflict) => {
                if let Some(result) = self.join_pending_operation(&request).await? {
                    return Ok(result);
                }
                return match self.lookup_record(&operation_id).await {
                    Ok(Some(record)) => {
                        if !constant_time_eq_32(&record.principal_mac, &principal_mac) {
                            Err(SessionLifecycleOperationError::NotFound)
                        } else if !constant_time_eq_32(&record.binding_hmac, &binding_hmac) {
                            Err(SessionLifecycleOperationError::PayloadConflict)
                        } else {
                            Ok(SessionLifecycleCommandResult::Accepted {
                                receipt: record.receipt,
                                state: externally_visible_state(&operation_id, record.state),
                            })
                        }
                    }
                    Ok(None) => Err(SessionLifecycleOperationError::PayloadConflict),
                    Err(_) => Ok(SessionLifecycleCommandResult::OutcomeUnknown {
                        request_id: request.request_id.clone(),
                    }),
                };
            }
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                if let Some(result) = self.join_pending_operation(&request).await? {
                    return Ok(result);
                }
                return self
                    .converge_or_reject(
                        &request,
                        &operation_id,
                        &principal_mac,
                        &binding_hmac,
                        SessionLifecycleRejection::Busy,
                    )
                    .await;
            }
            Err(CommitBatchError::CapacityExceeded) | Err(CommitBatchError::SequenceExhausted) => {
                return self
                    .converge_or_reject(
                        &request,
                        &operation_id,
                        &principal_mac,
                        &binding_hmac,
                        SessionLifecycleRejection::Failed {
                            failure: SafeOperationFailure::new(
                                SessionOperationFailureKind::CapacityExceeded,
                                false,
                                "The local event store rejected the batch for capacity.",
                                correlation("capacity"),
                            ),
                        },
                    )
                    .await;
            }
            Err(CommitBatchError::StorageUnavailable { failure })
                if failure.is_shutdown_in_progress() =>
            {
                return Err(SessionLifecycleOperationError::ShutdownInProgress);
            }
            Err(CommitBatchError::StorageUnavailable { failure }) => {
                return self
                    .converge_or_reject(
                        &request,
                        &operation_id,
                        &principal_mac,
                        &binding_hmac,
                        SessionLifecycleRejection::Failed { failure },
                    )
                    .await;
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                return Ok(SessionLifecycleCommandResult::OutcomeUnknown {
                    request_id: request.request_id.clone(),
                });
            }
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                return Err(SessionLifecycleOperationError::Internal { correlation_id });
            }
        };

        let state = if !requires_runtime_effect {
            SessionLifecycleOperationState::Completed
        } else if committed_fresh {
            match self.effect_is_reserved(&operation_id, &obligation_id).await {
                Ok(true) => {
                    self.run_effect(&request, &operation_id, obligation_id, active_turn_id)
                        .await
                }
                Ok(false) | Err(_) => {
                    let state = SessionLifecycleOperationState::ReconciliationRequired {
                        failure: SafeOperationFailure::new(
                            SessionOperationFailureKind::StorageUnavailable,
                            true,
                            "The lifecycle effect reservation could not be confirmed.",
                            correlation("effect-reservation-readback"),
                        ),
                    };
                    self.save_state(&operation_id, &obligation_id, &state).await
                }
            }
        } else {
            // A concurrent retry owns the effect; report the saved state.
            match self.lookup_record(&operation_id).await {
                Ok(Some(record)) => externally_visible_state(&operation_id, record.state),
                _ => persistence_reconciliation("replay-operation-read"),
            }
        };
        Ok(SessionLifecycleCommandResult::Accepted { receipt, state })
    }

    async fn run_effect(
        &self,
        request: &SessionLifecycleRequest,
        operation_id: &str,
        obligation_id: String,
        active_turn_id: Option<u64>,
    ) -> SessionLifecycleOperationState {
        let effect = SessionLifecycleEffect {
            operation_id: operation_id.to_string(),
            session_id: request.session_id.clone(),
            action: request.action.clone(),
            active_turn_id,
        };
        let outcome = tokio::time::timeout(LIFECYCLE_DEADLINE, self.gate.execute(&effect)).await;
        let state = match outcome {
            Ok(Ok(())) => SessionLifecycleOperationState::Completed,
            Ok(Err(failure)) => SessionLifecycleOperationState::ReconciliationRequired { failure },
            Err(_) => SessionLifecycleOperationState::ReconciliationRequired {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::DeadlineExceeded,
                    true,
                    "The runtime did not confirm the lifecycle result within 10 seconds.",
                    correlation("deadline"),
                ),
            },
        };
        self.save_state(operation_id, &obligation_id, &state).await
    }

    /// Persist the resolved operation state and settle its obligation in one
    /// batch. A terminal state is public only after a fresh durable readback;
    /// every write/read ambiguity is conservatively reported as
    /// `ReconciliationRequired`.
    async fn save_state(
        &self,
        operation_id: &str,
        obligation_id: &str,
        state: &SessionLifecycleOperationState,
    ) -> SessionLifecycleOperationState {
        let record = match self.lookup_record(operation_id).await {
            Ok(Some(record)) => record,
            _ => return persistence_reconciliation("state-operation-read"),
        };
        if !matches!(record.state, SessionLifecycleOperationState::Accepted) {
            return record.state;
        }
        let Some(next_revision) = record.revision.next() else {
            return persistence_reconciliation("state-operation-revision");
        };
        let mut mutations = vec![LocalStateMutation::SessionLifecycleOperation(
            OperationRecordMutation {
                kind: OperationKind::SessionLifecycle,
                operation_id: operation_id.to_string(),
                receipt: receipt_record(
                    &record.receipt,
                    record.commit_operation_kind,
                    &record.principal_mac,
                    &record.binding_hmac,
                ),
                latest_status: status_record(state),
                expected: RevisionGuard::Expected(record.revision),
                revision: next_revision,
            },
        )];
        let obligation = match self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.to_string(),
            })
            .await
        {
            Ok(LocalEventQueryResult::ObligationByIdentity(Some(obligation))) => obligation,
            _ => return persistence_reconciliation("state-obligation-read"),
        };
        let Some(next_obligation_revision) = obligation.revision.next() else {
            return persistence_reconciliation("state-obligation-revision");
        };
        let Some((stored_obligation_id, stored_operation_id, stored_session_id, stored_action, _)) =
            lifecycle_obligation(&obligation.record)
        else {
            return persistence_reconciliation("state-obligation-record");
        };
        if stored_obligation_id != obligation_id
            || stored_operation_id != operation_id
            || stored_session_id != record.receipt.session_id
            || stored_action != &action_record(&record.receipt.action)
        {
            return persistence_reconciliation("state-obligation-binding");
        }
        let mut obligation_record = obligation.record.clone();
        let ObligationRecord::SessionClose {
            state: obligation_state,
            ..
        } = &mut obligation_record
        else {
            unreachable!("lifecycle obligation was checked above")
        };
        *obligation_state = if matches!(state, SessionLifecycleOperationState::Completed) {
            ObligationStateRecord::Completed
        } else {
            ObligationStateRecord::ReconciliationRequired
        };
        let pending = if matches!(state, SessionLifecycleOperationState::Completed) {
            None
        } else {
            obligation
                .pending
                .as_ref()
                .map(|pending| PendingIndexEntry {
                    ordered_key: pending.ordered_key.clone(),
                    owner: pending.owner.clone(),
                    partition: pending.partition,
                    shutdown_plan: pending.shutdown_plan.clone(),
                })
        };
        mutations.push(LocalStateMutation::Obligation(ObligationMutation {
            obligation_id: obligation_id.to_string(),
            record: obligation_record,
            pending,
            expected: RevisionGuard::Expected(obligation.revision),
            revision: next_obligation_revision,
        }));
        let commit_digest = self
            .authority
            .digest(format!("slc-state\0{operation_id}\0{}", next_revision.value()).as_bytes());
        let Ok(commit_id) = CommitIdentity::parse(&hex_encode(&commit_digest)) else {
            return persistence_reconciliation("state-commit-identity");
        };
        let batch = LocalAtomicBatch {
            commit_id,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: if record.commit_operation_kind
                    == CommitOperationKind::ShutdownTarget
                {
                    CommitOperationKind::ShutdownTarget
                } else {
                    CommitOperationKind::OperationProgress
                },
                idempotency_key: format!("{operation_id}.st{}", next_revision.value()),
                payload_hash: self.authority.digest(&status_identity_material(state)),
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: mutations,
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_))
            | Err(CommitBatchError::OutcomeUnknown { .. }) => {}
            Err(_) => return persistence_reconciliation("state-commit"),
        }
        let saved = match self.lookup_record(operation_id).await {
            Ok(Some(saved)) => saved,
            _ => return persistence_reconciliation("state-readback-operation"),
        };
        if &saved.state != state {
            return match saved.state {
                SessionLifecycleOperationState::Completed
                | SessionLifecycleOperationState::ReconciliationRequired { .. } => saved.state,
                SessionLifecycleOperationState::Accepted => {
                    persistence_reconciliation("state-readback-status")
                }
            };
        }
        let obligation = match self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.to_string(),
            })
            .await
        {
            Ok(LocalEventQueryResult::ObligationByIdentity(Some(obligation))) => obligation,
            _ => return persistence_reconciliation("state-readback-obligation"),
        };
        let expected_pending = !matches!(state, SessionLifecycleOperationState::Completed);
        let expected_state = if expected_pending {
            ObligationStateRecord::ReconciliationRequired
        } else {
            ObligationStateRecord::Completed
        };
        let obligation_state = lifecycle_obligation(&obligation.record).map(|parts| parts.4);
        if obligation_state != Some(expected_state)
            || obligation.pending.is_some() != expected_pending
        {
            return persistence_reconciliation("state-readback-participants");
        }
        saved.state
    }

    /// Stable point lookup by backend-issued operation identity or by the
    /// original caller request identity after an acceptance response was
    /// lost. Another principal sees `NotFound`; the result is never rebuilt
    /// from current session state.
    pub async fn get_operation(
        &self,
        principal: &str,
        operation_id: &str,
    ) -> Result<
        (SessionLifecycleReceipt, SessionLifecycleOperationState),
        SessionLifecycleOperationError,
    > {
        if operation_id.is_empty() || operation_id.len() > 128 {
            return Err(SessionLifecycleOperationError::InvalidRequest);
        }
        let principal_mac = self.authority.mac(&principal_material(principal));
        let mut record = self
            .lookup_record(operation_id)
            .await
            .map_err(lookup_query_error)?;
        if record.is_none() && validate_operation_identity(operation_id).is_ok() {
            let derived = self.operation_id_for(principal, operation_id);
            record = self
                .lookup_record(&derived)
                .await
                .map_err(lookup_query_error)?;
        }
        let Some(record) = record else {
            return Err(SessionLifecycleOperationError::NotFound);
        };
        if !constant_time_eq_32(&record.principal_mac, &principal_mac) {
            return Err(SessionLifecycleOperationError::NotFound);
        }
        let state = externally_visible_state(&record.receipt.operation_id, record.state);
        Ok((record.receipt, state))
    }
}

#[async_trait::async_trait]
impl SessionCloseRecoveryReadbackPort for SessionLifecycleOperationUsecase {
    async fn read_session_close(
        &self,
        request: &SessionCloseRecoveryReadbackRequest,
    ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
        use crate::domain::agent_session::events::RecoveryResultClassification;

        if request.effect_identity.as_str() != self.obligation_id(&request.session_id) {
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::InvalidEffectIntent,
                false,
                "The session-close readback identity does not match its durable target.",
                correlation("readback-identity"),
            ));
        }
        let record = self
            .lookup_record(&request.operation_id)
            .await
            .map_err(|_| {
                SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "The session lifecycle operation could not be read.",
                    correlation("readback-operation"),
                )
            })?
            .ok_or_else(|| {
                SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "The session lifecycle operation is unavailable for readback.",
                    correlation("readback-missing"),
                )
            })?;
        if record.receipt.operation_id != request.operation_id
            || record.receipt.session_id != request.session_id
        {
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::InvalidEffectIntent,
                false,
                "The session lifecycle operation no longer matches the accepted effect.",
                correlation("readback-binding"),
            ));
        }
        let durable_close_completed = if record.receipt.action == SessionLifecycleAction::Close
            && !matches!(record.state, SessionLifecycleOperationState::Completed)
        {
            let snapshot = self
                .gate
                .session_snapshot(&request.session_id)
                .await
                .map_err(|_| {
                    SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageUnavailable,
                        true,
                        "The session-close owner projection could not be read.",
                        correlation("readback-owner"),
                    )
                })?;
            matches!(snapshot.lifecycle, SessionLifecycleState::Closed)
        } else {
            false
        };
        let (classification, safe_result, owner_mutations) = if durable_close_completed {
            let revision = record.revision.next().ok_or_else(|| {
                SafeOperationFailure::new(
                    SessionOperationFailureKind::CapacityExceeded,
                    false,
                    "The session lifecycle operation revision is exhausted.",
                    correlation("readback-revision"),
                )
            })?;
            let source = self
                .repository
                .query(LocalEventQuery::ObligationByIdentity {
                    obligation_id: request.effect_identity.as_str().to_string(),
                })
                .await
                .map_err(|_| {
                    SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageUnavailable,
                        true,
                        "The session-close obligation could not be read.",
                        correlation("readback-obligation"),
                    )
                })?;
            let LocalEventQueryResult::ObligationByIdentity(Some(source)) = source else {
                return Err(SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "The session-close obligation is unavailable for readback.",
                    correlation("readback-obligation-missing"),
                ));
            };
            let Some((
                source_obligation_id,
                source_operation_id,
                source_session_id,
                source_action,
                _,
            )) = lifecycle_obligation(&source.record)
            else {
                return Err(SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "The session-close obligation is incompatible with readback.",
                    correlation("readback-obligation-shape"),
                ));
            };
            if source_obligation_id != request.effect_identity.as_str()
                || source_operation_id != request.operation_id
                || source_session_id != request.session_id
                || source_action != &SessionLifecycleRecordAction::Close
            {
                return Err(SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "The session-close obligation no longer matches the accepted effect.",
                    correlation("readback-obligation-binding"),
                ));
            }
            let source_revision = source.revision.next().ok_or_else(|| {
                SafeOperationFailure::new(
                    SessionOperationFailureKind::CapacityExceeded,
                    false,
                    "The session-close obligation revision is exhausted.",
                    correlation("readback-obligation-revision"),
                )
            })?;
            (
                RecoveryResultClassification::Succeeded,
                "The accepted session-close effect completed in the durable owner projection."
                    .to_string(),
                vec![
                    LocalStateMutation::SessionLifecycleOperation(OperationRecordMutation {
                        kind: OperationKind::SessionLifecycle,
                        operation_id: record.receipt.operation_id.clone(),
                        receipt: receipt_record(
                            &record.receipt,
                            record.commit_operation_kind,
                            &record.principal_mac,
                            &record.binding_hmac,
                        ),
                        latest_status: status_record(&SessionLifecycleOperationState::Completed),
                        expected: RevisionGuard::Expected(record.revision),
                        revision,
                    }),
                    LocalStateMutation::Obligation(ObligationMutation {
                        obligation_id: request.effect_identity.as_str().to_string(),
                        record: lifecycle_obligation_record(
                            request.effect_identity.as_str(),
                            &record.receipt,
                            ObligationStateRecord::Completed,
                        ),
                        pending: None,
                        expected: RevisionGuard::Expected(source.revision),
                        revision: source_revision,
                    }),
                ],
            )
        } else {
            let (classification, safe_result) = match record.state {
                SessionLifecycleOperationState::Accepted => (
                    RecoveryResultClassification::Pending,
                    "The accepted session lifecycle effect is still pending.".to_string(),
                ),
                SessionLifecycleOperationState::Completed => (
                    RecoveryResultClassification::Succeeded,
                    "The accepted session lifecycle effect completed.".to_string(),
                ),
                SessionLifecycleOperationState::ReconciliationRequired { .. } => (
                    RecoveryResultClassification::Ambiguous,
                    "The accepted session lifecycle effect still requires reconciliation."
                        .to_string(),
                ),
            };
            (classification, safe_result, Vec::new())
        };
        Ok(RecoveryEffectResult {
            classification,
            safe_result,
            owner_mutations,
            owner_batch: None,
        })
    }
}
