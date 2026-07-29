//! Durable Stop acceptance, deadline, and terminal resolution.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::domain::agent_session::aggregates::session::StopCommandRejection;
use crate::domain::agent_session::events::{
    AgentSessionDomainEvent, InterruptReason, ObligationKind, ObligationState, StopResolution,
};
use crate::domain::agent_session::repository::{
    AgentSessionLifecycleRepository, AgentSessionLifecycleRepositoryError,
};
use crate::domain::local_event::{
    CallerOperationKey, CommitBatchError, CommitBatchResult, CommitIdentity, CommitOperationKind,
    CommitResolution, ExpectedStreamHead, IdempotencyBinding, LoadStreamRequest, LocalAtomicBatch,
    LocalDomainEvent, LocalEventQuery, LocalEventQueryError, LocalEventQueryResult,
    LocalEventTransactionRepository, LocalStateMutation, ObligationMutation, ObligationRecord,
    ObligationStateRecord, OperationBindingMutation, OperationKind, OperationReceiptRecord,
    OperationRecordMutation, OperationStatusRecord, OperationStatusValue, PendingIndexEntry,
    PendingPartition, RecordAuthentication, Revision, RevisionGuard, SafeOperationFailure,
    SessionOperationFailureKind, StopResolutionKind, StopResolutionMutation, StreamId,
    StreamVersion, TerminalInterruptReasonRecord, TerminalRecordMutation, TerminalResultRecord,
    UncommittedDomainEvent,
};

use super::identity::{constant_time_eq_32, validate_operation_identity};
use super::ports::{
    AcceptedStopEffect, OperationBindingAuthority, RecoveryEffectResult, RecoveryOwnerBatch,
    StopEffectObservation, StopEffectPort, StopRecoveryReadbackPort, StopRecoveryReadbackRequest,
    TerminalParticipants,
};
use super::record::hex_encode;
use super::send::principal_material;

const STOP_DEADLINE: Duration = Duration::from_secs(10);
const TERMINAL_READBACK_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_UNRESOLVED_TARGETS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopOperationRequest {
    pub principal: String,
    pub request_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub expected_session_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopOperationReceipt {
    pub operation_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub accepted_revision: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StopOperationState {
    Accepted,
    Completed { resolution: StopResolution },
    ReconciliationRequired { failure: SafeOperationFailure },
}

#[derive(Debug, Clone, PartialEq)]
pub enum StopCommandOutcome {
    Accepted {
        receipt: StopOperationReceipt,
        state: StopOperationState,
    },
    RejectedBeforeCommit {
        failure: SafeOperationFailure,
    },
    OutcomeUnknown {
        request_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopOperationError {
    InvalidRequest,
    PayloadConflict,
    ShutdownInProgress,
    NotFound,
    CapacityExceeded,
    StaleTarget,
    QueryBusy,
    DeadlineExceeded,
    StorageUnavailable { failure: SafeOperationFailure },
    Internal { correlation_id: String },
}

struct StoredStop {
    receipt: StopOperationReceipt,
    state: StopOperationState,
    principal_mac: [u8; 32],
    binding_hmac: [u8; 32],
    revision: Revision,
}

struct PendingStopObligation {
    obligation_id: String,
    operation_id: String,
    session_id: String,
    turn_id: String,
    deadline_ms: i64,
}

struct StopTerminalResolution<'a> {
    receipt: &'a StopOperationReceipt,
    principal_mac: [u8; 32],
    binding_hmac: [u8; 32],
    obligation_id: String,
    state: &'a StopOperationState,
    observation: Option<StopEffectObservation>,
    terminal_winner: Option<crate::domain::local_event::TerminalRecordView>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StopTerminalCommitOutcome {
    NotCommitted,
    Committed,
    OwnTimeoutCommitted,
}

impl StopTerminalCommitOutcome {
    fn committed(own_timeout_candidate: bool) -> Self {
        if own_timeout_candidate {
            Self::OwnTimeoutCommitted
        } else {
            Self::Committed
        }
    }

    fn for_stored(record: &StoredStop, own_timeout_candidate: bool) -> Self {
        match &record.state {
            StopOperationState::Completed {
                resolution: StopResolution::Succeeded,
            } if own_timeout_candidate => Self::OwnTimeoutCommitted,
            StopOperationState::Completed { .. } => Self::Committed,
            StopOperationState::Accepted | StopOperationState::ReconciliationRequired { .. } => {
                Self::NotCommitted
            }
        }
    }

    fn is_committed(self) -> bool {
        !matches!(self, Self::NotCommitted)
    }

    fn own_timeout_committed(self) -> bool {
        matches!(self, Self::OwnTimeoutCommitted)
    }
}

fn is_own_timeout_candidate(
    state: &StopOperationState,
    observation: Option<StopEffectObservation>,
    has_terminal_winner: bool,
) -> bool {
    matches!(
        state,
        StopOperationState::Completed {
            resolution: StopResolution::Succeeded,
        }
    ) && observation
        .is_some_and(|observation| observation.terminal_reason == Some(InterruptReason::Timeout))
        && !has_terminal_winner
}

enum StopJoinBindingDisposition {
    Saved,
    Rejected(SafeOperationFailure),
    OutcomeUnknown,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or(0)
}

fn correlation(label: &str) -> String {
    format!("stop-{label}-{:x}", now_ms())
}

fn storage_failure(label: &str) -> SafeOperationFailure {
    SafeOperationFailure::new(
        SessionOperationFailureKind::StorageUnavailable,
        true,
        "The Stop operation could not reach the local event store.",
        correlation(label),
    )
}

fn lifecycle_repository_failure(
    error: AgentSessionLifecycleRepositoryError,
    label: &str,
) -> SafeOperationFailure {
    let (kind, correlation_id) = match error {
        AgentSessionLifecycleRepositoryError::Corrupt(_) => (
            SessionOperationFailureKind::StorageCorrupt,
            correlation(label),
        ),
        AgentSessionLifecycleRepositoryError::Unavailable(correlation_id)
            if !correlation_id.is_empty() =>
        {
            (
                SessionOperationFailureKind::StorageUnavailable,
                correlation_id,
            )
        }
        AgentSessionLifecycleRepositoryError::NotFound
        | AgentSessionLifecycleRepositoryError::Unavailable(_) => (
            SessionOperationFailureKind::StorageUnavailable,
            correlation(label),
        ),
    };
    SafeOperationFailure::new(
        kind,
        true,
        "The Stop session snapshot is unavailable.",
        correlation_id,
    )
}

fn lookup_query_error(error: LocalEventQueryError) -> StopOperationError {
    match error {
        LocalEventQueryError::InvalidRequest => StopOperationError::InvalidRequest,
        LocalEventQueryError::NotFound => StopOperationError::NotFound,
        LocalEventQueryError::QueryBusy => StopOperationError::QueryBusy,
        LocalEventQueryError::DeadlineExceeded => StopOperationError::DeadlineExceeded,
        LocalEventQueryError::StorageUnavailable { failure } => {
            StopOperationError::StorageUnavailable { failure }
        }
        LocalEventQueryError::Corrupt { correlation_id }
        | LocalEventQueryError::IncompatibleStoredEvent { correlation_id }
        | LocalEventQueryError::ReplayRequired { correlation_id }
        | LocalEventQueryError::Internal { correlation_id } => {
            StopOperationError::Internal { correlation_id }
        }
        other => StopOperationError::Internal {
            correlation_id: correlation(&format!("get-{other:?}")),
        },
    }
}

fn receipt_record(
    receipt: &StopOperationReceipt,
    principal_mac: &[u8; 32],
    binding_hmac: &[u8; 32],
) -> OperationReceiptRecord {
    OperationReceiptRecord::Stop {
        operation_id: receipt.operation_id.clone(),
        session_id: receipt.session_id.clone(),
        turn_id: receipt.turn_id.clone(),
        accepted_revision: receipt.accepted_revision,
        authentication: RecordAuthentication {
            principal_mac: *principal_mac,
            binding_hmac: *binding_hmac,
        },
    }
}

fn status_record(state: &StopOperationState) -> OperationStatusRecord {
    let value = match state {
        StopOperationState::Accepted => OperationStatusValue::Accepted,
        StopOperationState::Completed { resolution } => OperationStatusValue::StopCompleted {
            resolution: *resolution,
        },
        StopOperationState::ReconciliationRequired { failure } => {
            OperationStatusValue::ReconciliationRequired {
                failure: failure.clone(),
            }
        }
    };
    OperationStatusRecord {
        kind: OperationKind::Stop,
        value,
    }
}

fn decode_record(
    receipt: OperationReceiptRecord,
    status: OperationStatusRecord,
    revision: Revision,
) -> Option<StoredStop> {
    let (operation_id, session_id, turn_id, accepted_revision, authentication) = match receipt {
        OperationReceiptRecord::Stop {
            operation_id,
            session_id,
            turn_id,
            accepted_revision,
            authentication,
        } => (
            operation_id,
            session_id,
            turn_id,
            accepted_revision,
            authentication,
        ),
        OperationReceiptRecord::Send { .. }
        | OperationReceiptRecord::PermissionResponse { .. }
        | OperationReceiptRecord::SessionLifecycle { .. }
        | OperationReceiptRecord::ApplicationQuit { .. } => return None,
    };
    if status.kind != OperationKind::Stop {
        return None;
    }
    let state = match status.value {
        OperationStatusValue::Accepted => StopOperationState::Accepted,
        OperationStatusValue::StopCompleted { resolution } => {
            StopOperationState::Completed { resolution }
        }
        OperationStatusValue::ReconciliationRequired { failure } => {
            StopOperationState::ReconciliationRequired { failure }
        }
        OperationStatusValue::AwaitingProviderStart { .. }
        | OperationStatusValue::AwaitingProviderResponse { .. }
        | OperationStatusValue::Queued { .. }
        | OperationStatusValue::ProviderStartReserved { .. }
        | OperationStatusValue::Running { .. }
        | OperationStatusValue::Completed
        | OperationStatusValue::PermissionCompleted { .. }
        | OperationStatusValue::Preparing
        | OperationStatusValue::Activated
        | OperationStatusValue::ExitPending
        | OperationStatusValue::Exited
        | OperationStatusValue::OutcomeUnknown { .. }
        | OperationStatusValue::FailedBeforeActivation { .. }
        | OperationStatusValue::Failed { .. }
        | OperationStatusValue::Terminal { .. } => return None,
    };
    Some(StoredStop {
        receipt: StopOperationReceipt {
            operation_id,
            session_id,
            turn_id,
            accepted_revision,
        },
        state,
        principal_mac: authentication.principal_mac,
        binding_hmac: authentication.binding_hmac,
        revision,
    })
}

fn stop_obligation(
    record: &ObligationRecord,
) -> Option<(&str, &str, &str, u64, i64, ObligationStateRecord)> {
    match record {
        ObligationRecord::RecoveryTransition { original, .. }
        | ObligationRecord::Observed { original, .. } => stop_obligation(original),
        ObligationRecord::StopInterrupt {
            operation_id,
            session_id,
            turn_id,
            expected_revision,
            deadline_ms,
            state,
        } => Some((
            operation_id,
            session_id,
            turn_id,
            *expected_revision,
            *deadline_ms,
            *state,
        )),
        ObligationRecord::Send { .. }
        | ObligationRecord::PermissionResponse { .. }
        | ObligationRecord::SessionClose { .. }
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

fn stop_obligation_record(
    receipt: &StopOperationReceipt,
    deadline_ms: i64,
    state: ObligationStateRecord,
) -> ObligationRecord {
    ObligationRecord::StopInterrupt {
        operation_id: receipt.operation_id.clone(),
        session_id: receipt.session_id.clone(),
        turn_id: receipt.turn_id.clone(),
        expected_revision: receipt.accepted_revision,
        deadline_ms,
        state,
    }
}

fn status_identity_material(state: &StopOperationState) -> Vec<u8> {
    match state {
        StopOperationState::Accepted => b"accepted".to_vec(),
        StopOperationState::Completed { resolution } => match resolution {
            StopResolution::Succeeded => b"completed\0succeeded".to_vec(),
            StopResolution::Superseded => b"completed\0superseded".to_vec(),
        },
        StopOperationState::ReconciliationRequired { failure } => {
            format!("reconciliation_required\0{}", failure.correlation_id).into_bytes()
        }
    }
}

pub struct StopOperationUsecase {
    repository: Arc<dyn LocalEventTransactionRepository>,
    authority: Arc<dyn OperationBindingAuthority>,
    lifecycle_repository: Arc<dyn AgentSessionLifecycleRepository>,
    effect: Arc<dyn StopEffectPort>,
    installation_id: String,
}

impl StopOperationUsecase {
    pub fn new(
        repository: Arc<dyn LocalEventTransactionRepository>,
        authority: Arc<dyn OperationBindingAuthority>,
        lifecycle_repository: Arc<dyn AgentSessionLifecycleRepository>,
        effect: Arc<dyn StopEffectPort>,
        installation_id: String,
    ) -> Self {
        Self {
            repository,
            authority,
            lifecycle_repository,
            effect,
            installation_id,
        }
    }

    fn operation_id(&self, principal: &str, request_id: &str) -> String {
        hex_encode(
            &self
                .authority
                .mac(format!("stop-backend-operation/v1\0{principal}\0{request_id}").as_bytes()),
        )
    }

    async fn lookup(&self, operation_id: &str) -> Result<Option<StoredStop>, LocalEventQueryError> {
        let result = self
            .repository
            .query(LocalEventQuery::OperationByIdentity {
                kind: OperationKind::Stop,
                operation_id: operation_id.to_string(),
            })
            .await?;
        let LocalEventQueryResult::OperationByIdentity(record) = result else {
            return Err(LocalEventQueryError::Internal {
                correlation_id: correlation("lookup-shape"),
            });
        };
        let Some(record) = record else {
            return Ok(None);
        };
        let decoded = decode_record(record.receipt, record.latest_status, record.revision)
            .ok_or_else(|| LocalEventQueryError::Corrupt {
                correlation_id: correlation("operation-decode"),
            })?;
        if let StopOperationState::Completed { resolution } = &decoded.state {
            let result = self
                .repository
                .query(LocalEventQuery::StopResolutionByOperation {
                    stop_operation_id: operation_id.to_string(),
                })
                .await?;
            let LocalEventQueryResult::StopResolutionByOperation(saved) = result else {
                return Err(LocalEventQueryError::Internal {
                    correlation_id: correlation("resolution-shape"),
                });
            };
            let expected = match resolution {
                StopResolution::Succeeded => StopResolutionKind::Succeeded,
                StopResolution::Superseded => StopResolutionKind::Superseded,
            };
            if saved.as_ref().map(|saved| saved.resolution) != Some(expected) {
                return Err(LocalEventQueryError::Corrupt {
                    correlation_id: correlation("completed-resolution"),
                });
            }
        }
        Ok(Some(decoded))
    }

    fn caller_key(&self, request: &StopOperationRequest) -> CallerOperationKey {
        CallerOperationKey {
            principal: request.principal.clone(),
            installation_id: self.installation_id.clone(),
            kind: OperationKind::Stop,
            caller_request_id: request.request_id.clone(),
        }
    }

    fn binding_for(&self, request: &StopOperationRequest, operation_id: &str) -> Vec<u8> {
        super::binding::stop(
            &request.principal,
            &self.installation_id,
            &request.request_id,
            operation_id,
            &request.session_id,
            &request.turn_id,
            request.expected_session_revision,
        )
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

    async fn save_join_binding(
        &self,
        request: &StopOperationRequest,
        operation_id: &str,
        binding: &[u8],
        binding_hmac: [u8; 32],
    ) -> Result<StopJoinBindingDisposition, StopOperationError> {
        let commit_id = CommitIdentity::parse(&hex_encode(
            &self.authority.digest(
                format!(
                    "stop-join/v1\0{}\0{}\0{}",
                    request.principal, request.request_id, operation_id
                )
                .as_bytes(),
            ),
        ))
        .map_err(|_| StopOperationError::Internal {
            correlation_id: correlation("join-identity"),
        })?;
        let batch = LocalAtomicBatch {
            commit_id: commit_id.clone(),
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: OperationKind::Stop.into(),
                idempotency_key: format!("{operation_id}.join.{}", request.request_id),
                payload_hash: self.authority.digest(binding),
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
                Ok(StopJoinBindingDisposition::Saved)
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                match self.repository.resolve_commit(commit_id).await {
                    Ok(CommitResolution::Committed(_)) => Ok(StopJoinBindingDisposition::Saved),
                    Ok(CommitResolution::NotCommitted) => Ok(StopJoinBindingDisposition::Rejected(
                        SafeOperationFailure::new(
                            SessionOperationFailureKind::PersistFailure,
                            true,
                            "The Stop join was not committed.",
                            correlation("join-not-committed"),
                        ),
                    )),
                    Err(_) => Ok(StopJoinBindingDisposition::OutcomeUnknown),
                }
            }
            Err(CommitBatchError::StorageUnavailable { failure })
                if failure.is_shutdown_in_progress() =>
            {
                Err(StopOperationError::ShutdownInProgress)
            }
            Err(CommitBatchError::StorageUnavailable { failure }) => {
                Ok(StopJoinBindingDisposition::Rejected(failure))
            }
            Err(CommitBatchError::PayloadConflict | CommitBatchError::EffectAdmissionBlocked) => {
                let saved = self
                    .lookup_binding(&self.caller_key(request))
                    .await
                    .map_err(|_| StopOperationError::Internal {
                        correlation_id: correlation("join-conflict-readback"),
                    })?;
                if saved.as_ref().is_some_and(|saved| {
                    saved.operation_id == operation_id
                        && constant_time_eq_32(&saved.binding_hmac, &binding_hmac)
                }) {
                    Ok(StopJoinBindingDisposition::Saved)
                } else {
                    Err(StopOperationError::PayloadConflict)
                }
            }
            Err(CommitBatchError::CapacityExceeded | CommitBatchError::SequenceExhausted) => {
                Err(StopOperationError::CapacityExceeded)
            }
            Err(CommitBatchError::StreamHeadConflict { .. }) => Ok(
                StopJoinBindingDisposition::Rejected(storage_failure("join-conflict")),
            ),
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                Err(StopOperationError::Internal { correlation_id })
            }
        }
    }

    async fn joined_outcome(
        &self,
        request: &StopOperationRequest,
        operation_id: &str,
        record: StoredStop,
    ) -> Result<StopCommandOutcome, StopOperationError> {
        let principal_mac = self.authority.mac(&principal_material(&request.principal));
        if !constant_time_eq_32(&record.principal_mac, &principal_mac) {
            return Err(StopOperationError::NotFound);
        }
        let binding = self.binding_for(request, operation_id);
        let binding_hmac = self.authority.mac(&binding);
        match self
            .save_join_binding(request, operation_id, &binding, binding_hmac)
            .await?
        {
            StopJoinBindingDisposition::Saved => Ok(StopCommandOutcome::Accepted {
                receipt: record.receipt,
                state: record.state,
            }),
            StopJoinBindingDisposition::Rejected(failure) => {
                Ok(StopCommandOutcome::RejectedBeforeCommit { failure })
            }
            StopJoinBindingDisposition::OutcomeUnknown => Ok(StopCommandOutcome::OutcomeUnknown {
                request_id: request.request_id.clone(),
            }),
        }
    }

    async fn head(&self, stream_id: &StreamId) -> Result<i64, LocalEventQueryError> {
        Ok(self
            .repository
            .load_stream(LoadStreamRequest {
                stream_id: stream_id.clone(),
                after: None,
                limit: 1,
            })
            .await?
            .head
            .value())
    }

    async fn terminal_winner(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Option<crate::domain::local_event::TerminalRecordView>, LocalEventQueryError> {
        let result = self
            .repository
            .query(LocalEventQuery::TerminalByTurn {
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
            })
            .await?;
        let LocalEventQueryResult::TerminalByTurn(value) = result else {
            return Err(LocalEventQueryError::Internal {
                correlation_id: correlation("terminal-shape"),
            });
        };
        Ok(value)
    }

    async fn wait_for_terminal_winner(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<crate::domain::local_event::TerminalRecordView, SafeOperationFailure> {
        loop {
            if let Some(winner) = self
                .terminal_winner(session_id, turn_id)
                .await
                .map_err(|_| storage_failure("terminal-readback"))?
            {
                return Ok(winner);
            }
            tokio::time::sleep(TERMINAL_READBACK_POLL_INTERVAL).await;
        }
    }

    async fn wait_for_terminal_after_interrupt_handoff(
        &self,
        effect: &AcceptedStopEffect,
    ) -> Result<
        (
            Option<StopEffectObservation>,
            Option<crate::domain::local_event::TerminalRecordView>,
        ),
        SafeOperationFailure,
    > {
        let observation = self.effect.interrupt(effect).await?;
        // Test and compatibility gates may still return an explicit terminal
        // observation. The production runtime gate always returns `None`: a
        // successful provider write is only a handoff, never terminal proof.
        if observation.terminal_reason.is_some() {
            let winner = self
                .terminal_winner(&effect.session_id, &effect.turn_id)
                .await
                .map_err(|_| storage_failure("terminal-readback"))?;
            return Ok((winner.is_none().then_some(observation), winner));
        }
        let winner = self
            .wait_for_terminal_winner(&effect.session_id, &effect.turn_id)
            .await?;
        Ok((None, Some(winner)))
    }

    fn target_obligation_id(&self, session_id: &str, turn_id: &str) -> String {
        format!(
            "stop-target-{}",
            hex_encode(
                &self.authority.digest(
                    format!("stop-target-obligation/v1\0{session_id}\0{turn_id}").as_bytes(),
                )
            )
        )
    }

    /// Same-target Stop joins are an exact point lookup. Admission capacity
    /// is enforced atomically by the store's `stop-target-` prefix guard; the
    /// command path never scans the pending-recovery inventory.
    async fn pending_stop_for_target(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Option<PendingStopObligation>, StopOperationError> {
        let obligation_id = self.target_obligation_id(session_id, turn_id);
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.clone(),
            })
            .await
            .map_err(lookup_query_error)?;
        let LocalEventQueryResult::ObligationByIdentity(obligation) = result else {
            return Err(StopOperationError::Internal {
                correlation_id: correlation("target-obligation-shape"),
            });
        };
        let Some(obligation) = obligation else {
            return Ok(None);
        };
        if obligation.pending.is_none() {
            return Ok(None);
        }
        let Some((operation_id, stored_session_id, stored_turn_id, _, deadline_ms, _)) =
            stop_obligation(&obligation.record)
        else {
            return Err(StopOperationError::Internal {
                correlation_id: correlation("target-obligation-integrity"),
            });
        };
        if stored_session_id != session_id || stored_turn_id != turn_id {
            return Err(StopOperationError::Internal {
                correlation_id: correlation("target-obligation-integrity"),
            });
        }
        Ok(Some(PendingStopObligation {
            obligation_id,
            operation_id: operation_id.to_string(),
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            deadline_ms,
        }))
    }

    /// Prepare the Stop-owned participants for a runtime terminal winner.
    /// The caller commits these CAS mutations in the same batch as the
    /// `(session, turn)` terminal record and canonical session projection.
    pub(crate) async fn prepare_runtime_terminal_participants(
        &self,
        terminal: &TerminalRecordMutation,
    ) -> Result<TerminalParticipants, String> {
        let Some(pending) = self
            .pending_stop_for_target(&terminal.session_id, &terminal.turn_id)
            .await
            .map_err(|error| format!("runtime terminal Stop lookup failed: {error:?}"))?
        else {
            return Ok(TerminalParticipants::default());
        };
        let obligation = match self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: pending.obligation_id.clone(),
            })
            .await
            .map_err(|_| "runtime terminal Stop obligation lookup failed".to_string())?
        {
            LocalEventQueryResult::ObligationByIdentity(Some(obligation))
                if obligation.pending.is_some() =>
            {
                obligation
            }
            LocalEventQueryResult::ObligationByIdentity(_) => {
                return Ok(TerminalParticipants::default())
            }
            _ => return Err("runtime terminal Stop obligation query shape is invalid".into()),
        };
        let operation = self
            .lookup(&pending.operation_id)
            .await
            .map_err(|_| "runtime terminal Stop operation lookup failed".to_string())?
            .ok_or_else(|| "runtime terminal Stop operation is missing".to_string())?;
        if operation.receipt.session_id != terminal.session_id
            || operation.receipt.turn_id != terminal.turn_id
        {
            return Err("runtime terminal Stop target binding is inconsistent".into());
        }
        if matches!(operation.state, StopOperationState::Completed { .. }) {
            return Err("completed Stop operation still owns a pending obligation".into());
        }
        let next_operation_revision = operation
            .revision
            .next()
            .ok_or_else(|| "runtime terminal Stop operation revision is exhausted".to_string())?;
        let next_obligation_revision = obligation
            .revision
            .next()
            .ok_or_else(|| "runtime terminal Stop obligation revision is exhausted".to_string())?;
        let Some((
            obligation_operation_id,
            obligation_session_id,
            obligation_turn_id,
            obligation_expected_revision,
            obligation_deadline_ms,
            _,
        )) = stop_obligation(&obligation.record)
        else {
            return Err("runtime terminal Stop obligation is incompatible".to_string());
        };
        if obligation_operation_id != operation.receipt.operation_id
            || obligation_session_id != operation.receipt.session_id
            || obligation_turn_id != operation.receipt.turn_id
            || obligation_expected_revision != operation.receipt.accepted_revision
        {
            return Err("runtime terminal Stop obligation binding is inconsistent".into());
        }
        let owns_terminal = matches!(
            &terminal.result,
            TerminalResultRecord::Stop {
                operation_id,
                ..
            } if operation_id == &operation.receipt.operation_id
        );
        let resolution = if owns_terminal {
            StopResolution::Succeeded
        } else {
            StopResolution::Superseded
        };
        let completed_state = StopOperationState::Completed { resolution };
        let resolution_detail = if owns_terminal {
            terminal.result.clone()
        } else {
            TerminalResultRecord::StopSuperseded {
                terminal_identity: terminal.terminal_identity.clone(),
                // The terminal participant digest is the durable semantic binding
                // available at this layer; persistence encoding remains gateway-owned.
                terminal_result_sha256: terminal.participant_digest,
            }
        };
        let operation_id = operation.receipt.operation_id.clone();
        let turn_id = operation
            .receipt
            .turn_id
            .parse::<u64>()
            .map_err(|_| "runtime terminal Stop turn identity is invalid".to_string())?;
        let mutations = vec![
            LocalStateMutation::OperationRecord(OperationRecordMutation {
                kind: OperationKind::Stop,
                operation_id: operation.receipt.operation_id.clone(),
                receipt: receipt_record(
                    &operation.receipt,
                    &operation.principal_mac,
                    &operation.binding_hmac,
                ),
                latest_status: status_record(&completed_state),
                expected: RevisionGuard::Expected(operation.revision),
                revision: next_operation_revision,
            }),
            LocalStateMutation::StopResolution(StopResolutionMutation {
                stop_operation_id: operation_id.clone(),
                resolution: match resolution {
                    StopResolution::Succeeded => StopResolutionKind::Succeeded,
                    StopResolution::Superseded => StopResolutionKind::Superseded,
                },
                detail: resolution_detail,
            }),
            LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: pending.obligation_id,
                record: stop_obligation_record(
                    &operation.receipt,
                    obligation_deadline_ms,
                    ObligationStateRecord::Completed,
                ),
                pending: None,
                expected: RevisionGuard::Expected(obligation.revision),
                revision: next_obligation_revision,
            }),
        ];
        Ok(TerminalParticipants {
            events: vec![AgentSessionDomainEvent::StopResolutionRecorded {
                operation_id,
                turn_id,
                resolution,
                at: now_ms() as f64,
            }],
            mutations,
        })
    }

    async fn pending_stop_obligations(
        &self,
    ) -> Result<Vec<PendingStopObligation>, StopOperationError> {
        let mut cursor = None;
        let mut pending = Vec::new();
        loop {
            let result = self
                .repository
                .query(LocalEventQuery::PendingRecoveryPage {
                    limit: 200,
                    partition: Some(PendingPartition::Owner),
                    owner: None,
                    ordered_key_prefix: None,
                    shutdown_plan: None,
                    cursor,
                })
                .await
                .map_err(|_| StopOperationError::Internal {
                    correlation_id: correlation("pending-query"),
                })?;
            let LocalEventQueryResult::PendingRecoveryPage(page) = result else {
                return Err(StopOperationError::Internal {
                    correlation_id: correlation("pending-shape"),
                });
            };
            for entry in page.entries {
                let stop_identity = entry.obligation_id.starts_with("stop-target-");
                let Some((operation_id, session_id, turn_id, _, deadline_ms, _)) =
                    stop_obligation(&entry.record)
                else {
                    if stop_identity {
                        return Err(StopOperationError::Internal {
                            correlation_id: correlation("pending-obligation-record"),
                        });
                    }
                    continue;
                };
                pending.push(PendingStopObligation {
                    obligation_id: entry.obligation_id,
                    operation_id: operation_id.to_string(),
                    session_id: session_id.to_string(),
                    turn_id: turn_id.to_string(),
                    deadline_ms,
                });
                if pending.len() > MAX_UNRESOLVED_TARGETS {
                    return Ok(pending);
                }
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(pending);
            }
        }
    }

    /// Fresh point readback required after the acceptance COMMIT and before
    /// crossing the provider I/O boundary.
    async fn stop_effect_is_reserved(
        &self,
        operation_id: &str,
        obligation_id: &str,
    ) -> Result<bool, StopOperationError> {
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
        let Some((record_operation_id, _, _, _, _, state)) = stop_obligation(&obligation.record)
        else {
            return Err(StopOperationError::Internal {
                correlation_id: correlation("reservation-readback-record"),
            });
        };
        Ok(record_operation_id == operation_id && state == ObligationStateRecord::EffectReserved)
    }

    /// Atomically claims a migrated/legacy `pending` Stop obligation during
    /// startup recovery. A replay, unknown outcome, or pre-existing claim
    /// never dispatches a second interrupt.
    async fn reserve_stop_effect(
        &self,
        operation_id: &str,
        obligation_id: &str,
    ) -> Result<bool, StopOperationError> {
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.to_string(),
            })
            .await
            .map_err(lookup_query_error)?;
        let LocalEventQueryResult::ObligationByIdentity(Some(obligation)) = result else {
            return Err(StopOperationError::Internal {
                correlation_id: correlation("reserve-obligation-missing"),
            });
        };
        let Some((record_operation_id, _, _, _, _, state)) = stop_obligation(&obligation.record)
        else {
            return Err(StopOperationError::Internal {
                correlation_id: correlation("reserve-obligation-record"),
            });
        };
        if record_operation_id != operation_id {
            return Err(StopOperationError::Internal {
                correlation_id: correlation("reserve-obligation-binding"),
            });
        }
        match state {
            ObligationStateRecord::Pending => {}
            ObligationStateRecord::EffectReserved
            | ObligationStateRecord::Completed
            | ObligationStateRecord::ReconciliationRequired => {
                return Ok(false);
            }
            ObligationStateRecord::Prepared
            | ObligationStateRecord::Running
            | ObligationStateRecord::WaitingApproval
            | ObligationStateRecord::OutcomeUnknown
            | ObligationStateRecord::Failed
            | ObligationStateRecord::Cancelled => {
                return Err(StopOperationError::Internal {
                    correlation_id: correlation("reserve-obligation-state"),
                });
            }
        }
        let next_revision = obligation
            .revision
            .next()
            .ok_or(StopOperationError::CapacityExceeded)?;
        let mut record = obligation.record.clone();
        let ObligationRecord::StopInterrupt { state, .. } = &mut record else {
            unreachable!("Stop obligation was checked above")
        };
        *state = ObligationStateRecord::EffectReserved;
        let pending = obligation
            .pending
            .as_ref()
            .map(|pending| PendingIndexEntry {
                ordered_key: pending.ordered_key.clone(),
                owner: pending.owner.clone(),
                partition: pending.partition,
                shutdown_plan: pending.shutdown_plan.clone(),
            });
        let binding = format!(
            "stop-effect-reservation/v1\0{operation_id}\0{obligation_id}\0{}",
            obligation.revision.value()
        );
        let binding_hash = self.authority.digest(binding.as_bytes());
        let commit_id = CommitIdentity::parse(&hex_encode(&binding_hash)).map_err(|_| {
            StopOperationError::Internal {
                correlation_id: correlation("reserve-commit-id"),
            }
        })?;
        let batch = LocalAtomicBatch {
            commit_id,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::OperationProgress,
                idempotency_key: format!(
                    "{operation_id}.effect-reservation.{}",
                    obligation.revision.value()
                ),
                payload_hash: binding_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: obligation_id.to_string(),
                record,
                pending,
                expected: RevisionGuard::Expected(obligation.revision),
                revision: next_revision,
            })],
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_)) => Ok(true),
            Ok(CommitBatchResult::Replayed(_))
            | Err(CommitBatchError::OutcomeUnknown { .. })
            | Err(CommitBatchError::StreamHeadConflict { .. })
            | Err(CommitBatchError::PayloadConflict)
            | Err(CommitBatchError::EffectAdmissionBlocked) => Ok(false),
            Err(CommitBatchError::StorageUnavailable { failure })
                if failure.is_shutdown_in_progress() =>
            {
                Err(StopOperationError::ShutdownInProgress)
            }
            Err(CommitBatchError::StorageUnavailable { failure }) => {
                Err(StopOperationError::StorageUnavailable { failure })
            }
            Err(CommitBatchError::CapacityExceeded | CommitBatchError::SequenceExhausted) => {
                Err(StopOperationError::CapacityExceeded)
            }
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                Err(StopOperationError::Internal { correlation_id })
            }
        }
    }

    pub async fn request(
        &self,
        request: StopOperationRequest,
    ) -> Result<StopCommandOutcome, StopOperationError> {
        if validate_operation_identity(&request.request_id).is_err()
            || request.session_id.is_empty()
            || request.turn_id.is_empty()
            || request.expected_session_revision > i64::MAX as u64
        {
            return Err(StopOperationError::InvalidRequest);
        }
        let caller_key = self.caller_key(&request);
        let saved_binding =
            self.lookup_binding(&caller_key)
                .await
                .map_err(|_| StopOperationError::Internal {
                    correlation_id: correlation("binding-lookup"),
                })?;
        if let Some(saved_binding) = saved_binding {
            let binding = self.binding_for(&request, &saved_binding.operation_id);
            let binding_hmac = self.authority.mac(&binding);
            if !constant_time_eq_32(&saved_binding.binding_hmac, &binding_hmac) {
                return Err(StopOperationError::PayloadConflict);
            }
            let record = self
                .lookup(&saved_binding.operation_id)
                .await
                .map_err(|_| StopOperationError::Internal {
                    correlation_id: correlation("bound-operation-lookup"),
                })?
                .ok_or_else(|| StopOperationError::Internal {
                    correlation_id: correlation("bound-operation-missing"),
                })?;
            let principal_mac = self.authority.mac(&principal_material(&request.principal));
            if !constant_time_eq_32(&record.principal_mac, &principal_mac) {
                return Err(StopOperationError::NotFound);
            }
            return Ok(StopCommandOutcome::Accepted {
                receipt: record.receipt,
                state: record.state,
            });
        }

        let operation_id = self.operation_id(&request.principal, &request.request_id);
        let principal_mac = self.authority.mac(&principal_material(&request.principal));
        let binding = self.binding_for(&request, &operation_id);
        let binding_hmac = self.authority.mac(&binding);
        if self
            .lookup(&operation_id)
            .await
            .map_err(|_| StopOperationError::Internal {
                correlation_id: correlation("unbound-operation-lookup"),
            })?
            .is_some()
        {
            return Err(StopOperationError::Internal {
                correlation_id: correlation("unbound-operation"),
            });
        }

        if let Some(joined) = self
            .pending_stop_for_target(&request.session_id, &request.turn_id)
            .await?
        {
            if let Some(record) = self.lookup(&joined.operation_id).await.map_err(|_| {
                StopOperationError::Internal {
                    correlation_id: correlation("join"),
                }
            })? {
                return self
                    .joined_outcome(&request, &joined.operation_id, record)
                    .await;
            }
        }
        let mut session = self
            .lifecycle_repository
            .restore_session(&request.session_id)
            .await
            .map_err(|error| StopOperationError::StorageUnavailable {
                failure: lifecycle_repository_failure(error, "snapshot"),
            })?;
        let stop_transition = session
            .apply_stop_command(request.expected_session_revision, &request.turn_id)
            .map_err(|rejection| match rejection {
                StopCommandRejection::InvalidTurnIdentity => StopOperationError::InvalidRequest,
                StopCommandRejection::Transition(_) => StopOperationError::StaleTarget,
            })?;
        let turn_id = stop_transition.turn_id;
        let queue_was_paused = stop_transition.queue_was_paused;

        let stream_id = StreamId::agent_session(&request.session_id)
            .map_err(|_| StopOperationError::InvalidRequest)?;
        let head = self
            .head(&stream_id)
            .await
            .map_err(|_| StopOperationError::Internal {
                correlation_id: correlation("head"),
            })?;
        let deadline = tokio::time::Instant::now() + STOP_DEADLINE;
        let at = now_ms();
        let obligation_id = self.target_obligation_id(&request.session_id, &request.turn_id);
        let receipt = StopOperationReceipt {
            operation_id: operation_id.clone(),
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            accepted_revision: request.expected_session_revision,
        };
        let mut events = vec![
            UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: LocalDomainEvent::AgentSession(
                    AgentSessionDomainEvent::StopOperationAccepted {
                        operation_id: operation_id.clone(),
                        target_turn_id: turn_id,
                        at: at as f64,
                    },
                ),
                occurred_at_ms: at,
            },
            UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: LocalDomainEvent::AgentSession(
                    AgentSessionDomainEvent::TurnInterruptRequested {
                        turn_id,
                        at: at as f64,
                    },
                ),
                occurred_at_ms: at,
            },
            UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: LocalDomainEvent::AgentSession(
                    AgentSessionDomainEvent::ObligationRecorded {
                        obligation_id: obligation_id.clone(),
                        kind: ObligationKind::ProviderInterrupt,
                        state: ObligationState::EffectReserved,
                        at: at as f64,
                    },
                ),
                occurred_at_ms: at,
            },
        ];
        if !queue_was_paused {
            events.push(UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueuePaused {
                    at: at as f64,
                }),
                occurred_at_ms: at,
            });
        }
        let obligation = stop_obligation_record(
            &receipt,
            at.saturating_add(10_000),
            ObligationStateRecord::EffectReserved,
        );
        let acceptance_events = events
            .iter()
            .filter_map(|event| match &event.event {
                LocalDomainEvent::AgentSession(event) => Some(event.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let Some(change) = self
            .lifecycle_repository
            .prepare_session_change(
                &request.session_id,
                request.expected_session_revision,
                &acceptance_events,
            )
            .await
            .map_err(|error| StopOperationError::StorageUnavailable {
                failure: lifecycle_repository_failure(error, "aggregate-change"),
            })?
        else {
            return Err(StopOperationError::StaleTarget);
        };
        let mut state_mutations = change.into_atomic_participant();
        state_mutations.extend([
            LocalStateMutation::OperationBinding(OperationBindingMutation {
                key: CallerOperationKey {
                    principal: request.principal.clone(),
                    installation_id: self.installation_id.clone(),
                    kind: OperationKind::Stop,
                    caller_request_id: request.request_id.clone(),
                },
                operation_id: operation_id.clone(),
                binding_hmac,
            }),
            LocalStateMutation::OperationRecord(OperationRecordMutation {
                kind: OperationKind::Stop,
                operation_id: operation_id.clone(),
                receipt: receipt_record(&receipt, &principal_mac, &binding_hmac),
                latest_status: status_record(&StopOperationState::Accepted),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).expect("zero revision"),
            }),
            LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: obligation_id.clone(),
                record: obligation,
                pending: Some(PendingIndexEntry {
                    ordered_key: format!("{at:020}-{obligation_id}"),
                    owner: request.session_id.clone(),
                    partition: PendingPartition::Owner,
                    shutdown_plan: None,
                }),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).expect("zero revision"),
            }),
        ]);
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex_encode(
                &self
                    .authority
                    .digest(format!("stop-accept/v1\0{operation_id}").as_bytes()),
            ))
            .map_err(|_| StopOperationError::Internal {
                correlation_id: correlation("commit-id"),
            })?,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: OperationKind::Stop.into(),
                idempotency_key: operation_id.clone(),
                payload_hash: self.authority.digest(&binding),
            },
            expected_heads: vec![ExpectedStreamHead {
                stream_id,
                expected: StreamVersion::new(head).map_err(|_| StopOperationError::Internal {
                    correlation_id: correlation("head-version"),
                })?,
            }],
            events,
            state_mutations,
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_)) => {}
            Ok(CommitBatchResult::Replayed(_)) => {
                if let Some(record) =
                    self.lookup(&operation_id)
                        .await
                        .map_err(|_| StopOperationError::Internal {
                            correlation_id: correlation("replay"),
                        })?
                {
                    return Ok(StopCommandOutcome::Accepted {
                        receipt: record.receipt,
                        state: record.state,
                    });
                }
            }
            Err(
                CommitBatchError::PayloadConflict
                | CommitBatchError::EffectAdmissionBlocked
                | CommitBatchError::StreamHeadConflict { .. },
            ) => {
                if let Some(joined) = self
                    .pending_stop_for_target(&request.session_id, &request.turn_id)
                    .await?
                {
                    if let Some(record) = self.lookup(&joined.operation_id).await.map_err(|_| {
                        StopOperationError::Internal {
                            correlation_id: correlation("concurrent-join"),
                        }
                    })? {
                        return self
                            .joined_outcome(&request, &joined.operation_id, record)
                            .await;
                    }
                }
                return match self
                    .lifecycle_repository
                    .restore_session(&request.session_id)
                    .await
                {
                    Ok(mut latest) => {
                        match latest
                            .apply_stop_command(request.expected_session_revision, &request.turn_id)
                        {
                            Ok(_) => Err(StopOperationError::PayloadConflict),
                            Err(_) => Err(StopOperationError::StaleTarget),
                        }
                    }
                    Err(error) => Err(StopOperationError::StorageUnavailable {
                        failure: lifecycle_repository_failure(error, "concurrent-snapshot"),
                    }),
                };
            }
            Err(CommitBatchError::CapacityExceeded | CommitBatchError::SequenceExhausted) => {
                return Err(StopOperationError::CapacityExceeded)
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                return Ok(StopCommandOutcome::OutcomeUnknown {
                    request_id: request.request_id,
                })
            }
            Err(CommitBatchError::StorageUnavailable { failure })
                if failure.is_shutdown_in_progress() =>
            {
                return Err(StopOperationError::ShutdownInProgress)
            }
            Err(CommitBatchError::StorageUnavailable { failure }) => {
                return Ok(StopCommandOutcome::RejectedBeforeCommit { failure })
            }
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                return Err(StopOperationError::Internal { correlation_id })
            }
        }
        let (may_start, reservation_failure) = match self
            .stop_effect_is_reserved(&operation_id, &obligation_id)
            .await
        {
            Ok(may_start) => (may_start, None),
            Err(error) => {
                log::warn!("Stop effect reservation readback failed: {error:?}");
                (false, Some(storage_failure("effect-reservation-readback")))
            }
        };
        let state = if may_start {
            let effect = AcceptedStopEffect {
                operation_id: operation_id.clone(),
                session_id: request.session_id.clone(),
                turn_id: request.turn_id.clone(),
                obligation_id: obligation_id.clone(),
            };
            let effect_outcome = tokio::time::timeout_at(
                deadline,
                self.wait_for_terminal_after_interrupt_handoff(&effect),
            )
            .await;
            let (mut state, observation, terminal_winner) = match effect_outcome {
                Ok(Ok((observation, terminal_winner))) => {
                    let resolution = if terminal_winner.is_some() {
                        StopResolution::Superseded
                    } else {
                        StopResolution::Succeeded
                    };
                    (
                        StopOperationState::Completed { resolution },
                        observation,
                        terminal_winner,
                    )
                }
                Err(_) => match self
                    .terminal_winner(&receipt.session_id, &receipt.turn_id)
                    .await
                {
                    Ok(Some(terminal_winner)) => (
                        StopOperationState::Completed {
                            resolution: StopResolution::Superseded,
                        },
                        None,
                        Some(terminal_winner),
                    ),
                    Ok(None) => (
                        StopOperationState::Completed {
                            resolution: StopResolution::Succeeded,
                        },
                        Some(StopEffectObservation {
                            terminal_reason: Some(InterruptReason::Timeout),
                        }),
                        None,
                    ),
                    Err(_) => (
                        StopOperationState::ReconciliationRequired {
                            failure: storage_failure("terminal-deadline-readback"),
                        },
                        None,
                        None,
                    ),
                },
                Ok(Err(failure)) => (
                    StopOperationState::ReconciliationRequired { failure },
                    None,
                    None,
                ),
            };
            let own_timeout_candidate =
                is_own_timeout_candidate(&state, observation, terminal_winner.is_some());
            let terminal_commit = self
                .resolve_terminal(StopTerminalResolution {
                    receipt: &receipt,
                    principal_mac,
                    binding_hmac,
                    obligation_id: obligation_id.clone(),
                    state: &state,
                    observation,
                    terminal_winner,
                })
                .await;
            if terminal_commit.own_timeout_committed() {
                self.effect.timeout_terminal_committed(&effect).await;
            }
            if !terminal_commit.is_committed()
                && matches!(state, StopOperationState::Completed { .. })
            {
                if let Some(saved) = self.lookup(&receipt.operation_id).await.ok().flatten() {
                    let recovered_commit =
                        StopTerminalCommitOutcome::for_stored(&saved, own_timeout_candidate);
                    if recovered_commit.own_timeout_committed() {
                        self.effect.timeout_terminal_committed(&effect).await;
                    }
                    if recovered_commit.is_committed() {
                        return Ok(StopCommandOutcome::Accepted {
                            receipt: saved.receipt,
                            state: saved.state,
                        });
                    }
                }
                state = StopOperationState::ReconciliationRequired {
                    failure: storage_failure("terminal-resolution"),
                };
                // The terminal attempt failed after durable acceptance. Keep
                // the same obligation/capacity permit pending and make the
                // stable operation query expose reconciliation whenever the
                // store has already recovered enough to save that fact.
                self.resolve_terminal(StopTerminalResolution {
                    receipt: &receipt,
                    principal_mac,
                    binding_hmac,
                    obligation_id,
                    state: &state,
                    observation: None,
                    terminal_winner: None,
                })
                .await;
            }
            state
        } else if let Some(failure) = reservation_failure {
            let state = StopOperationState::ReconciliationRequired { failure };
            self.resolve_terminal(StopTerminalResolution {
                receipt: &receipt,
                principal_mac,
                binding_hmac,
                obligation_id,
                state: &state,
                observation: None,
                terminal_winner: None,
            })
            .await;
            state
        } else {
            StopOperationState::Accepted
        };
        Ok(StopCommandOutcome::Accepted { receipt, state })
    }

    async fn resolve_terminal(
        &self,
        resolution: StopTerminalResolution<'_>,
    ) -> StopTerminalCommitOutcome {
        let StopTerminalResolution {
            receipt,
            principal_mac,
            binding_hmac,
            obligation_id,
            state,
            observation,
            terminal_winner,
        } = resolution;
        let own_timeout_candidate =
            is_own_timeout_candidate(state, observation, terminal_winner.is_some());
        // A competing runtime terminal may already have completed the Stop
        // record, resolution, and obligation in its own winner batch. That is
        // the canonical result; do not append a duplicate resolution event or
        // advance the operation revision a second time.
        if let Some(saved) = self.lookup(&receipt.operation_id).await.ok().flatten() {
            let outcome = StopTerminalCommitOutcome::for_stored(&saved, own_timeout_candidate);
            if outcome.is_committed() {
                return outcome;
            }
        }
        let Ok(Some(record)) = self.lookup(&receipt.operation_id).await else {
            return StopTerminalCommitOutcome::NotCommitted;
        };
        let Some(next_revision) = record.revision.next() else {
            return StopTerminalCommitOutcome::NotCommitted;
        };
        let obligation = match self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.clone(),
            })
            .await
        {
            Ok(LocalEventQueryResult::ObligationByIdentity(Some(obligation))) => obligation,
            _ => return StopTerminalCommitOutcome::NotCommitted,
        };
        let Some(next_obligation_revision) = obligation.revision.next() else {
            return StopTerminalCommitOutcome::NotCommitted;
        };
        let pending = obligation
            .pending
            .as_ref()
            .map(|pending| PendingIndexEntry {
                ordered_key: pending.ordered_key.clone(),
                owner: pending.owner.clone(),
                partition: pending.partition,
                shutdown_plan: pending.shutdown_plan.clone(),
            });
        let at = now_ms();
        let terminal_reason = observation.and_then(|value| value.terminal_reason);
        let (terminal_reason_record, reason_label, exit_code) = match terminal_reason {
            Some(InterruptReason::Abort) => {
                (Some(TerminalInterruptReasonRecord::Abort), "abort", 130)
            }
            Some(InterruptReason::Timeout) => {
                (Some(TerminalInterruptReasonRecord::Timeout), "timeout", 124)
            }
            Some(InterruptReason::Crash) => {
                (Some(TerminalInterruptReasonRecord::Crash), "crash", 1)
            }
            Some(InterruptReason::SessionClosed) => (
                Some(TerminalInterruptReasonRecord::SessionClosed),
                "session_closed",
                0,
            ),
            None => (None, "terminal_winner", 0),
        };
        let turn_result = terminal_reason.map_or_else(
            || crate::domain::agent_session::entities::TurnResult::Completed {
                stop_reason: None,
                token_usage: None,
            },
            |reason| crate::domain::agent_session::entities::TurnResult::Interrupted {
                reason: match reason {
                    InterruptReason::Abort => {
                        crate::domain::agent_session::entities::InterruptReason::Abort
                    }
                    InterruptReason::Timeout => {
                        crate::domain::agent_session::entities::InterruptReason::Timeout
                    }
                    InterruptReason::Crash => {
                        crate::domain::agent_session::entities::InterruptReason::Crash
                    }
                    InterruptReason::SessionClosed => {
                        crate::domain::agent_session::entities::InterruptReason::SessionClosed
                    }
                },
                error: None,
            },
        );
        let terminal = TerminalResultRecord::Stop {
            operation_id: receipt.operation_id.clone(),
            reason: terminal_reason_record,
            exit_code: Some(exit_code),
            result: turn_result,
        };
        let terminal_digest = self.authority.digest(
            format!(
                "stop-terminal-semantic/v1\0{}\0{reason_label}\0{exit_code}",
                receipt.operation_id
            )
            .as_bytes(),
        );
        let superseded = terminal_winner.is_some();
        let Some((
            obligation_operation_id,
            obligation_session_id,
            obligation_turn_id,
            obligation_expected_revision,
            _,
            _,
        )) = stop_obligation(&obligation.record)
        else {
            return StopTerminalCommitOutcome::NotCommitted;
        };
        if obligation_operation_id != receipt.operation_id
            || obligation_session_id != receipt.session_id
            || obligation_turn_id != receipt.turn_id
            || obligation_expected_revision != receipt.accepted_revision
        {
            return StopTerminalCommitOutcome::NotCommitted;
        }
        let mut completed_obligation = obligation.record.clone();
        let ObligationRecord::StopInterrupt {
            state: obligation_state,
            ..
        } = &mut completed_obligation
        else {
            unreachable!("Stop obligation was checked above")
        };
        *obligation_state = if matches!(state, StopOperationState::Completed { .. }) {
            ObligationStateRecord::Completed
        } else {
            ObligationStateRecord::ReconciliationRequired
        };
        let mut mutations = vec![LocalStateMutation::OperationRecord(
            OperationRecordMutation {
                kind: OperationKind::Stop,
                operation_id: receipt.operation_id.clone(),
                receipt: receipt_record(receipt, &principal_mac, &binding_hmac),
                latest_status: status_record(state),
                expected: RevisionGuard::Expected(record.revision),
                revision: next_revision,
            },
        )];
        let completed = matches!(state, StopOperationState::Completed { .. });
        let terminal_candidate = if completed && !superseded {
            Some(TerminalRecordMutation {
                session_id: receipt.session_id.clone(),
                turn_id: receipt.turn_id.clone(),
                terminal_identity: receipt.operation_id.clone(),
                result: terminal.clone(),
                participant_digest: terminal_digest,
            })
        } else {
            None
        };
        if completed {
            if let Some(terminal) = terminal_candidate.as_ref() {
                mutations.push(LocalStateMutation::TerminalRecord(terminal.clone()));
            }
            let resolution_detail = terminal_winner
                .as_ref()
                .map(|winner| TerminalResultRecord::StopSuperseded {
                    terminal_identity: winner.terminal_identity.clone(),
                    terminal_result_sha256: winner.participant_digest,
                })
                .unwrap_or_else(|| terminal.clone());
            mutations.extend([
                LocalStateMutation::StopResolution(StopResolutionMutation {
                    stop_operation_id: receipt.operation_id.clone(),
                    resolution: if superseded {
                        StopResolutionKind::Superseded
                    } else {
                        StopResolutionKind::Succeeded
                    },
                    detail: resolution_detail,
                }),
                LocalStateMutation::Obligation(ObligationMutation {
                    obligation_id: obligation_id.clone(),
                    record: completed_obligation.clone(),
                    pending: None,
                    expected: RevisionGuard::Expected(obligation.revision),
                    revision: next_obligation_revision,
                }),
            ]);
        } else {
            mutations.push(LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: obligation_id.clone(),
                record: completed_obligation,
                pending,
                expected: RevisionGuard::Expected(obligation.revision),
                revision: next_obligation_revision,
            }));
        }
        let mut expected_heads = Vec::new();
        let mut events = Vec::new();
        if completed {
            let Ok(stream_id) = StreamId::agent_session(&receipt.session_id) else {
                return StopTerminalCommitOutcome::NotCommitted;
            };
            let Ok(head) = self.head(&stream_id).await else {
                return StopTerminalCommitOutcome::NotCommitted;
            };
            let Ok(turn_id) = receipt.turn_id.parse::<u64>() else {
                return StopTerminalCommitOutcome::NotCommitted;
            };
            expected_heads.push(ExpectedStreamHead {
                stream_id: stream_id.clone(),
                expected: StreamVersion::new(head).expect("nonnegative head"),
            });
            if let Some(reason) = terminal_reason {
                events.push(UncommittedDomainEvent {
                    stream_id: stream_id.clone(),
                    event: LocalDomainEvent::AgentSession(
                        AgentSessionDomainEvent::TurnInterrupted {
                            turn_id,
                            reason,
                            exit_code: i64::from(exit_code),
                            error: None,
                        },
                    ),
                    occurred_at_ms: at,
                });
            }
            events.push(UncommittedDomainEvent {
                stream_id,
                event: LocalDomainEvent::AgentSession(
                    AgentSessionDomainEvent::StopResolutionRecorded {
                        operation_id: receipt.operation_id.clone(),
                        turn_id,
                        resolution: if superseded {
                            StopResolution::Superseded
                        } else {
                            StopResolution::Succeeded
                        },
                        at: at as f64,
                    },
                ),
                occurred_at_ms: at,
            });
        }
        let projection_events = events
            .iter()
            .filter_map(|event| match &event.event {
                LocalDomainEvent::AgentSession(event) => Some(event.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let Ok(projection_mutations) = self
            .effect
            .terminal_state_mutations(&receipt.session_id, &projection_events)
            .await
        else {
            return StopTerminalCommitOutcome::NotCommitted;
        };
        mutations.extend(projection_mutations);
        if let Some(terminal) = terminal_candidate.as_ref() {
            let Ok(participants) = self.effect.terminal_participants(terminal).await else {
                return StopTerminalCommitOutcome::NotCommitted;
            };
            let Ok(stream_id) = StreamId::agent_session(&receipt.session_id) else {
                return StopTerminalCommitOutcome::NotCommitted;
            };
            events.extend(
                participants
                    .events
                    .into_iter()
                    .map(|event| UncommittedDomainEvent {
                        stream_id: stream_id.clone(),
                        event: LocalDomainEvent::AgentSession(event),
                        occurred_at_ms: at,
                    }),
            );
            mutations.extend(participants.mutations);
        }
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex_encode(
                &self.authority.digest(
                    format!(
                        "stop-terminal/v1\0{}\0{}",
                        receipt.operation_id,
                        next_revision.value()
                    )
                    .as_bytes(),
                ),
            ))
            .expect("digest commit identity"),
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::OperationProgress,
                idempotency_key: format!(
                    "{}.terminal.{}",
                    receipt.operation_id,
                    next_revision.value()
                ),
                payload_hash: self.authority.digest(&status_identity_material(state)),
            },
            expected_heads,
            events,
            state_mutations: mutations,
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => {
                StopTerminalCommitOutcome::committed(own_timeout_candidate)
            }
            Err(CommitBatchError::OutcomeUnknown { identity }) => {
                if matches!(
                    self.repository.resolve_commit(identity).await,
                    Ok(CommitResolution::Committed(_))
                ) {
                    StopTerminalCommitOutcome::committed(own_timeout_candidate)
                } else {
                    StopTerminalCommitOutcome::NotCommitted
                }
            }
            Err(error) => {
                log::warn!("Stop terminal resolution remains pending: {error}");
                StopTerminalCommitOutcome::NotCommitted
            }
        }
    }

    /// Reconcile every durable Stop obligation without dispatching a second
    /// provider interrupt. A restarted process waits only for the remainder
    /// of the original acceptance deadline, then competes for the same
    /// terminal identity and Stop resolution through the normal CAS batch.
    #[cfg(test)]
    pub async fn recover_pending_stops(&self) -> Result<(), StopOperationError> {
        loop {
            if self.recover_pending_stops_pass().await? == 0 {
                return Ok(());
            }
        }
    }

    /// One bounded, snapshot-consistent pass. The public startup driver opens
    /// a fresh snapshot after every nonempty pass so work added while paging
    /// is rediscovered without joining cursors from different snapshots.
    pub(crate) async fn recover_pending_stops_pass(&self) -> Result<usize, StopOperationError> {
        let pending = self.pending_stop_obligations().await?;
        let discovered = pending.len();
        let mut first_error = None;
        for obligation in pending {
            if let Err(error) = self.recover_pending_stop(obligation).await {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(discovered), Err)
    }

    async fn recover_pending_stop(
        &self,
        pending: PendingStopObligation,
    ) -> Result<(), StopOperationError> {
        let obligation = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: pending.obligation_id.clone(),
            })
            .await
            .map_err(lookup_query_error)?;
        let LocalEventQueryResult::ObligationByIdentity(obligation) = obligation else {
            return Err(StopOperationError::Internal {
                correlation_id: correlation("recovery-obligation-shape"),
            });
        };
        let Some(obligation) = obligation.filter(|obligation| obligation.pending.is_some()) else {
            return Ok(());
        };
        let Some((
            obligation_operation_id,
            obligation_session_id,
            obligation_turn_id,
            _,
            obligation_deadline_ms,
            obligation_state,
        )) = stop_obligation(&obligation.record)
        else {
            return Err(StopOperationError::Internal {
                correlation_id: correlation("recovery-obligation-state"),
            });
        };
        if obligation_operation_id != pending.operation_id
            || obligation_session_id != pending.session_id
            || obligation_turn_id != pending.turn_id
            || obligation_deadline_ms != pending.deadline_ms
        {
            return Err(StopOperationError::Internal {
                correlation_id: correlation("recovery-obligation-binding"),
            });
        }
        let record = self
            .lookup(&pending.operation_id)
            .await
            .map_err(lookup_query_error)?
            .ok_or_else(|| StopOperationError::Internal {
                correlation_id: correlation("recovery-operation-missing"),
            })?;
        let remaining = pending.deadline_ms.saturating_sub(now_ms());
        let effect = AcceptedStopEffect {
            operation_id: pending.operation_id.clone(),
            session_id: pending.session_id.clone(),
            turn_id: pending.turn_id.clone(),
            obligation_id: pending.obligation_id.clone(),
        };
        if obligation_state == ObligationStateRecord::Pending
            && self
                .reserve_stop_effect(&pending.operation_id, &pending.obligation_id)
                .await?
        {
            let (state, observation, terminal_winner) = if remaining <= 0 {
                match self
                    .terminal_winner(&pending.session_id, &pending.turn_id)
                    .await
                {
                    Ok(Some(winner)) => (
                        StopOperationState::Completed {
                            resolution: StopResolution::Superseded,
                        },
                        None,
                        Some(winner),
                    ),
                    Ok(None) => (
                        StopOperationState::Completed {
                            resolution: StopResolution::Succeeded,
                        },
                        Some(StopEffectObservation {
                            terminal_reason: Some(InterruptReason::Timeout),
                        }),
                        None,
                    ),
                    Err(_) => (
                        StopOperationState::ReconciliationRequired {
                            failure: storage_failure("recovery-terminal-deadline-readback"),
                        },
                        None,
                        None,
                    ),
                }
            } else {
                match tokio::time::timeout(
                    Duration::from_millis(remaining as u64),
                    self.wait_for_terminal_after_interrupt_handoff(&effect),
                )
                .await
                {
                    Ok(Ok((observation, terminal_winner))) => (
                        StopOperationState::Completed {
                            resolution: if terminal_winner.is_some() {
                                StopResolution::Superseded
                            } else {
                                StopResolution::Succeeded
                            },
                        },
                        observation,
                        terminal_winner,
                    ),
                    Err(_) => match self
                        .terminal_winner(&pending.session_id, &pending.turn_id)
                        .await
                    {
                        Ok(Some(winner)) => (
                            StopOperationState::Completed {
                                resolution: StopResolution::Superseded,
                            },
                            None,
                            Some(winner),
                        ),
                        Ok(None) => (
                            StopOperationState::Completed {
                                resolution: StopResolution::Succeeded,
                            },
                            Some(StopEffectObservation {
                                terminal_reason: Some(InterruptReason::Timeout),
                            }),
                            None,
                        ),
                        Err(_) => (
                            StopOperationState::ReconciliationRequired {
                                failure: storage_failure("recovery-terminal-deadline-readback"),
                            },
                            None,
                            None,
                        ),
                    },
                    Ok(Err(failure)) => (
                        StopOperationState::ReconciliationRequired { failure },
                        None,
                        None,
                    ),
                }
            };
            let terminal_commit = self
                .resolve_terminal(StopTerminalResolution {
                    receipt: &record.receipt,
                    principal_mac: record.principal_mac,
                    binding_hmac: record.binding_hmac,
                    obligation_id: pending.obligation_id.clone(),
                    state: &state,
                    observation,
                    terminal_winner,
                })
                .await;
            if terminal_commit.own_timeout_committed() {
                self.effect.timeout_terminal_committed(&effect).await;
            }
            return if terminal_commit.is_committed() {
                Ok(())
            } else {
                Err(StopOperationError::StorageUnavailable {
                    failure: storage_failure("recovery-effect-result"),
                })
            };
        }
        let terminal_winner = if remaining > 0 {
            match tokio::time::timeout(
                Duration::from_millis(remaining as u64),
                self.wait_for_terminal_winner(&pending.session_id, &pending.turn_id),
            )
            .await
            {
                Ok(Ok(winner)) => Some(winner),
                Ok(Err(failure)) => {
                    return Err(StopOperationError::StorageUnavailable { failure });
                }
                Err(_) => self
                    .terminal_winner(&pending.session_id, &pending.turn_id)
                    .await
                    .map_err(lookup_query_error)?,
            }
        } else {
            self.terminal_winner(&pending.session_id, &pending.turn_id)
                .await
                .map_err(lookup_query_error)?
        };
        let observation = terminal_winner.is_none().then_some(StopEffectObservation {
            terminal_reason: Some(InterruptReason::Timeout),
        });
        let completed = StopOperationState::Completed {
            resolution: if terminal_winner.is_some() {
                StopResolution::Superseded
            } else {
                StopResolution::Succeeded
            },
        };
        let terminal_commit = self
            .resolve_terminal(StopTerminalResolution {
                receipt: &record.receipt,
                principal_mac: record.principal_mac,
                binding_hmac: record.binding_hmac,
                obligation_id: pending.obligation_id,
                state: &completed,
                observation,
                terminal_winner,
            })
            .await;
        if terminal_commit.own_timeout_committed() {
            self.effect.timeout_terminal_committed(&effect).await;
        }
        if terminal_commit.is_committed() {
            Ok(())
        } else {
            Err(StopOperationError::StorageUnavailable {
                failure: storage_failure("recovery-terminal"),
            })
        }
    }

    pub async fn get_operation(
        &self,
        principal: &str,
        operation_id: &str,
    ) -> Result<(StopOperationReceipt, StopOperationState), StopOperationError> {
        let mut record = self
            .lookup(operation_id)
            .await
            .map_err(lookup_query_error)?;
        // A caller that lost the acceptance response only knows its durable
        // request identity. Resolve that identity to the deterministic backend
        // operation without allocating or executing a replacement Stop.
        if record.is_none() && validate_operation_identity(operation_id).is_ok() {
            let derived = self.operation_id(principal, operation_id);
            record = self.lookup(&derived).await.map_err(lookup_query_error)?;
        }
        let Some(record) = record else {
            return Err(StopOperationError::NotFound);
        };
        let principal_mac = self.authority.mac(&principal_material(principal));
        if !constant_time_eq_32(&record.principal_mac, &principal_mac) {
            return Err(StopOperationError::NotFound);
        }
        Ok((record.receipt, record.state))
    }
}

#[async_trait::async_trait]
impl StopRecoveryReadbackPort for StopOperationUsecase {
    async fn read_stop(
        &self,
        request: &StopRecoveryReadbackRequest,
    ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
        use crate::domain::agent_session::events::RecoveryResultClassification;

        if request.effect_identity.as_str()
            != self.target_obligation_id(&request.session_id, &request.turn_id)
        {
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::InvalidEffectIntent,
                false,
                "The Stop readback identity does not match its durable target.",
                correlation("readback-identity"),
            ));
        }
        let record = self
            .lookup(&request.operation_id)
            .await
            .map_err(|_| storage_failure("readback-operation"))?
            .ok_or_else(|| {
                SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "The Stop operation is unavailable for readback.",
                    correlation("readback-missing"),
                )
            })?;
        if record.receipt.operation_id != request.operation_id
            || record.receipt.session_id != request.session_id
            || record.receipt.turn_id != request.turn_id
        {
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::InvalidEffectIntent,
                false,
                "The Stop operation no longer matches the accepted effect.",
                correlation("readback-binding"),
            ));
        }
        let terminal_winner = self
            .terminal_winner(&request.session_id, &request.turn_id)
            .await
            .map_err(|_| storage_failure("readback-terminal"))?;
        let terminal_participants = if !matches!(record.state, StopOperationState::Completed { .. })
        {
            if let Some(terminal) = terminal_winner {
                let terminal = TerminalRecordMutation {
                    session_id: terminal.session_id,
                    turn_id: terminal.turn_id,
                    terminal_identity: terminal.terminal_identity,
                    result: terminal.result,
                    participant_digest: terminal.participant_digest,
                };
                Some((
                    terminal.clone(),
                    self.prepare_runtime_terminal_participants(&terminal)
                        .await
                        .map_err(|_| storage_failure("readback-participants"))?,
                ))
            } else {
                None
            }
        } else {
            None
        };
        let (classification, safe_result, owner_mutations, owner_batch) =
            if let Some((terminal, mut participants)) = terminal_participants {
                let at = match &terminal.result {
                    TerminalResultRecord::AgentTurn {
                        completed_at_bits, ..
                    } => f64::from_bits(*completed_at_bits),
                    _ => 0.0,
                };
                for event in &mut participants.events {
                    if let AgentSessionDomainEvent::StopResolutionRecorded {
                        at: event_at, ..
                    } = event
                    {
                        *event_at = at;
                    }
                }
                let projection_mutations = self
                    .effect
                    .terminal_state_mutations(&request.session_id, &participants.events)
                    .await?;
                participants.mutations.extend(projection_mutations);
                let stream_id = StreamId::agent_session(&request.session_id)
                    .map_err(|_| storage_failure("readback-stream"))?;
                let head = self
                    .repository
                    .load_stream(LoadStreamRequest {
                        stream_id: stream_id.clone(),
                        after: None,
                        limit: 1,
                    })
                    .await
                    .map_err(|_| storage_failure("readback-stream-head"))?
                    .head;
                let occurred_at_ms = (at * 1000.0).round() as i64;
                let events = participants
                    .events
                    .into_iter()
                    .map(|event| UncommittedDomainEvent {
                        stream_id: stream_id.clone(),
                        event: LocalDomainEvent::AgentSession(event),
                        occurred_at_ms,
                    })
                    .collect::<Vec<_>>();
                let canonical_events = self
                    .repository
                    .canonical_event_batch_identity_v1(&events)
                    .map_err(|_| storage_failure("readback-event-identity"))?;
                fn hash_field(hasher: &mut Sha256, value: &[u8]) {
                    hasher.update((value.len() as u64).to_be_bytes());
                    hasher.update(value);
                }
                let mut digest = Sha256::new();
                hash_field(&mut digest, b"stop_recovery_readback_participants_v1");
                hash_field(&mut digest, stream_id.as_str().as_bytes());
                digest.update(head.value().to_be_bytes());
                hash_field(&mut digest, &canonical_events);
                let mut mutation_identities = Vec::new();
                for mutation in &participants.mutations {
                    if matches!(
                        mutation,
                        LocalStateMutation::Obligation(obligation)
                            if obligation.obligation_id == request.effect_identity.as_str()
                    ) {
                        continue;
                    }
                    let identity = super::recovery::recovery_owner_identity_v1(
                        self.repository.as_ref(),
                        mutation,
                    )
                    .map_err(|_| storage_failure("readback-participant-identity"))?;
                    mutation_identities.push(identity);
                }
                mutation_identities.sort();
                for identity in mutation_identities {
                    hash_field(&mut digest, &identity);
                }
                let participant_digest: [u8; 32] = digest.finalize().into();
                let owner_batch = RecoveryOwnerBatch {
                    expected_heads: vec![ExpectedStreamHead {
                        stream_id: stream_id.clone(),
                        expected: head,
                    }],
                    events,
                    canonical_events,
                    participant_digest,
                };
                (
                    RecoveryResultClassification::Succeeded,
                    "The accepted Stop was superseded by the durable terminal winner.".to_string(),
                    participants.mutations,
                    Some(owner_batch),
                )
            } else {
                let (classification, safe_result) = match record.state {
                    StopOperationState::Accepted => (
                        RecoveryResultClassification::Pending,
                        "The accepted Stop is still waiting for its terminal winner.".to_string(),
                    ),
                    StopOperationState::Completed { resolution } => (
                        RecoveryResultClassification::Succeeded,
                        match resolution {
                            StopResolution::Succeeded => {
                                "The accepted Stop owns the durable terminal result."
                            }
                            StopResolution::Superseded => {
                                "The accepted Stop was superseded by the durable terminal winner."
                            }
                        }
                        .to_string(),
                    ),
                    StopOperationState::ReconciliationRequired { .. } => (
                        RecoveryResultClassification::Ambiguous,
                        "The accepted Stop still requires reconciliation.".to_string(),
                    ),
                };
                (classification, safe_result, Vec::new(), None)
            };
        Ok(RecoveryEffectResult {
            classification,
            safe_result,
            owner_mutations,
            owner_batch,
        })
    }
}
