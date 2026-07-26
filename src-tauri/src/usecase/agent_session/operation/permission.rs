//! Durable permission-response operation.
//!
//! The exact validated provider payload, caller binding, public operation
//! record, and recovery obligation are accepted in one local-event batch.
//! A second claim batch is the ambiguity fence: only the call that freshly
//! commits that claim may hand the immutable effect to the provider. A saved
//! claim is never replayed automatically after reply loss or restart.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::domain::agent_session::entities::{PermissionResponse, PermissionResponseDecision};
use crate::domain::agent_session::events::{
    AgentSessionDomainEvent, ObligationKind, ObligationState, PermissionDecision,
};
use crate::domain::local_event::mutation::RECOVERY_RESULT_MAX_BYTES;
use crate::domain::local_event::{
    CallerOperationKey, CommitBatchError, CommitBatchResult, CommitIdentity, CommitOperationKind,
    ExpectedStreamHead, IdempotencyBinding, LoadStreamRequest, LocalAtomicBatch, LocalDomainEvent,
    LocalEventQuery, LocalEventQueryError, LocalEventQueryResult, LocalEventTransactionRepository,
    LocalStateMutation, ObligationMutation, ObligationRecord, ObligationStateRecord,
    OperationBindingMutation, OperationKind, OperationReceiptRecord, OperationRecordMutation,
    OperationStatusRecord, OperationStatusValue, PendingIndexEntry, PendingPartition,
    PermissionDecisionRecord, RecordAuthentication, Revision, RevisionGuard, SafeOperationFailure,
    SessionOperationFailureKind, StreamId, StreamVersion, UncommittedDomainEvent,
};

use super::identity::{constant_time_eq_32, validate_operation_identity};
use super::ports::{
    AcceptedPermissionResponseEffect, OperationBindingAuthority, PermissionResponseGate,
    PermissionResponsePlan,
};
use super::record::hex_encode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionResponseDecisionKind {
    Allowed,
    Denied,
}

impl PermissionResponseDecisionKind {
    fn label(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionResponseOperationReceipt {
    pub operation_id: String,
    pub session_id: String,
    pub request_id: String,
    /// Opaque reference to the owner-private exact response payload.
    pub input_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionResponseExecutionStatus {
    AwaitingProviderResponse {
        obligation_id: String,
    },
    ReconciliationRequired {
        failure: SafeOperationFailure,
    },
    Failed {
        failure: SafeOperationFailure,
    },
    Completed {
        decision: PermissionResponseDecisionKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedPermissionResponseOperation {
    pub receipt: PermissionResponseOperationReceipt,
    pub latest_status: PermissionResponseExecutionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionResponseCommandOutcome {
    Accepted(AcceptedPermissionResponseOperation),
    RejectedBeforeCommit { failure: SafeOperationFailure },
    OutcomeUnknown { operation_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionResponseOperationError {
    InvalidRequest,
    PayloadConflict,
    ShutdownInProgress,
    NotFound,
    CapacityExceeded,
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetPermissionResponseOperationError {
    InvalidRequest,
    NotFound,
    QueryBusy,
    DeadlineExceeded,
    StorageUnavailable { failure: SafeOperationFailure },
    Internal { correlation_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionResponseOperationRequest {
    pub principal: String,
    pub operation_id: String,
    pub session_id: String,
    pub response: PermissionResponse,
}

#[derive(Debug, Clone)]
struct StoredPermissionResponseOperation {
    receipt: PermissionResponseOperationReceipt,
    latest_status: PermissionResponseExecutionStatus,
    principal_mac: [u8; 32],
    binding_hmac: [u8; 32],
    revision: Revision,
}

#[derive(Debug, Clone)]
struct StoredPermissionObligation {
    operation_id: String,
    plan: PermissionResponsePlan,
    state: ObligationStateRecord,
}

pub struct PermissionResponseOperationUsecase {
    repository: Arc<dyn LocalEventTransactionRepository>,
    authority: Arc<dyn OperationBindingAuthority>,
    gate: Arc<dyn PermissionResponseGate>,
    installation_id: String,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

fn correlation(context: &str) -> String {
    format!("permission-{context}-{:x}", now_ms())
}

fn internal(context: &str) -> PermissionResponseOperationError {
    PermissionResponseOperationError::Internal {
        correlation_id: correlation(context),
    }
}

fn storage_failure(context: &str) -> SafeOperationFailure {
    SafeOperationFailure::new(
        SessionOperationFailureKind::StorageUnavailable,
        true,
        "The permission response could not be saved or read.",
        correlation(context),
    )
}

fn reconciliation_failure(context: &str) -> SafeOperationFailure {
    SafeOperationFailure::new(
        SessionOperationFailureKind::OutcomeUnknown,
        false,
        "The provider permission response requires reconciliation.",
        correlation(context),
    )
    .with_detail("The saved effect identity must be resolved before another response is sent.")
}

fn decision_kind(response: &PermissionResponse) -> PermissionResponseDecisionKind {
    match response.decision {
        PermissionResponseDecision::Allow { .. } => PermissionResponseDecisionKind::Allowed,
        PermissionResponseDecision::Deny { .. } => PermissionResponseDecisionKind::Denied,
    }
}

fn exact_response_value(
    response: &PermissionResponse,
) -> Result<Value, PermissionResponseOperationError> {
    let decision = match &response.decision {
        PermissionResponseDecision::Allow {
            updated_input,
            answers,
        } => serde_json::json!({
            "type": "allow",
            "updated_input": updated_input
                .as_ref()
                .map(|value| serde_json::from_str::<Value>(value.as_str()))
                .transpose()
                .map_err(|_| PermissionResponseOperationError::InvalidRequest)?,
            "answers": answers
                .as_ref()
                .map(|value| serde_json::from_str::<Value>(value.as_str()))
                .transpose()
                .map_err(|_| PermissionResponseOperationError::InvalidRequest)?,
        }),
        PermissionResponseDecision::Deny { message } => serde_json::json!({
            "type": "deny",
            "message": message,
        }),
    };
    Ok(serde_json::json!({
        "request_id": response.request_id,
        "decision": decision,
    }))
}

pub(super) fn canonical_payload(
    session_id: &str,
    response: &PermissionResponse,
) -> Result<String, PermissionResponseOperationError> {
    let exact_response = exact_response_value(response)?;
    let value = serde_json::json!({
        "schema": "permission_response_command_v1",
        "session_id": session_id,
        "exact_response": exact_response,
    });
    let encoded = value.to_string();
    if session_id.is_empty()
        || response.request_id.is_empty()
        || encoded.len() > RECOVERY_RESULT_MAX_BYTES
    {
        return Err(PermissionResponseOperationError::InvalidRequest);
    }
    Ok(encoded)
}

fn receipt_record(
    receipt: &PermissionResponseOperationReceipt,
    principal_mac: &[u8; 32],
    binding_hmac: &[u8; 32],
) -> OperationReceiptRecord {
    OperationReceiptRecord::PermissionResponse {
        operation_id: receipt.operation_id.clone(),
        session_id: receipt.session_id.clone(),
        request_id: receipt.request_id.clone(),
        input_ref: receipt.input_ref.clone(),
        authentication: RecordAuthentication {
            principal_mac: *principal_mac,
            binding_hmac: *binding_hmac,
        },
    }
}

fn status_record(status: &PermissionResponseExecutionStatus) -> OperationStatusRecord {
    let value = match status {
        PermissionResponseExecutionStatus::AwaitingProviderResponse { obligation_id } => {
            OperationStatusValue::AwaitingProviderResponse {
                obligation_id: obligation_id.clone(),
            }
        }
        PermissionResponseExecutionStatus::ReconciliationRequired { failure } => {
            OperationStatusValue::ReconciliationRequired {
                failure: failure.clone(),
            }
        }
        PermissionResponseExecutionStatus::Failed { failure } => OperationStatusValue::Failed {
            failure: failure.clone(),
        },
        PermissionResponseExecutionStatus::Completed { decision } => {
            OperationStatusValue::PermissionCompleted {
                decision: match decision {
                    PermissionResponseDecisionKind::Allowed => PermissionDecisionRecord::Allowed,
                    PermissionResponseDecisionKind::Denied => PermissionDecisionRecord::Denied,
                },
            }
        }
    };
    OperationStatusRecord {
        kind: OperationKind::PermissionResponse,
        value,
    }
}

fn status_identity_material(status: &PermissionResponseExecutionStatus) -> Vec<u8> {
    match status {
        PermissionResponseExecutionStatus::AwaitingProviderResponse { obligation_id } => {
            format!("awaiting_provider_response\0{obligation_id}").into_bytes()
        }
        PermissionResponseExecutionStatus::ReconciliationRequired { failure } => {
            format!("reconciliation_required\0{}", failure.correlation_id).into_bytes()
        }
        PermissionResponseExecutionStatus::Failed { failure } => {
            format!("failed\0{}", failure.correlation_id).into_bytes()
        }
        PermissionResponseExecutionStatus::Completed { decision } => {
            format!("completed\0{}", decision.label()).into_bytes()
        }
    }
}

fn decode_record(
    receipt: OperationReceiptRecord,
    status: OperationStatusRecord,
    revision: Revision,
) -> Option<StoredPermissionResponseOperation> {
    let (operation_id, session_id, request_id, input_ref, authentication) = match receipt {
        OperationReceiptRecord::PermissionResponse {
            operation_id,
            session_id,
            request_id,
            input_ref,
            authentication,
        } => (
            operation_id,
            session_id,
            request_id,
            input_ref,
            authentication,
        ),
        OperationReceiptRecord::Send { .. }
        | OperationReceiptRecord::Stop { .. }
        | OperationReceiptRecord::SessionLifecycle { .. }
        | OperationReceiptRecord::ApplicationQuit { .. } => return None,
    };
    if status.kind != OperationKind::PermissionResponse {
        return None;
    }
    let latest_status = match status.value {
        OperationStatusValue::AwaitingProviderResponse { obligation_id } => {
            PermissionResponseExecutionStatus::AwaitingProviderResponse { obligation_id }
        }
        OperationStatusValue::ReconciliationRequired { failure } => {
            PermissionResponseExecutionStatus::ReconciliationRequired { failure }
        }
        OperationStatusValue::Failed { failure } => {
            PermissionResponseExecutionStatus::Failed { failure }
        }
        OperationStatusValue::PermissionCompleted { decision } => {
            PermissionResponseExecutionStatus::Completed {
                decision: match decision {
                    PermissionDecisionRecord::Allowed => PermissionResponseDecisionKind::Allowed,
                    PermissionDecisionRecord::Denied => PermissionResponseDecisionKind::Denied,
                },
            }
        }
        OperationStatusValue::Accepted
        | OperationStatusValue::AwaitingProviderStart { .. }
        | OperationStatusValue::Queued { .. }
        | OperationStatusValue::ProviderStartReserved { .. }
        | OperationStatusValue::Running { .. }
        | OperationStatusValue::Completed
        | OperationStatusValue::StopCompleted { .. }
        | OperationStatusValue::Preparing
        | OperationStatusValue::Activated
        | OperationStatusValue::ExitPending
        | OperationStatusValue::Exited
        | OperationStatusValue::OutcomeUnknown { .. }
        | OperationStatusValue::FailedBeforeActivation { .. }
        | OperationStatusValue::Terminal { .. } => return None,
    };
    Some(StoredPermissionResponseOperation {
        receipt: PermissionResponseOperationReceipt {
            operation_id,
            session_id,
            request_id,
            input_ref,
        },
        latest_status,
        principal_mac: authentication.principal_mac,
        binding_hmac: authentication.binding_hmac,
        revision,
    })
}

fn obligation_record(
    operation_id: &str,
    plan: &PermissionResponsePlan,
    state: ObligationStateRecord,
) -> ObligationRecord {
    ObligationRecord::PermissionResponse {
        operation_id: operation_id.to_string(),
        effect_identity: format!("permission-response:{operation_id}"),
        session_id: plan.session_id.clone(),
        turn_id: plan.turn_id.to_string(),
        response: plan.response.clone(),
        owner_access: true,
        from_runtime_state: plan.from_runtime_state,
        state,
    }
}

fn decode_obligation(record: &ObligationRecord) -> Option<StoredPermissionObligation> {
    let (
        operation_id,
        effect_identity,
        session_id,
        turn_id,
        response,
        owner_access,
        from_runtime_state,
        state,
    ) = match record {
        ObligationRecord::PermissionResponse {
            operation_id,
            effect_identity,
            session_id,
            turn_id,
            response,
            owner_access,
            from_runtime_state,
            state,
        } => (
            operation_id,
            effect_identity,
            session_id,
            turn_id,
            response,
            owner_access,
            from_runtime_state,
            state,
        ),
        ObligationRecord::RecoveryTransition { original, .. }
        | ObligationRecord::Observed { original, .. } => return decode_obligation(original),
        ObligationRecord::Send { .. }
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
    };
    if !owner_access || effect_identity != &format!("permission-response:{operation_id}") {
        return None;
    }
    Some(StoredPermissionObligation {
        operation_id: operation_id.clone(),
        plan: PermissionResponsePlan {
            session_id: session_id.clone(),
            request_id: response.request_id.clone(),
            turn_id: turn_id.parse().ok()?,
            response: response.clone(),
            from_runtime_state: *from_runtime_state,
        },
        state: *state,
    })
}

fn resolved_event(plan: &PermissionResponsePlan) -> AgentSessionDomainEvent {
    let (decision, answers) = match &plan.response.decision {
        PermissionResponseDecision::Allow { answers, .. } => {
            (PermissionDecision::Allowed, answers.clone())
        }
        PermissionResponseDecision::Deny { .. } => (PermissionDecision::Denied, None),
    };
    AgentSessionDomainEvent::PermissionResolved {
        turn_id: plan.turn_id,
        tool_use_id: None,
        request_id: Some(plan.request_id.clone()),
        decision,
        answers,
    }
}

impl PermissionResponseOperationUsecase {
    pub fn new(
        repository: Arc<dyn LocalEventTransactionRepository>,
        authority: Arc<dyn OperationBindingAuthority>,
        gate: Arc<dyn PermissionResponseGate>,
        installation_id: String,
    ) -> Self {
        Self {
            repository,
            authority,
            gate,
            installation_id,
        }
    }

    fn binding_material(
        &self,
        principal: &str,
        operation_id: &str,
        canonical_payload: &str,
    ) -> Vec<u8> {
        super::binding::permission_response(
            principal,
            &self.installation_id,
            operation_id,
            canonical_payload.as_bytes(),
        )
    }

    fn commit_identity(
        &self,
        stage: &str,
        operation_id: &str,
        revision: i64,
    ) -> Result<CommitIdentity, PermissionResponseOperationError> {
        let digest = self.authority.digest(
            format!("permission-response-commit\0{stage}\0{operation_id}\0{revision}").as_bytes(),
        );
        CommitIdentity::parse(&hex_encode(&digest)).map_err(|_| internal("commit-identity"))
    }

    async fn lookup_record(
        &self,
        operation_id: &str,
    ) -> Result<Option<StoredPermissionResponseOperation>, LocalEventQueryError> {
        let result = self
            .repository
            .query(LocalEventQuery::OperationByIdentity {
                kind: OperationKind::PermissionResponse,
                operation_id: operation_id.to_string(),
            })
            .await?;
        let LocalEventQueryResult::OperationByIdentity(record) = result else {
            return Err(LocalEventQueryError::Internal {
                correlation_id: correlation("lookup-shape"),
            });
        };
        record
            .map(|record| {
                decode_record(record.receipt, record.latest_status, record.revision).ok_or(
                    LocalEventQueryError::Corrupt {
                        correlation_id: correlation("lookup-decode"),
                    },
                )
            })
            .transpose()
    }

    async fn lookup_obligation(
        &self,
        obligation_id: &str,
    ) -> Result<
        Option<(
            StoredPermissionObligation,
            crate::domain::local_event::ObligationView,
        )>,
        LocalEventQueryError,
    > {
        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: obligation_id.to_string(),
            })
            .await?;
        let LocalEventQueryResult::ObligationByIdentity(obligation) = result else {
            return Err(LocalEventQueryError::Internal {
                correlation_id: correlation("obligation-shape"),
            });
        };
        obligation
            .map(|view| {
                let decoded =
                    decode_obligation(&view.record).ok_or(LocalEventQueryError::Corrupt {
                        correlation_id: correlation("obligation-decode"),
                    })?;
                Ok((decoded, view))
            })
            .transpose()
    }

    async fn current_stream_head(&self, stream_id: &StreamId) -> Result<i64, LocalEventQueryError> {
        self.repository
            .load_stream(LoadStreamRequest {
                stream_id: stream_id.clone(),
                after: None,
                limit: 1,
            })
            .await
            .map(|page| page.head.value())
    }

    async fn converge_or_reject(
        &self,
        operation_id: &str,
        principal_mac: &[u8; 32],
        binding_hmac: &[u8; 32],
        failure: SafeOperationFailure,
    ) -> Result<PermissionResponseCommandOutcome, PermissionResponseOperationError> {
        match self.lookup_record(operation_id).await {
            Ok(Some(record)) => {
                if !constant_time_eq_32(&record.principal_mac, principal_mac) {
                    return Err(PermissionResponseOperationError::NotFound);
                }
                if !constant_time_eq_32(&record.binding_hmac, binding_hmac) {
                    return Err(PermissionResponseOperationError::PayloadConflict);
                }
                Ok(PermissionResponseCommandOutcome::Accepted(
                    Self::public_operation(record),
                ))
            }
            Ok(None) => Ok(PermissionResponseCommandOutcome::RejectedBeforeCommit { failure }),
            Err(_) => Ok(PermissionResponseCommandOutcome::OutcomeUnknown {
                operation_id: operation_id.to_string(),
            }),
        }
    }

    fn public_operation(
        record: StoredPermissionResponseOperation,
    ) -> AcceptedPermissionResponseOperation {
        AcceptedPermissionResponseOperation {
            receipt: record.receipt,
            latest_status: record.latest_status,
        }
    }

    pub async fn get_operation(
        &self,
        principal: &str,
        operation_id: &str,
    ) -> Result<AcceptedPermissionResponseOperation, GetPermissionResponseOperationError> {
        if validate_operation_identity(operation_id).is_err() || principal.is_empty() {
            return Err(GetPermissionResponseOperationError::InvalidRequest);
        }
        let record = self
            .lookup_record(operation_id)
            .await
            .map_err(|error| match error {
                LocalEventQueryError::QueryBusy => GetPermissionResponseOperationError::QueryBusy,
                LocalEventQueryError::DeadlineExceeded => {
                    GetPermissionResponseOperationError::DeadlineExceeded
                }
                LocalEventQueryError::StorageUnavailable { failure } => {
                    GetPermissionResponseOperationError::StorageUnavailable { failure }
                }
                LocalEventQueryError::Corrupt { correlation_id }
                | LocalEventQueryError::Internal { correlation_id }
                | LocalEventQueryError::IncompatibleStoredEvent { correlation_id }
                | LocalEventQueryError::ReplayRequired { correlation_id } => {
                    GetPermissionResponseOperationError::Internal { correlation_id }
                }
                _ => GetPermissionResponseOperationError::Internal {
                    correlation_id: correlation("query"),
                },
            })?
            .ok_or(GetPermissionResponseOperationError::NotFound)?;
        let principal_mac = self
            .authority
            .mac(&super::send::principal_material(principal));
        if !constant_time_eq_32(&record.principal_mac, &principal_mac) {
            return Err(GetPermissionResponseOperationError::NotFound);
        }
        Ok(Self::public_operation(record))
    }

    /// Accept, claim, and execute one exact permission response. Same-identity
    /// replays converge before current admission/planning is consulted.
    pub async fn request(
        &self,
        request: PermissionResponseOperationRequest,
    ) -> Result<PermissionResponseCommandOutcome, PermissionResponseOperationError> {
        if validate_operation_identity(&request.operation_id).is_err()
            || request.principal.is_empty()
        {
            return Err(PermissionResponseOperationError::InvalidRequest);
        }
        let canonical_payload = canonical_payload(&request.session_id, &request.response)?;
        let principal_mac = self
            .authority
            .mac(&super::send::principal_material(&request.principal));
        let binding_material = self.binding_material(
            &request.principal,
            &request.operation_id,
            &canonical_payload,
        );
        let binding_hmac = self.authority.mac(&binding_material);

        match self.lookup_record(&request.operation_id).await {
            Ok(Some(record)) => {
                if !constant_time_eq_32(&record.principal_mac, &principal_mac) {
                    return Err(PermissionResponseOperationError::NotFound);
                }
                if !constant_time_eq_32(&record.binding_hmac, &binding_hmac) {
                    return Err(PermissionResponseOperationError::PayloadConflict);
                }
                let operation = self.drive_if_pending(record).await;
                return Ok(PermissionResponseCommandOutcome::Accepted(operation));
            }
            Ok(None) => {}
            Err(_) => {
                // Absence was not proven. The same identity may already have
                // committed before a lost reply, so it must never be replaced
                // by a fresh provider-effect identity.
                return Ok(PermissionResponseCommandOutcome::OutcomeUnknown {
                    operation_id: request.operation_id,
                });
            }
        }

        let plan = match self
            .gate
            .plan_response(&request.session_id, &request.response)
            .await
        {
            Ok(plan) => plan,
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
        if plan.session_id != request.session_id
            || plan.request_id != request.response.request_id
            || plan.response != request.response
        {
            return Err(internal("plan-binding"));
        }
        let stream_id =
            StreamId::agent_session(&plan.session_id).map_err(|_| internal("stream-id"))?;
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
                        storage_failure("stream-head"),
                    )
                    .await;
            }
        };
        let obligation_digest = self.authority.digest(
            format!(
                "permission-response-target\0{}\0{}\0{}",
                plan.session_id, plan.turn_id, plan.request_id
            )
            .as_bytes(),
        );
        let obligation_id = format!("permission-response-{}", hex_encode(&obligation_digest));
        let receipt = PermissionResponseOperationReceipt {
            operation_id: request.operation_id.clone(),
            session_id: plan.session_id.clone(),
            request_id: plan.request_id.clone(),
            input_ref: obligation_id.clone(),
        };
        let status = PermissionResponseExecutionStatus::AwaitingProviderResponse {
            obligation_id: obligation_id.clone(),
        };
        let at = now_ms();
        let events = vec![UncommittedDomainEvent {
            stream_id: stream_id.clone(),
            event: LocalDomainEvent::AgentSession(AgentSessionDomainEvent::ObligationRecorded {
                obligation_id: obligation_id.clone(),
                kind: ObligationKind::PermissionResponse,
                state: ObligationState::Pending,
                at: at as f64,
            }),
            occurred_at_ms: at,
        }];
        let mutations = vec![
            LocalStateMutation::OperationBinding(OperationBindingMutation {
                key: CallerOperationKey {
                    principal: request.principal.clone(),
                    installation_id: self.installation_id.clone(),
                    kind: OperationKind::PermissionResponse,
                    caller_request_id: request.operation_id.clone(),
                },
                operation_id: request.operation_id.clone(),
                binding_hmac,
            }),
            LocalStateMutation::OperationRecord(OperationRecordMutation {
                kind: OperationKind::PermissionResponse,
                operation_id: request.operation_id.clone(),
                receipt: receipt_record(&receipt, &principal_mac, &binding_hmac),
                latest_status: status_record(&status),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).map_err(|_| internal("revision"))?,
            }),
            LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: obligation_id.clone(),
                record: obligation_record(
                    &request.operation_id,
                    &plan,
                    ObligationStateRecord::Pending,
                ),
                pending: Some(PendingIndexEntry {
                    ordered_key: format!("permission-response:{at:020}:{obligation_id}"),
                    owner: plan.session_id.clone(),
                    partition: PendingPartition::Owner,
                    shutdown_plan: None,
                }),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).map_err(|_| internal("revision"))?,
            }),
        ];
        let batch = LocalAtomicBatch {
            commit_id: self.commit_identity("accept", &request.operation_id, 0)?,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: OperationKind::PermissionResponse.into(),
                idempotency_key: request.operation_id.clone(),
                payload_hash: self.authority.digest(&binding_material),
            },
            expected_heads: vec![ExpectedStreamHead {
                stream_id,
                expected: StreamVersion::new(head).map_err(|_| internal("stream-version"))?,
            }],
            events,
            state_mutations: mutations,
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_)) => {
                let saved = match self.lookup_record(&request.operation_id).await {
                    Ok(Some(saved))
                        if constant_time_eq_32(&saved.principal_mac, &principal_mac)
                            && constant_time_eq_32(&saved.binding_hmac, &binding_hmac) =>
                    {
                        saved
                    }
                    _ => {
                        return Ok(PermissionResponseCommandOutcome::Accepted(
                            AcceptedPermissionResponseOperation {
                                receipt,
                                latest_status:
                                    PermissionResponseExecutionStatus::ReconciliationRequired {
                                        failure: storage_failure("acceptance-readback"),
                                    },
                            },
                        ));
                    }
                };
                let obligation_matches = self
                    .lookup_obligation(&obligation_id)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|(obligation, _)| {
                        obligation.operation_id == request.operation_id
                            && obligation.state == ObligationStateRecord::Pending
                            && obligation.plan.response == plan.response
                    });
                if !obligation_matches {
                    return Ok(PermissionResponseCommandOutcome::Accepted(
                        AcceptedPermissionResponseOperation {
                            receipt: saved.receipt,
                            latest_status:
                                PermissionResponseExecutionStatus::ReconciliationRequired {
                                    failure: storage_failure("obligation-readback"),
                                },
                        },
                    ));
                }
                Ok(PermissionResponseCommandOutcome::Accepted(
                    self.drive_if_pending(saved).await,
                ))
            }
            Ok(CommitBatchResult::Replayed(_)) => {
                match self.lookup_record(&request.operation_id).await {
                    Ok(Some(saved)) => Ok(PermissionResponseCommandOutcome::Accepted(
                        Self::public_operation(saved),
                    )),
                    _ => Ok(PermissionResponseCommandOutcome::OutcomeUnknown {
                        operation_id: request.operation_id,
                    }),
                }
            }
            Err(CommitBatchError::PayloadConflict | CommitBatchError::EffectAdmissionBlocked) => {
                match self.lookup_record(&request.operation_id).await {
                    Ok(Some(saved)) => {
                        if !constant_time_eq_32(&saved.principal_mac, &principal_mac) {
                            Err(PermissionResponseOperationError::NotFound)
                        } else if !constant_time_eq_32(&saved.binding_hmac, &binding_hmac) {
                            Err(PermissionResponseOperationError::PayloadConflict)
                        } else {
                            Ok(PermissionResponseCommandOutcome::Accepted(
                                Self::public_operation(saved),
                            ))
                        }
                    }
                    Ok(None) => Err(PermissionResponseOperationError::PayloadConflict),
                    Err(_) => Ok(PermissionResponseCommandOutcome::OutcomeUnknown {
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
                        "The session changed while accepting the permission response.",
                        correlation("acceptance-conflict"),
                    ),
                )
                .await
            }
            Err(CommitBatchError::CapacityExceeded | CommitBatchError::SequenceExhausted) => {
                Err(PermissionResponseOperationError::CapacityExceeded)
            }
            Err(CommitBatchError::StorageUnavailable { failure })
                if failure.is_shutdown_in_progress() =>
            {
                Err(PermissionResponseOperationError::ShutdownInProgress)
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
            Err(CommitBatchError::OutcomeUnknown { .. }) => {
                Ok(PermissionResponseCommandOutcome::OutcomeUnknown {
                    operation_id: request.operation_id,
                })
            }
            Err(CommitBatchError::Corrupt { correlation_id }) => {
                Err(PermissionResponseOperationError::Internal { correlation_id })
            }
        }
    }

    /// Resume only a still-pending accepted operation. This is the sole seam
    /// used by startup/manual recovery; claimed work is observation-only.
    pub async fn resume_operation(
        &self,
        operation_id: &str,
    ) -> Result<AcceptedPermissionResponseOperation, PermissionResponseOperationError> {
        if validate_operation_identity(operation_id).is_err() {
            return Err(PermissionResponseOperationError::InvalidRequest);
        }
        let record = self
            .lookup_record(operation_id)
            .await
            .map_err(|_| internal("resume-lookup"))?
            .ok_or(PermissionResponseOperationError::NotFound)?;
        Ok(self.drive_if_pending(record).await)
    }

    #[cfg(test)]
    pub async fn recover_pending_permission_responses(
        &self,
    ) -> Result<(), PermissionResponseOperationError> {
        loop {
            if self.recover_pending_permission_responses_pass().await? == 0 {
                return Ok(());
            }
        }
    }

    pub(crate) async fn recover_pending_permission_responses_pass(
        &self,
    ) -> Result<usize, PermissionResponseOperationError> {
        let mut cursor = None;
        let mut discovered = 0usize;
        loop {
            let result = self
                .repository
                .query(LocalEventQuery::PendingRecoveryPage {
                    limit: 200,
                    partition: None,
                    owner: None,
                    ordered_key_prefix: Some("permission-response:".to_string()),
                    shutdown_plan: None,
                    cursor,
                })
                .await
                .map_err(|_| internal("recovery-page"))?;
            let LocalEventQueryResult::PendingRecoveryPage(page) = result else {
                return Err(internal("recovery-page-shape"));
            };
            for entry in page.entries {
                // The ordered-key filter is the permission-response namespace.
                // Any row it returns is recovery work; an undecodable row must
                // keep the supervisor alive instead of looking like an empty
                // pass and being silently abandoned.
                discovered = discovered.saturating_add(1);
                let obligation = decode_obligation(&entry.record)
                    .ok_or_else(|| internal("recovery-obligation-decode"))?;
                let record = self
                    .lookup_record(&obligation.operation_id)
                    .await
                    .map_err(|_| internal("recovery-operation"))?
                    .ok_or_else(|| internal("recovery-operation-missing"))?;
                if record.receipt.operation_id != obligation.operation_id {
                    return Err(internal("recovery-operation-reference"));
                }
                if obligation.state != ObligationStateRecord::Pending {
                    continue;
                }
                if matches!(
                    &record.latest_status,
                    PermissionResponseExecutionStatus::AwaitingProviderResponse { .. }
                ) {
                    let _ = self.drive_if_pending(record).await;
                    let still_pending = self
                        .lookup_record(&obligation.operation_id)
                        .await
                        .map_err(|_| internal("recovery-drive-readback"))?
                        .is_some_and(|saved| {
                            matches!(
                                saved.latest_status,
                                PermissionResponseExecutionStatus::AwaitingProviderResponse { .. }
                            )
                        });
                    if still_pending {
                        return Err(internal("recovery-drive"));
                    }
                }
            }
            let Some(next) = page.next_cursor else {
                break;
            };
            cursor = Some(next);
        }
        Ok(discovered)
    }

    /// Test/diagnostic seam: recovery always uses the indexed namespace and
    /// never a mixed unfiltered first page.
    #[cfg(test)]
    pub(crate) fn recovery_ordered_key_prefix() -> &'static str {
        "permission-response:"
    }

    async fn drive_if_pending(
        &self,
        record: StoredPermissionResponseOperation,
    ) -> AcceptedPermissionResponseOperation {
        let PermissionResponseExecutionStatus::AwaitingProviderResponse { obligation_id } =
            &record.latest_status
        else {
            return Self::public_operation(record);
        };
        let obligation_id = obligation_id.clone();
        let (obligation, view) = match self.lookup_obligation(&obligation_id).await {
            Ok(Some(value)) => value,
            _ => {
                return AcceptedPermissionResponseOperation {
                    receipt: record.receipt,
                    latest_status: PermissionResponseExecutionStatus::Failed {
                        failure: storage_failure("pending-obligation"),
                    },
                };
            }
        };
        if obligation.operation_id != record.receipt.operation_id
            || obligation.plan.session_id != record.receipt.session_id
            || obligation.plan.request_id != record.receipt.request_id
        {
            return AcceptedPermissionResponseOperation {
                receipt: record.receipt,
                latest_status: PermissionResponseExecutionStatus::Failed {
                    failure: SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageCorrupt,
                        false,
                        "The permission response binding is incompatible.",
                        correlation("obligation-binding"),
                    ),
                },
            };
        }
        match obligation.state {
            ObligationStateRecord::Pending => {
                self.claim_and_execute(record, obligation, view).await
            }
            ObligationStateRecord::EffectReserved
            | ObligationStateRecord::ReconciliationRequired => {
                AcceptedPermissionResponseOperation {
                    receipt: record.receipt,
                    latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                        failure: reconciliation_failure("claimed"),
                    },
                }
            }
            ObligationStateRecord::Completed => Self::public_operation(record),
            ObligationStateRecord::Prepared
            | ObligationStateRecord::Running
            | ObligationStateRecord::WaitingApproval
            | ObligationStateRecord::OutcomeUnknown
            | ObligationStateRecord::Failed
            | ObligationStateRecord::Cancelled => AcceptedPermissionResponseOperation {
                receipt: record.receipt,
                latest_status: PermissionResponseExecutionStatus::Failed {
                    failure: storage_failure("obligation-state"),
                },
            },
        }
    }

    async fn claim_and_execute(
        &self,
        record: StoredPermissionResponseOperation,
        obligation: StoredPermissionObligation,
        view: crate::domain::local_event::ObligationView,
    ) -> AcceptedPermissionResponseOperation {
        let operation_revision = match record.revision.next() {
            Some(revision) => revision,
            None => {
                return AcceptedPermissionResponseOperation {
                    receipt: record.receipt,
                    latest_status: PermissionResponseExecutionStatus::Failed {
                        failure: storage_failure("operation-revision"),
                    },
                };
            }
        };
        let obligation_revision = match view.revision.next() {
            Some(revision) => revision,
            None => {
                return AcceptedPermissionResponseOperation {
                    receipt: record.receipt,
                    latest_status: PermissionResponseExecutionStatus::Failed {
                        failure: storage_failure("obligation-revision"),
                    },
                };
            }
        };
        let claim_status = PermissionResponseExecutionStatus::ReconciliationRequired {
            failure: reconciliation_failure("effect-reserved"),
        };
        let pending = match view.pending.as_ref() {
            Some(pending) => Some(PendingIndexEntry {
                ordered_key: pending.ordered_key.clone(),
                owner: pending.owner.clone(),
                partition: pending.partition,
                shutdown_plan: pending.shutdown_plan.clone(),
            }),
            None => {
                return AcceptedPermissionResponseOperation {
                    receipt: record.receipt,
                    latest_status: PermissionResponseExecutionStatus::Failed {
                        failure: storage_failure("pending-index"),
                    },
                };
            }
        };
        let batch = LocalAtomicBatch {
            commit_id: match self.commit_identity(
                "claim",
                &record.receipt.operation_id,
                operation_revision.value(),
            ) {
                Ok(identity) => identity,
                Err(_) => {
                    return AcceptedPermissionResponseOperation {
                        receipt: record.receipt,
                        latest_status: PermissionResponseExecutionStatus::Failed {
                            failure: storage_failure("claim-identity"),
                        },
                    };
                }
            },
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::OperationProgress,
                idempotency_key: format!("{}.claim", record.receipt.operation_id),
                payload_hash: self.authority.digest(
                    format!(
                        "{}\0{}\0{}",
                        record.receipt.operation_id,
                        view.obligation_id,
                        view.revision.value()
                    )
                    .as_bytes(),
                ),
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![
                LocalStateMutation::OperationRecord(OperationRecordMutation {
                    kind: OperationKind::PermissionResponse,
                    operation_id: record.receipt.operation_id.clone(),
                    receipt: receipt_record(
                        &record.receipt,
                        &record.principal_mac,
                        &record.binding_hmac,
                    ),
                    latest_status: status_record(&claim_status),
                    expected: RevisionGuard::Expected(record.revision),
                    revision: operation_revision,
                }),
                LocalStateMutation::Obligation(ObligationMutation {
                    obligation_id: view.obligation_id.clone(),
                    record: obligation_record(
                        &obligation.operation_id,
                        &obligation.plan,
                        ObligationStateRecord::EffectReserved,
                    ),
                    pending,
                    expected: RevisionGuard::Expected(view.revision),
                    revision: obligation_revision,
                }),
            ],
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_)) => {}
            Ok(CommitBatchResult::Replayed(_))
            | Err(CommitBatchError::OutcomeUnknown { .. })
            | Err(CommitBatchError::StreamHeadConflict { .. })
            | Err(CommitBatchError::PayloadConflict)
            | Err(CommitBatchError::EffectAdmissionBlocked) => {
                return AcceptedPermissionResponseOperation {
                    receipt: record.receipt,
                    latest_status: claim_status,
                };
            }
            Err(_) => {
                return AcceptedPermissionResponseOperation {
                    receipt: record.receipt,
                    latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                        failure: storage_failure("claim"),
                    },
                };
            }
        }
        let claimed = self
            .lookup_obligation(&view.obligation_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|(saved, saved_view)| {
                saved.operation_id == obligation.operation_id
                    && saved.state == ObligationStateRecord::EffectReserved
                    && saved.plan.response == obligation.plan.response
                    && saved_view.revision == obligation_revision
            });
        if !claimed {
            return AcceptedPermissionResponseOperation {
                receipt: record.receipt,
                latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                    failure: storage_failure("claim-readback"),
                },
            };
        }
        let effect = AcceptedPermissionResponseEffect {
            operation_id: record.receipt.operation_id.clone(),
            obligation_id: view.obligation_id,
            plan: obligation.plan,
        };
        if let Err(failure) = self.gate.execute(&effect).await {
            let persisted = self
                .record_reconciliation(&record, &effect, failure.clone())
                .await;
            return AcceptedPermissionResponseOperation {
                receipt: record.receipt,
                // `claim_status` was confirmed by the readback above. Never
                // return a more specific provider failure unless that exact
                // failure was also confirmed in the canonical record.
                latest_status: if persisted {
                    PermissionResponseExecutionStatus::ReconciliationRequired { failure }
                } else {
                    claim_status
                },
            };
        }
        self.complete_after_provider(record, effect).await
    }

    async fn record_reconciliation(
        &self,
        original: &StoredPermissionResponseOperation,
        effect: &AcceptedPermissionResponseEffect,
        failure: SafeOperationFailure,
    ) -> bool {
        let Ok(Some(current)) = self.lookup_record(&effect.operation_id).await else {
            return false;
        };
        let Ok(Some((obligation, view))) = self.lookup_obligation(&effect.obligation_id).await
        else {
            return false;
        };
        if obligation.state != ObligationStateRecord::EffectReserved {
            return false;
        }
        let Some(operation_revision) = current.revision.next() else {
            return false;
        };
        let Some(obligation_revision) = view.revision.next() else {
            return false;
        };
        let status = PermissionResponseExecutionStatus::ReconciliationRequired { failure };
        let pending = view.pending.as_ref().map(|pending| PendingIndexEntry {
            ordered_key: pending.ordered_key.clone(),
            owner: pending.owner.clone(),
            partition: pending.partition,
            shutdown_plan: pending.shutdown_plan.clone(),
        });
        let Ok(commit_id) = self.commit_identity(
            "reconcile",
            &effect.operation_id,
            operation_revision.value(),
        ) else {
            return false;
        };
        let batch = LocalAtomicBatch {
            commit_id,
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::OperationProgress,
                idempotency_key: format!("{}.reconcile", effect.operation_id),
                payload_hash: self.authority.digest(&status_identity_material(&status)),
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![
                LocalStateMutation::OperationRecord(OperationRecordMutation {
                    kind: OperationKind::PermissionResponse,
                    operation_id: effect.operation_id.clone(),
                    receipt: receipt_record(
                        &original.receipt,
                        &original.principal_mac,
                        &original.binding_hmac,
                    ),
                    latest_status: status_record(&status),
                    expected: RevisionGuard::Expected(current.revision),
                    revision: operation_revision,
                }),
                LocalStateMutation::Obligation(ObligationMutation {
                    obligation_id: effect.obligation_id.clone(),
                    record: obligation_record(
                        &obligation.operation_id,
                        &obligation.plan,
                        ObligationStateRecord::ReconciliationRequired,
                    ),
                    pending,
                    expected: RevisionGuard::Expected(view.revision),
                    revision: obligation_revision,
                }),
            ],
        };
        // A writer result (including Replayed or OutcomeUnknown) is not proof
        // that this exact public failure became canonical. Confirm by identity
        // before returning it to the caller; otherwise the already-confirmed
        // effect-reserved status remains the public result.
        let write_may_have_committed = matches!(
            self.repository.commit_batch(batch).await,
            Ok(CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_))
                | Err(CommitBatchError::OutcomeUnknown { .. }
                    | CommitBatchError::StreamHeadConflict { .. }
                    | CommitBatchError::PayloadConflict)
        );
        if !write_may_have_committed {
            return false;
        }
        self.lookup_record(&effect.operation_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|saved| saved.latest_status == status)
    }

    async fn complete_after_provider(
        &self,
        original: StoredPermissionResponseOperation,
        effect: AcceptedPermissionResponseEffect,
    ) -> AcceptedPermissionResponseOperation {
        let current = match self.lookup_record(&effect.operation_id).await {
            Ok(Some(current)) => current,
            _ => {
                return AcceptedPermissionResponseOperation {
                    receipt: original.receipt,
                    latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                        failure: storage_failure("completion-operation-read"),
                    },
                };
            }
        };
        let (obligation, view) = match self.lookup_obligation(&effect.obligation_id).await {
            Ok(Some(value)) => value,
            _ => {
                return AcceptedPermissionResponseOperation {
                    receipt: original.receipt,
                    latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                        failure: storage_failure("completion-obligation-read"),
                    },
                };
            }
        };
        if obligation.state == ObligationStateRecord::Completed {
            return Self::public_operation(current);
        }
        if obligation.state != ObligationStateRecord::EffectReserved {
            return AcceptedPermissionResponseOperation {
                receipt: original.receipt,
                latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                    failure: reconciliation_failure("completion-state"),
                },
            };
        }
        let stream_id = match StreamId::agent_session(&effect.plan.session_id) {
            Ok(stream_id) => stream_id,
            Err(_) => {
                return AcceptedPermissionResponseOperation {
                    receipt: original.receipt,
                    latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                        failure: storage_failure("completion-stream"),
                    },
                };
            }
        };
        let head = match self.current_stream_head(&stream_id).await {
            Ok(head) => head,
            Err(_) => {
                return AcceptedPermissionResponseOperation {
                    receipt: original.receipt,
                    latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                        failure: storage_failure("completion-head"),
                    },
                };
            }
        };
        let at = now_ms();
        let event_values = vec![
            resolved_event(&effect.plan),
            AgentSessionDomainEvent::ObligationRecorded {
                obligation_id: effect.obligation_id.clone(),
                kind: ObligationKind::PermissionResponse,
                state: ObligationState::Completed,
                at: at as f64,
            },
        ];
        let mut state_mutations = match self
            .gate
            .completion_state_mutations(&effect, &event_values)
            .await
        {
            Ok(mutations) => mutations,
            Err(_) => {
                return AcceptedPermissionResponseOperation {
                    receipt: original.receipt,
                    latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                        failure: storage_failure("completion-participants"),
                    },
                };
            }
        };
        let Some(operation_revision) = current.revision.next() else {
            return AcceptedPermissionResponseOperation {
                receipt: original.receipt,
                latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                    failure: storage_failure("completion-operation-revision"),
                },
            };
        };
        let Some(obligation_revision) = view.revision.next() else {
            return AcceptedPermissionResponseOperation {
                receipt: original.receipt,
                latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                    failure: storage_failure("completion-obligation-revision"),
                },
            };
        };
        let status = PermissionResponseExecutionStatus::Completed {
            decision: decision_kind(&effect.plan.response),
        };
        state_mutations.extend([
            LocalStateMutation::OperationRecord(OperationRecordMutation {
                kind: OperationKind::PermissionResponse,
                operation_id: effect.operation_id.clone(),
                receipt: receipt_record(
                    &original.receipt,
                    &original.principal_mac,
                    &original.binding_hmac,
                ),
                latest_status: status_record(&status),
                expected: RevisionGuard::Expected(current.revision),
                revision: operation_revision,
            }),
            LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: effect.obligation_id.clone(),
                record: obligation_record(
                    &obligation.operation_id,
                    &obligation.plan,
                    ObligationStateRecord::Completed,
                ),
                pending: None,
                expected: RevisionGuard::Expected(view.revision),
                revision: obligation_revision,
            }),
        ]);
        let events = event_values
            .into_iter()
            .map(|event| UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: LocalDomainEvent::AgentSession(event),
                occurred_at_ms: at,
            })
            .collect();
        let batch = LocalAtomicBatch {
            commit_id: match self.commit_identity(
                "complete",
                &effect.operation_id,
                operation_revision.value(),
            ) {
                Ok(identity) => identity,
                Err(_) => {
                    return AcceptedPermissionResponseOperation {
                        receipt: original.receipt,
                        latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                            failure: storage_failure("completion-identity"),
                        },
                    };
                }
            },
            idempotency: IdempotencyBinding {
                installation_id: self.installation_id.clone(),
                operation_kind: CommitOperationKind::OperationProgress,
                idempotency_key: format!("{}.complete", effect.operation_id),
                payload_hash: self.authority.digest(&status_identity_material(&status)),
            },
            expected_heads: vec![ExpectedStreamHead {
                stream_id,
                expected: StreamVersion::new(head).expect("nonnegative stream head"),
            }],
            events,
            state_mutations,
        };
        match self.repository.commit_batch(batch).await {
            Ok(CommitBatchResult::Committed(_)) | Ok(CommitBatchResult::Replayed(_)) => {
                let saved = self
                    .lookup_record(&effect.operation_id)
                    .await
                    .ok()
                    .flatten();
                if let Some(saved) = saved {
                    if matches!(
                        saved.latest_status,
                        PermissionResponseExecutionStatus::Completed { .. }
                    ) {
                        self.gate.after_completion(&effect).await;
                        return Self::public_operation(saved);
                    }
                }
                AcceptedPermissionResponseOperation {
                    receipt: original.receipt,
                    latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                        failure: storage_failure("completion-readback"),
                    },
                }
            }
            Err(_) => AcceptedPermissionResponseOperation {
                receipt: original.receipt,
                latest_status: PermissionResponseExecutionStatus::ReconciliationRequired {
                    failure: reconciliation_failure("completion-commit"),
                },
            },
        }
    }
}
