//! Normal send acceptance and operation identity contract (R-001..R-005).
//!
//! One `commit_batch` fixes the operation binding, the human-input fact, the
//! turn-or-queue disposition, and the turn execution obligation. The immutable
//! Accepted receipt is returned only after the commit is confirmed; provider
//! effects start only afterwards, keyed by obligation identity.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::agent_session::events::{
    AgentSessionDomainEvent, ObligationKind, ObligationState, SendDisposition,
};
use crate::domain::local_event::{
    AgentTurnTerminalResultRecord, CallerAttemptResolution, CallerOperationKey, CommitBatchError,
    CommitBatchResult, CommitIdentity, CommitOperationKind, CommitResolution, ExpectedStreamHead,
    IdempotencyBinding, LoadStreamRequest, LocalAtomicBatch, LocalDomainEvent, LocalEventQuery,
    LocalEventQueryError, LocalEventQueryResult, LocalEventTransactionRepository,
    LocalStateMutation, ObligationMutation, ObligationRecord, ObligationStateRecord,
    OperationBindingMutation, OperationKind, OperationReceiptRecord, OperationRecordMutation,
    OperationStatusRecord, OperationStatusValue, PendingIndexEntry, PendingPartition,
    RecordAuthentication, Revision, RevisionGuard, SafeOperationFailure,
    SendObligationDispositionRecord, SendObligationKindRecord, SessionOperationFailureKind,
    StreamId, StreamVersion, TerminalRecordMutation, TerminalResultRecord, UncommittedDomainEvent,
};

use super::identity::{constant_time_eq_32, validate_operation_identity};
use super::ports::{
    AcceptedSendEffect, LegacyProviderEstablishRecovery, OperationBindingAuthority,
    RecoveryEffectResult, SendAdmissionGate, SendEffectDispatch, SendRecoveryReadbackKind,
    SendRecoveryReadbackPort, SendRecoveryReadbackRequest, TerminalParticipants,
};
use super::record::hex_encode;
use super::recovery::unresolved_recovery_original_identity;

/// Closed public execution status of an accepted send (design "Public
/// closed types"). No serde: adapters map this to their own DTOs.
#[derive(Debug, Clone, PartialEq)]
pub enum SendExecutionStatus {
    AwaitingProviderStart {
        dependency_obligation_ids: Vec<String>,
    },
    Queued {
        queue_item_id: String,
        reserved_turn_id: String,
    },
    ProviderStartReserved {
        obligation_id: String,
    },
    Running {
        turn_id: String,
    },
    ReconciliationRequired {
        failure: SafeOperationFailure,
    },
    Failed {
        failure: SafeOperationFailure,
    },
    Terminal {
        result: crate::domain::agent_session::entities::TurnResult,
    },
}

pub(crate) struct ObligationTransition<'a> {
    pub operation_id: &'a str,
    pub obligation_id: &'a str,
    pub expected_kind: &'a str,
    pub expected_state: &'a str,
    pub next_state: &'a str,
    pub keep_pending: bool,
    pub status: Option<SendExecutionStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObligationTransitionOutcome {
    Applied,
    AlreadyAtTarget,
}

/// Immutable Accepted receipt (R-001). Never changes after acceptance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendOperationReceipt {
    pub operation_id: String,
    pub session_id: String,
    /// Opaque reference to the durably saved exact input.
    pub input_ref: String,
    pub disposition: SendDisposition,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AcceptedSendOperation {
    pub receipt: SendOperationReceipt,
    pub latest_status: SendExecutionStatus,
}

/// Top-level result of the send command.
#[derive(Debug, Clone, PartialEq)]
pub enum SendCommandOutcome {
    Accepted(AcceptedSendOperation),
    /// Canonical acceptance could not be saved; zero provider I/O, zero
    /// public state change.
    RejectedBeforeCommit {
        failure: SafeOperationFailure,
    },
    /// The acceptance commit result could not be confirmed. The same
    /// operation identity must be used to resolve; no automatic new send.
    OutcomeUnknown {
        operation_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendAgentMessageError {
    InvalidRequest,
    PayloadConflict,
    ShutdownInProgress,
    /// The operation identity is bound to another principal; existence is
    /// not disclosed beyond this typed result.
    NotFound,
    CapacityExceeded,
    Internal {
        correlation_id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum GetSendOperationError {
    InvalidRequest,
    /// The caller journal durably owns this identity, but canonical send
    /// acceptance has not yet become queryable. The caller must resolve this
    /// same identity; absence is not proven.
    OutcomeUnknown {
        operation_id: String,
    },
    NotFound,
    QueryBusy,
    DeadlineExceeded,
    StorageUnavailable {
        failure: SafeOperationFailure,
    },
    Internal {
        correlation_id: String,
    },
}

/// Exact send request bound to the caller operation identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendOperationRequest {
    pub principal: String,
    pub operation_id: String,
    /// Deterministic canonical encoding of the caller's exact payload
    /// (content, images, mentions, editor context, session target,
    /// worktree, execution configuration). Produced once by the caller
    /// adapter and reused unchanged for every retry.
    pub canonical_payload: String,
}

#[derive(Clone)]
pub struct AgentSendOperationUsecase {
    repository: Arc<dyn LocalEventTransactionRepository>,
    authority: Arc<dyn OperationBindingAuthority>,
    gate: Arc<dyn SendAdmissionGate>,
    installation_id: String,
    pending_recovery_wakeup: Arc<tokio::sync::Notify>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

const SEND_BACKGROUND_RETRY_ATTEMPTS: usize = 8;
const SEND_OWNER_REVISION_REPLAN_ATTEMPTS: usize = 4;

fn send_background_retry_delay(attempt: usize) -> std::time::Duration {
    let shift = u32::try_from(attempt.min(5)).unwrap_or(5);
    std::time::Duration::from_millis(25_u64.saturating_mul(1_u64 << shift))
}

fn internal_error(context: &str) -> SendAgentMessageError {
    SendAgentMessageError::Internal {
        correlation_id: format!("send-{}-{}", context, uuid_like()),
    }
}

fn uuid_like() -> String {
    format!("{:x}", now_ms())
}

fn storage_failure(context: &str) -> SafeOperationFailure {
    SafeOperationFailure::new(
        SessionOperationFailureKind::StorageUnavailable,
        true,
        "The local event store is unavailable.",
        format!("send-{context}-{}", uuid_like()),
    )
}

pub(super) fn principal_material(principal: &str) -> Vec<u8> {
    super::binding::principal(principal)
}

struct StoredSendRecord {
    receipt: SendOperationReceipt,
    latest_status: SendExecutionStatus,
    principal_mac: [u8; 32],
    binding_hmac: [u8; 32],
    revision: Revision,
}

fn receipt_record(
    receipt: &SendOperationReceipt,
    principal_mac: &[u8; 32],
    binding_hmac: &[u8; 32],
) -> OperationReceiptRecord {
    OperationReceiptRecord::Send {
        operation_id: receipt.operation_id.clone(),
        session_id: receipt.session_id.clone(),
        input_ref: receipt.input_ref.clone(),
        disposition: receipt.disposition.clone(),
        authentication: RecordAuthentication {
            principal_mac: *principal_mac,
            binding_hmac: *binding_hmac,
        },
    }
}

fn status_record(status: &SendExecutionStatus) -> OperationStatusRecord {
    let value = match status {
        SendExecutionStatus::AwaitingProviderStart {
            dependency_obligation_ids,
        } => OperationStatusValue::AwaitingProviderStart {
            dependency_obligation_ids: dependency_obligation_ids.clone(),
        },
        SendExecutionStatus::Queued {
            queue_item_id,
            reserved_turn_id,
        } => OperationStatusValue::Queued {
            queue_item_id: queue_item_id.clone(),
            reserved_turn_id: reserved_turn_id.clone(),
        },
        SendExecutionStatus::ProviderStartReserved { obligation_id } => {
            OperationStatusValue::ProviderStartReserved {
                obligation_id: obligation_id.clone(),
            }
        }
        SendExecutionStatus::Running { turn_id } => OperationStatusValue::Running {
            turn_id: turn_id.clone(),
        },
        SendExecutionStatus::ReconciliationRequired { failure } => {
            OperationStatusValue::ReconciliationRequired {
                failure: failure.clone(),
            }
        }
        SendExecutionStatus::Failed { failure } => OperationStatusValue::Failed {
            failure: failure.clone(),
        },
        SendExecutionStatus::Terminal { result } => OperationStatusValue::Terminal {
            result: result.clone(),
        },
    };
    OperationStatusRecord {
        kind: OperationKind::Send,
        value,
    }
}

fn decode_send_record(
    receipt_record: OperationReceiptRecord,
    status_record: OperationStatusRecord,
    revision: Revision,
) -> Option<StoredSendRecord> {
    let (operation_id, session_id, input_ref, disposition, authentication) = match receipt_record {
        OperationReceiptRecord::Send {
            operation_id,
            session_id,
            input_ref,
            disposition,
            authentication,
        } => (
            operation_id,
            session_id,
            input_ref,
            disposition,
            authentication,
        ),
        OperationReceiptRecord::PermissionResponse { .. }
        | OperationReceiptRecord::Stop { .. }
        | OperationReceiptRecord::SessionLifecycle { .. }
        | OperationReceiptRecord::ApplicationQuit { .. } => return None,
    };
    if status_record.kind != OperationKind::Send {
        return None;
    }
    let latest_status = match status_record.value {
        OperationStatusValue::AwaitingProviderStart {
            dependency_obligation_ids,
        } => SendExecutionStatus::AwaitingProviderStart {
            dependency_obligation_ids,
        },
        OperationStatusValue::Queued {
            queue_item_id,
            reserved_turn_id,
        } => SendExecutionStatus::Queued {
            queue_item_id,
            reserved_turn_id,
        },
        OperationStatusValue::ProviderStartReserved { obligation_id } => {
            SendExecutionStatus::ProviderStartReserved { obligation_id }
        }
        OperationStatusValue::Running { turn_id } => SendExecutionStatus::Running { turn_id },
        OperationStatusValue::ReconciliationRequired { failure } => {
            SendExecutionStatus::ReconciliationRequired { failure }
        }
        OperationStatusValue::Failed { failure } => SendExecutionStatus::Failed { failure },
        OperationStatusValue::Terminal { result } => SendExecutionStatus::Terminal { result },
        OperationStatusValue::Accepted
        | OperationStatusValue::AwaitingProviderResponse { .. }
        | OperationStatusValue::Completed
        | OperationStatusValue::PermissionCompleted { .. }
        | OperationStatusValue::StopCompleted { .. }
        | OperationStatusValue::Preparing
        | OperationStatusValue::Activated
        | OperationStatusValue::ExitPending
        | OperationStatusValue::Exited
        | OperationStatusValue::OutcomeUnknown { .. }
        | OperationStatusValue::FailedBeforeActivation { .. } => return None,
    };
    Some(StoredSendRecord {
        receipt: SendOperationReceipt {
            operation_id,
            session_id,
            input_ref,
            disposition,
        },
        latest_status,
        principal_mac: authentication.principal_mac,
        binding_hmac: authentication.binding_hmac,
        revision,
    })
}

#[derive(Clone)]
struct SendObligationData {
    obligation_id: String,
    operation_id: String,
    session_id: String,
    kind: SendObligationKindRecord,
    disposition: SendObligationDispositionRecord,
    human_message_id: Option<String>,
    assistant_message_id: Option<String>,
    reserved_turn_id: Option<String>,
    turn_id: Option<String>,
    dependency_obligation_ids: Vec<String>,
    canonical_payload: String,
    state: ObligationStateRecord,
}

impl SendObligationData {
    fn decode(record: &ObligationRecord) -> Option<Self> {
        if let ObligationRecord::RecoveryTransition { original, .. }
        | ObligationRecord::Observed { original, .. } = record
        {
            return Self::decode(original);
        }
        let (
            obligation_id,
            operation_id,
            session_id,
            kind,
            disposition,
            human_message_id,
            assistant_message_id,
            reserved_turn_id,
            turn_id,
            dependency_obligation_ids,
            canonical_payload,
            state,
        ) = match record {
            ObligationRecord::Send {
                obligation_id,
                operation_id,
                session_id,
                kind,
                disposition,
                human_message_id,
                assistant_message_id,
                reserved_turn_id,
                turn_id,
                dependency_obligation_ids,
                canonical_payload,
                state,
            } => (
                obligation_id,
                operation_id,
                session_id,
                kind,
                disposition,
                human_message_id,
                assistant_message_id,
                reserved_turn_id,
                turn_id,
                dependency_obligation_ids,
                canonical_payload,
                state,
            ),
            ObligationRecord::PermissionResponse { .. }
            | ObligationRecord::StopInterrupt { .. }
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
            | ObligationRecord::WorkflowExecution { .. } => return None,
            ObligationRecord::RecoveryTransition { original, .. }
            | ObligationRecord::Observed { original, .. } => return Self::decode(original),
        };
        Some(Self {
            obligation_id: obligation_id.clone(),
            operation_id: operation_id.clone(),
            session_id: session_id.clone(),
            kind: *kind,
            disposition: *disposition,
            human_message_id: human_message_id.clone(),
            assistant_message_id: assistant_message_id.clone(),
            reserved_turn_id: reserved_turn_id.clone(),
            turn_id: turn_id.clone(),
            dependency_obligation_ids: dependency_obligation_ids.clone(),
            canonical_payload: canonical_payload.clone(),
            state: *state,
        })
    }

    fn into_record_with_state(self, state: ObligationStateRecord) -> ObligationRecord {
        ObligationRecord::Send {
            obligation_id: self.obligation_id,
            operation_id: self.operation_id,
            session_id: self.session_id,
            kind: self.kind,
            disposition: self.disposition,
            human_message_id: self.human_message_id,
            assistant_message_id: self.assistant_message_id,
            reserved_turn_id: self.reserved_turn_id,
            turn_id: self.turn_id,
            dependency_obligation_ids: self.dependency_obligation_ids,
            canonical_payload: self.canonical_payload,
            state,
        }
    }
}

fn send_obligation_record_with_state(
    record: &ObligationRecord,
    next_state: ObligationStateRecord,
) -> Option<ObligationRecord> {
    let mut updated = record.clone();
    fn replace(record: &mut ObligationRecord, next_state: ObligationStateRecord) -> bool {
        match record {
            ObligationRecord::Send { state, .. } => {
                *state = next_state;
                true
            }
            ObligationRecord::RecoveryTransition { original, .. }
            | ObligationRecord::Observed { original, .. } => replace(original, next_state),
            ObligationRecord::PermissionResponse { .. }
            | ObligationRecord::StopInterrupt { .. }
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
            | ObligationRecord::WorkflowExecution { .. } => false,
        }
    }
    replace(&mut updated, next_state).then_some(updated)
}

fn has_recovery_wrapper(record: &ObligationRecord) -> bool {
    match record {
        ObligationRecord::RecoveryTransition { .. } | ObligationRecord::Observed { .. } => true,
        ObligationRecord::Send { .. }
        | ObligationRecord::PermissionResponse { .. }
        | ObligationRecord::StopInterrupt { .. }
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
        | ObligationRecord::WorkflowExecution { .. } => false,
    }
}

fn send_obligation_kind(raw: &str) -> Option<SendObligationKindRecord> {
    match raw {
        "provider_establish" => Some(SendObligationKindRecord::ProviderEstablish),
        "turn_execution" => Some(SendObligationKindRecord::TurnExecution),
        _ => None,
    }
}

fn obligation_state(raw: &str) -> Option<ObligationStateRecord> {
    match raw {
        "prepared" => Some(ObligationStateRecord::Prepared),
        "pending" => Some(ObligationStateRecord::Pending),
        "effect_reserved" => Some(ObligationStateRecord::EffectReserved),
        "running" => Some(ObligationStateRecord::Running),
        "waiting_approval" => Some(ObligationStateRecord::WaitingApproval),
        "outcome_unknown" => Some(ObligationStateRecord::OutcomeUnknown),
        "reconciliation_required" => Some(ObligationStateRecord::ReconciliationRequired),
        "failed" => Some(ObligationStateRecord::Failed),
        "completed" => Some(ObligationStateRecord::Completed),
        "cancelled" => Some(ObligationStateRecord::Cancelled),
        _ => None,
    }
}

fn status_identity_material(status: &SendExecutionStatus) -> Vec<u8> {
    fn text(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    let mut bytes = b"send-status-identity-v1".to_vec();
    match status {
        SendExecutionStatus::AwaitingProviderStart {
            dependency_obligation_ids,
        } => {
            bytes.push(0);
            for dependency in dependency_obligation_ids {
                text(&mut bytes, dependency);
            }
        }
        SendExecutionStatus::Queued {
            queue_item_id,
            reserved_turn_id,
        } => {
            bytes.push(1);
            text(&mut bytes, queue_item_id);
            text(&mut bytes, reserved_turn_id);
        }
        SendExecutionStatus::ProviderStartReserved { obligation_id } => {
            bytes.push(2);
            text(&mut bytes, obligation_id);
        }
        SendExecutionStatus::Running { turn_id } => {
            bytes.push(3);
            text(&mut bytes, turn_id);
        }
        SendExecutionStatus::ReconciliationRequired { failure } => {
            bytes.push(4);
            text(&mut bytes, &failure.correlation_id);
        }
        SendExecutionStatus::Failed { failure } => {
            bytes.push(5);
            text(&mut bytes, &failure.correlation_id);
        }
        SendExecutionStatus::Terminal { result } => {
            bytes.push(6);
            text(&mut bytes, &format!("{result:?}"));
        }
    }
    bytes
}

impl AgentSendOperationUsecase {
    pub fn new(
        repository: Arc<dyn LocalEventTransactionRepository>,
        authority: Arc<dyn OperationBindingAuthority>,
        gate: Arc<dyn SendAdmissionGate>,
        installation_id: String,
    ) -> Self {
        Self {
            repository,
            authority,
            gate,
            installation_id,
            pending_recovery_wakeup: Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub(crate) fn pending_recovery_wakeup(&self) -> Arc<tokio::sync::Notify> {
        self.pending_recovery_wakeup.clone()
    }

    pub(crate) fn wake_pending_recovery(&self) {
        self.pending_recovery_wakeup.notify_one();
    }

    fn commit_identity(&self, operation_id: &str) -> Result<CommitIdentity, SendAgentMessageError> {
        let digest = self
            .authority
            .digest(format!("send-commit\0{operation_id}").as_bytes());
        CommitIdentity::parse(&hex_encode(&digest)).map_err(|_| internal_error("commit-id"))
    }

    fn retry_turn_reconciliation_in_background(
        &self,
        operation_id: String,
        obligation_id: String,
        failure: SafeOperationFailure,
    ) {
        let retry = self.clone();
        tokio::spawn(async move {
            for attempt in 0..SEND_BACKGROUND_RETRY_ATTEMPTS {
                if retry
                    .mark_turn_reconciliation_required(
                        &operation_id,
                        &obligation_id,
                        failure.clone(),
                    )
                    .await
                    .is_ok()
                {
                    return;
                }
                if attempt + 1 < SEND_BACKGROUND_RETRY_ATTEMPTS {
                    tokio::time::sleep(send_background_retry_delay(attempt)).await;
                }
            }
            log::error!(
                "send reconciliation retry budget exhausted [{operation_id}/{obligation_id}]"
            );
            retry.wake_pending_recovery();
        });
    }

    async fn lookup_record(
        &self,
        operation_id: &str,
    ) -> Result<Option<StoredSendRecord>, LocalEventQueryError> {
        let result = self
            .repository
            .query(LocalEventQuery::OperationByIdentity {
                kind: OperationKind::Send,
                operation_id: operation_id.to_string(),
            })
            .await?;
        let LocalEventQueryResult::OperationByIdentity(view) = result else {
            return Err(LocalEventQueryError::Internal {
                correlation_id: "send-lookup-shape".to_string(),
            });
        };
        Ok(view
            .and_then(|view| decode_send_record(view.receipt, view.latest_status, view.revision)))
    }

    pub(crate) async fn accepted_effect_is_dispatchable(
        &self,
        effect: &AcceptedSendEffect,
    ) -> Result<bool, SendAgentMessageError> {
        let Some(operation) = self
            .lookup_record(&effect.operation_id)
            .await
            .map_err(|_| internal_error("dispatchability-operation"))?
        else {
            return Ok(false);
        };
        let operation_status_matches = match (&effect.disposition, &operation.latest_status) {
            (
                SendDisposition::StartedTurn { .. },
                SendExecutionStatus::AwaitingProviderStart { .. },
            ) => true,
            (
                SendDisposition::Queued {
                    queue_item_id: expected_queue_item_id,
                },
                SendExecutionStatus::Queued {
                    queue_item_id,
                    reserved_turn_id,
                },
            ) => {
                queue_item_id == expected_queue_item_id
                    && effect.reserved_turn_id.as_deref() == Some(reserved_turn_id)
            }
            _ => false,
        };
        if operation.receipt.operation_id != effect.operation_id
            || operation.receipt.session_id != effect.session_id
            || operation.receipt.disposition != effect.disposition
            || !operation_status_matches
        {
            return Ok(false);
        }

        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: effect.execution_obligation_id.clone(),
            })
            .await
            .map_err(|_| internal_error("dispatchability-obligation"))?;
        let LocalEventQueryResult::ObligationByIdentity(Some(obligation)) = result else {
            return Ok(false);
        };
        let Some(obligation) = SendObligationData::decode(&obligation.record) else {
            return Err(internal_error("dispatchability-obligation-decode"));
        };
        let obligation_matches = obligation.obligation_id == effect.execution_obligation_id
            && obligation.operation_id == effect.operation_id
            && obligation.session_id == effect.session_id
            && obligation.kind == SendObligationKindRecord::TurnExecution
            && obligation.state == ObligationStateRecord::Pending
            && obligation.human_message_id.as_deref() == Some(effect.human_message_id.as_str())
            && obligation.assistant_message_id == effect.assistant_message_id
            && obligation.reserved_turn_id == effect.reserved_turn_id
            && obligation.canonical_payload == effect.canonical_payload;
        if !obligation_matches {
            return Ok(false);
        }
        let SendDisposition::StartedTurn { turn_id } = &effect.disposition else {
            return Ok(true);
        };
        let turn_id = turn_id
            .parse::<u64>()
            .map_err(|_| internal_error("dispatchability-turn-identity"))?;
        self.gate
            .canonical_immediate_turn_is_current(&effect.session_id, turn_id)
            .await
            .map_err(|_| internal_error("dispatchability-canonical-turn"))
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

    async fn converge_or_reject(
        &self,
        operation_id: &str,
        principal_mac: &[u8; 32],
        binding_hmac: &[u8; 32],
        failure: SafeOperationFailure,
    ) -> Result<SendCommandOutcome, SendAgentMessageError> {
        match self.lookup_record(operation_id).await {
            Ok(Some(record)) => {
                if !constant_time_eq_32(&record.principal_mac, principal_mac) {
                    return Err(SendAgentMessageError::NotFound);
                }
                if !constant_time_eq_32(&record.binding_hmac, binding_hmac) {
                    return Err(SendAgentMessageError::PayloadConflict);
                }
                Ok(SendCommandOutcome::Accepted(AcceptedSendOperation {
                    receipt: record.receipt,
                    latest_status: record.latest_status,
                }))
            }
            Ok(None) => Ok(SendCommandOutcome::RejectedBeforeCommit { failure }),
            Err(_) => Ok(SendCommandOutcome::OutcomeUnknown {
                operation_id: operation_id.to_string(),
            }),
        }
    }

    async fn converge_or_capacity_exceeded(
        &self,
        operation_id: &str,
        principal_mac: &[u8; 32],
        binding_hmac: &[u8; 32],
    ) -> Result<SendCommandOutcome, SendAgentMessageError> {
        match self.lookup_record(operation_id).await {
            Ok(Some(record)) => {
                if !constant_time_eq_32(&record.principal_mac, principal_mac) {
                    return Err(SendAgentMessageError::NotFound);
                }
                if !constant_time_eq_32(&record.binding_hmac, binding_hmac) {
                    return Err(SendAgentMessageError::PayloadConflict);
                }
                Ok(SendCommandOutcome::Accepted(AcceptedSendOperation {
                    receipt: record.receipt,
                    latest_status: record.latest_status,
                }))
            }
            Ok(None) => Err(SendAgentMessageError::CapacityExceeded),
            Err(_) => Ok(SendCommandOutcome::OutcomeUnknown {
                operation_id: operation_id.to_string(),
            }),
        }
    }

    /// Accept (or converge onto) a normal send for a caller operation
    /// identity. Provider effects start only after a fresh acceptance
    /// commit, keyed by the obligation identities in the batch.
    pub async fn send(
        &self,
        request: SendOperationRequest,
    ) -> Result<SendCommandOutcome, SendAgentMessageError> {
        for attempt in 0..SEND_OWNER_REVISION_REPLAN_ATTEMPTS {
            let outcome = self.send_once(request.clone()).await?;
            if matches!(
                &outcome,
                SendCommandOutcome::RejectedBeforeCommit { failure }
                    if failure.kind == SessionOperationFailureKind::OwnerRevisionChanged
            ) && attempt + 1 < SEND_OWNER_REVISION_REPLAN_ATTEMPTS
            {
                continue;
            }
            return Ok(outcome);
        }
        unreachable!("the bounded send replan loop always returns")
    }

    async fn send_once(
        &self,
        request: SendOperationRequest,
    ) -> Result<SendCommandOutcome, SendAgentMessageError> {
        if validate_operation_identity(&request.operation_id).is_err() {
            return Err(SendAgentMessageError::InvalidRequest);
        }
        let principal_mac = self.authority.mac(&principal_material(&request.principal));
        let binding_hmac = self.authority.mac(&super::binding::send(
            &request.principal,
            &self.installation_id,
            &request.operation_id,
            request.canonical_payload.as_bytes(),
        ));

        // Point lookup of the existing binding: same principal / payload
        // converges on the saved receipt, a different payload is a typed
        // conflict, another principal sees NotFound without disclosure.
        match self.lookup_record(&request.operation_id).await {
            Ok(Some(record)) => {
                if !constant_time_eq_32(&record.principal_mac, &principal_mac) {
                    return Err(SendAgentMessageError::NotFound);
                }
                if !constant_time_eq_32(&record.binding_hmac, &binding_hmac) {
                    return Err(SendAgentMessageError::PayloadConflict);
                }
                return Ok(SendCommandOutcome::Accepted(AcceptedSendOperation {
                    receipt: record.receipt,
                    latest_status: record.latest_status,
                }));
            }
            Ok(None) => {}
            Err(_) => {
                // A previous acceptance may exist behind a temporarily
                // unreadable point query. Absence is not proven, so keep the
                // caller's exact identity instead of permitting a new effect.
                return Ok(SendCommandOutcome::OutcomeUnknown {
                    operation_id: request.operation_id,
                });
            }
        }

        // Plan the one-shot disposition; no provider I/O yet.
        let plan = match self
            .gate
            .plan_send(
                &request.principal,
                &request.operation_id,
                &request.canonical_payload,
            )
            .await
        {
            Ok(plan) => plan,
            Err(failure) => {
                if failure.kind == SessionOperationFailureKind::CapacityExceeded {
                    return self
                        .converge_or_capacity_exceeded(
                            &request.operation_id,
                            &principal_mac,
                            &binding_hmac,
                        )
                        .await;
                }
                return self
                    .converge_or_reject(
                        &request.operation_id,
                        &principal_mac,
                        &binding_hmac,
                        failure,
                    )
                    .await;
            }
        };

        let stream_id =
            StreamId::agent_session(&plan.session_id).map_err(|_| internal_error("stream-id"))?;
        let head = match self.current_stream_head(&stream_id).await {
            Ok(head) => head,
            Err(LocalEventQueryError::StorageUnavailable { failure }) => {
                return self
                    .converge_or_reject(
                        &request.operation_id,
                        &principal_mac,
                        &binding_hmac,
                        failure,
                    )
                    .await;
            }
            Err(_) => {
                return self
                    .converge_or_reject(
                        &request.operation_id,
                        &principal_mac,
                        &binding_hmac,
                        storage_failure("head"),
                    )
                    .await;
            }
        };

        let receipt = SendOperationReceipt {
            operation_id: request.operation_id.clone(),
            session_id: plan.session_id.clone(),
            input_ref: plan.input_ref.clone(),
            disposition: plan.disposition.clone(),
        };
        let execution_obligation_id = format!("{}.exec", request.operation_id);
        let terminal_turn_id = match &plan.disposition {
            SendDisposition::StartedTurn { turn_id } => turn_id.clone(),
            SendDisposition::Queued { .. } => plan
                .reserved_turn_id
                .clone()
                .ok_or_else(|| internal_error("queued-turn-identity"))?,
        };
        let latest_status = match &plan.disposition {
            SendDisposition::StartedTurn { .. } => SendExecutionStatus::AwaitingProviderStart {
                dependency_obligation_ids: Vec::new(),
            },
            SendDisposition::Queued { queue_item_id } => SendExecutionStatus::Queued {
                queue_item_id: queue_item_id.clone(),
                reserved_turn_id: terminal_turn_id.clone(),
            },
        };
        // Both immediate and queued sends reserve the assistant identity at
        // acceptance. A later queue drain can therefore materialize
        // TurnStarted + assistant projection idempotently without inventing
        // an identity after restart.
        let assistant_message_id = Some(format!("{}:agent", plan.human_message_id));

        let at = now_ms();
        let mut events = vec![UncommittedDomainEvent {
            stream_id: stream_id.clone(),
            event: LocalDomainEvent::AgentSession(AgentSessionDomainEvent::SendOperationAccepted {
                operation_id: request.operation_id.clone(),
                disposition: plan.disposition.clone(),
                human_message_id: Some(plan.human_message_id.clone()),
                prompt: Some(plan.prompt.clone()),
                reserved_turn_id: plan.reserved_turn_id.clone(),
                at: at as f64,
            }),
            occurred_at_ms: at,
        }];
        if let SendDisposition::StartedTurn { turn_id } = &plan.disposition {
            let turn_id = turn_id
                .parse::<u64>()
                .map_err(|_| internal_error("turn-identity"))?;
            events.push(UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: LocalDomainEvent::AgentSession(AgentSessionDomainEvent::TurnStarted {
                    turn_id,
                    message_id: plan.human_message_id.clone(),
                    assistant_message_id: assistant_message_id.clone(),
                    prompt: plan.prompt.clone(),
                    at: at as f64,
                }),
                occurred_at_ms: at,
            });
        }
        let acceptance_events = events
            .iter()
            .filter_map(|event| match &event.event {
                LocalDomainEvent::AgentSession(event) => Some(event.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut mutations = match self
            .gate
            .acceptance_state_mutations(&plan, &acceptance_events)
            .await
        {
            Ok(mutations) => mutations,
            Err(failure) => {
                return self
                    .converge_or_reject(
                        &request.operation_id,
                        &principal_mac,
                        &binding_hmac,
                        failure,
                    )
                    .await;
            }
        };
        mutations.extend([
            LocalStateMutation::OperationBinding(OperationBindingMutation {
                key: CallerOperationKey {
                    principal: request.principal.clone(),
                    installation_id: self.installation_id.clone(),
                    kind: OperationKind::Send,
                    caller_request_id: request.operation_id.clone(),
                },
                operation_id: request.operation_id.clone(),
                binding_hmac,
            }),
            LocalStateMutation::OperationRecord(OperationRecordMutation {
                kind: OperationKind::Send,
                operation_id: request.operation_id.clone(),
                receipt: receipt_record(&receipt, &principal_mac, &binding_hmac),
                latest_status: status_record(&latest_status),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).map_err(|_| internal_error("revision"))?,
            }),
        ]);
        let mut push_turn_execution_obligation =
            |obligation_id: &str| -> Result<(), SendAgentMessageError> {
                let record = ObligationRecord::Send {
                    obligation_id: obligation_id.to_string(),
                    operation_id: request.operation_id.clone(),
                    session_id: plan.session_id.clone(),
                    kind: SendObligationKindRecord::TurnExecution,
                    disposition: match &plan.disposition {
                        SendDisposition::StartedTurn { .. } => {
                            SendObligationDispositionRecord::StartedTurn
                        }
                        SendDisposition::Queued { .. } => SendObligationDispositionRecord::Queued,
                    },
                    human_message_id: Some(plan.human_message_id.clone()),
                    assistant_message_id: assistant_message_id.clone(),
                    reserved_turn_id: plan.reserved_turn_id.clone(),
                    turn_id: Some(terminal_turn_id.clone()),
                    dependency_obligation_ids: Vec::new(),
                    canonical_payload: request.canonical_payload.clone(),
                    state: ObligationStateRecord::Pending,
                };
                events.push(UncommittedDomainEvent {
                    stream_id: stream_id.clone(),
                    event: LocalDomainEvent::AgentSession(
                        AgentSessionDomainEvent::ObligationRecorded {
                            obligation_id: obligation_id.to_string(),
                            kind: ObligationKind::TurnExecution,
                            state: ObligationState::Pending,
                            at: at as f64,
                        },
                    ),
                    occurred_at_ms: at,
                });
                mutations.push(LocalStateMutation::Obligation(ObligationMutation {
                    obligation_id: obligation_id.to_string(),
                    record,
                    pending: Some(PendingIndexEntry {
                        ordered_key: format!("{at:020}-{obligation_id}"),
                        owner: plan.session_id.clone(),
                        partition: PendingPartition::Owner,
                        shutdown_plan: None,
                    }),
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).map_err(|_| internal_error("revision"))?,
                }));
                Ok(())
            };
        push_turn_execution_obligation(&execution_obligation_id)?;

        let batch = LocalAtomicBatch {
            commit_id: self.commit_identity(&request.operation_id)?,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: OperationKind::Send.into(),
                idempotency_key: request.operation_id.clone(),
                payload_hash: self.authority.digest(&super::binding::send(
                    &request.principal,
                    &self.installation_id,
                    &request.operation_id,
                    request.canonical_payload.as_bytes(),
                )),
            },
            expected_heads: vec![ExpectedStreamHead {
                stream_id: stream_id.clone(),
                expected: StreamVersion::new(head).map_err(|_| internal_error("head"))?,
            }],
            events,
            state_mutations: mutations,
        };

        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_)) => {
                // Dispatch is fenced by a fresh point readback of the complete
                // acceptance participants. A confirmed COMMIT without a
                // readable operation record remains accepted; the app-lifetime
                // recovery owner retries the readback before claiming the
                // provider effect.
                let accepted_effect = AcceptedSendEffect {
                    operation_id: request.operation_id.clone(),
                    session_id: plan.session_id.clone(),
                    human_message_id: plan.human_message_id.clone(),
                    assistant_message_id: assistant_message_id.clone(),
                    disposition: plan.disposition.clone(),
                    reserved_turn_id: plan.reserved_turn_id.clone(),
                    execution_obligation_id: execution_obligation_id.clone(),
                    canonical_payload: request.canonical_payload.clone(),
                };
                let saved = match self.lookup_record(&request.operation_id).await {
                    Ok(Some(saved))
                        if constant_time_eq_32(&saved.principal_mac, &principal_mac)
                            && constant_time_eq_32(&saved.binding_hmac, &binding_hmac) => saved,
                    _ => {
                        // The acceptance is durable and the provider claim is
                        // still Pending. Wake the one app-lifetime supervisor;
                        // it reconstructs the effect from current durable
                        // state instead of retaining this captured request.
                        self.wake_pending_recovery();
                        return Ok(SendCommandOutcome::Accepted(AcceptedSendOperation {
                            receipt,
                            latest_status,
                        }));
                    }
                };
                let dispatch = self.gate.start_provider_effect(&accepted_effect).await;
                let latest_status = if let Err(failure) = dispatch {
                    if self
                        .mark_turn_reconciliation_required(
                            &request.operation_id,
                            &execution_obligation_id,
                            failure.clone(),
                        )
                        .await
                        .is_ok()
                    {
                        self.lookup_record(&request.operation_id)
                            .await
                            .ok()
                            .flatten()
                            .map(|record| record.latest_status)
                            .unwrap_or(SendExecutionStatus::ReconciliationRequired { failure })
                    } else {
                        self.retry_turn_reconciliation_in_background(
                            request.operation_id.clone(),
                            execution_obligation_id.clone(),
                            failure,
                        );
                        SendExecutionStatus::ReconciliationRequired {
                            failure: storage_failure("accepted-dispatch-reconciliation"),
                        }
                    }
                } else {
                    saved.latest_status
                };
                Ok(SendCommandOutcome::Accepted(AcceptedSendOperation {
                    receipt: saved.receipt,
                    latest_status,
                }))
            }
            Ok(CommitBatchResult::Replayed(_)) => {
                // A concurrent retry won the commit; converge on the saved
                // record without starting another effect.
                match self.lookup_record(&request.operation_id).await {
                    Ok(Some(record)) => Ok(SendCommandOutcome::Accepted(AcceptedSendOperation {
                        receipt: record.receipt,
                        latest_status: record.latest_status,
                    })),
                    _ => Ok(SendCommandOutcome::Accepted(AcceptedSendOperation {
                        receipt,
                        latest_status,
                    })),
                }
            }
            Err(
                CommitBatchError::PayloadConflict | CommitBatchError::EffectAdmissionBlocked,
            ) => {
                // Either the same identity with a different payload, or a
                // race with another principal; re-read to classify without
                // disclosing another principal's operation.
                match self.lookup_record(&request.operation_id).await {
                    Ok(Some(record)) => {
                        if !constant_time_eq_32(&record.principal_mac, &principal_mac) {
                            Err(SendAgentMessageError::NotFound)
                        } else if !constant_time_eq_32(&record.binding_hmac, &binding_hmac) {
                            Err(SendAgentMessageError::PayloadConflict)
                        } else {
                            Ok(SendCommandOutcome::Accepted(AcceptedSendOperation {
                                receipt: record.receipt,
                                latest_status: record.latest_status,
                            }))
                        }
                    }
                    Ok(None) => Err(SendAgentMessageError::PayloadConflict),
                    Err(_) => Ok(SendCommandOutcome::OutcomeUnknown {
                        operation_id: request.operation_id.clone(),
                    }),
                }
            }
            Err(CommitBatchError::StreamHeadConflict { .. }) => {
                self.converge_or_reject(
                    &request.operation_id,
                    &principal_mac,
                    &binding_hmac,
                    SafeOperationFailure::new(
                        SessionOperationFailureKind::OwnerRevisionChanged,
                        true,
                        "The session changed while accepting the send. Retry with the same operation identity.",
                        format!("send-head-{}", uuid_like()),
                    ),
                )
                .await
            }
            Err(CommitBatchError::CapacityExceeded)
            | Err(CommitBatchError::SequenceExhausted) => {
                Err(SendAgentMessageError::CapacityExceeded)
            }
            Err(CommitBatchError::StorageUnavailable { failure })
                if failure.is_shutdown_in_progress() =>
            {
                Err(SendAgentMessageError::ShutdownInProgress)
            }
            Err(CommitBatchError::StorageUnavailable { failure }) => {
                self.converge_or_reject(
                    &request.operation_id,
                    &principal_mac,
                    &binding_hmac,
                    failure,
                )
                .await
            }
            Err(CommitBatchError::OutcomeUnknown { .. }) => Ok(SendCommandOutcome::OutcomeUnknown {
                operation_id: request.operation_id.clone(),
            }),
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                Err(SendAgentMessageError::Internal { correlation_id })
            }
        }
    }

    /// Point lookup of a saved send operation (never rebuilt from current
    /// session projections).
    pub async fn get_operation(
        &self,
        principal: &str,
        operation_id: &str,
    ) -> Result<AcceptedSendOperation, GetSendOperationError> {
        if validate_operation_identity(operation_id).is_err() {
            return Err(GetSendOperationError::InvalidRequest);
        }
        let principal_mac = self.authority.mac(&principal_material(principal));
        let record = match self.lookup_record(operation_id).await {
            Ok(record) => record,
            Err(LocalEventQueryError::QueryBusy) => return Err(GetSendOperationError::QueryBusy),
            Err(LocalEventQueryError::DeadlineExceeded) => {
                return Err(GetSendOperationError::DeadlineExceeded)
            }
            Err(LocalEventQueryError::StorageUnavailable { failure }) => {
                return Err(GetSendOperationError::StorageUnavailable { failure })
            }
            Err(_) => {
                return Err(GetSendOperationError::Internal {
                    correlation_id: format!("send-get-{}", uuid_like()),
                })
            }
        };
        let Some(record) = record else {
            let attempt = self
                .repository
                .query(LocalEventQuery::CallerAttemptByIdentity {
                    key: CallerOperationKey {
                        principal: principal.to_string(),
                        installation_id: self.installation_id.clone(),
                        kind: OperationKind::Send,
                        caller_request_id: operation_id.to_string(),
                    },
                })
                .await;
            let attempt = match attempt {
                Ok(LocalEventQueryResult::CallerAttemptByIdentity(attempt)) => attempt,
                Err(LocalEventQueryError::QueryBusy) => {
                    return Err(GetSendOperationError::QueryBusy)
                }
                Err(LocalEventQueryError::DeadlineExceeded) => {
                    return Err(GetSendOperationError::DeadlineExceeded)
                }
                Err(LocalEventQueryError::StorageUnavailable { failure }) => {
                    return Err(GetSendOperationError::StorageUnavailable { failure })
                }
                Ok(_) | Err(_) => {
                    return Err(GetSendOperationError::Internal {
                        correlation_id: format!("send-attempt-get-{}", uuid_like()),
                    })
                }
            };
            return if attempt
                .is_some_and(|attempt| attempt.resolution == CallerAttemptResolution::Pending)
            {
                Err(GetSendOperationError::OutcomeUnknown {
                    operation_id: operation_id.to_string(),
                })
            } else {
                Err(GetSendOperationError::NotFound)
            };
        };
        if !constant_time_eq_32(&record.principal_mac, &principal_mac) {
            return Err(GetSendOperationError::NotFound);
        }
        Ok(AcceptedSendOperation {
            receipt: record.receipt,
            latest_status: record.latest_status,
        })
    }

    async fn restore_queued_effect_after_restart(
        &self,
        execution_obligation_id: &str,
        obligation: &SendObligationData,
        record: &StoredSendRecord,
        queue_item_id: &str,
        reserved_turn_id: &str,
        awaiting_dependencies: Option<&[String]>,
    ) -> Result<bool, SendAgentMessageError> {
        let receipt_queue_matches = matches!(
            &record.receipt.disposition,
            SendDisposition::Queued {
                queue_item_id: receipt_queue_item_id,
            } if receipt_queue_item_id == queue_item_id
        );
        let valid_human_message_id = obligation
            .human_message_id
            .as_deref()
            .filter(|message_id| !message_id.is_empty());
        let valid_assistant_message_id = obligation
            .assistant_message_id
            .as_deref()
            .filter(|message_id| !message_id.is_empty());
        let intent_is_valid = obligation.kind == SendObligationKindRecord::TurnExecution
            && obligation.disposition == SendObligationDispositionRecord::Queued
            && obligation.state == ObligationStateRecord::Pending
            && obligation.session_id == record.receipt.session_id
            && !obligation.session_id.is_empty()
            && !queue_item_id.is_empty()
            && !reserved_turn_id.is_empty()
            && reserved_turn_id.parse::<u64>().is_ok()
            && obligation.reserved_turn_id.as_deref() == Some(reserved_turn_id)
            && obligation.turn_id.as_deref() == Some(reserved_turn_id)
            && awaiting_dependencies.is_none_or(|dependencies| {
                dependencies == obligation.dependency_obligation_ids.as_slice()
            })
            && !obligation.canonical_payload.is_empty()
            && receipt_queue_matches
            && valid_human_message_id.is_some()
            && valid_assistant_message_id.is_some();
        if !intent_is_valid {
            self.mark_turn_reconciliation_required(
                &record.receipt.operation_id,
                execution_obligation_id,
                SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "The accepted queued send intent is incompatible.",
                    format!("send-queued-intent-restart-{}", uuid_like()),
                ),
            )
            .await?;
            return Ok(true);
        }

        // A crash may happen after the acceptance batch but before the
        // post-accept worker records the immutable Queued status. Converge the
        // operation without claiming either provider obligation.
        if awaiting_dependencies.is_some() {
            self.record_execution_status(
                &record.receipt.operation_id,
                SendExecutionStatus::Queued {
                    queue_item_id: queue_item_id.to_string(),
                    reserved_turn_id: reserved_turn_id.to_string(),
                },
            )
            .await?;
        }

        // For a Queued disposition the gate's post-accept path only restores
        // the exact queue item. Omitting the establishment identity and leaving
        // TurnExecution Pending prevents provider I/O before the item becomes
        // executable.
        let dispatch = self
            .gate
            .start_provider_effect(&AcceptedSendEffect {
                operation_id: record.receipt.operation_id.clone(),
                session_id: obligation.session_id.clone(),
                human_message_id: valid_human_message_id
                    .expect("validated queued human identity")
                    .to_string(),
                assistant_message_id: Some(
                    valid_assistant_message_id
                        .expect("validated queued assistant identity")
                        .to_string(),
                ),
                disposition: record.receipt.disposition.clone(),
                reserved_turn_id: Some(reserved_turn_id.to_string()),
                execution_obligation_id: execution_obligation_id.to_string(),
                canonical_payload: obligation.canonical_payload.clone(),
            })
            .await;
        match dispatch {
            Err(failure) => {
                self.mark_turn_reconciliation_required(
                    &record.receipt.operation_id,
                    execution_obligation_id,
                    failure,
                )
                .await?;
                Ok(true)
            }
            Ok(SendEffectDispatch::Scheduled) => Ok(true),
            Ok(SendEffectDispatch::AlreadyScheduled) => Ok(false),
        }
    }

    fn legacy_provider_resume_status(
        record: &StoredSendRecord,
        obligation: &SendObligationData,
    ) -> Option<SendExecutionStatus> {
        match &record.latest_status {
            SendExecutionStatus::AwaitingProviderStart { .. } => {
                match &record.receipt.disposition {
                    SendDisposition::StartedTurn { .. } => {
                        Some(SendExecutionStatus::AwaitingProviderStart {
                            dependency_obligation_ids: Vec::new(),
                        })
                    }
                    SendDisposition::Queued { queue_item_id } => {
                        Some(SendExecutionStatus::Queued {
                            queue_item_id: queue_item_id.clone(),
                            reserved_turn_id: obligation.reserved_turn_id.clone()?,
                        })
                    }
                }
            }
            status @ SendExecutionStatus::Queued { .. } => Some(status.clone()),
            SendExecutionStatus::ReconciliationRequired { .. } => {
                match &record.receipt.disposition {
                    SendDisposition::StartedTurn { .. } => {
                        Some(SendExecutionStatus::AwaitingProviderStart {
                            dependency_obligation_ids: Vec::new(),
                        })
                    }
                    SendDisposition::Queued { queue_item_id } => {
                        Some(SendExecutionStatus::Queued {
                            queue_item_id: queue_item_id.clone(),
                            reserved_turn_id: obligation.reserved_turn_id.clone()?,
                        })
                    }
                }
            }
            SendExecutionStatus::ProviderStartReserved { .. }
            | SendExecutionStatus::Running { .. }
            | SendExecutionStatus::Failed { .. }
            | SendExecutionStatus::Terminal { .. } => None,
        }
    }

    /// Retire the ProviderEstablish dependency written by the superseded
    /// two-flight protocol. Pending means no provider effect was claimed and
    /// is always safe to supersede. A claimed dependency needs an
    /// adapter-specific proof before the exact TurnExecution may continue.
    async fn normalize_legacy_provider_dependency(
        &self,
        obligation: &SendObligationData,
        record: &StoredSendRecord,
    ) -> Result<Option<SendExecutionStatus>, SendAgentMessageError> {
        let [dependency_id] = obligation.dependency_obligation_ids.as_slice() else {
            self.mark_turn_reconciliation_required(
                &record.receipt.operation_id,
                &obligation.obligation_id,
                SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "The accepted send has incompatible legacy dependencies.",
                    format!("send-legacy-dependencies-{}", uuid_like()),
                ),
            )
            .await?;
            return Ok(None);
        };
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: dependency_id.clone(),
            })
            .await
            .map_err(|_| internal_error("legacy-provider-dependency"))?;
        let LocalEventQueryResult::ObligationByIdentity(Some(dependency)) = result else {
            self.mark_turn_reconciliation_required(
                &record.receipt.operation_id,
                &obligation.obligation_id,
                SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "The accepted send legacy dependency is unavailable.",
                    format!("send-legacy-dependency-missing-{}", uuid_like()),
                ),
            )
            .await?;
            return Ok(None);
        };
        let Some(dependency_value) =
            SendObligationData::decode(&dependency.record).filter(|value| {
                value.obligation_id == *dependency_id
                    && value.operation_id == obligation.operation_id
                    && value.session_id == obligation.session_id
                    && value.kind == SendObligationKindRecord::ProviderEstablish
            })
        else {
            self.mark_turn_reconciliation_required(
                &record.receipt.operation_id,
                &obligation.obligation_id,
                SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "The accepted send legacy dependency is incompatible.",
                    format!("send-legacy-dependency-invalid-{}", uuid_like()),
                ),
            )
            .await?;
            return Ok(None);
        };
        if has_recovery_wrapper(&dependency.record)
            && (unresolved_recovery_original_identity(dependency_id, &dependency.record).is_some()
                || !matches!(
                    dependency_value.state,
                    ObligationStateRecord::Completed | ObligationStateRecord::Cancelled
                ))
        {
            return Ok(None);
        }
        let Some(resume_status) = Self::legacy_provider_resume_status(record, obligation) else {
            return Ok(None);
        };

        let can_continue = match dependency_value.state {
            ObligationStateRecord::Pending
            | ObligationStateRecord::Completed
            | ObligationStateRecord::Cancelled => true,
            ObligationStateRecord::EffectReserved
            | ObligationStateRecord::ReconciliationRequired => {
                self.gate
                    .classify_legacy_provider_establish(&obligation.session_id)
                    .await
                    .map_err(|_| internal_error("legacy-provider-classification"))?
                    == LegacyProviderEstablishRecovery::ContinueTurnExecution
            }
            ObligationStateRecord::Prepared
            | ObligationStateRecord::Running
            | ObligationStateRecord::WaitingApproval
            | ObligationStateRecord::OutcomeUnknown
            | ObligationStateRecord::Failed => false,
        };
        if !can_continue {
            let failure = match &record.latest_status {
                SendExecutionStatus::ReconciliationRequired { failure } => failure.clone(),
                _ => SafeOperationFailure::new(
                    SessionOperationFailureKind::OutcomeUnknown,
                    true,
                    "Provider establishment requires same-effect readback after restart.",
                    format!("send-establish-restart-{}", uuid_like()),
                ),
            };
            match dependency_value.state {
                ObligationStateRecord::EffectReserved => {
                    self.transition_obligation(ObligationTransition {
                        operation_id: &obligation.operation_id,
                        obligation_id: dependency_id,
                        expected_kind: "provider_establish",
                        expected_state: "effect_reserved",
                        next_state: "reconciliation_required",
                        keep_pending: true,
                        status: Some(SendExecutionStatus::ReconciliationRequired { failure }),
                    })
                    .await?;
                }
                ObligationStateRecord::ReconciliationRequired => {
                    if !matches!(
                        record.latest_status,
                        SendExecutionStatus::ReconciliationRequired { .. }
                    ) {
                        self.record_execution_status(
                            &obligation.operation_id,
                            SendExecutionStatus::ReconciliationRequired { failure },
                        )
                        .await?;
                    }
                }
                _ => {
                    self.mark_turn_reconciliation_required(
                        &obligation.operation_id,
                        &obligation.obligation_id,
                        failure,
                    )
                    .await?;
                }
            }
            return Ok(None);
        }

        match dependency_value.state {
            ObligationStateRecord::Pending => {
                self.transition_obligation(ObligationTransition {
                    operation_id: &obligation.operation_id,
                    obligation_id: dependency_id,
                    expected_kind: "provider_establish",
                    expected_state: "pending",
                    next_state: "cancelled",
                    keep_pending: false,
                    status: Some(resume_status.clone()),
                })
                .await?;
            }
            ObligationStateRecord::EffectReserved => {
                self.transition_obligation(ObligationTransition {
                    operation_id: &obligation.operation_id,
                    obligation_id: dependency_id,
                    expected_kind: "provider_establish",
                    expected_state: "effect_reserved",
                    next_state: "cancelled",
                    keep_pending: false,
                    status: Some(resume_status.clone()),
                })
                .await?;
            }
            ObligationStateRecord::ReconciliationRequired => {
                self.transition_obligation(ObligationTransition {
                    operation_id: &obligation.operation_id,
                    obligation_id: dependency_id,
                    expected_kind: "provider_establish",
                    expected_state: "reconciliation_required",
                    next_state: "cancelled",
                    keep_pending: false,
                    status: Some(resume_status.clone()),
                })
                .await?;
            }
            ObligationStateRecord::Completed | ObligationStateRecord::Cancelled => {
                if record.latest_status != resume_status {
                    self.record_execution_status(&obligation.operation_id, resume_status.clone())
                        .await?;
                }
            }
            ObligationStateRecord::Prepared
            | ObligationStateRecord::Running
            | ObligationStateRecord::WaitingApproval
            | ObligationStateRecord::OutcomeUnknown
            | ObligationStateRecord::Failed => unreachable!("non-continuable state returned early"),
        }
        Ok(Some(resume_status))
    }

    async fn immediate_turn_is_canonical_current(
        &self,
        obligation: &SendObligationData,
        record: &StoredSendRecord,
    ) -> Result<bool, SendAgentMessageError> {
        let SendDisposition::StartedTurn { turn_id } = &record.receipt.disposition else {
            return Ok(true);
        };
        if obligation.turn_id.as_deref() != Some(turn_id.as_str()) {
            return Ok(false);
        }
        let turn_id = turn_id
            .parse::<u64>()
            .map_err(|_| internal_error("recovery-canonical-turn-identity"))?;
        self.gate
            .canonical_immediate_turn_is_current(&obligation.session_id, turn_id)
            .await
            .map_err(|_| internal_error("recovery-canonical-turn"))
    }

    async fn retire_stale_pending_immediate_turn(
        &self,
        obligation: &SendObligationData,
    ) -> Result<(), SendAgentMessageError> {
        let failure = SafeOperationFailure::new(
            SessionOperationFailureKind::InvalidEffectIntent,
            false,
            "The accepted send no longer owns the canonical active turn.",
            format!("send-stale-turn-{}", uuid_like()),
        );
        self.transition_obligation(ObligationTransition {
            operation_id: &obligation.operation_id,
            obligation_id: &obligation.obligation_id,
            expected_kind: "turn_execution",
            expected_state: "pending",
            next_state: "cancelled",
            keep_pending: false,
            status: Some(SendExecutionStatus::Failed { failure }),
        })
        .await
        .map(|_| ())
    }

    async fn complete_obligation_for_terminal_operation(
        &self,
        operation_id: &str,
        obligation_id: &str,
    ) -> Result<(), SendAgentMessageError> {
        let operation = self
            .lookup_record(operation_id)
            .await
            .map_err(|_| internal_error("terminal-convergence-operation-lookup"))?
            .ok_or(SendAgentMessageError::NotFound)?;
        if !matches!(
            operation.latest_status,
            SendExecutionStatus::Terminal { .. }
        ) {
            return Err(internal_error("terminal-convergence-operation-state"));
        }
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.to_string(),
            })
            .await
            .map_err(|_| internal_error("terminal-convergence-obligation-lookup"))?;
        let LocalEventQueryResult::ObligationByIdentity(Some(obligation)) = result else {
            return Err(internal_error("terminal-convergence-obligation-shape"));
        };
        let value = SendObligationData::decode(&obligation.record)
            .ok_or_else(|| internal_error("terminal-convergence-obligation-decode"))?;
        if value.obligation_id != obligation_id
            || value.operation_id != operation_id
            || value.session_id != operation.receipt.session_id
        {
            return Err(internal_error("terminal-convergence-obligation-owner"));
        }
        if value.state == ObligationStateRecord::Completed && obligation.pending.is_none() {
            return Ok(());
        }
        let completed =
            send_obligation_record_with_state(&obligation.record, ObligationStateRecord::Completed)
                .ok_or_else(|| internal_error("terminal-convergence-obligation-wrapper"))?;
        let next_revision = obligation
            .revision
            .next()
            .ok_or(SendAgentMessageError::CapacityExceeded)?;
        let idempotency_key = format!(
            "{operation_id}.{obligation_id}.terminal-convergence.{}",
            obligation.revision.value()
        );
        let digest = self
            .authority
            .digest(format!("send-terminal-convergence/v1\0{idempotency_key}").as_bytes());
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex_encode(&digest))
                .map_err(|_| internal_error("terminal-convergence-commit-id"))?,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::OperationProgress,
                idempotency_key,
                payload_hash: self.authority.digest(&[
                    ObligationStateRecord::Completed as u8,
                    (obligation.pending.is_some()) as u8,
                ]),
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: obligation_id.to_string(),
                record: completed,
                pending: None,
                expected: RevisionGuard::Expected(obligation.revision),
                revision: next_revision,
            })],
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)) => Ok(()),
            Err(_) => {
                let operation = self
                    .lookup_record(operation_id)
                    .await
                    .map_err(|_| internal_error("terminal-convergence-readback-operation"))?
                    .ok_or(SendAgentMessageError::NotFound)?;
                let result = self
                    .repository
                    .query(LocalEventQuery::ObligationByIdentity {
                        obligation_id: obligation_id.to_string(),
                    })
                    .await
                    .map_err(|_| internal_error("terminal-convergence-readback-obligation"))?;
                let LocalEventQueryResult::ObligationByIdentity(Some(obligation)) = result else {
                    return Err(internal_error("terminal-convergence-readback-shape"));
                };
                let converged =
                    SendObligationData::decode(&obligation.record).is_some_and(|value| {
                        value.obligation_id == obligation_id
                            && value.operation_id == operation_id
                            && value.state == ObligationStateRecord::Completed
                    }) && obligation.pending.is_none()
                        && matches!(
                            operation.latest_status,
                            SendExecutionStatus::Terminal { .. }
                        );
                if converged {
                    Ok(())
                } else {
                    Err(internal_error("terminal-convergence-commit"))
                }
            }
        }
    }

    /// Resume only provider effects that were durably accepted but never
    /// reserved. A reserved effect has an ambiguous external outcome after a
    /// process restart and is fenced as ReconciliationRequired instead of
    /// being run blindly a second time.
    #[cfg(test)]
    pub async fn recover_pending_provider_effects(&self) -> Result<(), SendAgentMessageError> {
        self.recover_pending_provider_effects_pass()
            .await
            .map(|_| ())
    }

    pub(crate) async fn recover_pending_provider_effects_pass(
        &self,
    ) -> Result<usize, SendAgentMessageError> {
        let mut cursor = None;
        let mut processed = 0usize;
        loop {
            let result = self
                .repository
                .query(LocalEventQuery::PendingRecoveryPage {
                    limit: 200,
                    partition: None,
                    owner: None,
                    ordered_key_prefix: None,
                    shutdown_plan: None,
                    cursor,
                })
                .await
                .map_err(|_| internal_error("recovery-page"))?;
            let LocalEventQueryResult::PendingRecoveryPage(page) = result else {
                return Err(internal_error("recovery-page-shape"));
            };
            for entry in page.entries {
                if entry.partition != PendingPartition::Owner {
                    continue;
                }
                let send_identity = entry.obligation_id.ends_with(".exec")
                    || entry.obligation_id.ends_with(".establish");
                let obligation = match SendObligationData::decode(&entry.record) {
                    Some(obligation) => obligation,
                    None if !send_identity => continue,
                    None => return Err(internal_error("recovery-obligation-decode")),
                };
                if obligation.obligation_id != entry.obligation_id {
                    if send_identity {
                        return Err(internal_error("recovery-obligation-reference"));
                    }
                    continue;
                }
                if has_recovery_wrapper(&entry.record)
                    && (unresolved_recovery_original_identity(&entry.obligation_id, &entry.record)
                        .is_some()
                        || !matches!(
                            obligation.state,
                            ObligationStateRecord::Completed | ObligationStateRecord::Cancelled
                        ))
                {
                    continue;
                }
                let operation_id = obligation.operation_id.as_str();
                let record = self
                    .lookup_record(operation_id)
                    .await
                    .map_err(|_| internal_error("recovery-operation"))?
                    .ok_or_else(|| internal_error("recovery-operation-missing"))?;
                if record.receipt.operation_id != operation_id {
                    return Err(internal_error("recovery-operation-reference"));
                }
                if matches!(&record.latest_status, SendExecutionStatus::Terminal { .. }) {
                    self.complete_obligation_for_terminal_operation(
                        operation_id,
                        &entry.obligation_id,
                    )
                    .await?;
                    processed = processed.saturating_add(1);
                    continue;
                }
                if obligation.kind == SendObligationKindRecord::ProviderEstablish {
                    continue;
                }
                if matches!(
                    obligation.state,
                    ObligationStateRecord::Completed | ObligationStateRecord::Cancelled
                ) {
                    // A terminal obligation must never be handed to the
                    // provider even if an incompatible older store retained
                    // its pending-index row. Leave that row visible for
                    // explicit repair instead of replaying the effect.
                    continue;
                }
                let pending_legacy_dependency = obligation.state == ObligationStateRecord::Pending
                    && !obligation.dependency_obligation_ids.is_empty();
                let recovery_status = if pending_legacy_dependency {
                    let Some(status) = self
                        .normalize_legacy_provider_dependency(&obligation, &record)
                        .await?
                    else {
                        continue;
                    };
                    status
                } else {
                    record.latest_status.clone()
                };
                if self
                    .gate
                    .owns_current_process_turn_execution(
                        &obligation.session_id,
                        operation_id,
                        &entry.obligation_id,
                    )
                    .await
                {
                    continue;
                }
                if obligation.state == ObligationStateRecord::Pending
                    && matches!(
                        &record.receipt.disposition,
                        SendDisposition::StartedTurn { .. }
                    )
                    && !self
                        .immediate_turn_is_canonical_current(&obligation, &record)
                        .await?
                {
                    // A superseded two-flight send never claimed its input.
                    // Once its legacy ProviderEstablish dependency has been
                    // safely retired above, remove this stale execution from
                    // the owner inventory instead of running it against the
                    // newer canonical turn.
                    self.retire_stale_pending_immediate_turn(&obligation)
                        .await?;
                    processed = processed.saturating_add(1);
                    continue;
                }
                let state_status_mismatch = match obligation.state {
                    ObligationStateRecord::EffectReserved => true,
                    ObligationStateRecord::ReconciliationRequired => !matches!(
                        recovery_status,
                        SendExecutionStatus::ReconciliationRequired { .. }
                    ),
                    ObligationStateRecord::Pending => matches!(
                        recovery_status,
                        SendExecutionStatus::ProviderStartReserved { .. }
                            | SendExecutionStatus::Running { .. }
                            | SendExecutionStatus::ReconciliationRequired { .. }
                            | SendExecutionStatus::Failed { .. }
                    ),
                    ObligationStateRecord::Prepared
                    | ObligationStateRecord::Running
                    | ObligationStateRecord::WaitingApproval
                    | ObligationStateRecord::OutcomeUnknown
                    | ObligationStateRecord::Failed => true,
                    ObligationStateRecord::Completed | ObligationStateRecord::Cancelled => false,
                };
                if state_status_mismatch {
                    let failure = match &record.latest_status {
                        SendExecutionStatus::ReconciliationRequired { failure }
                        | SendExecutionStatus::Failed { failure } => failure.clone(),
                        _ => SafeOperationFailure::new(
                            SessionOperationFailureKind::OutcomeUnknown,
                            true,
                            "The accepted provider effect requires same-effect readback.",
                            format!("send-state-status-mismatch-{}", uuid_like()),
                        ),
                    };
                    self.mark_turn_reconciliation_required(
                        operation_id,
                        &entry.obligation_id,
                        failure,
                    )
                    .await?;
                    processed = processed.saturating_add(1);
                    continue;
                }
                match &recovery_status {
                    SendExecutionStatus::AwaitingProviderStart {
                        dependency_obligation_ids,
                    } if matches!(&record.receipt.disposition, SendDisposition::Queued { .. }) => {
                        let queue_item_id = match &record.receipt.disposition {
                            SendDisposition::Queued { queue_item_id } => queue_item_id,
                            SendDisposition::StartedTurn { .. } => unreachable!(),
                        };
                        let reserved_turn_id =
                            obligation.reserved_turn_id.as_deref().unwrap_or_default();
                        if self
                            .restore_queued_effect_after_restart(
                                &entry.obligation_id,
                                &obligation,
                                &record,
                                queue_item_id,
                                reserved_turn_id,
                                (!pending_legacy_dependency)
                                    .then_some(dependency_obligation_ids.as_slice()),
                            )
                            .await?
                        {
                            processed = processed.saturating_add(1);
                        }
                        continue;
                    }
                    SendExecutionStatus::Queued {
                        queue_item_id,
                        reserved_turn_id,
                    } => {
                        if self
                            .restore_queued_effect_after_restart(
                                &entry.obligation_id,
                                &obligation,
                                &record,
                                queue_item_id,
                                reserved_turn_id,
                                None,
                            )
                            .await?
                        {
                            processed = processed.saturating_add(1);
                        }
                        continue;
                    }
                    _ => {}
                }
                match recovery_status {
                    SendExecutionStatus::AwaitingProviderStart { .. } => {
                        let valid_human_message_id = obligation
                            .human_message_id
                            .as_deref()
                            .filter(|message_id| !message_id.is_empty());
                        let Some(human_message_id) = valid_human_message_id.filter(|_| {
                            !obligation.session_id.is_empty()
                                && !obligation.canonical_payload.is_empty()
                        }) else {
                            self.mark_turn_reconciliation_required(
                                operation_id,
                                &entry.obligation_id,
                                SafeOperationFailure::new(
                                    SessionOperationFailureKind::InvalidEffectIntent,
                                    false,
                                    "The accepted provider effect intent is incompatible.",
                                    format!("send-intent-restart-{}", uuid_like()),
                                ),
                            )
                            .await?;
                            processed = processed.saturating_add(1);
                            continue;
                        };
                        // The gate performs the one canonical Pending ->
                        // EffectReserved CAS immediately before handoff. Both
                        // startup recovery and the post-accept worker enter
                        // through that same claim, so only its commit winner
                        // may execute the accepted turn.
                        let dispatch = self
                            .gate
                            .start_provider_effect(&AcceptedSendEffect {
                                operation_id: operation_id.to_string(),
                                session_id: obligation.session_id.clone(),
                                human_message_id: human_message_id.to_string(),
                                assistant_message_id: obligation.assistant_message_id.clone(),
                                disposition: record.receipt.disposition,
                                reserved_turn_id: obligation.reserved_turn_id.clone(),
                                execution_obligation_id: entry.obligation_id.clone(),
                                canonical_payload: obligation.canonical_payload.clone(),
                            })
                            .await;
                        match dispatch {
                            Err(failure) => {
                                self.mark_turn_reconciliation_required(
                                    operation_id,
                                    &entry.obligation_id,
                                    failure,
                                )
                                .await?;
                                processed = processed.saturating_add(1);
                            }
                            Ok(SendEffectDispatch::Scheduled) => {
                                processed = processed.saturating_add(1);
                            }
                            Ok(SendEffectDispatch::AlreadyScheduled) => {}
                        }
                    }
                    SendExecutionStatus::ProviderStartReserved { .. } => {
                        self.mark_turn_reconciliation_required(
                            operation_id,
                            &entry.obligation_id,
                            SafeOperationFailure::new(
                                SessionOperationFailureKind::OutcomeUnknown,
                                true,
                                "The accepted provider effect requires readback after restart.",
                                format!("send-restart-{}", uuid_like()),
                            ),
                        )
                        .await?;
                        processed = processed.saturating_add(1);
                    }
                    SendExecutionStatus::Queued { .. } => unreachable!(),
                    SendExecutionStatus::Running { .. }
                    | SendExecutionStatus::ReconciliationRequired { .. }
                    | SendExecutionStatus::Failed { .. }
                    | SendExecutionStatus::Terminal { .. } => {}
                }
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(processed)
    }

    /// Prepare the send-owned operation/obligation participants for the
    /// canonical terminal winner. The caller commits these mutations in the
    /// same batch as the `(session, turn)` terminal record and projections.
    pub(crate) async fn prepare_runtime_terminal_participants(
        &self,
        terminal: &TerminalRecordMutation,
    ) -> Result<TerminalParticipants, String> {
        let turn_result = match &terminal.result {
            TerminalResultRecord::AgentTurn {
                result: AgentTurnTerminalResultRecord::Current(turn_result),
                ..
            }
            | TerminalResultRecord::SessionClosed {
                result: turn_result,
                ..
            }
            | TerminalResultRecord::Stop {
                result: turn_result,
                ..
            } => turn_result,
            TerminalResultRecord::StopSuperseded { .. } => {
                return Err("runtime terminal send result is incompatible".to_string())
            }
        };
        let mut cursor = None;
        let mut matched = None;
        loop {
            let page = self
                .repository
                .query(LocalEventQuery::PendingRecoveryPage {
                    limit: 200,
                    // Owner is an indexed query dimension of its own.  The
                    // store rejects combining it with a primary partition
                    // filter, so constrain by owner here and validate the
                    // returned primary partition before interpreting any
                    // record as a send-owned terminal participant.
                    partition: None,
                    owner: Some(terminal.session_id.clone()),
                    ordered_key_prefix: None,
                    shutdown_plan: None,
                    cursor,
                })
                .await
                .map_err(|_| "runtime terminal send inventory lookup failed".to_string())?;
            let LocalEventQueryResult::PendingRecoveryPage(page) = page else {
                return Err("runtime terminal send inventory query shape is invalid".into());
            };
            for entry in page.entries {
                if entry.partition != PendingPartition::Owner || entry.owner != terminal.session_id
                {
                    return Err(
                        "runtime terminal send inventory owner partition is inconsistent".into(),
                    );
                }
                let Some(obligation) = SendObligationData::decode(&entry.record) else {
                    continue;
                };
                if obligation.kind != SendObligationKindRecord::TurnExecution
                    || obligation.session_id != terminal.session_id
                {
                    continue;
                }
                let operation_id = obligation.operation_id.clone();
                let operation = self
                    .lookup_record(&operation_id)
                    .await
                    .map_err(|_| "runtime terminal send operation lookup failed".to_string())?
                    .ok_or_else(|| "runtime terminal send operation is missing".to_string())?;
                let bound_turn_id = obligation
                    .turn_id
                    .clone()
                    .or_else(|| match &operation.receipt.disposition {
                        SendDisposition::StartedTurn { turn_id } => Some(turn_id.clone()),
                        SendDisposition::Queued { .. } => obligation.reserved_turn_id.clone(),
                    })
                    .ok_or_else(|| {
                        "runtime terminal send obligation has no turn identity".to_string()
                    })?;
                if bound_turn_id != terminal.turn_id {
                    continue;
                }
                if matched.is_some() {
                    return Err("multiple send operations own the same terminal turn".into());
                }
                if operation.receipt.session_id != terminal.session_id
                    || !matches!(
                        obligation.state,
                        ObligationStateRecord::EffectReserved
                            | ObligationStateRecord::ReconciliationRequired
                    )
                {
                    return Err("runtime terminal send binding is inconsistent".into());
                }
                if matches!(
                    operation.latest_status,
                    SendExecutionStatus::Terminal { .. }
                ) {
                    return Err("terminal send operation still owns a pending obligation".into());
                }
                let operation_revision = operation.revision.next().ok_or_else(|| {
                    "runtime terminal send operation revision is exhausted".to_string()
                })?;
                let obligation_revision = entry.revision.next().ok_or_else(|| {
                    "runtime terminal send obligation revision is exhausted".to_string()
                })?;
                // The terminal winner replaces the mutable recovery wrapper
                // with the canonical plain Send owner. RecoveryActionUsecase
                // retains authority over its own action record; carrying that
                // wrapper into this independent terminal mutation would make
                // the terminal participant impersonate the recovery owner.
                let completed_obligation =
                    obligation.into_record_with_state(ObligationStateRecord::Completed);
                matched = Some(vec![
                    LocalStateMutation::OperationRecord(OperationRecordMutation {
                        kind: OperationKind::Send,
                        operation_id: operation_id.clone(),
                        receipt: receipt_record(
                            &operation.receipt,
                            &operation.principal_mac,
                            &operation.binding_hmac,
                        ),
                        latest_status: status_record(&SendExecutionStatus::Terminal {
                            result: turn_result.clone(),
                        }),
                        expected: RevisionGuard::Expected(operation.revision),
                        revision: operation_revision,
                    }),
                    LocalStateMutation::Obligation(ObligationMutation {
                        obligation_id: entry.obligation_id,
                        record: completed_obligation,
                        pending: None,
                        expected: RevisionGuard::Expected(entry.revision),
                        revision: obligation_revision,
                    }),
                ]);
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(TerminalParticipants {
            events: Vec::new(),
            mutations: matched.unwrap_or_default(),
        })
    }

    /// Update the mutable latest status of an accepted operation. The
    /// immutable receipt is preserved byte-for-byte; the update is a CAS on
    /// the operation record revision.
    pub async fn record_execution_status(
        &self,
        operation_id: &str,
        status: SendExecutionStatus,
    ) -> Result<(), SendAgentMessageError> {
        if validate_operation_identity(operation_id).is_err() {
            return Err(SendAgentMessageError::InvalidRequest);
        }
        let record = match self.lookup_record(operation_id).await {
            Ok(Some(record)) => record,
            Ok(None) => return Err(SendAgentMessageError::NotFound),
            Err(_) => return Err(internal_error("status-lookup")),
        };
        if record.latest_status == status {
            return Ok(());
        }
        // A canonical terminal winner is absorbing. Late runtime bookkeeping
        // (Running or reconciliation after a very fast terminal event) must
        // never move the operation back to a nonterminal status.
        if matches!(record.latest_status, SendExecutionStatus::Terminal { .. })
            && !matches!(status, SendExecutionStatus::Terminal { .. })
        {
            return Ok(());
        }
        if matches!(status, SendExecutionStatus::Queued { .. })
            && matches!(
                record.latest_status,
                SendExecutionStatus::ProviderStartReserved { .. }
                    | SendExecutionStatus::Running { .. }
                    | SendExecutionStatus::ReconciliationRequired { .. }
                    | SendExecutionStatus::Failed { .. }
                    | SendExecutionStatus::Terminal { .. }
            )
        {
            return Ok(());
        }
        let next_revision = record
            .revision
            .next()
            .ok_or(SendAgentMessageError::CapacityExceeded)?;
        let status_material = status_identity_material(&status);
        let status_hash = self.authority.digest(&status_material);
        let reserved_obligation = match &status {
            SendExecutionStatus::ProviderStartReserved { obligation_id } => {
                let result = self
                    .repository
                    .query(LocalEventQuery::ObligationByIdentity {
                        obligation_id: obligation_id.clone(),
                    })
                    .await
                    .map_err(|_| internal_error("status-obligation-lookup"))?;
                let LocalEventQueryResult::ObligationByIdentity(obligation) = result else {
                    return Err(internal_error("status-obligation-shape"));
                };
                let obligation = obligation.ok_or(SendAgentMessageError::NotFound)?;
                let value = SendObligationData::decode(&obligation.record)
                    .ok_or_else(|| internal_error("status-obligation-decode"))?;
                if value.operation_id != operation_id
                    || value.state != ObligationStateRecord::Pending
                {
                    return Err(internal_error("status-obligation-invariant"));
                }
                let reserved_record = send_obligation_record_with_state(
                    &obligation.record,
                    ObligationStateRecord::EffectReserved,
                )
                .ok_or_else(|| internal_error("status-obligation-wrapper"))?;
                let pending = obligation.pending.map(|pending| PendingIndexEntry {
                    ordered_key: pending.ordered_key,
                    owner: pending.owner,
                    partition: pending.partition,
                    shutdown_plan: pending.shutdown_plan,
                });
                Some(ObligationMutation {
                    obligation_id: obligation.obligation_id,
                    record: reserved_record,
                    pending,
                    expected: RevisionGuard::Expected(obligation.revision),
                    revision: obligation
                        .revision
                        .next()
                        .ok_or(SendAgentMessageError::CapacityExceeded)?,
                })
            }
            _ => None,
        };
        let commit_digest = self
            .authority
            .digest(format!("send-status\0{operation_id}\0{}", next_revision.value()).as_bytes());
        let mut state_mutations = vec![LocalStateMutation::OperationRecord(
            OperationRecordMutation {
                kind: OperationKind::Send,
                operation_id: operation_id.to_string(),
                receipt: receipt_record(
                    &record.receipt,
                    &record.principal_mac,
                    &record.binding_hmac,
                ),
                latest_status: status_record(&status),
                expected: RevisionGuard::Expected(record.revision),
                revision: next_revision,
            },
        )];
        if let Some(obligation) = reserved_obligation {
            state_mutations.push(LocalStateMutation::Obligation(obligation));
        }
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex_encode(&commit_digest))
                .map_err(|_| internal_error("status-commit-id"))?,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::OperationProgress,
                idempotency_key: format!("{operation_id}.st{}", next_revision.value()),
                payload_hash: status_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations,
        };
        match self.repository.commit_batch(batch).await {
            Ok(_) => Ok(()),
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                Err(SendAgentMessageError::Internal { correlation_id })
            }
            Err(_) => Err(internal_error("status-commit")),
        }
    }

    /// Fail an accepted TurnExecution that was rejected before any durable
    /// provider-effect reservation could be obtained.
    ///
    /// This is a terminal operation outcome: the obligation leaves the pending
    /// recovery index, while the immutable Accepted receipt remains available
    /// with a visible `Failed` status.
    pub(crate) async fn fail_unclaimed_turn_execution(
        &self,
        operation_id: &str,
        obligation_id: &str,
        failure: SafeOperationFailure,
    ) -> Result<(), SendAgentMessageError> {
        if validate_operation_identity(operation_id).is_err() {
            return Err(SendAgentMessageError::InvalidRequest);
        }
        let record = self
            .lookup_record(operation_id)
            .await
            .map_err(|_| internal_error("turn-failure-operation-lookup"))?
            .ok_or(SendAgentMessageError::NotFound)?;
        if matches!(record.latest_status, SendExecutionStatus::Terminal { .. }) {
            return self
                .complete_obligation_for_terminal_operation(operation_id, obligation_id)
                .await;
        }
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.to_string(),
            })
            .await
            .map_err(|_| internal_error("turn-failure-obligation-lookup"))?;
        let LocalEventQueryResult::ObligationByIdentity(Some(obligation)) = result else {
            return Err(internal_error("turn-failure-obligation-shape"));
        };
        let value = SendObligationData::decode(&obligation.record)
            .ok_or_else(|| internal_error("turn-failure-obligation-decode"))?;
        if value.operation_id != operation_id
            || value.obligation_id != obligation_id
            || value.kind != SendObligationKindRecord::TurnExecution
        {
            return Err(internal_error("turn-failure-owner"));
        }
        if value.state == ObligationStateRecord::Failed
            && matches!(record.latest_status, SendExecutionStatus::Failed { .. })
        {
            return Ok(());
        }
        if value.state != ObligationStateRecord::Pending {
            return Err(internal_error("turn-failure-obligation-state"));
        }
        self.transition_obligation(ObligationTransition {
            operation_id,
            obligation_id,
            expected_kind: "turn_execution",
            expected_state: "pending",
            next_state: "failed",
            keep_pending: false,
            status: Some(SendExecutionStatus::Failed { failure }),
        })
        .await
        .map(|_| ())
    }

    /// Atomically publish an ambiguous TurnExecution effect as recovery work.
    ///
    /// The operation and its execution obligation advance in one batch. This
    /// covers both an unclaimed invalid intent and an ambiguous claimed
    /// effect. A legacy operation-only reconciliation is repaired while
    /// preserving the already-published failure. A terminal operation is
    /// absorbing, including when it wins a race with this update.
    pub(crate) async fn mark_turn_reconciliation_required(
        &self,
        operation_id: &str,
        obligation_id: &str,
        failure: SafeOperationFailure,
    ) -> Result<(), SendAgentMessageError> {
        if validate_operation_identity(operation_id).is_err() {
            return Err(SendAgentMessageError::InvalidRequest);
        }
        let record = self
            .lookup_record(operation_id)
            .await
            .map_err(|_| internal_error("turn-reconciliation-operation-lookup"))?
            .ok_or(SendAgentMessageError::NotFound)?;
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.to_string(),
            })
            .await
            .map_err(|_| internal_error("turn-reconciliation-obligation-lookup"))?;
        let LocalEventQueryResult::ObligationByIdentity(obligation) = result else {
            return Err(internal_error("turn-reconciliation-obligation-shape"));
        };
        let obligation = obligation.ok_or(SendAgentMessageError::NotFound)?;
        let value = SendObligationData::decode(&obligation.record)
            .ok_or_else(|| internal_error("turn-reconciliation-obligation-decode"))?;
        if value.obligation_id != obligation_id
            || value.operation_id != operation_id
            || value.kind != SendObligationKindRecord::TurnExecution
        {
            return Err(internal_error("turn-reconciliation-owner"));
        }
        if matches!(&record.latest_status, SendExecutionStatus::Terminal { .. }) {
            return self
                .complete_obligation_for_terminal_operation(operation_id, obligation_id)
                .await;
        }
        if !matches!(
            value.state,
            ObligationStateRecord::Pending
                | ObligationStateRecord::EffectReserved
                | ObligationStateRecord::ReconciliationRequired
        ) {
            return Err(internal_error("turn-reconciliation-obligation-state"));
        }

        let operation_needs_update = !matches!(
            &record.latest_status,
            SendExecutionStatus::ReconciliationRequired { .. }
        );
        let obligation_needs_update = value.state != ObligationStateRecord::ReconciliationRequired;
        if !operation_needs_update && !obligation_needs_update {
            return Ok(());
        }

        let reconciliation = match &record.latest_status {
            existing @ SendExecutionStatus::ReconciliationRequired { .. } => existing.clone(),
            _ => SendExecutionStatus::ReconciliationRequired { failure },
        };
        let mut mutations = Vec::with_capacity(2);
        if operation_needs_update {
            mutations.push(LocalStateMutation::OperationRecord(
                OperationRecordMutation {
                    kind: OperationKind::Send,
                    operation_id: operation_id.to_string(),
                    receipt: receipt_record(
                        &record.receipt,
                        &record.principal_mac,
                        &record.binding_hmac,
                    ),
                    latest_status: status_record(&reconciliation),
                    expected: RevisionGuard::Expected(record.revision),
                    revision: record
                        .revision
                        .next()
                        .ok_or(SendAgentMessageError::CapacityExceeded)?,
                },
            ));
        }
        if obligation_needs_update {
            let pending = obligation
                .pending
                .as_ref()
                .map(|pending| PendingIndexEntry {
                    ordered_key: pending.ordered_key.clone(),
                    owner: pending.owner.clone(),
                    partition: pending.partition,
                    shutdown_plan: pending.shutdown_plan.clone(),
                })
                .ok_or_else(|| internal_error("turn-reconciliation-pending-index"))?;
            let reconciliation_record = send_obligation_record_with_state(
                &obligation.record,
                ObligationStateRecord::ReconciliationRequired,
            )
            .ok_or_else(|| internal_error("turn-reconciliation-obligation-wrapper"))?;
            mutations.push(LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: obligation_id.to_string(),
                record: reconciliation_record,
                pending: Some(pending),
                expected: RevisionGuard::Expected(obligation.revision),
                revision: obligation
                    .revision
                    .next()
                    .ok_or(SendAgentMessageError::CapacityExceeded)?,
            }));
        }

        let mut payload_material = status_identity_material(&reconciliation);
        payload_material.extend_from_slice(operation_id.as_bytes());
        payload_material.extend_from_slice(obligation_id.as_bytes());
        payload_material.extend_from_slice(&record.revision.value().to_be_bytes());
        payload_material.extend_from_slice(&obligation.revision.value().to_be_bytes());
        let idempotency_key = format!(
            "{operation_id}.{obligation_id}.reconciliation.{}.{}",
            record.revision.value(),
            obligation.revision.value()
        );
        let commit_digest = self
            .authority
            .digest(format!("send-turn-reconciliation/v1\0{idempotency_key}").as_bytes());
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex_encode(&commit_digest))
                .map_err(|_| internal_error("turn-reconciliation-commit-id"))?,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::OperationProgress,
                idempotency_key,
                payload_hash: self.authority.digest(&payload_material),
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: mutations,
        };
        match self.repository.commit_batch(batch).await {
            Ok(_) => Ok(()),
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                Err(SendAgentMessageError::Internal { correlation_id })
            }
            Err(_) => {
                if self
                    .turn_reconciliation_is_converged(operation_id, obligation_id)
                    .await?
                {
                    Ok(())
                } else {
                    Err(internal_error("turn-reconciliation-commit"))
                }
            }
        }
    }

    async fn turn_reconciliation_is_converged(
        &self,
        operation_id: &str,
        obligation_id: &str,
    ) -> Result<bool, SendAgentMessageError> {
        let operation = self
            .lookup_record(operation_id)
            .await
            .map_err(|_| internal_error("turn-reconciliation-readback-operation"))?
            .ok_or(SendAgentMessageError::NotFound)?;
        if matches!(
            operation.latest_status,
            SendExecutionStatus::Terminal { .. }
        ) {
            return Ok(true);
        }
        if !matches!(
            operation.latest_status,
            SendExecutionStatus::ReconciliationRequired { .. }
        ) {
            return Ok(false);
        }
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.to_string(),
            })
            .await
            .map_err(|_| internal_error("turn-reconciliation-readback-obligation"))?;
        let LocalEventQueryResult::ObligationByIdentity(Some(obligation)) = result else {
            return Ok(false);
        };
        let Some(value) = SendObligationData::decode(&obligation.record) else {
            return Ok(false);
        };
        Ok(value.obligation_id == obligation_id
            && value.operation_id == operation_id
            && value.kind == SendObligationKindRecord::TurnExecution
            && value.state == ObligationStateRecord::ReconciliationRequired)
    }

    async fn ensure_turn_execution_dependencies_satisfied(
        &self,
        obligation: &SendObligationData,
    ) -> Result<(), SendAgentMessageError> {
        for dependency_id in &obligation.dependency_obligation_ids {
            let dependency = self
                .repository
                .query(LocalEventQuery::ObligationByIdentity {
                    obligation_id: dependency_id.to_string(),
                })
                .await
                .map_err(|_| internal_error("obligation-dependency-lookup"))?;
            let LocalEventQueryResult::ObligationByIdentity(Some(dependency)) = dependency else {
                return Err(internal_error("obligation-dependency-missing"));
            };
            let dependency = SendObligationData::decode(&dependency.record)
                .ok_or_else(|| internal_error("obligation-dependency-decode"))?;
            let dependency_satisfied = dependency.obligation_id == *dependency_id
                && dependency.operation_id == obligation.operation_id
                && dependency.kind == SendObligationKindRecord::ProviderEstablish
                && matches!(
                    dependency.state,
                    ObligationStateRecord::Completed | ObligationStateRecord::Cancelled
                );
            if !dependency_satisfied {
                return Err(internal_error("obligation-dependency-incomplete"));
            }
        }
        Ok(())
    }

    fn exact_turn_claim_commit_identity(
        &self,
        operation_id: &str,
        obligation_id: &str,
        owner_identity: &str,
    ) -> Result<(CommitIdentity, [u8; 32]), SendAgentMessageError> {
        if owner_identity.is_empty() {
            return Err(internal_error("turn-claim-owner-identity"));
        }
        let digest = self.authority.digest(
            format!(
                "send-turn-execution-claim/v1\0{operation_id}\0{obligation_id}\0{owner_identity}"
            )
            .as_bytes(),
        );
        let commit_id = CommitIdentity::parse(&hex_encode(&digest))
            .map_err(|_| internal_error("turn-claim-commit-identity"))?;
        Ok((commit_id, digest))
    }

    pub(crate) async fn claim_turn_execution(
        &self,
        operation_id: &str,
        obligation_id: &str,
        owner_identity: &str,
    ) -> Result<ObligationTransitionOutcome, SendAgentMessageError> {
        self.transition_obligation_inner(
            ObligationTransition {
                operation_id,
                obligation_id,
                expected_kind: "turn_execution",
                expected_state: "pending",
                next_state: "effect_reserved",
                keep_pending: true,
                status: Some(SendExecutionStatus::ProviderStartReserved {
                    obligation_id: obligation_id.to_string(),
                }),
            },
            Some(owner_identity),
        )
        .await
    }

    pub(crate) async fn transition_obligation(
        &self,
        transition: ObligationTransition<'_>,
    ) -> Result<ObligationTransitionOutcome, SendAgentMessageError> {
        self.transition_obligation_inner(transition, None).await
    }

    async fn transition_obligation_inner(
        &self,
        transition: ObligationTransition<'_>,
        exact_claim_owner: Option<&str>,
    ) -> Result<ObligationTransitionOutcome, SendAgentMessageError> {
        let ObligationTransition {
            operation_id,
            obligation_id,
            expected_kind,
            expected_state,
            next_state,
            keep_pending,
            status,
        } = transition;
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.to_string(),
            })
            .await
            .map_err(|_| internal_error("obligation-transition-lookup"))?;
        let LocalEventQueryResult::ObligationByIdentity(obligation) = result else {
            return Err(internal_error("obligation-transition-shape"));
        };
        let obligation = obligation.ok_or(SendAgentMessageError::NotFound)?;
        let value = SendObligationData::decode(&obligation.record)
            .ok_or_else(|| internal_error("obligation-transition-decode"))?;
        let expected_kind = send_obligation_kind(expected_kind)
            .ok_or_else(|| internal_error("obligation-transition-kind"))?;
        let expected_state = obligation_state(expected_state)
            .ok_or_else(|| internal_error("obligation-transition-expected-state"))?;
        let next_state = obligation_state(next_state)
            .ok_or_else(|| internal_error("obligation-transition-next-state"))?;
        let exact_claim_commit = exact_claim_owner
            .map(|owner| self.exact_turn_claim_commit_identity(operation_id, obligation_id, owner))
            .transpose()?;
        if value.operation_id != operation_id || value.kind != expected_kind {
            return Err(internal_error("obligation-transition-owner"));
        }
        if value.state == next_state {
            if let Some((commit_id, _)) = exact_claim_commit.as_ref() {
                return self
                    .repository
                    .resolve_commit(commit_id.clone())
                    .await
                    .map_err(|_| internal_error("turn-claim-commit-resolution"))
                    .map(|resolution| match resolution {
                        CommitResolution::Committed(_) => ObligationTransitionOutcome::Applied,
                        CommitResolution::NotCommitted => {
                            ObligationTransitionOutcome::AlreadyAtTarget
                        }
                    });
            }
            return Ok(ObligationTransitionOutcome::AlreadyAtTarget);
        }
        if value.state != expected_state {
            return Err(internal_error("obligation-transition-state"));
        }
        if expected_kind == SendObligationKindRecord::TurnExecution
            && expected_state == ObligationStateRecord::Pending
        {
            self.ensure_turn_execution_dependencies_satisfied(&value)
                .await?;
        }
        let transitioned_record = send_obligation_record_with_state(&obligation.record, next_state)
            .ok_or_else(|| internal_error("obligation-transition-wrapper"))?;
        let next_obligation_revision = obligation
            .revision
            .next()
            .ok_or(SendAgentMessageError::CapacityExceeded)?;
        let pending = keep_pending
            .then(|| {
                obligation
                    .pending
                    .map(|pending| PendingIndexEntry {
                        ordered_key: pending.ordered_key,
                        owner: pending.owner,
                        partition: pending.partition,
                        shutdown_plan: pending.shutdown_plan,
                    })
                    .ok_or_else(|| internal_error("obligation-pending-index"))
            })
            .transpose()?;
        let mut mutations = vec![LocalStateMutation::Obligation(ObligationMutation {
            obligation_id: obligation.obligation_id,
            record: transitioned_record,
            pending,
            expected: RevisionGuard::Expected(obligation.revision),
            revision: next_obligation_revision,
        })];
        let mut status_payload = vec![next_state as u8];
        if let Some(status) = status {
            let record = self
                .lookup_record(operation_id)
                .await
                .map_err(|_| internal_error("obligation-operation-lookup"))?
                .ok_or(SendAgentMessageError::NotFound)?;
            let next_revision = record
                .revision
                .next()
                .ok_or(SendAgentMessageError::CapacityExceeded)?;
            status_payload.extend_from_slice(&status_identity_material(&status));
            mutations.push(LocalStateMutation::OperationRecord(
                OperationRecordMutation {
                    kind: OperationKind::Send,
                    operation_id: operation_id.to_string(),
                    receipt: receipt_record(
                        &record.receipt,
                        &record.principal_mac,
                        &record.binding_hmac,
                    ),
                    latest_status: status_record(&status),
                    expected: RevisionGuard::Expected(record.revision),
                    revision: next_revision,
                },
            ));
        }
        let (commit_id, idempotency_key) =
            if let Some((commit_id, digest)) = exact_claim_commit.as_ref() {
                (
                    commit_id.clone(),
                    format!("send-turn-claim-v1:{}", hex_encode(digest)),
                )
            } else {
                let digest = self.authority.digest(
                    format!(
                        "send-obligation\0{operation_id}\0{obligation_id}\0{}",
                        next_obligation_revision.value()
                    )
                    .as_bytes(),
                );
                (
                    CommitIdentity::parse(&hex_encode(&digest))
                        .map_err(|_| internal_error("obligation-transition-commit"))?,
                    format!(
                        "{operation_id}.{obligation_id}.{}",
                        next_obligation_revision.value()
                    ),
                )
            };
        let batch = LocalAtomicBatch {
            commit_id,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::OperationProgress,
                idempotency_key,
                payload_hash: self.authority.digest(&status_payload),
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: mutations,
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_)) => Ok(ObligationTransitionOutcome::Applied),
            Ok(CommitBatchResult::Replayed(_)) if exact_claim_owner.is_some() => {
                Ok(ObligationTransitionOutcome::Applied)
            }
            Ok(CommitBatchResult::Replayed(_)) => Ok(ObligationTransitionOutcome::AlreadyAtTarget),
            Err(_) => Err(internal_error("obligation-transition-commit")),
        }
    }

    /// Freeze the operation-side participants for one atomic accepted queue
    /// start. This method only prepares guarded mutations; the session owner
    /// commits them in the same batch as the exact canonical queue dequeue and
    /// `TurnStarted` event.
    pub(crate) async fn prepare_queued_turn_start_participant_mutations(
        &self,
        operation_id: &str,
        obligation_id: &str,
        session_id: &str,
        queue_item_id: &str,
        event: &AgentSessionDomainEvent,
    ) -> Result<Vec<LocalStateMutation>, SendAgentMessageError> {
        let AgentSessionDomainEvent::TurnStarted {
            turn_id,
            message_id,
            assistant_message_id,
            ..
        } = event
        else {
            return Err(internal_error("queued-start-event-kind"));
        };
        let operation = self
            .lookup_record(operation_id)
            .await
            .map_err(|_| internal_error("queued-start-operation-lookup"))?
            .ok_or(SendAgentMessageError::NotFound)?;
        let (
            SendDisposition::Queued {
                queue_item_id: receipt_queue_item_id,
            },
            SendExecutionStatus::Queued {
                queue_item_id: status_queue_item_id,
                reserved_turn_id,
            },
        ) = (&operation.receipt.disposition, &operation.latest_status)
        else {
            return Err(internal_error("queued-start-operation-state"));
        };
        if operation.receipt.operation_id != operation_id
            || operation.receipt.session_id != session_id
            || receipt_queue_item_id != queue_item_id
            || status_queue_item_id != queue_item_id
            || reserved_turn_id != &turn_id.to_string()
        {
            return Err(internal_error("queued-start-operation-owner"));
        }

        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.to_string(),
            })
            .await
            .map_err(|_| internal_error("queued-start-obligation-lookup"))?;
        let LocalEventQueryResult::ObligationByIdentity(Some(obligation)) = result else {
            return Err(internal_error("queued-start-obligation-shape"));
        };
        if has_recovery_wrapper(&obligation.record) {
            return Err(internal_error("queued-start-obligation-recovery-owner"));
        }
        let value = SendObligationData::decode(&obligation.record)
            .ok_or_else(|| internal_error("queued-start-obligation-decode"))?;
        if value.obligation_id != obligation_id
            || value.operation_id != operation_id
            || value.session_id != session_id
            || value.kind != SendObligationKindRecord::TurnExecution
            || value.disposition != SendObligationDispositionRecord::Queued
            || value.state != ObligationStateRecord::Pending
            || value.human_message_id.as_deref() != Some(message_id.as_str())
            || value.assistant_message_id.as_deref() != assistant_message_id.as_deref()
            || value.reserved_turn_id.as_deref() != Some(reserved_turn_id.as_str())
            || value.turn_id.as_deref() != Some(reserved_turn_id.as_str())
        {
            return Err(internal_error("queued-start-obligation-owner"));
        }
        self.ensure_turn_execution_dependencies_satisfied(&value)
            .await?;
        let pending = obligation
            .pending
            .filter(|pending| {
                pending.owner == session_id
                    && pending.partition == PendingPartition::Owner
                    && pending.shutdown_plan.is_none()
            })
            .map(|pending| PendingIndexEntry {
                ordered_key: pending.ordered_key,
                owner: pending.owner,
                partition: pending.partition,
                shutdown_plan: pending.shutdown_plan,
            })
            .ok_or_else(|| internal_error("queued-start-pending-index"))?;
        let reserved_obligation = send_obligation_record_with_state(
            &obligation.record,
            ObligationStateRecord::EffectReserved,
        )
        .ok_or_else(|| internal_error("queued-start-obligation-wrapper"))?;
        let operation_revision = operation
            .revision
            .next()
            .ok_or(SendAgentMessageError::CapacityExceeded)?;
        let obligation_revision = obligation
            .revision
            .next()
            .ok_or(SendAgentMessageError::CapacityExceeded)?;

        Ok(vec![
            LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: obligation_id.to_string(),
                record: reserved_obligation,
                pending: Some(pending),
                expected: RevisionGuard::Expected(obligation.revision),
                revision: obligation_revision,
            }),
            LocalStateMutation::OperationRecord(OperationRecordMutation {
                kind: OperationKind::Send,
                operation_id: operation_id.to_string(),
                receipt: receipt_record(
                    &operation.receipt,
                    &operation.principal_mac,
                    &operation.binding_hmac,
                ),
                latest_status: status_record(&SendExecutionStatus::ProviderStartReserved {
                    obligation_id: obligation_id.to_string(),
                }),
                expected: RevisionGuard::Expected(operation.revision),
                revision: operation_revision,
            }),
        ])
    }

    /// Mark a reserved turn as running without completing its obligation.
    /// The pending obligation remains the terminal participant locator until
    /// the same terminal transaction writes the final send status.
    pub(crate) async fn mark_turn_running(
        &self,
        operation_id: &str,
        obligation_id: &str,
        turn_id: u64,
    ) -> Result<(), SendAgentMessageError> {
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.to_string(),
            })
            .await
            .map_err(|_| internal_error("running-obligation-lookup"))?;
        let LocalEventQueryResult::ObligationByIdentity(obligation) = result else {
            return Err(internal_error("running-obligation-shape"));
        };
        let obligation = obligation.ok_or(SendAgentMessageError::NotFound)?;
        let value = SendObligationData::decode(&obligation.record)
            .ok_or_else(|| internal_error("running-obligation-decode"))?;
        if value.operation_id != operation_id
            || value.kind != SendObligationKindRecord::TurnExecution
        {
            return Err(internal_error("running-obligation-owner"));
        }
        let record = self
            .lookup_record(operation_id)
            .await
            .map_err(|_| internal_error("running-operation-lookup"))?
            .ok_or(SendAgentMessageError::NotFound)?;
        let running = SendExecutionStatus::Running {
            turn_id: turn_id.to_string(),
        };
        if record.latest_status == running {
            return Ok(());
        }
        if matches!(record.latest_status, SendExecutionStatus::Terminal { .. }) {
            return Ok(());
        }
        if value.state != ObligationStateRecord::EffectReserved {
            return Err(internal_error("running-obligation-state"));
        }
        let next_operation_revision = record
            .revision
            .next()
            .ok_or(SendAgentMessageError::CapacityExceeded)?;
        let next_obligation_revision = obligation
            .revision
            .next()
            .ok_or(SendAgentMessageError::CapacityExceeded)?;
        let pending = obligation
            .pending
            .map(|pending| PendingIndexEntry {
                ordered_key: pending.ordered_key,
                owner: pending.owner,
                partition: pending.partition,
                shutdown_plan: pending.shutdown_plan,
            })
            .ok_or_else(|| internal_error("running-pending-index"))?;
        let status_material = status_identity_material(&running);
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex_encode(
                &self.authority.digest(
                    format!(
                        "send-running/v1\0{operation_id}\0{obligation_id}\0{}",
                        next_obligation_revision.value()
                    )
                    .as_bytes(),
                ),
            ))
            .map_err(|_| internal_error("running-commit-id"))?,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::OperationProgress,
                idempotency_key: format!(
                    "{operation_id}.{obligation_id}.running.{}",
                    next_obligation_revision.value()
                ),
                payload_hash: self.authority.digest(&status_material),
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![
                LocalStateMutation::Obligation(ObligationMutation {
                    obligation_id: obligation.obligation_id,
                    record: obligation.record,
                    pending: Some(pending),
                    expected: RevisionGuard::Expected(obligation.revision),
                    revision: next_obligation_revision,
                }),
                LocalStateMutation::OperationRecord(OperationRecordMutation {
                    kind: OperationKind::Send,
                    operation_id: operation_id.to_string(),
                    receipt: receipt_record(
                        &record.receipt,
                        &record.principal_mac,
                        &record.binding_hmac,
                    ),
                    latest_status: status_record(&running),
                    expected: RevisionGuard::Expected(record.revision),
                    revision: next_operation_revision,
                }),
            ],
        };
        match self.repository.commit_batch(batch).await {
            Ok(_) => Ok(()),
            Err(
                CommitBatchError::StreamHeadConflict { .. } | CommitBatchError::PayloadConflict,
            ) => match self.lookup_record(operation_id).await {
                Ok(Some(record))
                    if matches!(record.latest_status, SendExecutionStatus::Terminal { .. }) =>
                {
                    Ok(())
                }
                _ => Err(internal_error("running-conflict")),
            },
            Err(_) => Err(internal_error("running-commit")),
        }
    }
}

#[async_trait::async_trait]
impl SendRecoveryReadbackPort for AgentSendOperationUsecase {
    async fn read_send(
        &self,
        request: &SendRecoveryReadbackRequest,
    ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
        use crate::domain::agent_session::events::RecoveryResultClassification;

        let expected_effect_identity = match request.kind {
            SendRecoveryReadbackKind::TurnExecution => format!("{}.exec", request.operation_id),
        };
        if request.effect_identity.as_str() != expected_effect_identity {
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::InvalidEffectIntent,
                false,
                "The send readback identity does not match its durable effect.",
                format!("send-readback-identity-{}", uuid_like()),
            ));
        }
        let record = self
            .lookup_record(&request.operation_id)
            .await
            .map_err(|_| {
                SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "The send operation could not be read.",
                    format!("send-readback-operation-{}", uuid_like()),
                )
            })?
            .ok_or_else(|| {
                SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "The send operation is unavailable for readback.",
                    format!("send-readback-missing-{}", uuid_like()),
                )
            })?;
        if record.receipt.operation_id != request.operation_id
            || record.receipt.session_id != request.session_id
        {
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::InvalidEffectIntent,
                false,
                "The send operation no longer matches the accepted effect.",
                format!("send-readback-binding-{}", uuid_like()),
            ));
        }
        let terminal_participants = if request.kind == SendRecoveryReadbackKind::TurnExecution
            && !matches!(record.latest_status, SendExecutionStatus::Terminal { .. })
        {
            let obligation = self
                .repository
                .query(LocalEventQuery::ObligationByIdentity {
                    obligation_id: request.effect_identity.as_str().to_string(),
                })
                .await
                .map_err(|_| {
                    SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageUnavailable,
                        true,
                        "The send effect obligation could not be read.",
                        format!("send-readback-obligation-{}", uuid_like()),
                    )
                })?;
            let LocalEventQueryResult::ObligationByIdentity(Some(obligation)) = obligation else {
                return Err(SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "The send effect obligation is unavailable for readback.",
                    format!("send-readback-obligation-missing-{}", uuid_like()),
                ));
            };
            let obligation = SendObligationData::decode(&obligation.record).ok_or_else(|| {
                SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "The send effect obligation is incompatible with readback.",
                    format!("send-readback-obligation-shape-{}", uuid_like()),
                )
            })?;
            if obligation.obligation_id != request.effect_identity.as_str()
                || obligation.operation_id != request.operation_id
                || obligation.session_id != request.session_id
                || obligation.kind != SendObligationKindRecord::TurnExecution
            {
                return Err(SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "The send effect obligation no longer matches the accepted effect.",
                    format!("send-readback-obligation-binding-{}", uuid_like()),
                ));
            }
            let turn_id = obligation
                .turn_id
                .or(obligation.reserved_turn_id)
                .or_else(|| match &record.receipt.disposition {
                    SendDisposition::StartedTurn { turn_id } => Some(turn_id.clone()),
                    SendDisposition::Queued { .. } => None,
                });
            if let Some(turn_id) = turn_id {
                let terminal = self
                    .repository
                    .query(LocalEventQuery::TerminalByTurn {
                        session_id: request.session_id.clone(),
                        turn_id,
                    })
                    .await
                    .map_err(|_| {
                        SafeOperationFailure::new(
                            SessionOperationFailureKind::StorageUnavailable,
                            true,
                            "The send terminal winner could not be read.",
                            format!("send-readback-terminal-{}", uuid_like()),
                        )
                    })?;
                match terminal {
                    LocalEventQueryResult::TerminalByTurn(Some(terminal)) => {
                        let terminal = TerminalRecordMutation {
                            session_id: terminal.session_id,
                            turn_id: terminal.turn_id,
                            terminal_identity: terminal.terminal_identity,
                            result: terminal.result,
                            participant_digest: terminal.participant_digest,
                        };
                        Some(
                            self.prepare_runtime_terminal_participants(&terminal)
                                .await
                                .map_err(|_| {
                                    SafeOperationFailure::new(
                                        SessionOperationFailureKind::StorageUnavailable,
                                        true,
                                        "The send terminal participants could not be prepared.",
                                        format!("send-readback-participants-{}", uuid_like()),
                                    )
                                })?,
                        )
                    }
                    LocalEventQueryResult::TerminalByTurn(None) => None,
                    _ => {
                        return Err(SafeOperationFailure::new(
                            SessionOperationFailureKind::StorageUnavailable,
                            true,
                            "The send terminal readback returned an incompatible result.",
                            format!("send-readback-terminal-shape-{}", uuid_like()),
                        ))
                    }
                }
            } else {
                None
            }
        } else {
            None
        };
        let (classification, safe_result, owner_mutations) =
            if let Some(participants) = terminal_participants {
                (
                    RecoveryResultClassification::Succeeded,
                    "The accepted send reached its durable terminal result.".to_string(),
                    participants.mutations,
                )
            } else {
                let (classification, safe_result) = match record.latest_status {
                    SendExecutionStatus::AwaitingProviderStart { .. }
                    | SendExecutionStatus::Queued { .. }
                    | SendExecutionStatus::ProviderStartReserved { .. }
                    | SendExecutionStatus::Running { .. } => (
                        RecoveryResultClassification::Pending,
                        "The accepted send effect remains pending.".to_string(),
                    ),
                    SendExecutionStatus::Terminal { .. } => (
                        RecoveryResultClassification::Succeeded,
                        "The accepted send reached its durable terminal result.".to_string(),
                    ),
                    SendExecutionStatus::ReconciliationRequired { .. }
                    | SendExecutionStatus::Failed { .. } => (
                        RecoveryResultClassification::Ambiguous,
                        "The accepted send still requires effect reconciliation.".to_string(),
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
