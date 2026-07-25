//! Normal send acceptance and operation identity contract (R-001..R-005).
//!
//! One `commit_batch` fixes the operation binding, the human-input fact, the
//! turn-or-queue disposition, and the provider obligations. The immutable
//! Accepted receipt is returned only after the commit is confirmed; provider
//! effects start only afterwards, keyed by obligation identity.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::agent_session::events::{
    AgentSessionDomainEvent, ObligationKind, ObligationState, SendDisposition,
};
use crate::domain::local_event::{
    AgentTurnTerminalResultRecord, CallerAttemptResolution, CallerOperationKey, CommitBatchError,
    CommitBatchResult, CommitIdentity, CommitOperationKind, ExpectedStreamHead, IdempotencyBinding,
    LoadStreamRequest, LocalAtomicBatch, LocalDomainEvent, LocalEventQuery, LocalEventQueryError,
    LocalEventQueryResult, LocalEventTransactionRepository, LocalStateMutation, ObligationMutation,
    ObligationRecord, ObligationStateRecord, OperationBindingMutation, OperationKind,
    OperationReceiptRecord, OperationRecordMutation, OperationStatusRecord, OperationStatusValue,
    PendingIndexEntry, PendingPartition, RecordAuthentication, Revision, RevisionGuard,
    SafeOperationFailure, SendObligationDispositionRecord, SendObligationKindRecord,
    SessionOperationFailureKind, StreamId, StreamVersion, TerminalRecordMutation,
    TerminalResultRecord, UncommittedDomainEvent,
};

use super::identity::{constant_time_eq_32, validate_operation_identity};
use super::ports::{
    AcceptedSendEffect, OperationBindingAuthority, RecoveryEffectResult, SendAdmissionGate,
    SendRecoveryReadbackKind, SendRecoveryReadbackPort, SendRecoveryReadbackRequest,
    TerminalParticipants,
};
use super::record::hex_encode;

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

#[derive(Debug, Clone, PartialEq)]
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

pub struct AgentSendOperationUsecase {
    repository: Arc<dyn LocalEventTransactionRepository>,
    authority: Arc<dyn OperationBindingAuthority>,
    gate: Arc<dyn SendAdmissionGate>,
    installation_id: String,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
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

    fn into_record(self) -> ObligationRecord {
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
            state: self.state,
        }
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
        }
    }

    fn commit_identity(&self, operation_id: &str) -> Result<CommitIdentity, SendAgentMessageError> {
        let digest = self
            .authority
            .digest(format!("send-commit\0{operation_id}").as_bytes());
        CommitIdentity::parse(&hex_encode(&digest)).map_err(|_| internal_error("commit-id"))
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
            .plan_send(&request.principal, &request.canonical_payload)
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
        let establish_obligation_id =
            (!plan.provider_established).then(|| format!("{}.establish", request.operation_id));
        let execution_obligation_id = format!("{}.exec", request.operation_id);
        let terminal_turn_id = match &plan.disposition {
            SendDisposition::StartedTurn { turn_id } => turn_id.clone(),
            SendDisposition::Queued { .. } => plan
                .reserved_turn_id
                .clone()
                .ok_or_else(|| internal_error("queued-turn-identity"))?,
        };
        let latest_status = SendExecutionStatus::AwaitingProviderStart {
            dependency_obligation_ids: establish_obligation_id.iter().cloned().collect(),
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
        let mut push_obligation = |obligation_id: &str,
                                   kind: ObligationKind|
         -> Result<(), SendAgentMessageError> {
            let record_kind = match kind {
                ObligationKind::ProviderEstablish => SendObligationKindRecord::ProviderEstablish,
                ObligationKind::TurnExecution => SendObligationKindRecord::TurnExecution,
                ObligationKind::PermissionResponse
                | ObligationKind::ProviderInterrupt
                | ObligationKind::SessionClose
                | ObligationKind::QueuePause => return Err(internal_error("obligation-kind")),
            };
            let record = ObligationRecord::Send {
                obligation_id: obligation_id.to_string(),
                operation_id: request.operation_id.clone(),
                session_id: plan.session_id.clone(),
                kind: record_kind,
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
                dependency_obligation_ids: if matches!(kind, ObligationKind::TurnExecution) {
                    establish_obligation_id.iter().cloned().collect()
                } else {
                    Vec::new()
                },
                canonical_payload: request.canonical_payload.clone(),
                state: ObligationStateRecord::Pending,
            };
            events.push(UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: LocalDomainEvent::AgentSession(
                    AgentSessionDomainEvent::ObligationRecorded {
                        obligation_id: obligation_id.to_string(),
                        kind,
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
        if let Some(establish) = &establish_obligation_id {
            push_obligation(establish, ObligationKind::ProviderEstablish)?;
        }
        push_obligation(&execution_obligation_id, ObligationKind::TurnExecution)?;

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
                // readable operation record remains accepted but starts no
                // external effect until recovery can reconcile it.
                let saved = match self.lookup_record(&request.operation_id).await {
                    Ok(Some(saved))
                        if constant_time_eq_32(&saved.principal_mac, &principal_mac)
                            && constant_time_eq_32(&saved.binding_hmac, &binding_hmac) => saved,
                    _ => {
                        return Ok(SendCommandOutcome::Accepted(AcceptedSendOperation {
                            receipt,
                            latest_status: SendExecutionStatus::ReconciliationRequired {
                                failure: storage_failure("acceptance-readback"),
                            },
                        }));
                    }
                };
                self.gate
                    .start_provider_effect(&AcceptedSendEffect {
                        operation_id: request.operation_id.clone(),
                        session_id: plan.session_id.clone(),
                        human_message_id: plan.human_message_id.clone(),
                        assistant_message_id: assistant_message_id.clone(),
                        disposition: plan.disposition.clone(),
                        reserved_turn_id: plan.reserved_turn_id.clone(),
                        establish_obligation_id,
                        execution_obligation_id,
                        canonical_payload: request.canonical_payload.clone(),
                    })
                    .await;
                Ok(SendCommandOutcome::Accepted(AcceptedSendOperation {
                    receipt: saved.receipt,
                    latest_status: saved.latest_status,
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
            Err(CommitBatchError::PayloadConflict) => {
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
                        SessionOperationFailureKind::PersistFailure,
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
    ) -> Result<(), SendAgentMessageError> {
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
            self.record_execution_status(
                &record.receipt.operation_id,
                SendExecutionStatus::ReconciliationRequired {
                    failure: SafeOperationFailure::new(
                        SessionOperationFailureKind::InvalidEffectIntent,
                        false,
                        "The accepted queued send intent is incompatible.",
                        format!("send-queued-intent-restart-{}", uuid_like()),
                    ),
                },
            )
            .await?;
            return Ok(());
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
        self.gate
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
                establish_obligation_id: None,
                execution_obligation_id: execution_obligation_id.to_string(),
                canonical_payload: obligation.canonical_payload.clone(),
            })
            .await;
        Ok(())
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
        let mut discovered = 0usize;
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
                discovered = discovered.saturating_add(1);
                let operation_id = obligation.operation_id.as_str();
                let record = self
                    .lookup_record(operation_id)
                    .await
                    .map_err(|_| internal_error("recovery-operation"))?
                    .ok_or_else(|| internal_error("recovery-operation-missing"))?;
                if record.receipt.operation_id != operation_id {
                    return Err(internal_error("recovery-operation-reference"));
                }
                if obligation.kind == SendObligationKindRecord::ProviderEstablish {
                    continue;
                }
                match &record.latest_status {
                    SendExecutionStatus::AwaitingProviderStart {
                        dependency_obligation_ids,
                    } if matches!(&record.receipt.disposition, SendDisposition::Queued { .. }) => {
                        let queue_item_id = match &record.receipt.disposition {
                            SendDisposition::Queued { queue_item_id } => queue_item_id,
                            SendDisposition::StartedTurn { .. } => unreachable!(),
                        };
                        let reserved_turn_id =
                            obligation.reserved_turn_id.as_deref().unwrap_or_default();
                        self.restore_queued_effect_after_restart(
                            &entry.obligation_id,
                            &obligation,
                            &record,
                            queue_item_id,
                            reserved_turn_id,
                            Some(dependency_obligation_ids),
                        )
                        .await?;
                        continue;
                    }
                    SendExecutionStatus::Queued {
                        queue_item_id,
                        reserved_turn_id,
                    } => {
                        self.restore_queued_effect_after_restart(
                            &entry.obligation_id,
                            &obligation,
                            &record,
                            queue_item_id,
                            reserved_turn_id,
                            None,
                        )
                        .await?;
                        continue;
                    }
                    _ => {}
                }
                match record.latest_status {
                    SendExecutionStatus::AwaitingProviderStart {
                        dependency_obligation_ids,
                    } => {
                        let mut establish_obligation_id = None;
                        let mut requires_reconciliation = dependency_obligation_ids.len() > 1;
                        if let [dependency_id] = dependency_obligation_ids.as_slice() {
                            let dependency = self
                                .repository
                                .query(LocalEventQuery::ObligationByIdentity {
                                    obligation_id: dependency_id.clone(),
                                })
                                .await
                                .map_err(|_| internal_error("recovery-dependency"))?;
                            let LocalEventQueryResult::ObligationByIdentity(dependency) =
                                dependency
                            else {
                                return Err(internal_error("recovery-dependency-shape"));
                            };
                            let dependency_state = dependency
                                .as_ref()
                                .and_then(|dependency| {
                                    SendObligationData::decode(&dependency.record)
                                })
                                .filter(|dependency| {
                                    dependency.obligation_id == *dependency_id
                                        && dependency.operation_id == operation_id
                                        && dependency.kind
                                            == SendObligationKindRecord::ProviderEstablish
                                })
                                .map(|dependency| dependency.state);
                            match dependency_state {
                                // The worker is allowed to claim this exact pending
                                // establishment before making provider I/O.
                                Some(ObligationStateRecord::Pending) => {
                                    establish_obligation_id = Some(dependency_id.clone());
                                }
                                // Establishment is durably complete. Resume at the
                                // dependent turn reservation without trying to claim
                                // or repeat provider establishment.
                                Some(ObligationStateRecord::Completed) => {}
                                // A claimed establishment has an ambiguous provider
                                // outcome after restart. It can never be replayed
                                // blindly, even while the send itself still says it
                                // is awaiting provider start.
                                Some(
                                    ObligationStateRecord::EffectReserved
                                    | ObligationStateRecord::ReconciliationRequired,
                                )
                                | None => {
                                    requires_reconciliation = true;
                                }
                                Some(_) => {
                                    requires_reconciliation = true;
                                }
                            }
                        }
                        if requires_reconciliation {
                            self.record_execution_status(
                                operation_id,
                                SendExecutionStatus::ReconciliationRequired {
                                    failure: SafeOperationFailure::new(
                                        SessionOperationFailureKind::OutcomeUnknown,
                                        true,
                                        "Provider establishment requires same-effect readback after restart.",
                                        format!("send-establish-restart-{}", uuid_like()),
                                    ),
                                },
                            )
                            .await?;
                            continue;
                        }
                        let valid_human_message_id = obligation
                            .human_message_id
                            .as_deref()
                            .filter(|message_id| !message_id.is_empty());
                        let Some(human_message_id) = valid_human_message_id.filter(|_| {
                            !obligation.session_id.is_empty()
                                && !obligation.canonical_payload.is_empty()
                        }) else {
                            self.record_execution_status(
                                operation_id,
                                SendExecutionStatus::ReconciliationRequired {
                                    failure: SafeOperationFailure::new(
                                        SessionOperationFailureKind::InvalidEffectIntent,
                                        false,
                                        "The accepted provider effect intent is incompatible.",
                                        format!("send-intent-restart-{}", uuid_like()),
                                    ),
                                },
                            )
                            .await?;
                            continue;
                        };
                        // Claim this exact stable effect identity in the same
                        // durable authority before any provider handoff. A
                        // crash after this point is readback/reconciliation,
                        // never a blind second provider start.
                        self.record_execution_status(
                            operation_id,
                            SendExecutionStatus::ProviderStartReserved {
                                obligation_id: entry.obligation_id.clone(),
                            },
                        )
                        .await?;
                        self.gate
                            .start_provider_effect(&AcceptedSendEffect {
                                operation_id: operation_id.to_string(),
                                session_id: obligation.session_id.clone(),
                                human_message_id: human_message_id.to_string(),
                                assistant_message_id: obligation.assistant_message_id.clone(),
                                disposition: record.receipt.disposition,
                                reserved_turn_id: obligation.reserved_turn_id.clone(),
                                establish_obligation_id,
                                execution_obligation_id: entry.obligation_id,
                                canonical_payload: obligation.canonical_payload.clone(),
                            })
                            .await;
                    }
                    SendExecutionStatus::ProviderStartReserved { .. } => {
                        self.record_execution_status(
                            operation_id,
                            SendExecutionStatus::ReconciliationRequired {
                                failure: SafeOperationFailure::new(
                                    SessionOperationFailureKind::OutcomeUnknown,
                                    true,
                                    "The accepted provider effect requires readback after restart.",
                                    format!("send-restart-{}", uuid_like()),
                                ),
                            },
                        )
                        .await?;
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
        Ok(discovered)
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
                let Some(mut obligation) = SendObligationData::decode(&entry.record) else {
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
                    || obligation.state != ObligationStateRecord::EffectReserved
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
                obligation.state = ObligationStateRecord::Completed;
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
                        record: obligation.into_record(),
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
                let mut value = SendObligationData::decode(&obligation.record)
                    .ok_or_else(|| internal_error("status-obligation-decode"))?;
                if value.operation_id != operation_id
                    || value.state != ObligationStateRecord::Pending
                {
                    return Err(internal_error("status-obligation-invariant"));
                }
                value.state = ObligationStateRecord::EffectReserved;
                let pending = obligation.pending.map(|pending| PendingIndexEntry {
                    ordered_key: pending.ordered_key,
                    owner: pending.owner,
                    partition: pending.partition,
                    shutdown_plan: pending.shutdown_plan,
                });
                Some(ObligationMutation {
                    obligation_id: obligation.obligation_id,
                    record: value.into_record(),
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

    pub(crate) async fn transition_obligation(
        &self,
        transition: ObligationTransition<'_>,
    ) -> Result<(), SendAgentMessageError> {
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
        let mut value = SendObligationData::decode(&obligation.record)
            .ok_or_else(|| internal_error("obligation-transition-decode"))?;
        let expected_kind = send_obligation_kind(expected_kind)
            .ok_or_else(|| internal_error("obligation-transition-kind"))?;
        let expected_state = obligation_state(expected_state)
            .ok_or_else(|| internal_error("obligation-transition-expected-state"))?;
        let next_state = obligation_state(next_state)
            .ok_or_else(|| internal_error("obligation-transition-next-state"))?;
        if value.operation_id != operation_id || value.kind != expected_kind {
            return Err(internal_error("obligation-transition-owner"));
        }
        if value.state == next_state {
            return Ok(());
        }
        if value.state != expected_state {
            return Err(internal_error("obligation-transition-state"));
        }
        if expected_kind == SendObligationKindRecord::TurnExecution
            && expected_state == ObligationStateRecord::Pending
        {
            for dependency in &value.dependency_obligation_ids {
                let dependency = self
                    .repository
                    .query(LocalEventQuery::ObligationByIdentity {
                        obligation_id: dependency.to_string(),
                    })
                    .await
                    .map_err(|_| internal_error("obligation-dependency-lookup"))?;
                let LocalEventQueryResult::ObligationByIdentity(Some(dependency)) = dependency
                else {
                    return Err(internal_error("obligation-dependency-missing"));
                };
                let dependency = SendObligationData::decode(&dependency.record)
                    .ok_or_else(|| internal_error("obligation-dependency-decode"))?;
                if dependency.state != ObligationStateRecord::Completed {
                    return Err(internal_error("obligation-dependency-incomplete"));
                }
            }
        }
        value.state = next_state;
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
            record: value.into_record(),
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
        let digest = self.authority.digest(
            format!(
                "send-obligation\0{operation_id}\0{obligation_id}\0{}",
                next_obligation_revision.value()
            )
            .as_bytes(),
        );
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&hex_encode(&digest))
                .map_err(|_| internal_error("obligation-transition-commit"))?,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::OperationProgress,
                idempotency_key: format!(
                    "{operation_id}.{obligation_id}.{}",
                    next_obligation_revision.value()
                ),
                payload_hash: self.authority.digest(&status_payload),
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: mutations,
        };
        self.repository
            .commit_batch(batch)
            .await
            .map(|_| ())
            .map_err(|_| internal_error("obligation-transition-commit"))
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
        let running = SendExecutionStatus::Running {
            turn_id: turn_id.to_string(),
        };
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
