//! T-03 acceptance-contract tests over a fake in-memory store, a fake
//! binding authority, and recording gates (B-001..B-011, B-014, B-017,
//! B-053..B-056, B-095, B-099, B-101..B-103, identity bounds).

use std::collections::HashMap;
use std::hash::Hasher;
use std::sync::{Arc, Mutex, Weak};

use crate::domain::agent_session::entities::{
    InterruptReason as TurnInterruptReason, PermissionResponse, PermissionResponseDecision,
    TurnResult,
};
use crate::domain::agent_session::events::{
    AgentSessionDomainEvent, InterruptReason, ObligationState, RecoveryActionKind,
    RecoveryResultClassification, SendDisposition, StopResolution,
};
use crate::domain::local_event::{
    AgentTerminalKind, AgentTurnTerminalResultRecord, AuthoritativeEffectObservationRecord,
    CommitBatchError, CommitBatchResult, CommitIdentity, CommitResolution, CommittedBatch,
    CommittedStreamHead, DomainEventPage, GlobalSequence, IdempotencyBinding, LoadStreamRequest,
    LocalAtomicBatch, LocalDomainEvent, LocalEventQuery, LocalEventQueryError,
    LocalEventQueryResult, LocalEventSubscription, LocalEventTransactionRepository,
    LocalStateMutation, ObligationRecord, ObligationRecoveryActionRecord, ObligationStateRecord,
    OperationBindingView, OperationKind, OperationReceiptRecord, OperationRecordView,
    OperationStatusRecord, OperationStatusValue, RecordAuthentication, RecoveryAttemptRecord,
    RecoveryPublicationMessageKindRecord, RecoveryPublicationMessageRecord,
    RecoveryPublicationObligationRecord, RecoveryResultRecord, RevisionGuard, SafeOperationFailure,
    SendObligationDispositionRecord, SendObligationKindRecord, SessionLifecycleRecordAction,
    SessionOperationFailureKind, StopResolutionKind, StreamVersion, TerminalRecordMutation,
    TerminalResultRecord, UncommittedDomainEvent,
};

use super::caller_journal::{CallerAttemptJournal, CallerJournalError};
use super::lifecycle::{
    SessionLifecycleAction, SessionLifecycleCommandResult, SessionLifecycleOperationError,
    SessionLifecycleOperationState, SessionLifecycleOperationUsecase, SessionLifecycleRejection,
    SessionLifecycleRequest,
};
use super::permission::{
    PermissionResponseCommandOutcome, PermissionResponseExecutionStatus,
    PermissionResponseOperationError, PermissionResponseOperationRequest,
    PermissionResponseOperationUsecase,
};
use super::ports::{
    AcceptedPermissionResponseEffect, AcceptedSendEffect, AcceptedStopEffect,
    LegacyProviderEstablishRecovery, OperationBindingAuthority, PermissionResponseGate,
    PermissionResponsePlan, RecoveryEffectExecutor, RecoveryEffectHandoff, RecoveryEffectRequest,
    RecoveryEffectResult, SendAdmissionGate, SendEffectDispatch, SendPlan, SessionLifecycleEffect,
    SessionLifecycleGate, SessionLifecycleSnapshot, SessionLifecycleState, StopAdmissionGate,
    StopEffectObservation, StopTargetSnapshot,
};
use super::recovery::{
    PendingRecoveryCategory, PendingRecoveryKnownStatus, RecoveryActionOutcome,
    RecoveryActionRejection, RecoveryActionRequest, RecoveryActionUsecase, RecoveryResourceState,
};
use super::send::{
    AgentSendOperationUsecase, GetSendOperationError, ObligationTransition,
    ObligationTransitionOutcome, SendAgentMessageError, SendCommandOutcome, SendExecutionStatus,
    SendOperationRequest,
};
use super::stop::{
    StopCommandOutcome, StopOperationError, StopOperationRequest, StopOperationState,
    StopOperationUsecase,
};

const GENERATION: &str = "gen-1";

// --- Fake binding authority -------------------------------------------------

struct FakeAuthority;

fn fake_hash(prefix: u8, data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for chunk in 0..4u8 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u8(prefix);
        hasher.write_u8(chunk);
        hasher.write(data);
        let bytes = hasher.finish().to_be_bytes();
        out[(chunk as usize) * 8..(chunk as usize + 1) * 8].copy_from_slice(&bytes);
    }
    out
}

impl OperationBindingAuthority for FakeAuthority {
    fn mac(&self, message: &[u8]) -> [u8; 32] {
        fake_hash(1, message)
    }

    fn digest(&self, message: &[u8]) -> [u8; 32] {
        fake_hash(2, message)
    }

    fn seal_command(&self, context: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, ()> {
        let mut sealed = b"test-sealed/v1\0".to_vec();
        sealed.extend_from_slice(&fake_hash(3, context));
        sealed.extend_from_slice(plaintext);
        Ok(sealed)
    }

    fn open_command(&self, context: &[u8], envelope: &[u8]) -> Result<Vec<u8>, ()> {
        let prefix_len = b"test-sealed/v1\0".len();
        if envelope.len() < prefix_len + 32
            || &envelope[..prefix_len] != b"test-sealed/v1\0"
            || envelope[prefix_len..prefix_len + 32] != fake_hash(3, context)
        {
            return Err(());
        }
        Ok(envelope[prefix_len + 32..].to_vec())
    }
}

impl super::RecoveryResultCanonicalizer for FakeAuthority {}

// --- Fake local event repository -------------------------------------------

type CallerAttemptState = ([u8; 32], Vec<u8>, Option<String>, String, i64);

#[derive(Default)]
struct FakeState {
    /// (kind label, idempotency key) -> payload hash
    idempotency: HashMap<(String, String), [u8; 32]>,
    /// (principal, kind label, caller request id) -> (operation id, hmac)
    bindings: HashMap<(String, String, String), (String, [u8; 32])>,
    /// (kind label, operation id) -> (receipt, status, revision)
    records: HashMap<(String, String), (OperationReceiptRecord, OperationStatusRecord, i64)>,
    /// (principal, kind label, request id) -> (hash, sealed, scope, resolution, revision)
    attempts: HashMap<(String, String, String), CallerAttemptState>,
    /// obligation id -> (record, pending?, revision)
    obligations: HashMap<String, (ObligationRecord, bool, i64)>,
    /// (session, turn) -> (terminal identity, result, participant digest)
    terminals: HashMap<(String, String), (String, TerminalResultRecord, [u8; 32])>,
    /// Stop operation ID -> immutable resolution label.
    stop_resolutions: HashMap<String, (StopResolutionKind, TerminalResultRecord)>,
    /// action id -> (binding hash, attempt, completed, revision)
    recovery_actions: HashMap<
        String,
        (
            [u8; 32],
            RecoveryAttemptRecord,
            Option<RecoveryResultRecord>,
            i64,
        ),
    >,
    /// stream id -> head
    heads: HashMap<String, i64>,
    events: Vec<LocalDomainEvent>,
    committed_batches: Vec<LocalAtomicBatch>,
    pending_page_queries: Vec<(Option<String>, Option<String>)>,
    commit_calls: usize,
    fail_commit: Option<CommitBatchError>,
    fail_commit_once: Option<CommitBatchError>,
    fail_commit_on_call: Option<(usize, CommitBatchError)>,
    outcome_unknown_after_commit_on_call: Option<usize>,
    fail_resolve_commit_once: bool,
    fail_query: bool,
    fail_operation_query_once_after_commit_call: Option<usize>,
    fail_terminal_query_after_commit_call: Option<usize>,
    fail_obligation_query: Option<String>,
    pending_insert_after_page: Option<(String, (ObligationRecord, bool, i64))>,
}

struct FakeRepo {
    state: Mutex<FakeState>,
    commit_barrier: Mutex<Option<Arc<tokio::sync::Barrier>>>,
}

fn obligation_session_id(record: &ObligationRecord) -> Option<&str> {
    match record {
        ObligationRecord::Send { session_id, .. }
        | ObligationRecord::PermissionResponse { session_id, .. }
        | ObligationRecord::StopInterrupt { session_id, .. }
        | ObligationRecord::SessionClose { session_id, .. }
        | ObligationRecord::BackendSessionRecovery { session_id, .. }
        | ObligationRecord::WorkflowTurnCompletion { session_id, .. }
        | ObligationRecord::RecoveryPublication { session_id, .. }
        | ObligationRecord::ProviderEstablish { session_id, .. }
        | ObligationRecord::TurnExecution { session_id, .. }
        | ObligationRecord::TerminalCommit { session_id, .. }
        | ObligationRecord::FeedbackReservation { session_id, .. }
        | ObligationRecord::Feedback { session_id, .. } => Some(session_id),
        ObligationRecord::RecoveryTransition { original, .. }
        | ObligationRecord::Observed { original, .. } => obligation_session_id(original),
        ObligationRecord::WorkflowShutdown { .. }
        | ObligationRecord::RecoveryReserved { .. }
        | ObligationRecord::RecoveryCompleted { .. }
        | ObligationRecord::WorkflowExecution { .. } => None,
    }
}

impl FakeRepo {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(FakeState::default()),
            commit_barrier: Mutex::new(None),
        })
    }

    fn with_state<T>(&self, f: impl FnOnce(&mut FakeState) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }

    fn committed(commit_id: &CommitIdentity) -> CommittedBatch {
        CommittedBatch {
            commit_id: commit_id.clone(),
            sequence_range: None,
            stream_heads: Vec::<CommittedStreamHead>::new(),
            event_count: 0,
            mutation_count: 0,
            result_hash: [0u8; 32],
        }
    }
}

#[async_trait::async_trait]
impl LocalEventTransactionRepository for FakeRepo {
    fn canonical_event_batch_identity_v1(
        &self,
        events: &[UncommittedDomainEvent],
    ) -> Result<Vec<u8>, String> {
        crate::adaptor::gateway::local_event_store::envelope::canonical_event_batch_identity_v1(
            &crate::adaptor::gateway::local_event_store::envelope::EventCodecRegistry::new(),
            events,
        )
    }

    async fn commit_batch(
        &self,
        batch: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        let commit_barrier = self.commit_barrier.lock().unwrap().clone();
        if let Some(commit_barrier) = commit_barrier {
            commit_barrier.wait().await;
        }
        let mut state = self.state.lock().unwrap();
        state.commit_calls += 1;
        if state
            .fail_commit_on_call
            .as_ref()
            .is_some_and(|(call, _)| *call == state.commit_calls)
        {
            return Err(state.fail_commit_on_call.take().unwrap().1);
        }
        if let Some(error) = state.fail_commit_once.take() {
            return Err(error);
        }
        if let Some(error) = &state.fail_commit {
            return Err(error.clone());
        }
        let idem_key = (
            batch.idempotency.operation_kind.label().to_string(),
            batch.idempotency.idempotency_key.clone(),
        );
        if let Some(saved) = state.idempotency.get(&idem_key) {
            if *saved == batch.idempotency.payload_hash {
                return Ok(CommitBatchResult::Replayed(Self::committed(
                    &batch.commit_id,
                )));
            }
            return Err(CommitBatchError::PayloadConflict);
        }
        let current_stop_capacity = state
            .obligations
            .iter()
            .filter(|(id, (_, pending, _))| id.starts_with("stop-target-") && *pending)
            .count();
        let added_stop_capacity = batch
            .state_mutations
            .iter()
            .filter(|mutation| match mutation {
                LocalStateMutation::Obligation(obligation) => {
                    obligation.obligation_id.starts_with("stop-target-")
                        && obligation.pending.is_some()
                        && !state.obligations.contains_key(&obligation.obligation_id)
                }
                _ => false,
            })
            .count();
        if current_stop_capacity.saturating_add(added_stop_capacity) > 32 {
            return Err(CommitBatchError::CapacityExceeded);
        }
        // Validate guards first (no partial application).
        for head in &batch.expected_heads {
            let current = *state.heads.get(head.stream_id.as_str()).unwrap_or(&0);
            if current != head.expected.value() {
                return Err(CommitBatchError::StreamHeadConflict {
                    current: StreamVersion::new(current).unwrap(),
                });
            }
        }
        for mutation in &batch.state_mutations {
            match mutation {
                LocalStateMutation::OperationBinding(m) => {
                    let key = (
                        m.key.principal.clone(),
                        m.key.kind.label().to_string(),
                        m.key.caller_request_id.clone(),
                    );
                    if let Some((operation_id, hmac)) = state.bindings.get(&key) {
                        if operation_id != &m.operation_id || hmac != &m.binding_hmac {
                            return Err(CommitBatchError::PayloadConflict);
                        }
                    }
                }
                LocalStateMutation::OperationRecord(m)
                | LocalStateMutation::SessionLifecycleOperation(m) => {
                    let key = (m.kind.label().to_string(), m.operation_id.clone());
                    let existing = state.records.get(&key).map(|(_, _, revision)| *revision);
                    match (existing, m.expected) {
                        (None, RevisionGuard::Absent) => {}
                        (Some(revision), RevisionGuard::Expected(expected))
                            if revision == expected.value() => {}
                        _ => {
                            return Err(CommitBatchError::StreamHeadConflict {
                                current: StreamVersion::new(existing.unwrap_or(0)).unwrap(),
                            })
                        }
                    }
                }
                LocalStateMutation::CallerAttempt(m) => {
                    let key = (
                        m.key.principal.clone(),
                        m.key.kind.label().to_string(),
                        m.key.caller_request_id.clone(),
                    );
                    if let Some((hash, _, _, _, revision)) = state.attempts.get(&key) {
                        if hash != &m.command_hash {
                            return Err(CommitBatchError::PayloadConflict);
                        }
                        match m.expected {
                            RevisionGuard::Expected(expected) if expected.value() == *revision => {}
                            RevisionGuard::Absent => {
                                return Ok(CommitBatchResult::Replayed(Self::committed(
                                    &batch.commit_id,
                                )))
                            }
                            _ => {
                                return Err(CommitBatchError::StreamHeadConflict {
                                    current: StreamVersion::new(*revision).unwrap(),
                                })
                            }
                        }
                    } else if !matches!(m.expected, RevisionGuard::Absent) {
                        return Err(CommitBatchError::StreamHeadConflict {
                            current: StreamVersion::new(0).unwrap(),
                        });
                    }
                }
                LocalStateMutation::TerminalRecord(m) => {
                    let key = (m.session_id.clone(), m.turn_id.clone());
                    if let Some((identity, result, participant_digest)) = state.terminals.get(&key)
                    {
                        if identity != &m.terminal_identity
                            || result != &m.result
                            || participant_digest != &m.participant_digest
                        {
                            return Err(CommitBatchError::PayloadConflict);
                        }
                    }
                }
                LocalStateMutation::StopResolution(m) => {
                    if let Some((resolution, detail)) =
                        state.stop_resolutions.get(&m.stop_operation_id)
                    {
                        if resolution != &m.resolution || detail != &m.detail {
                            return Err(CommitBatchError::PayloadConflict);
                        }
                    }
                }
                LocalStateMutation::Obligation(m) => {
                    let existing = state.obligations.get(&m.obligation_id).map(|(_, _, r)| *r);
                    match (existing, m.expected) {
                        (None, RevisionGuard::Absent) => {}
                        (Some(revision), RevisionGuard::Expected(expected))
                            if revision == expected.value() => {}
                        _ => {
                            return Err(CommitBatchError::StreamHeadConflict {
                                current: StreamVersion::new(existing.unwrap_or(0)).unwrap(),
                            })
                        }
                    }
                }
                LocalStateMutation::RecoveryAction(m) => {
                    let existing = state.recovery_actions.get(&m.action_id);
                    match (existing, m.expected) {
                        (None, RevisionGuard::Absent) => {}
                        (Some((binding, _, _, revision)), RevisionGuard::Expected(expected))
                            if binding == &m.binding_hash && *revision == expected.value() => {}
                        _ => {
                            return Err(CommitBatchError::StreamHeadConflict {
                                current: StreamVersion::new(
                                    existing.map_or(0, |(_, _, _, revision)| *revision),
                                )
                                .unwrap(),
                            })
                        }
                    }
                }
                _ => {}
            }
        }
        // Apply.
        for mutation in &batch.state_mutations {
            match mutation {
                LocalStateMutation::OperationBinding(m) => {
                    state.bindings.insert(
                        (
                            m.key.principal.clone(),
                            m.key.kind.label().to_string(),
                            m.key.caller_request_id.clone(),
                        ),
                        (m.operation_id.clone(), m.binding_hmac),
                    );
                }
                LocalStateMutation::OperationRecord(m)
                | LocalStateMutation::SessionLifecycleOperation(m) => {
                    state.records.insert(
                        (m.kind.label().to_string(), m.operation_id.clone()),
                        (
                            m.receipt.clone(),
                            m.latest_status.clone(),
                            m.revision.value(),
                        ),
                    );
                }
                LocalStateMutation::CallerAttempt(m) => {
                    state.attempts.insert(
                        (
                            m.key.principal.clone(),
                            m.key.kind.label().to_string(),
                            m.key.caller_request_id.clone(),
                        ),
                        (
                            m.command_hash,
                            m.sealed_command.clone(),
                            m.scope_id.clone(),
                            m.resolution.label().to_string(),
                            m.revision.value(),
                        ),
                    );
                }
                LocalStateMutation::TerminalRecord(m) => {
                    state.terminals.insert(
                        (m.session_id.clone(), m.turn_id.clone()),
                        (
                            m.terminal_identity.clone(),
                            m.result.clone(),
                            m.participant_digest,
                        ),
                    );
                }
                LocalStateMutation::StopResolution(m) => {
                    state.stop_resolutions.insert(
                        m.stop_operation_id.clone(),
                        (m.resolution, m.detail.clone()),
                    );
                }
                LocalStateMutation::Obligation(m) => {
                    state.obligations.insert(
                        m.obligation_id.clone(),
                        (m.record.clone(), m.pending.is_some(), m.revision.value()),
                    );
                }
                LocalStateMutation::RecoveryAction(m) => {
                    state.recovery_actions.insert(
                        m.action_id.clone(),
                        (
                            m.binding_hash,
                            m.attempt.clone(),
                            m.completed.clone(),
                            m.revision.value(),
                        ),
                    );
                }
                _ => {}
            }
        }
        for event in &batch.events {
            *state
                .heads
                .entry(event.stream_id.as_str().to_string())
                .or_insert(0) += 1;
            state.events.push(event.event.clone());
        }
        state
            .idempotency
            .insert(idem_key, batch.idempotency.payload_hash);
        let committed = Self::committed(&batch.commit_id);
        let commit_identity = batch.commit_id.clone();
        let lose_reply = state
            .outcome_unknown_after_commit_on_call
            .is_some_and(|call| call == state.commit_calls);
        if lose_reply {
            state.outcome_unknown_after_commit_on_call = None;
        }
        state.committed_batches.push(batch);
        if lose_reply {
            Err(CommitBatchError::OutcomeUnknown {
                identity: commit_identity,
            })
        } else {
            Ok(CommitBatchResult::Committed(committed))
        }
    }

    async fn resolve_commit(
        &self,
        identity: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError> {
        let mut state = self.state.lock().unwrap();
        if state.fail_resolve_commit_once {
            state.fail_resolve_commit_once = false;
            return Err(LocalEventQueryError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "fake commit resolution outage",
                    "fake-resolve-commit",
                ),
            });
        }
        if state
            .committed_batches
            .iter()
            .any(|batch| batch.commit_id == identity)
        {
            Ok(CommitResolution::Committed(Self::committed(&identity)))
        } else {
            Ok(CommitResolution::NotCommitted)
        }
    }

    async fn load_stream(
        &self,
        request: LoadStreamRequest,
    ) -> Result<DomainEventPage, LocalEventQueryError> {
        let state = self.state.lock().unwrap();
        if state.fail_query {
            return Err(LocalEventQueryError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "fake outage",
                    "fake",
                ),
            });
        }
        let head = *state.heads.get(request.stream_id.as_str()).unwrap_or(&0);
        Ok(DomainEventPage {
            events: Vec::new(),
            head: StreamVersion::new(head).unwrap(),
            next_after: None,
        })
    }

    async fn query(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        let mut state = self.state.lock().unwrap();
        if state.fail_query {
            return Err(LocalEventQueryError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "fake outage",
                    "fake",
                ),
            });
        }
        match request {
            LocalEventQuery::OperationBindingByIdentity { key } => {
                let binding = state
                    .bindings
                    .get(&(
                        key.principal.clone(),
                        key.kind.label().to_string(),
                        key.caller_request_id.clone(),
                    ))
                    .map(|(operation_id, binding_hmac)| OperationBindingView {
                        key,
                        operation_id: operation_id.clone(),
                        binding_hmac: *binding_hmac,
                    });
                Ok(LocalEventQueryResult::OperationBindingByIdentity(binding))
            }
            LocalEventQuery::OperationByIdentity { kind, operation_id } => {
                if state
                    .fail_operation_query_once_after_commit_call
                    .is_some_and(|call| state.commit_calls >= call)
                {
                    state.fail_operation_query_once_after_commit_call = None;
                    return Err(LocalEventQueryError::StorageUnavailable {
                        failure: SafeOperationFailure::new(
                            SessionOperationFailureKind::StorageUnavailable,
                            true,
                            "fake operation query outage",
                            "fake-operation-query",
                        ),
                    });
                }
                let view = state
                    .records
                    .get(&(kind.label().to_string(), operation_id.clone()))
                    .map(|(receipt, status, revision)| OperationRecordView {
                        kind,
                        operation_id,
                        receipt: receipt.clone(),
                        latest_status: status.clone(),
                        revision: crate::domain::local_event::Revision::new(*revision).unwrap(),
                    });
                Ok(LocalEventQueryResult::OperationByIdentity(view))
            }
            LocalEventQuery::CallerAttemptByIdentity { key } => {
                let view = state
                    .attempts
                    .get(&(
                        key.principal.clone(),
                        key.kind.label().to_string(),
                        key.caller_request_id.clone(),
                    ))
                    .map(|(hash, sealed, scope, resolution, revision)| {
                        crate::domain::local_event::CallerAttemptView {
                            key,
                            scope_id: scope.clone(),
                            operation_id: None,
                            command_hash: *hash,
                            sealed_command: sealed.clone(),
                            resolution: crate::domain::local_event::CallerAttemptResolution::parse(
                                resolution,
                            )
                            .unwrap(),
                            revision: crate::domain::local_event::Revision::new(*revision).unwrap(),
                        }
                    });
                Ok(LocalEventQueryResult::CallerAttemptByIdentity(view))
            }
            LocalEventQuery::TerminalByTurn {
                session_id,
                turn_id,
            } => {
                if state
                    .fail_terminal_query_after_commit_call
                    .is_some_and(|call| state.commit_calls >= call)
                {
                    return Err(LocalEventQueryError::StorageUnavailable {
                        failure: SafeOperationFailure::new(
                            SessionOperationFailureKind::StorageUnavailable,
                            true,
                            "fake terminal query outage",
                            "fake-terminal-query",
                        ),
                    });
                }
                let terminal = state
                    .terminals
                    .get(&(session_id.clone(), turn_id.clone()))
                    .map(|(terminal_identity, result, participant_digest)| {
                        crate::domain::local_event::TerminalRecordView {
                            session_id,
                            turn_id,
                            terminal_identity: terminal_identity.clone(),
                            result: result.clone(),
                            participant_digest: *participant_digest,
                        }
                    });
                Ok(LocalEventQueryResult::TerminalByTurn(terminal))
            }
            LocalEventQuery::CallerAttemptPage {
                principal,
                scope_id,
                limit,
                after_kind,
                after_caller_request_id,
                ..
            } => {
                let mut entries = state
                    .attempts
                    .iter()
                    .filter_map(
                        |((saved_principal, kind, id), (hash, _, scope, resolution, revision))| {
                            if saved_principal != &principal
                                || scope.as_deref() != Some(scope_id.as_str())
                                || resolution == "cleared"
                            {
                                return None;
                            }
                            let operation_kind =
                                crate::domain::local_event::OperationKind::parse(kind)?;
                            if after_kind.is_some_and(|after| {
                                operation_kind.label() < after.label()
                                    || (operation_kind == after
                                        && after_caller_request_id
                                            .as_deref()
                                            .is_some_and(|after_id| id.as_str() <= after_id))
                            }) {
                                return None;
                            }
                            Some(crate::domain::local_event::CallerAttemptView {
                                key: crate::domain::local_event::CallerOperationKey {
                                    principal: principal.clone(),
                                    installation_id: GENERATION.to_string(),
                                    kind: operation_kind,
                                    caller_request_id: id.clone(),
                                },
                                scope_id: scope.clone(),
                                operation_id: None,
                                command_hash: *hash,
                                sealed_command: Vec::new(),
                                resolution:
                                    crate::domain::local_event::CallerAttemptResolution::parse(
                                        resolution,
                                    )
                                    .unwrap(),
                                revision: crate::domain::local_event::Revision::new(*revision)
                                    .unwrap(),
                            })
                        },
                    )
                    .collect::<Vec<_>>();
                entries.sort_by(|left, right| {
                    (left.key.kind.label(), left.key.caller_request_id.as_str())
                        .cmp(&(right.key.kind.label(), right.key.caller_request_id.as_str()))
                });
                entries.truncate(limit);
                Ok(LocalEventQueryResult::CallerAttemptPage(entries))
            }
            LocalEventQuery::StopResolutionByOperation { stop_operation_id } => {
                let resolution =
                    state
                        .stop_resolutions
                        .get(&stop_operation_id)
                        .map(
                            |(kind, detail)| crate::domain::local_event::StopResolutionView {
                                stop_operation_id: stop_operation_id.clone(),
                                resolution: *kind,
                                detail: detail.clone(),
                            },
                        );
                Ok(LocalEventQueryResult::StopResolutionByOperation(resolution))
            }
            LocalEventQuery::ObligationByIdentity { obligation_id } => {
                if state.fail_obligation_query.as_deref() == Some(obligation_id.as_str()) {
                    return Err(LocalEventQueryError::StorageUnavailable {
                        failure: SafeOperationFailure::new(
                            SessionOperationFailureKind::StorageUnavailable,
                            true,
                            "fake obligation query outage",
                            "fake-obligation-query",
                        ),
                    });
                }
                let view =
                    state
                        .obligations
                        .get(&obligation_id)
                        .map(|(record, pending, revision)| {
                            crate::domain::local_event::ObligationView {
                                obligation_id: obligation_id.clone(),
                                record: record.clone(),
                                record_sha256: [0; 32],
                                pending: pending.then(|| {
                                    crate::domain::local_event::PendingIndexEntryView {
                                        ordered_key: format!("test:{obligation_id}"),
                                        owner: obligation_session_id(record)
                                            .unwrap_or("test")
                                            .to_string(),
                                        partition:
                                            crate::domain::local_event::PendingPartition::Owner,
                                        shutdown_plan: None,
                                    }
                                }),
                                revision: crate::domain::local_event::Revision::new(*revision)
                                    .unwrap(),
                            }
                        });
                Ok(LocalEventQueryResult::ObligationByIdentity(view))
            }
            LocalEventQuery::RecoveryActionByIdentity { action_id } => {
                let view = state.recovery_actions.get(&action_id).map(
                    |(binding_hash, attempt, completed, revision)| {
                        crate::domain::local_event::RecoveryActionView {
                            action_id: action_id.clone(),
                            binding_hash: *binding_hash,
                            attempt: attempt.clone(),
                            completed: completed.clone(),
                            revision: crate::domain::local_event::Revision::new(*revision).unwrap(),
                        }
                    },
                );
                Ok(LocalEventQueryResult::RecoveryActionByIdentity(view))
            }
            LocalEventQuery::SessionProjectionByIdentity { .. } => {
                Ok(LocalEventQueryResult::SessionProjectionByIdentity(None))
            }
            LocalEventQuery::PendingRecoveryPage {
                limit,
                cursor,
                ordered_key_prefix,
                ..
            } => {
                state.pending_page_queries.push((
                    ordered_key_prefix.clone(),
                    cursor.as_ref().map(|cursor| cursor.as_str().to_string()),
                ));
                let after = cursor.as_ref().map(|cursor| cursor.as_str());
                let mut entries = state
                    .obligations
                    .iter()
                    .filter(|(_, (_, pending, _))| *pending)
                    .filter_map(|(obligation_id, (record, _, revision))| {
                        let ordered_key = if obligation_id.starts_with("permission-response-") {
                            format!("permission-response:{obligation_id}")
                        } else {
                            format!("test:{obligation_id}")
                        };
                        if ordered_key_prefix
                            .as_deref()
                            .is_some_and(|prefix| !ordered_key.starts_with(prefix))
                            || after.is_some_and(|after| ordered_key.as_str() <= after)
                        {
                            return None;
                        }
                        let owner = obligation_session_id(record).unwrap_or("test").to_string();
                        Some(crate::domain::local_event::PendingObligationView {
                            obligation_id: obligation_id.clone(),
                            ordered_key,
                            owner,
                            partition: crate::domain::local_event::PendingPartition::Owner,
                            shutdown_plan: None,
                            record: record.clone(),
                            record_sha256: [0; 32],
                            owner_projection: None,
                            revision: crate::domain::local_event::Revision::new(*revision).unwrap(),
                        })
                    })
                    .collect::<Vec<_>>();
                entries.sort_by(|left, right| left.ordered_key.cmp(&right.ordered_key));
                let has_more = entries.len() > limit;
                entries.truncate(limit);
                let continuation_cursors = entries
                    .iter()
                    .map(|entry| {
                        crate::domain::local_event::QueryCursor::from_opaque(
                            entry.ordered_key.clone(),
                        )
                    })
                    .collect();
                let next_cursor = has_more.then(|| entries.last()).flatten().map(|entry| {
                    crate::domain::local_event::QueryCursor::from_opaque(entry.ordered_key.clone())
                });
                if let Some((obligation_id, pending)) = state.pending_insert_after_page.take() {
                    state.obligations.insert(obligation_id, pending);
                }
                Ok(LocalEventQueryResult::PendingRecoveryPage(
                    crate::domain::local_event::PendingRecoveryPageView {
                        entries,
                        continuation_cursors,
                        next_cursor,
                    },
                ))
            }
            _ => Err(LocalEventQueryError::InvalidRequest),
        }
    }

    fn query_blocking(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        match request {
            LocalEventQuery::SessionProjectionByIdentity { .. } => {
                Ok(LocalEventQueryResult::SessionProjectionByIdentity(None))
            }
            _ => Err(LocalEventQueryError::InvalidRequest),
        }
    }

    fn subscribe(&self, _after: GlobalSequence) -> LocalEventSubscription {
        LocalEventSubscription::new(Box::pin(futures_util::stream::empty()))
    }
}

// --- Fake gates -------------------------------------------------------------

struct FakeSendGate {
    plan: Mutex<Result<SendPlan, SafeOperationFailure>>,
    effects: Mutex<Vec<AcceptedSendEffect>>,
    legacy_provider_recovery: Mutex<LegacyProviderEstablishRecovery>,
    canonical_current_turn: Mutex<Option<u64>>,
    claim_sink: Mutex<Option<Weak<AgentSendOperationUsecase>>>,
    dispatch_failure: Mutex<Option<SafeOperationFailure>>,
    scheduled_queue_effects: Mutex<std::collections::HashSet<String>>,
}

impl FakeSendGate {
    fn started_turn(session_id: &str) -> Arc<Self> {
        Arc::new(Self {
            plan: Mutex::new(Ok(SendPlan {
                session_id: session_id.to_string(),
                initial_session: None,
                session_projection_guard: RevisionGuard::Absent,
                disposition: SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                input_ref: "input-1".to_string(),
                human_message_id: "human-1".to_string(),
                prompt: crate::domain::agent_session::events::PromptInput {
                    content: "hello".to_string(),
                    ..Default::default()
                },
                reserved_turn_id: None,
            })),
            effects: Mutex::new(Vec::new()),
            legacy_provider_recovery: Mutex::new(
                LegacyProviderEstablishRecovery::RequiresManualResolution,
            ),
            canonical_current_turn: Mutex::new(None),
            claim_sink: Mutex::new(None),
            dispatch_failure: Mutex::new(None),
            scheduled_queue_effects: Mutex::new(std::collections::HashSet::new()),
        })
    }

    fn set_plan(&self, plan: Result<SendPlan, SafeOperationFailure>) {
        *self.plan.lock().unwrap() = plan;
    }

    fn effect_count(&self) -> usize {
        self.effects.lock().unwrap().len()
    }

    fn effects(&self) -> Vec<AcceptedSendEffect> {
        self.effects.lock().unwrap().clone()
    }

    fn allow_legacy_provider_turn_continuation(&self) {
        *self.legacy_provider_recovery.lock().unwrap() =
            LegacyProviderEstablishRecovery::ContinueTurnExecution;
    }

    fn set_canonical_current_turn(&self, turn_id: u64) {
        *self.canonical_current_turn.lock().unwrap() = Some(turn_id);
    }

    fn bind_claim_sink(&self, sink: Weak<AgentSendOperationUsecase>) {
        *self.claim_sink.lock().unwrap() = Some(sink);
    }

    fn fail_dispatch(&self, failure: SafeOperationFailure) {
        *self.dispatch_failure.lock().unwrap() = Some(failure);
    }
}

#[async_trait::async_trait]
impl SendAdmissionGate for FakeSendGate {
    async fn plan_send(
        &self,
        _principal: &str,
        _operation_id: &str,
        _canonical_payload: &str,
    ) -> Result<SendPlan, SafeOperationFailure> {
        self.plan.lock().unwrap().clone()
    }

    async fn classify_legacy_provider_establish(
        &self,
        _session_id: &str,
    ) -> Result<LegacyProviderEstablishRecovery, SafeOperationFailure> {
        Ok(*self.legacy_provider_recovery.lock().unwrap())
    }

    async fn canonical_immediate_turn_is_current(
        &self,
        _session_id: &str,
        turn_id: u64,
    ) -> Result<bool, SafeOperationFailure> {
        Ok(self
            .canonical_current_turn
            .lock()
            .unwrap()
            .is_none_or(|current| current == turn_id))
    }

    async fn start_provider_effect(
        &self,
        effect: &AcceptedSendEffect,
    ) -> Result<SendEffectDispatch, SafeOperationFailure> {
        if let Some(failure) = self.dispatch_failure.lock().unwrap().clone() {
            return Err(failure);
        }
        if matches!(effect.disposition, SendDisposition::Queued { .. })
            && !self
                .scheduled_queue_effects
                .lock()
                .unwrap()
                .insert(effect.execution_obligation_id.clone())
        {
            return Ok(SendEffectDispatch::AlreadyScheduled);
        }
        let claim_sink = self.claim_sink.lock().unwrap().clone();
        if let Some(claim_sink) = claim_sink {
            let Some(claim_sink) = claim_sink.upgrade() else {
                return Err(SafeOperationFailure::new(
                    SessionOperationFailureKind::PersistFailure,
                    true,
                    "The fake claim authority is unavailable.",
                    "fake-send-claim-authority",
                ));
            };
            let claim = claim_sink
                .transition_obligation(ObligationTransition {
                    operation_id: &effect.operation_id,
                    obligation_id: &effect.execution_obligation_id,
                    expected_kind: "turn_execution",
                    expected_state: "pending",
                    next_state: "effect_reserved",
                    keep_pending: true,
                    status: Some(SendExecutionStatus::ProviderStartReserved {
                        obligation_id: effect.execution_obligation_id.clone(),
                    }),
                })
                .await;
            if claim != Ok(ObligationTransitionOutcome::Applied) {
                return Err(SafeOperationFailure::new(
                    SessionOperationFailureKind::OutcomeUnknown,
                    true,
                    "The fake execution claim did not win.",
                    "fake-send-claim",
                ));
            }
        }
        self.effects.lock().unwrap().push(effect.clone());
        Ok(SendEffectDispatch::Scheduled)
    }
}

struct RacingSendGate {
    plan: SendPlan,
    plan_calls: std::sync::atomic::AtomicUsize,
    first_entered: tokio::sync::Notify,
    release_first: tokio::sync::Notify,
    effects: Mutex<Vec<AcceptedSendEffect>>,
}

impl RacingSendGate {
    fn new(plan: SendPlan) -> Arc<Self> {
        Arc::new(Self {
            plan,
            plan_calls: std::sync::atomic::AtomicUsize::new(0),
            first_entered: tokio::sync::Notify::new(),
            release_first: tokio::sync::Notify::new(),
            effects: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl SendAdmissionGate for RacingSendGate {
    async fn plan_send(
        &self,
        _principal: &str,
        _operation_id: &str,
        _canonical_payload: &str,
    ) -> Result<SendPlan, SafeOperationFailure> {
        let call = self
            .plan_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call == 0 {
            self.first_entered.notify_one();
            self.release_first.notified().await;
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::ExternalEffectFailed,
                true,
                "stale admission snapshot",
                "send-race-first",
            ));
        }
        Ok(self.plan.clone())
    }

    async fn canonical_immediate_turn_is_current(
        &self,
        _session_id: &str,
        _turn_id: u64,
    ) -> Result<bool, SafeOperationFailure> {
        Ok(true)
    }

    async fn start_provider_effect(
        &self,
        effect: &AcceptedSendEffect,
    ) -> Result<SendEffectDispatch, SafeOperationFailure> {
        self.effects.lock().unwrap().push(effect.clone());
        Ok(SendEffectDispatch::Scheduled)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PermissionExecuteMode {
    Succeed,
    Fail,
}

struct FakePermissionGate {
    plan: Mutex<Result<PermissionResponsePlan, SafeOperationFailure>>,
    mode: Mutex<PermissionExecuteMode>,
    effects: Mutex<Vec<AcceptedPermissionResponseEffect>>,
    completion_event_sets: Mutex<Vec<Vec<AgentSessionDomainEvent>>>,
    after_completion: Mutex<Vec<AcceptedPermissionResponseEffect>>,
}

impl FakePermissionGate {
    fn accepting(response: PermissionResponse) -> Arc<Self> {
        Arc::new(Self {
            plan: Mutex::new(Ok(PermissionResponsePlan {
                session_id: "permission-session".to_string(),
                request_id: response.request_id.clone(),
                turn_id: 7,
                response,
                from_runtime_state: true,
            })),
            mode: Mutex::new(PermissionExecuteMode::Succeed),
            effects: Mutex::new(Vec::new()),
            completion_event_sets: Mutex::new(Vec::new()),
            after_completion: Mutex::new(Vec::new()),
        })
    }

    fn set_mode(&self, mode: PermissionExecuteMode) {
        *self.mode.lock().unwrap() = mode;
    }

    fn effect_count(&self) -> usize {
        self.effects.lock().unwrap().len()
    }

    fn after_completion_count(&self) -> usize {
        self.after_completion.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl PermissionResponseGate for FakePermissionGate {
    async fn plan_response(
        &self,
        _session_id: &str,
        _response: &PermissionResponse,
    ) -> Result<PermissionResponsePlan, SafeOperationFailure> {
        self.plan.lock().unwrap().clone()
    }

    async fn completion_state_mutations(
        &self,
        _effect: &AcceptedPermissionResponseEffect,
        events: &[AgentSessionDomainEvent],
    ) -> Result<Vec<LocalStateMutation>, SafeOperationFailure> {
        self.completion_event_sets
            .lock()
            .unwrap()
            .push(events.to_vec());
        Ok(Vec::new())
    }

    async fn execute(
        &self,
        effect: &AcceptedPermissionResponseEffect,
    ) -> Result<(), SafeOperationFailure> {
        self.effects.lock().unwrap().push(effect.clone());
        match *self.mode.lock().unwrap() {
            PermissionExecuteMode::Succeed => Ok(()),
            PermissionExecuteMode::Fail => Err(SafeOperationFailure::new(
                SessionOperationFailureKind::ExternalEffectFailed,
                false,
                "provider permission response result is unavailable",
                "fake-permission-provider",
            )),
        }
    }

    async fn after_completion(&self, effect: &AcceptedPermissionResponseEffect) {
        self.after_completion.lock().unwrap().push(effect.clone());
    }
}

struct RacingPermissionGate {
    plan: PermissionResponsePlan,
    plan_calls: std::sync::atomic::AtomicUsize,
    first_entered: tokio::sync::Notify,
    release_first: tokio::sync::Notify,
    effects: Mutex<Vec<AcceptedPermissionResponseEffect>>,
}

#[async_trait::async_trait]
impl PermissionResponseGate for RacingPermissionGate {
    async fn plan_response(
        &self,
        _session_id: &str,
        _response: &PermissionResponse,
    ) -> Result<PermissionResponsePlan, SafeOperationFailure> {
        let call = self
            .plan_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call == 0 {
            self.first_entered.notify_one();
            self.release_first.notified().await;
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::ExternalEffectFailed,
                true,
                "stale permission admission snapshot",
                "permission-race-first",
            ));
        }
        Ok(self.plan.clone())
    }

    async fn execute(
        &self,
        effect: &AcceptedPermissionResponseEffect,
    ) -> Result<(), SafeOperationFailure> {
        self.effects.lock().unwrap().push(effect.clone());
        Ok(())
    }
}

struct RacingLifecycleGate {
    snapshot: SessionLifecycleSnapshot,
    snapshot_calls: std::sync::atomic::AtomicUsize,
    first_entered: tokio::sync::Notify,
    release_first: tokio::sync::Notify,
    executions: Mutex<Vec<SessionLifecycleEffect>>,
}

#[async_trait::async_trait]
impl SessionLifecycleGate for RacingLifecycleGate {
    async fn session_snapshot(
        &self,
        _session_id: &str,
    ) -> Result<SessionLifecycleSnapshot, SafeOperationFailure> {
        let call = self
            .snapshot_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call == 0 {
            self.first_entered.notify_one();
            self.release_first.notified().await;
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::ExternalEffectFailed,
                true,
                "stale lifecycle admission snapshot",
                "lifecycle-race-first",
            ));
        }
        Ok(self.snapshot.clone())
    }

    async fn execute(&self, effect: &SessionLifecycleEffect) -> Result<(), SafeOperationFailure> {
        self.executions.lock().unwrap().push(effect.clone());
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum LifecycleExecuteMode {
    Succeed,
    Fail,
    Hang,
}

struct FakeLifecycleGate {
    snapshot: Mutex<Result<SessionLifecycleSnapshot, SafeOperationFailure>>,
    mode: Mutex<LifecycleExecuteMode>,
    executions: Mutex<Vec<SessionLifecycleEffect>>,
}

impl FakeLifecycleGate {
    fn with_snapshot(snapshot: SessionLifecycleSnapshot) -> Arc<Self> {
        Arc::new(Self {
            snapshot: Mutex::new(Ok(snapshot)),
            mode: Mutex::new(LifecycleExecuteMode::Succeed),
            executions: Mutex::new(Vec::new()),
        })
    }

    fn set_mode(&self, mode: LifecycleExecuteMode) {
        *self.mode.lock().unwrap() = mode;
    }

    fn set_snapshot(&self, snapshot: SessionLifecycleSnapshot) {
        *self.snapshot.lock().unwrap() = Ok(snapshot);
    }

    fn execution_count(&self) -> usize {
        self.executions.lock().unwrap().len()
    }
}

fn open_idle_snapshot(revision: i64) -> SessionLifecycleSnapshot {
    SessionLifecycleSnapshot {
        session_revision: revision,
        lifecycle: SessionLifecycleState::Open {
            idle: true,
            active_turn_id: None,
        },
        queue_paused: false,
        has_runtime: true,
        has_pending_permission: false,
        has_pending_recovery: false,
        has_pending_provider_operation: false,
    }
}

fn open_active_snapshot(revision: i64, turn_id: u64) -> SessionLifecycleSnapshot {
    SessionLifecycleSnapshot {
        session_revision: revision,
        lifecycle: SessionLifecycleState::Open {
            idle: false,
            active_turn_id: Some(turn_id),
        },
        queue_paused: false,
        has_runtime: true,
        has_pending_permission: false,
        has_pending_recovery: false,
        has_pending_provider_operation: false,
    }
}

#[async_trait::async_trait]
impl SessionLifecycleGate for FakeLifecycleGate {
    async fn session_snapshot(
        &self,
        _session_id: &str,
    ) -> Result<SessionLifecycleSnapshot, SafeOperationFailure> {
        self.snapshot.lock().unwrap().clone()
    }

    async fn execute(&self, effect: &SessionLifecycleEffect) -> Result<(), SafeOperationFailure> {
        self.executions.lock().unwrap().push(effect.clone());
        let mode = *self.mode.lock().unwrap();
        match mode {
            LifecycleExecuteMode::Succeed => Ok(()),
            LifecycleExecuteMode::Fail => Err(SafeOperationFailure::new(
                SessionOperationFailureKind::ExternalEffectFailed,
                true,
                "runtime close failed",
                "fake-close",
            )),
            LifecycleExecuteMode::Hang => {
                futures_util::future::pending::<()>().await;
                unreachable!()
            }
        }
    }
}

struct LateLifecycleGate {
    snapshot: SessionLifecycleSnapshot,
    executions: Mutex<Vec<SessionLifecycleEffect>>,
    release_result: Arc<tokio::sync::Notify>,
    late_results: Arc<std::sync::atomic::AtomicUsize>,
}

impl LateLifecycleGate {
    fn with_snapshot(snapshot: SessionLifecycleSnapshot) -> Arc<Self> {
        Arc::new(Self {
            snapshot,
            executions: Mutex::new(Vec::new()),
            release_result: Arc::new(tokio::sync::Notify::new()),
            late_results: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
    }

    fn execution_count(&self) -> usize {
        self.executions.lock().unwrap().len()
    }

    fn release_late_result(&self) {
        self.release_result.notify_one();
    }

    fn late_result_count(&self) -> usize {
        self.late_results.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl SessionLifecycleGate for LateLifecycleGate {
    async fn session_snapshot(
        &self,
        _session_id: &str,
    ) -> Result<SessionLifecycleSnapshot, SafeOperationFailure> {
        Ok(self.snapshot.clone())
    }

    async fn execute(&self, effect: &SessionLifecycleEffect) -> Result<(), SafeOperationFailure> {
        self.executions.lock().unwrap().push(effect.clone());
        let release_result = Arc::clone(&self.release_result);
        let late_results = Arc::clone(&self.late_results);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            release_result.notified().await;
            late_results.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _ = sender.send(());
        });
        receiver.await.map_err(|_| {
            SafeOperationFailure::new(
                SessionOperationFailureKind::OutcomeUnknown,
                true,
                "The detached lifecycle result was not observed.",
                "b056-detached-result",
            )
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StopInterruptMode {
    Complete,
    Fail,
    Hang,
}

struct FakeStopGate {
    snapshot: Mutex<StopTargetSnapshot>,
    snapshot_failure: Mutex<Option<SafeOperationFailure>>,
    stale_on_acceptance: Mutex<bool>,
    revision_after_acceptance: Mutex<Option<u64>>,
    failure_after_acceptance: Mutex<Option<SafeOperationFailure>>,
    mode: Mutex<StopInterruptMode>,
    interrupts: Mutex<Vec<AcceptedStopEffect>>,
    timeout_terminal_commits: Mutex<Vec<AcceptedStopEffect>>,
}

impl FakeStopGate {
    fn active(revision: u64, turn_id: &str) -> Arc<Self> {
        Arc::new(Self {
            snapshot: Mutex::new(StopTargetSnapshot {
                session_revision: revision,
                active_turn_id: turn_id.to_string(),
                queue_paused: false,
            }),
            snapshot_failure: Mutex::new(None),
            stale_on_acceptance: Mutex::new(false),
            revision_after_acceptance: Mutex::new(None),
            failure_after_acceptance: Mutex::new(None),
            mode: Mutex::new(StopInterruptMode::Complete),
            interrupts: Mutex::new(Vec::new()),
            timeout_terminal_commits: Mutex::new(Vec::new()),
        })
    }

    fn set_stale_on_acceptance(&self) {
        *self.stale_on_acceptance.lock().unwrap() = true;
    }

    fn set_revision_after_acceptance(&self, revision: u64) {
        *self.revision_after_acceptance.lock().unwrap() = Some(revision);
    }

    fn set_failure_after_acceptance(&self, failure: SafeOperationFailure) {
        *self.failure_after_acceptance.lock().unwrap() = Some(failure);
    }

    fn set_mode(&self, mode: StopInterruptMode) {
        *self.mode.lock().unwrap() = mode;
    }

    fn interrupt_count(&self) -> usize {
        self.interrupts.lock().unwrap().len()
    }

    fn timeout_terminal_commit_count(&self) -> usize {
        self.timeout_terminal_commits.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl StopAdmissionGate for FakeStopGate {
    async fn target_snapshot(
        &self,
        _session_id: &str,
    ) -> Result<StopTargetSnapshot, SafeOperationFailure> {
        if let Some(failure) = self.snapshot_failure.lock().unwrap().as_ref() {
            return Err(failure.clone());
        }
        Ok(self.snapshot.lock().unwrap().clone())
    }

    async fn acceptance_state_mutations(
        &self,
        _session_id: &str,
        _expected_session_revision: u64,
        _events: &[AgentSessionDomainEvent],
    ) -> Result<Option<Vec<LocalStateMutation>>, SafeOperationFailure> {
        if *self.stale_on_acceptance.lock().unwrap() {
            return Ok(None);
        }
        if let Some(revision) = self.revision_after_acceptance.lock().unwrap().take() {
            self.snapshot.lock().unwrap().session_revision = revision;
        }
        if let Some(failure) = self.failure_after_acceptance.lock().unwrap().take() {
            *self.snapshot_failure.lock().unwrap() = Some(failure);
        }
        Ok(Some(Vec::new()))
    }

    async fn interrupt(
        &self,
        effect: &AcceptedStopEffect,
    ) -> Result<StopEffectObservation, SafeOperationFailure> {
        self.interrupts.lock().unwrap().push(effect.clone());
        let mode = *self.mode.lock().unwrap();
        match mode {
            StopInterruptMode::Complete => Ok(StopEffectObservation {
                terminal_reason: Some(InterruptReason::Abort),
            }),
            StopInterruptMode::Fail => Err(SafeOperationFailure::new(
                SessionOperationFailureKind::ExternalEffectFailed,
                true,
                "provider interrupt result is unavailable",
                "fake-stop",
            )),
            StopInterruptMode::Hang => {
                futures_util::future::pending::<()>().await;
                unreachable!()
            }
        }
    }

    async fn timeout_terminal_committed(&self, effect: &AcceptedStopEffect) {
        self.timeout_terminal_commits
            .lock()
            .unwrap()
            .push(effect.clone());
    }
}

struct FakeRecoveryExecutor {
    result: Mutex<Result<RecoveryEffectResult, SafeOperationFailure>>,
    handoff: Mutex<RecoveryEffectHandoff>,
    effects: Mutex<Vec<RecoveryEffectRequest>>,
    owned_effects: Mutex<std::collections::HashSet<String>>,
}

impl FakeRecoveryExecutor {
    fn returning(classification: RecoveryResultClassification, resource_view: &str) -> Arc<Self> {
        Arc::new(Self {
            result: Mutex::new(Ok(RecoveryEffectResult {
                classification,
                safe_result: resource_view.to_string(),
                owner_mutations: Vec::new(),
                owner_batch: None,
            })),
            handoff: Mutex::new(RecoveryEffectHandoff::Ready),
            effects: Mutex::new(Vec::new()),
            owned_effects: Mutex::new(std::collections::HashSet::new()),
        })
    }

    fn set_handoff(&self, handoff: RecoveryEffectHandoff) {
        *self.handoff.lock().unwrap() = handoff;
    }

    fn effect_count(&self) -> usize {
        self.effects.lock().unwrap().len()
    }

    fn own_current_process_effect(&self, obligation_id: &str) {
        self.owned_effects
            .lock()
            .unwrap()
            .insert(obligation_id.to_string());
    }
}

#[async_trait::async_trait]
impl RecoveryEffectExecutor for FakeRecoveryExecutor {
    async fn owns_current_process_effect(
        &self,
        obligation_id: &str,
        _immutable_obligation: &ObligationRecord,
    ) -> bool {
        self.owned_effects.lock().unwrap().contains(obligation_id)
    }

    fn supports_read_again(
        &self,
        _obligation_id: &str,
        _immutable_obligation: &ObligationRecord,
    ) -> bool {
        true
    }

    async fn validate_handoff(
        &self,
        _request: &RecoveryEffectRequest,
    ) -> Result<RecoveryEffectHandoff, SafeOperationFailure> {
        Ok(*self.handoff.lock().unwrap())
    }

    async fn execute(
        &self,
        request: &RecoveryEffectRequest,
    ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
        self.effects.lock().unwrap().push(request.clone());
        self.result.lock().unwrap().clone()
    }
}

fn seed_pending_obligation(repo: &Arc<FakeRepo>, obligation_id: &str, record: ObligationRecord) {
    repo.with_state(|state| {
        state
            .obligations
            .insert(obligation_id.to_string(), (record, true, 0));
    });
}

fn seed_send_operation_status(
    repo: &Arc<FakeRepo>,
    operation_id: &str,
    session_id: &str,
    status: OperationStatusValue,
) {
    repo.with_state(|state| {
        state.records.insert(
            (
                OperationKind::Send.label().to_string(),
                operation_id.to_string(),
            ),
            (
                OperationReceiptRecord::Send {
                    operation_id: operation_id.to_string(),
                    session_id: session_id.to_string(),
                    input_ref: format!("input-{operation_id}"),
                    disposition: SendDisposition::StartedTurn {
                        turn_id: "1".to_string(),
                    },
                    authentication: RecordAuthentication {
                        principal_mac: [1; 32],
                        binding_hmac: [2; 32],
                    },
                },
                OperationStatusRecord {
                    kind: OperationKind::Send,
                    value: status,
                },
                0,
            ),
        );
    });
}

fn agent_turn_terminal_result(
    session_id: &str,
    turn_id: &str,
    result: TurnResult,
) -> TerminalResultRecord {
    TerminalResultRecord::AgentTurn {
        kind: match &result {
            TurnResult::Completed { .. } => AgentTerminalKind::Completed,
            TurnResult::Interrupted { reason, .. } => match reason {
                TurnInterruptReason::Abort => AgentTerminalKind::Abort,
                TurnInterruptReason::Timeout => AgentTerminalKind::Timeout,
                TurnInterruptReason::Crash => AgentTerminalKind::Crash,
                TurnInterruptReason::SessionClosed => AgentTerminalKind::SessionClosed,
            },
            TurnResult::Failed { .. } => AgentTerminalKind::Crash,
        },
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        message_id: format!("assistant-{turn_id}"),
        streaming_final_sequence: 0,
        completed_at_bits: 0,
        result: AgentTurnTerminalResultRecord::Current(result),
    }
}

fn pending_send_obligation(
    obligation_id: &str,
    operation_id: &str,
    kind: SendObligationKindRecord,
) -> ObligationRecord {
    ObligationRecord::Send {
        obligation_id: obligation_id.to_string(),
        operation_id: operation_id.to_string(),
        session_id: "s-1".to_string(),
        kind,
        disposition: SendObligationDispositionRecord::StartedTurn,
        human_message_id: Some("human-1".to_string()),
        assistant_message_id: None,
        reserved_turn_id: Some("1".to_string()),
        turn_id: Some("1".to_string()),
        dependency_obligation_ids: Vec::new(),
        canonical_payload: "payload".to_string(),
        state: ObligationStateRecord::Pending,
    }
}

fn send_obligation_in_state(
    obligation_id: &str,
    operation_id: &str,
    kind: SendObligationKindRecord,
    state: ObligationStateRecord,
) -> ObligationRecord {
    let mut record = pending_send_obligation(obligation_id, operation_id, kind);
    let ObligationRecord::Send {
        state: saved_state, ..
    } = &mut record
    else {
        unreachable!("pending send helper returns a send obligation");
    };
    *saved_state = state;
    record
}

/// Recreate the two-obligation shape written before ProviderEstablish was
/// folded into TurnExecution. Recovery must continue to understand shipped
/// databases even though new acceptance never writes this shape.
fn seed_legacy_provider_dependency(repo: &Arc<FakeRepo>, operation_id: &str) {
    let execution_id = format!("{operation_id}.exec");
    let establish_id = format!("{operation_id}.establish");
    repo.with_state(|state| {
        let (execution, _, _) = state
            .obligations
            .get_mut(&execution_id)
            .expect("new execution obligation");
        let mut establishment = execution.clone();
        let ObligationRecord::Send {
            dependency_obligation_ids,
            ..
        } = execution
        else {
            panic!("expected send execution obligation");
        };
        *dependency_obligation_ids = vec![establish_id.clone()];
        let ObligationRecord::Send {
            obligation_id,
            kind,
            dependency_obligation_ids,
            state: obligation_state,
            ..
        } = &mut establishment
        else {
            unreachable!("cloned send obligation");
        };
        *obligation_id = establish_id.clone();
        *kind = SendObligationKindRecord::ProviderEstablish;
        dependency_obligation_ids.clear();
        *obligation_state = ObligationStateRecord::Pending;
        state
            .obligations
            .insert(establish_id.clone(), (establishment, true, 0));

        let (_, status, _) = state
            .records
            .get_mut(&("send".to_string(), operation_id.to_string()))
            .expect("accepted send operation");
        *status = OperationStatusRecord {
            kind: OperationKind::Send,
            value: OperationStatusValue::AwaitingProviderStart {
                dependency_obligation_ids: vec![establish_id],
            },
        };
    });
}

fn test_stop_obligation(
    operation_id: &str,
    session_id: &str,
    turn_id: &str,
    state: ObligationStateRecord,
) -> ObligationRecord {
    ObligationRecord::StopInterrupt {
        operation_id: operation_id.to_string(),
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        expected_revision: 0,
        deadline_ms: 0,
        state,
    }
}

fn recovery_usecase(
    repo: &Arc<FakeRepo>,
    executor: &Arc<FakeRecoveryExecutor>,
) -> RecoveryActionUsecase {
    RecoveryActionUsecase::new(
        repo.clone(),
        Arc::new(FakeAuthority),
        executor.clone(),
        GENERATION.to_string(),
    )
}

// --- Helpers ----------------------------------------------------------------

fn send_usecase(repo: &Arc<FakeRepo>, gate: &Arc<FakeSendGate>) -> AgentSendOperationUsecase {
    AgentSendOperationUsecase::new(
        repo.clone() as Arc<dyn LocalEventTransactionRepository>,
        Arc::new(FakeAuthority),
        gate.clone() as Arc<dyn SendAdmissionGate>,
        GENERATION.to_string(),
    )
}

fn lifecycle_usecase(
    repo: &Arc<FakeRepo>,
    gate: &Arc<FakeLifecycleGate>,
) -> SessionLifecycleOperationUsecase {
    SessionLifecycleOperationUsecase::new(
        repo.clone() as Arc<dyn LocalEventTransactionRepository>,
        Arc::new(FakeAuthority),
        gate.clone() as Arc<dyn SessionLifecycleGate>,
        GENERATION.to_string(),
    )
}

fn stop_usecase(repo: &Arc<FakeRepo>, gate: &Arc<FakeStopGate>) -> StopOperationUsecase {
    StopOperationUsecase::new(
        repo.clone() as Arc<dyn LocalEventTransactionRepository>,
        Arc::new(FakeAuthority),
        gate.clone() as Arc<dyn StopAdmissionGate>,
        GENERATION.to_string(),
    )
}

fn send_request(operation_id: &str, payload: &str) -> SendOperationRequest {
    SendOperationRequest {
        principal: "p-1".to_string(),
        operation_id: operation_id.to_string(),
        canonical_payload: payload.to_string(),
    }
}

fn lifecycle_request(
    request_id: &str,
    session_id: &str,
    revision: i64,
    action: SessionLifecycleAction,
) -> SessionLifecycleRequest {
    SessionLifecycleRequest {
        principal: "p-1".to_string(),
        request_id: request_id.to_string(),
        session_id: session_id.to_string(),
        expected_session_revision: revision,
        action,
    }
}

fn stop_request(
    request_id: &str,
    session_id: &str,
    turn_id: &str,
    revision: u64,
) -> StopOperationRequest {
    StopOperationRequest {
        principal: "p-1".to_string(),
        request_id: request_id.to_string(),
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        expected_session_revision: revision,
    }
}

fn expect_accepted(outcome: SendCommandOutcome) -> super::send::AcceptedSendOperation {
    match outcome {
        SendCommandOutcome::Accepted(accepted) => accepted,
        other => panic!("expected Accepted, got {other:?}"),
    }
}

fn shutdown_gate_failure() -> CommitBatchError {
    CommitBatchError::StorageUnavailable {
        failure: SafeOperationFailure::new(
            SessionOperationFailureKind::PreviousShutdownReconciliationRequired,
            true,
            "Application shutdown is in progress.",
            "shutdown-admission-race",
        ),
    }
}

#[tokio::test]
async fn b059_writer_race_maps_to_typed_shutdown_for_every_durable_operation() {
    let send_repo = FakeRepo::new();
    let send_gate = FakeSendGate::started_turn("shutdown-send-session");
    let send = send_usecase(&send_repo, &send_gate);
    send_repo.with_state(|state| state.fail_commit_once = Some(shutdown_gate_failure()));
    assert_eq!(
        send.send(send_request("shutdown-send", "exact")).await,
        Err(SendAgentMessageError::ShutdownInProgress)
    );
    assert_eq!(send_gate.effect_count(), 0);

    let stop_repo = FakeRepo::new();
    let stop_gate = FakeStopGate::active(0, "1");
    let stop = stop_usecase(&stop_repo, &stop_gate);
    stop_repo.with_state(|state| state.fail_commit_once = Some(shutdown_gate_failure()));
    assert_eq!(
        stop.request(stop_request(
            "shutdown-stop",
            "shutdown-stop-session",
            "1",
            0
        ))
        .await,
        Err(StopOperationError::ShutdownInProgress)
    );
    assert_eq!(stop_gate.interrupt_count(), 0);

    let lifecycle_repo = FakeRepo::new();
    let lifecycle_gate = FakeLifecycleGate::with_snapshot(open_idle_snapshot(0));
    let lifecycle = lifecycle_usecase(&lifecycle_repo, &lifecycle_gate);
    lifecycle_repo.with_state(|state| state.fail_commit_once = Some(shutdown_gate_failure()));
    assert_eq!(
        lifecycle
            .request(lifecycle_request(
                "shutdown-lifecycle",
                "shutdown-lifecycle-session",
                0,
                SessionLifecycleAction::Close,
            ))
            .await,
        Err(SessionLifecycleOperationError::ShutdownInProgress)
    );
    assert_eq!(lifecycle_gate.execution_count(), 0);

    let response = permission_allow("shutdown-permission-request", "{}", "{}");
    let permission_repo = FakeRepo::new();
    let permission_gate = FakePermissionGate::accepting(response.clone());
    let permission = permission_usecase(&permission_repo, &permission_gate);
    permission_repo.with_state(|state| state.fail_commit_once = Some(shutdown_gate_failure()));
    assert_eq!(
        permission
            .request(permission_request("shutdown-permission", response))
            .await,
        Err(PermissionResponseOperationError::ShutdownInProgress)
    );
    assert_eq!(permission_gate.effect_count(), 0);

    let recovery_repo = FakeRepo::new();
    seed_pending_obligation(
        &recovery_repo,
        "permission-response:shutdown-recovery-session:7:permission-1",
        permission_obligation(ObligationStateRecord::Pending, true),
    );
    let recovery_executor = FakeRecoveryExecutor::returning(
        RecoveryResultClassification::Succeeded,
        "must not execute",
    );
    let recovery = recovery_usecase(&recovery_repo, &recovery_executor);
    let recovery_request =
        first_recovery_action(&recovery, RecoveryActionKind::RetrySameEffect).await;
    recovery_repo.with_state(|state| state.fail_commit_once = Some(shutdown_gate_failure()));
    assert_eq!(
        recovery.request(recovery_request).await,
        Err(super::recovery::RecoveryActionError::ShutdownInProgress)
    );
    assert_eq!(recovery_executor.effect_count(), 0);

    let journal_repo = FakeRepo::new();
    let journal = CallerAttemptJournal::new(
        journal_repo.clone(),
        Arc::new(FakeAuthority),
        GENERATION.to_string(),
    );
    journal_repo.with_state(|state| state.fail_commit_once = Some(shutdown_gate_failure()));
    assert_eq!(
        journal
            .record_attempt("p-1", OperationKind::Send, "shutdown-journal", b"exact")
            .await,
        Err(CallerJournalError::ShutdownInProgress)
    );
}

#[tokio::test]
async fn b059_same_send_identity_replays_after_shutdown_gate_closes() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("shutdown-replay-session");
    let usecase = send_usecase(&repo, &gate);
    let request = send_request("shutdown-replay-send", "exact");
    let first = usecase.send(request.clone()).await.unwrap();
    assert!(matches!(first, SendCommandOutcome::Accepted(_)));
    repo.with_state(|state| state.fail_commit = Some(shutdown_gate_failure()));

    let replay = usecase.send(request).await.unwrap();
    assert!(matches!(replay, SendCommandOutcome::Accepted(_)));
    assert_eq!(gate.effect_count(), 1);
}

// --- Send: B-001, B-002/B-003, B-006, B-007, B-009, B-010, B-011, B-014,
// --- B-017, B-099 -----------------------------------------------------------

#[tokio::test]
async fn b001_first_send_returns_immutable_receipt_and_starts_effect_once() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    let accepted = expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());
    assert_eq!(accepted.receipt.operation_id, "op-1");
    assert_eq!(accepted.receipt.session_id, "s-1");
    assert_eq!(accepted.receipt.input_ref, "input-1");
    assert_eq!(
        accepted.receipt.disposition,
        SendDisposition::StartedTurn {
            turn_id: "1".to_string()
        }
    );
    assert_eq!(gate.effect_count(), 1);
    assert_eq!(
        gate.effects.lock().unwrap()[0]
            .assistant_message_id
            .as_deref(),
        Some("human-1:agent")
    );
    repo.with_state(|state| {
        assert!(state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::SendOperationAccepted {
                operation_id,
                ..
            }) if operation_id == "op-1"
        )));
        assert!(state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::ObligationRecorded {
                state: ObligationState::Pending,
                ..
            })
        )));
        assert!(state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::TurnStarted {
                turn_id: 1,
                message_id,
                assistant_message_id: Some(assistant_message_id),
                prompt,
                ..
            }) if message_id == "human-1"
                && assistant_message_id == "human-1:agent"
                && prompt.content == "hello"
        )));
    });
}

#[tokio::test]
async fn b002_b003_same_payload_retry_converges_without_new_effects() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    let first = expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());
    // Restart equivalence: a fresh usecase over the same durable state.
    let restarted = send_usecase(&repo, &gate);
    let second = expect_accepted(restarted.send(send_request("op-1", "hello")).await.unwrap());
    assert_eq!(first.receipt, second.receipt);
    assert_eq!(gate.effect_count(), 1);
    repo.with_state(|state| {
        let accepted_events = state
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    LocalDomainEvent::AgentSession(
                        AgentSessionDomainEvent::SendOperationAccepted { .. }
                    )
                )
            })
            .count();
        assert_eq!(accepted_events, 1);
    });
}

#[tokio::test]
async fn b005_new_session_response_loss_and_restart_replay_the_chosen_session() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("session-new");
    gate.set_plan(Ok(SendPlan {
        session_id: "session-new".to_string(),
        initial_session: Some(
            crate::usecase::agent_session::session::build_new_session_with_id(
                "session-new".to_string(),
                "/tmp/send-new-session-replay",
                Some("codex".to_string()),
                crate::domain::agent_session::PermissionMode::Ask,
                None,
                false,
                false,
                None,
            ),
        ),
        session_projection_guard: RevisionGuard::Absent,
        disposition: SendDisposition::StartedTurn {
            turn_id: "1".to_string(),
        },
        input_ref: "input-new".to_string(),
        human_message_id: "human-new".to_string(),
        prompt: crate::domain::agent_session::events::PromptInput {
            content: "hello".to_string(),
            ..Default::default()
        },
        reserved_turn_id: None,
    }));
    let first = expect_accepted(
        send_usecase(&repo, &gate)
            .send(send_request("op-new", "hello"))
            .await
            .unwrap(),
    );

    gate.set_plan(Err(SafeOperationFailure::new(
        SessionOperationFailureKind::Internal,
        true,
        "a replay must not re-plan a new session",
        "b005-replan",
    )));
    let replay = expect_accepted(
        send_usecase(&repo, &gate)
            .send(send_request("op-new", "hello"))
            .await
            .unwrap(),
    );

    assert_eq!(replay.receipt, first.receipt);
    assert_eq!(replay.receipt.session_id, "session-new");
    assert_eq!(gate.effect_count(), 1);
    repo.with_state(|state| {
        assert_eq!(
            state
                .events
                .iter()
                .filter(|event| matches!(
                    event,
                    LocalDomainEvent::AgentSession(
                        AgentSessionDomainEvent::SendOperationAccepted { operation_id, .. }
                    ) if operation_id == "op-new"
                ))
                .count(),
            1
        );
    });
}

#[tokio::test]
async fn b006_queued_disposition_is_fixed_at_acceptance() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    gate.set_plan(Ok(SendPlan {
        session_id: "s-1".to_string(),
        initial_session: None,
        session_projection_guard: RevisionGuard::Absent,
        disposition: SendDisposition::Queued {
            queue_item_id: "q-1".to_string(),
        },
        input_ref: "input-q".to_string(),
        human_message_id: "human-q".to_string(),
        prompt: crate::domain::agent_session::events::PromptInput {
            content: "queued".to_string(),
            ..Default::default()
        },
        reserved_turn_id: Some("t-9".to_string()),
    }));
    let usecase = send_usecase(&repo, &gate);
    let first = expect_accepted(usecase.send(send_request("op-q", "queued")).await.unwrap());
    assert_eq!(
        first.receipt.disposition,
        SendDisposition::Queued {
            queue_item_id: "q-1".to_string()
        }
    );
    // Even if the runtime would now start a turn, the saved disposition wins.
    gate.set_plan(Ok(SendPlan {
        session_id: "s-1".to_string(),
        initial_session: None,
        session_projection_guard: RevisionGuard::Absent,
        disposition: SendDisposition::StartedTurn {
            turn_id: "2".to_string(),
        },
        input_ref: "other".to_string(),
        human_message_id: "human-other".to_string(),
        prompt: crate::domain::agent_session::events::PromptInput {
            content: "queued".to_string(),
            ..Default::default()
        },
        reserved_turn_id: None,
    }));
    let second = expect_accepted(usecase.send(send_request("op-q", "queued")).await.unwrap());
    assert_eq!(first.receipt, second.receipt);
    assert_eq!(gate.effect_count(), 1);
}

#[tokio::test]
async fn queued_turn_start_prepares_exact_guarded_operation_and_obligation_participants() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    gate.set_plan(Ok(SendPlan {
        session_id: "s-1".to_string(),
        initial_session: None,
        session_projection_guard: RevisionGuard::Absent,
        disposition: SendDisposition::Queued {
            queue_item_id: "q-atomic".to_string(),
        },
        input_ref: "input-q-atomic".to_string(),
        human_message_id: "human-q-atomic".to_string(),
        prompt: crate::domain::agent_session::events::PromptInput {
            content: "atomic queued start".to_string(),
            ..Default::default()
        },
        reserved_turn_id: Some("9".to_string()),
    }));
    let usecase = send_usecase(&repo, &gate);
    expect_accepted(
        usecase
            .send(send_request("op-q-atomic", "queued-payload"))
            .await
            .unwrap(),
    );
    let commits_before = repo.with_state(|state| state.commit_calls);

    let mutations = usecase
        .prepare_queued_turn_start_participant_mutations(
            "op-q-atomic",
            "op-q-atomic.exec",
            "s-1",
            "q-atomic",
            &AgentSessionDomainEvent::TurnStarted {
                turn_id: 9,
                message_id: "human-q-atomic".to_string(),
                assistant_message_id: Some("human-q-atomic:agent".to_string()),
                prompt: crate::domain::agent_session::events::PromptInput {
                    content: "atomic queued start".to_string(),
                    ..Default::default()
                },
                at: 1.0,
            },
        )
        .await
        .unwrap();

    assert_eq!(mutations.len(), 2);
    assert!(mutations.iter().any(|mutation| matches!(
        mutation,
        LocalStateMutation::Obligation(obligation)
            if obligation.obligation_id == "op-q-atomic.exec"
                && obligation.pending.as_ref().is_some_and(|pending| {
                    pending.owner == "s-1"
                        && pending.partition == crate::domain::local_event::PendingPartition::Owner
                })
                && obligation.expected == RevisionGuard::Expected(
                    crate::domain::local_event::Revision::new(0).unwrap()
                )
                && matches!(
                    &obligation.record,
                    ObligationRecord::Send {
                        operation_id,
                        session_id,
                        kind: SendObligationKindRecord::TurnExecution,
                        disposition: SendObligationDispositionRecord::Queued,
                        state: ObligationStateRecord::EffectReserved,
                        ..
                    } if operation_id == "op-q-atomic" && session_id == "s-1"
                )
    )));
    assert!(mutations.iter().any(|mutation| matches!(
        mutation,
        LocalStateMutation::OperationRecord(operation)
            if operation.operation_id == "op-q-atomic"
                && operation.expected == RevisionGuard::Expected(
                    crate::domain::local_event::Revision::new(0).unwrap()
                )
                && matches!(
                    &operation.latest_status.value,
                    OperationStatusValue::ProviderStartReserved { obligation_id }
                        if obligation_id == "op-q-atomic.exec"
                )
    )));
    assert_eq!(
        repo.with_state(|state| state.commit_calls),
        commits_before,
        "participant preparation must not commit outside the session transaction"
    );
}

#[tokio::test]
async fn queued_turn_start_rejects_an_unfinished_legacy_provider_dependency() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    gate.set_plan(Ok(SendPlan {
        session_id: "s-1".to_string(),
        initial_session: None,
        session_projection_guard: RevisionGuard::Absent,
        disposition: SendDisposition::Queued {
            queue_item_id: "q-dependent".to_string(),
        },
        input_ref: "input-q-dependent".to_string(),
        human_message_id: "human-q-dependent".to_string(),
        prompt: crate::domain::agent_session::events::PromptInput {
            content: "queued with legacy dependency".to_string(),
            ..Default::default()
        },
        reserved_turn_id: Some("12".to_string()),
    }));
    let usecase = send_usecase(&repo, &gate);
    expect_accepted(
        usecase
            .send(send_request("op-q-dependent", "queued-payload"))
            .await
            .unwrap(),
    );
    seed_legacy_provider_dependency(&repo, "op-q-dependent");
    repo.with_state(|state| {
        let (_, status, _) = state
            .records
            .get_mut(&("send".to_string(), "op-q-dependent".to_string()))
            .expect("accepted queued operation");
        *status = OperationStatusRecord {
            kind: OperationKind::Send,
            value: OperationStatusValue::Queued {
                queue_item_id: "q-dependent".to_string(),
                reserved_turn_id: "12".to_string(),
            },
        };
    });

    let result = usecase
        .prepare_queued_turn_start_participant_mutations(
            "op-q-dependent",
            "op-q-dependent.exec",
            "s-1",
            "q-dependent",
            &AgentSessionDomainEvent::TurnStarted {
                turn_id: 12,
                message_id: "human-q-dependent".to_string(),
                assistant_message_id: Some("human-q-dependent:agent".to_string()),
                prompt: crate::domain::agent_session::events::PromptInput {
                    content: "queued with legacy dependency".to_string(),
                    ..Default::default()
                },
                at: 1.0,
            },
        )
        .await;

    assert!(matches!(
        result,
        Err(SendAgentMessageError::Internal { .. })
    ));
}

#[tokio::test]
async fn b006_restart_restores_exact_queued_item_without_claiming_provider_effect() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    gate.set_plan(Ok(SendPlan {
        session_id: "s-1".to_string(),
        initial_session: None,
        session_projection_guard: RevisionGuard::Absent,
        disposition: SendDisposition::Queued {
            queue_item_id: "q-restart".to_string(),
        },
        input_ref: "input-q-restart".to_string(),
        human_message_id: "human-q-restart".to_string(),
        prompt: crate::domain::agent_session::events::PromptInput {
            content: "queued across restart".to_string(),
            ..Default::default()
        },
        reserved_turn_id: Some("9".to_string()),
    }));
    let usecase = send_usecase(&repo, &gate);
    let accepted = expect_accepted(
        usecase
            .send(send_request("op-q-restart", "queued-payload"))
            .await
            .unwrap(),
    );
    usecase
        .record_execution_status(
            &accepted.receipt.operation_id,
            SendExecutionStatus::Queued {
                queue_item_id: "q-restart".to_string(),
                reserved_turn_id: "9".to_string(),
            },
        )
        .await
        .unwrap();

    let restart_gate = FakeSendGate::started_turn("s-1");
    let restarted = send_usecase(&repo, &restart_gate);
    assert_eq!(
        restarted
            .recover_pending_provider_effects_pass()
            .await
            .unwrap(),
        1,
        "the first pass installs one process-local queue-restore owner"
    );
    assert_eq!(
        restarted
            .recover_pending_provider_effects_pass()
            .await
            .unwrap(),
        0,
        "the installed owner suppresses another queue-restore worker"
    );
    assert_eq!(
        restarted
            .recover_pending_provider_effects_pass()
            .await
            .unwrap(),
        0,
        "a second zero pass lets the bounded startup worker quiesce"
    );
    let effects = restart_gate.effects();
    assert_eq!(
        effects.len(),
        1,
        "startup recovery schedules the exact queued restore once"
    );
    assert_eq!(
        effects.last().unwrap(),
        &AcceptedSendEffect {
            operation_id: "op-q-restart".to_string(),
            session_id: "s-1".to_string(),
            human_message_id: "human-q-restart".to_string(),
            assistant_message_id: Some("human-q-restart:agent".to_string()),
            disposition: SendDisposition::Queued {
                queue_item_id: "q-restart".to_string(),
            },
            reserved_turn_id: Some("9".to_string()),
            execution_obligation_id: "op-q-restart.exec".to_string(),
            canonical_payload: "queued-payload".to_string(),
        }
    );
    repo.with_state(|state| {
        let (obligation, pending, revision) = state
            .obligations
            .get("op-q-restart.exec")
            .expect("queued execution obligation");
        assert!(*pending);
        assert_eq!(*revision, 0);
        assert!(matches!(
            obligation,
            ObligationRecord::Send {
                state: ObligationStateRecord::Pending,
                ..
            }
        ));
    });
}

#[tokio::test]
async fn legacy_queued_restart_retires_establishment_and_restores_the_exact_queue_item() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    gate.set_plan(Ok(SendPlan {
        session_id: "s-1".to_string(),
        initial_session: None,
        session_projection_guard: RevisionGuard::Absent,
        disposition: SendDisposition::Queued {
            queue_item_id: "q-before-handoff".to_string(),
        },
        input_ref: "input-before-handoff".to_string(),
        human_message_id: "human-before-handoff".to_string(),
        prompt: crate::domain::agent_session::events::PromptInput {
            content: "queued before handoff".to_string(),
            ..Default::default()
        },
        reserved_turn_id: Some("11".to_string()),
    }));
    let usecase = send_usecase(&repo, &gate);
    let accepted = expect_accepted(
        usecase
            .send(send_request("op-before-handoff", "before-handoff-payload"))
            .await
            .unwrap(),
    );
    assert_eq!(
        accepted.latest_status,
        SendExecutionStatus::Queued {
            queue_item_id: "q-before-handoff".to_string(),
            reserved_turn_id: "11".to_string(),
        }
    );
    gate.effects.lock().unwrap().clear();
    seed_legacy_provider_dependency(&repo, "op-before-handoff");

    let restart_gate = FakeSendGate::started_turn("s-1");
    let restarted = send_usecase(&repo, &restart_gate);
    assert_eq!(
        restarted
            .recover_pending_provider_effects_pass()
            .await
            .unwrap(),
        1
    );
    let recovered = restarted
        .get_operation("p-1", "op-before-handoff")
        .await
        .unwrap();
    assert_eq!(
        recovered.latest_status,
        SendExecutionStatus::Queued {
            queue_item_id: "q-before-handoff".to_string(),
            reserved_turn_id: "11".to_string(),
        }
    );
    let effects = restart_gate.effects();
    assert_eq!(effects.len(), 1);
    repo.with_state(|state| {
        let (establishment, establishment_pending, _) = state
            .obligations
            .get("op-before-handoff.establish")
            .expect("legacy establishment obligation");
        assert!(!establishment_pending);
        assert!(matches!(
            establishment,
            ObligationRecord::Send {
                state: ObligationStateRecord::Cancelled,
                ..
            }
        ));
        let (obligation, pending, revision) = state
            .obligations
            .get("op-before-handoff.exec")
            .expect("queued execution obligation");
        assert!(*pending);
        assert_eq!(*revision, 0);
        assert!(matches!(
            obligation,
            ObligationRecord::Send {
                state: ObligationStateRecord::Pending,
                ..
            }
        ));
    });
}

#[tokio::test]
async fn b007_outcome_unknown_keeps_identity_and_allows_same_identity_retry() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    repo.with_state(|state| {
        state.fail_commit_once = Some(CommitBatchError::OutcomeUnknown {
            identity: CommitIdentity::parse("unknown-commit").unwrap(),
        });
    });
    let outcome = usecase.send(send_request("op-1", "hello")).await.unwrap();
    assert_eq!(
        outcome,
        SendCommandOutcome::OutcomeUnknown {
            operation_id: "op-1".to_string()
        }
    );
    assert_eq!(gate.effect_count(), 0);
    // Resolution by retrying the same identity, never a new one.
    let accepted = expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());
    assert_eq!(accepted.receipt.operation_id, "op-1");
    assert_eq!(gate.effect_count(), 1);
}

#[tokio::test]
async fn b007_pending_caller_attempt_is_queryable_only_as_same_identity_outcome_unknown() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    let journal = CallerAttemptJournal::new(
        repo.clone() as Arc<dyn LocalEventTransactionRepository>,
        Arc::new(FakeAuthority),
        GENERATION.to_string(),
    );
    journal
        .record_attempt(
            "p-1",
            OperationKind::Send,
            "op-pending-query",
            b"exact-command",
        )
        .await
        .unwrap();

    assert_eq!(
        usecase.get_operation("p-1", "op-pending-query").await,
        Err(GetSendOperationError::OutcomeUnknown {
            operation_id: "op-pending-query".to_string(),
        })
    );
    assert_eq!(
        usecase.get_operation("p-2", "op-pending-query").await,
        Err(GetSendOperationError::NotFound),
        "a pending attempt must not disclose another principal's identity"
    );
    assert_eq!(gate.effect_count(), 0);

    journal
        .clear_attempt(
            "p-1",
            OperationKind::Send,
            "op-pending-query",
            b"exact-command",
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        usecase.get_operation("p-1", "op-pending-query").await,
        Err(GetSendOperationError::NotFound),
        "a resolved pre-commit rejection proves canonical absence"
    );
}

#[tokio::test]
async fn send_retry_query_failure_keeps_the_same_unknown_identity() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    let accepted = expect_accepted(
        usecase
            .send(send_request("send-query-loss", "hello"))
            .await
            .unwrap(),
    );
    assert_eq!(accepted.receipt.operation_id, "send-query-loss");
    assert_eq!(gate.effect_count(), 1);

    repo.with_state(|state| state.fail_query = true);
    assert_eq!(
        usecase
            .send(send_request("send-query-loss", "hello"))
            .await
            .unwrap(),
        SendCommandOutcome::OutcomeUnknown {
            operation_id: "send-query-loss".to_string(),
        }
    );
    assert_eq!(gate.effect_count(), 1);
}

#[tokio::test]
async fn b004_concurrent_same_installation_send_retries_converge_on_one_receipt() {
    let repo = FakeRepo::new();
    let base = FakeSendGate::started_turn("s-1");
    let gate = RacingSendGate::new(base.plan.lock().unwrap().clone().unwrap());
    let usecase = Arc::new(AgentSendOperationUsecase::new(
        repo.clone() as Arc<dyn LocalEventTransactionRepository>,
        Arc::new(FakeAuthority),
        gate.clone() as Arc<dyn SendAdmissionGate>,
        GENERATION.to_string(),
    ));
    let first_usecase = usecase.clone();
    let first = tokio::spawn(async move {
        first_usecase
            .send(send_request("send-concurrent-same", "hello"))
            .await
            .unwrap()
    });
    gate.first_entered.notified().await;

    let winner = expect_accepted(
        usecase
            .send(send_request("send-concurrent-same", "hello"))
            .await
            .unwrap(),
    );
    gate.release_first.notify_one();
    let delayed = expect_accepted(first.await.unwrap());
    assert_eq!(delayed.receipt, winner.receipt);
    assert_eq!(gate.effects.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn b008_status_update_keeps_receipt_immutable() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    let first = expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());
    usecase
        .record_execution_status(
            "op-1",
            SendExecutionStatus::ReconciliationRequired {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::OutcomeUnknown,
                    true,
                    "provider result unknown",
                    "corr-1",
                ),
            },
        )
        .await
        .unwrap();
    let fetched = usecase.get_operation("p-1", "op-1").await.unwrap();
    assert_eq!(fetched.receipt, first.receipt);
    assert!(matches!(
        fetched.latest_status,
        SendExecutionStatus::ReconciliationRequired { .. }
    ));
}

#[tokio::test]
async fn b009_identity_bounds_are_enforced_with_zero_effects() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    let ok_1 = "a".to_string();
    let ok_128 = "a".repeat(128);
    for id in [ok_1.as_str(), ok_128.as_str()] {
        let outcome = usecase.send(send_request(id, "hello")).await.unwrap();
        assert!(matches!(outcome, SendCommandOutcome::Accepted(_)));
    }
    let too_long = "a".repeat(129);
    for invalid in ["", too_long.as_str(), "オペ", "bad/char", "sp ace"] {
        let commits_before = repo.with_state(|state| state.commit_calls);
        let result = usecase.send(send_request(invalid, "hello")).await;
        assert_eq!(result, Err(SendAgentMessageError::InvalidRequest));
        let commits_after = repo.with_state(|state| state.commit_calls);
        assert_eq!(commits_before, commits_after, "no commit for {invalid:?}");
    }
    assert_eq!(gate.effect_count(), 2);
}

#[tokio::test]
async fn b010_different_payload_same_identity_is_payload_conflict() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    let first = expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());
    let conflict = usecase.send(send_request("op-1", "hello-changed")).await;
    assert_eq!(conflict, Err(SendAgentMessageError::PayloadConflict));
    let fetched = usecase.get_operation("p-1", "op-1").await.unwrap();
    assert_eq!(fetched.receipt, first.receipt);
    assert_eq!(gate.effect_count(), 1);
}

#[tokio::test]
async fn b011_state_change_after_acceptance_is_not_a_conflict() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    let first = expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());
    usecase
        .record_execution_status(
            "op-1",
            SendExecutionStatus::Running {
                turn_id: "1".to_string(),
            },
        )
        .await
        .unwrap();
    let retry = expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());
    assert_eq!(retry.receipt, first.receipt);
    assert!(matches!(
        retry.latest_status,
        SendExecutionStatus::Running { .. }
    ));
    assert_eq!(gate.effect_count(), 1);
}

#[tokio::test]
async fn session_closed_terminal_winner_atomically_finishes_send_and_execution_obligation() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-terminal");
    let usecase = send_usecase(&repo, &gate);
    let accepted = expect_accepted(
        usecase
            .send(send_request("send-terminal", "hello"))
            .await
            .unwrap(),
    );
    usecase
        .transition_obligation(ObligationTransition {
            operation_id: "send-terminal",
            obligation_id: "send-terminal.exec",
            expected_kind: "turn_execution",
            expected_state: "pending",
            next_state: "effect_reserved",
            keep_pending: true,
            status: Some(SendExecutionStatus::ProviderStartReserved {
                obligation_id: "send-terminal.exec".to_string(),
            }),
        })
        .await
        .unwrap();
    usecase
        .mark_turn_running("send-terminal", "send-terminal.exec", 1)
        .await
        .unwrap();
    repo.with_state(|state| {
        let (record, pending, _) = state.obligations.get("send-terminal.exec").unwrap();
        assert!(*pending, "running turn remains a terminal participant");
        assert!(matches!(
            record,
            ObligationRecord::Send {
                state: ObligationStateRecord::EffectReserved,
                ..
            }
        ));
    });

    let terminal_result = crate::domain::agent_session::entities::TurnResult::Failed {
        error: "fatal provider failure".to_string(),
        token_usage: Some(crate::domain::agent_session::entities::TokenUsage {
            input_tokens: 7,
            output_tokens: 11,
            total_tokens: Some(18),
            context_window_tokens: Some(4096),
        }),
    };
    let terminal = TerminalRecordMutation {
        session_id: "s-terminal".to_string(),
        turn_id: "1".to_string(),
        terminal_identity: "terminal-winner-1".to_string(),
        result: TerminalResultRecord::SessionClosed {
            operation_id: "close-terminal".to_string(),
            reason: crate::domain::local_event::TerminalInterruptReasonRecord::SessionClosed,
            result: terminal_result.clone(),
        },
        participant_digest: [9; 32],
    };
    let participants = usecase
        .prepare_runtime_terminal_participants(&terminal)
        .await
        .unwrap();
    assert!(participants.events.is_empty());
    assert_eq!(participants.mutations.len(), 2);
    let mut state_mutations = vec![LocalStateMutation::TerminalRecord(terminal)];
    state_mutations.extend(participants.mutations);
    repo.commit_batch(LocalAtomicBatch {
        commit_id: CommitIdentity::parse("send-terminal-commit").unwrap(),
        idempotency: IdempotencyBinding {
            installation_id: GENERATION.to_string(),
            operation_kind: OperationKind::Send.into(),
            idempotency_key: "send-terminal-final".to_string(),
            payload_hash: fake_hash(2, b"send-terminal-final"),
        },
        expected_heads: Vec::new(),
        events: Vec::new(),
        state_mutations,
    })
    .await
    .unwrap();

    let terminal_view = usecase.get_operation("p-1", "send-terminal").await.unwrap();
    assert_eq!(terminal_view.receipt, accepted.receipt);
    assert_eq!(
        terminal_view.latest_status,
        SendExecutionStatus::Terminal {
            result: terminal_result,
        }
    );
    repo.with_state(|state| {
        let (record, pending, _) = state.obligations.get("send-terminal.exec").unwrap();
        assert!(!pending);
        assert!(matches!(
            record,
            ObligationRecord::Send {
                state: ObligationStateRecord::Completed,
                ..
            }
        ));
    });

    // A late runtime failure path cannot regress the terminal winner.
    usecase
        .record_execution_status(
            "send-terminal",
            SendExecutionStatus::ReconciliationRequired {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::OutcomeUnknown,
                    true,
                    "late status",
                    "late-status",
                ),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        usecase
            .get_operation("p-1", "send-terminal")
            .await
            .unwrap()
            .latest_status,
        SendExecutionStatus::Terminal { .. }
    ));
}

#[tokio::test]
async fn provider_terminal_wins_after_running_status_persist_failure_requires_reconciliation() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-terminal-after-reconciliation");
    let usecase = send_usecase(&repo, &gate);
    expect_accepted(
        usecase
            .send(send_request("send-terminal-after-reconciliation", "hello"))
            .await
            .unwrap(),
    );
    usecase
        .transition_obligation(ObligationTransition {
            operation_id: "send-terminal-after-reconciliation",
            obligation_id: "send-terminal-after-reconciliation.exec",
            expected_kind: "turn_execution",
            expected_state: "pending",
            next_state: "effect_reserved",
            keep_pending: true,
            status: Some(SendExecutionStatus::ProviderStartReserved {
                obligation_id: "send-terminal-after-reconciliation.exec".to_string(),
            }),
        })
        .await
        .unwrap();

    // The provider has accepted the turn, but persisting Running fails. The
    // worker must publish the ambiguous handoff as reconciliation work while
    // retaining the exact turn identity for a later authoritative terminal.
    repo.with_state(|state| {
        state.fail_commit_once = Some(CommitBatchError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                true,
                "running status unavailable",
                "running-status-unavailable",
            ),
        });
    });
    assert!(usecase
        .mark_turn_running(
            "send-terminal-after-reconciliation",
            "send-terminal-after-reconciliation.exec",
            1,
        )
        .await
        .is_err());
    usecase
        .mark_turn_reconciliation_required(
            "send-terminal-after-reconciliation",
            "send-terminal-after-reconciliation.exec",
            SafeOperationFailure::new(
                SessionOperationFailureKind::OutcomeUnknown,
                true,
                "The accepted turn start requires same-effect readback.",
                "running-status-reconciliation",
            ),
        )
        .await
        .unwrap();
    assert!(matches!(
        usecase
            .get_operation("p-1", "send-terminal-after-reconciliation")
            .await
            .unwrap()
            .latest_status,
        SendExecutionStatus::ReconciliationRequired { .. }
    ));
    repo.with_state(|state| {
        let (record, pending, _) = state
            .obligations
            .get("send-terminal-after-reconciliation.exec")
            .unwrap();
        assert!(*pending);
        assert!(matches!(
            record,
            ObligationRecord::Send {
                state: ObligationStateRecord::ReconciliationRequired,
                turn_id: Some(turn_id),
                ..
            } if turn_id == "1"
        ));
    });

    let terminal_result = TurnResult::Completed {
        stop_reason: None,
        token_usage: None,
    };
    let terminal = TerminalRecordMutation {
        session_id: "s-terminal-after-reconciliation".to_string(),
        turn_id: "1".to_string(),
        terminal_identity: "terminal-after-reconciliation-winner".to_string(),
        result: agent_turn_terminal_result(
            "s-terminal-after-reconciliation",
            "1",
            terminal_result.clone(),
        ),
        participant_digest: [6; 32],
    };
    let participants = usecase
        .prepare_runtime_terminal_participants(&terminal)
        .await
        .unwrap();
    assert_eq!(participants.mutations.len(), 2);
    let mut state_mutations = vec![LocalStateMutation::TerminalRecord(terminal)];
    state_mutations.extend(participants.mutations);
    repo.commit_batch(LocalAtomicBatch {
        commit_id: CommitIdentity::parse("send-terminal-after-reconciliation-commit").unwrap(),
        idempotency: IdempotencyBinding {
            installation_id: GENERATION.to_string(),
            operation_kind: OperationKind::Send.into(),
            idempotency_key: "send-terminal-after-reconciliation-final".to_string(),
            payload_hash: fake_hash(2, b"send-terminal-after-reconciliation-final"),
        },
        expected_heads: Vec::new(),
        events: Vec::new(),
        state_mutations,
    })
    .await
    .unwrap();

    assert_eq!(
        usecase
            .get_operation("p-1", "send-terminal-after-reconciliation")
            .await
            .unwrap()
            .latest_status,
        SendExecutionStatus::Terminal {
            result: terminal_result,
        }
    );
    repo.with_state(|state| {
        let (record, pending, _) = state
            .obligations
            .get("send-terminal-after-reconciliation.exec")
            .unwrap();
        assert!(!pending);
        assert!(matches!(
            record,
            ObligationRecord::Send {
                state: ObligationStateRecord::Completed,
                ..
            }
        ));
    });
}

#[tokio::test]
async fn f05_runtime_terminal_participant_replaces_recovery_wrapper_with_plain_completed_send() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-terminal-wrapper");
    let usecase = send_usecase(&repo, &gate);
    expect_accepted(
        usecase
            .send(send_request("send-terminal-wrapper", "hello"))
            .await
            .unwrap(),
    );
    usecase
        .transition_obligation(ObligationTransition {
            operation_id: "send-terminal-wrapper",
            obligation_id: "send-terminal-wrapper.exec",
            expected_kind: "turn_execution",
            expected_state: "pending",
            next_state: "effect_reserved",
            keep_pending: true,
            status: Some(SendExecutionStatus::ProviderStartReserved {
                obligation_id: "send-terminal-wrapper.exec".to_string(),
            }),
        })
        .await
        .unwrap();
    repo.with_state(|state| {
        let (record, _, revision) = state
            .obligations
            .get_mut("send-terminal-wrapper.exec")
            .unwrap();
        let original = record.clone();
        *record = ObligationRecord::RecoveryTransition {
            original: Box::new(original),
            recovery_action: ObligationRecoveryActionRecord {
                action_id: "terminal-wrapper-readback".to_string(),
                origin_revision: *revision as u64,
                action: RecoveryActionKind::ReadAgain,
                effect_identity: "send-terminal-wrapper.exec".to_string(),
                state: ObligationStateRecord::EffectReserved,
                classification: None,
            },
        };
    });

    let participants = usecase
        .prepare_runtime_terminal_participants(&TerminalRecordMutation {
            session_id: "s-terminal-wrapper".to_string(),
            turn_id: "1".to_string(),
            terminal_identity: "terminal-wrapper-winner".to_string(),
            result: agent_turn_terminal_result(
                "s-terminal-wrapper",
                "1",
                TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                },
            ),
            participant_digest: [5; 32],
        })
        .await
        .unwrap();

    let completed = participants
        .mutations
        .iter()
        .find_map(|mutation| match mutation {
            LocalStateMutation::Obligation(mutation) => Some(&mutation.record),
            _ => None,
        })
        .expect("terminal execution obligation participant");
    assert!(matches!(
        completed,
        ObligationRecord::Send {
            obligation_id,
            operation_id,
            state: ObligationStateRecord::Completed,
            ..
        } if obligation_id == "send-terminal-wrapper.exec"
            && operation_id == "send-terminal-wrapper"
    ));
}

#[tokio::test]
async fn terminal_operation_converges_nonterminal_execution_without_status_regression() {
    for reserved in [false, true] {
        let boundary = if reserved { "reserved" } else { "pending" };
        let operation_id = format!("terminal-mismatch-{boundary}");
        let obligation_id = format!("{operation_id}.exec");
        let session_id = format!("terminal-session-{boundary}");
        let repo = FakeRepo::new();
        let gate = FakeSendGate::started_turn(&session_id);
        let usecase = send_usecase(&repo, &gate);
        expect_accepted(
            usecase
                .send(send_request(&operation_id, "hello"))
                .await
                .unwrap(),
        );
        gate.effects.lock().unwrap().clear();
        if reserved {
            usecase
                .transition_obligation(ObligationTransition {
                    operation_id: &operation_id,
                    obligation_id: &obligation_id,
                    expected_kind: "turn_execution",
                    expected_state: "pending",
                    next_state: "effect_reserved",
                    keep_pending: true,
                    status: Some(SendExecutionStatus::ProviderStartReserved {
                        obligation_id: obligation_id.clone(),
                    }),
                })
                .await
                .unwrap();
        }
        let terminal_result = TurnResult::Completed {
            stop_reason: None,
            token_usage: None,
        };
        usecase
            .record_execution_status(
                &operation_id,
                SendExecutionStatus::Terminal {
                    result: terminal_result.clone(),
                },
            )
            .await
            .unwrap();

        if !reserved {
            usecase
                .mark_turn_reconciliation_required(
                    &operation_id,
                    &obligation_id,
                    SafeOperationFailure::new(
                        SessionOperationFailureKind::OutcomeUnknown,
                        true,
                        "A late mismatch must not replace Terminal.",
                        format!("terminal-mismatch-{boundary}"),
                    ),
                )
                .await
                .unwrap();
        }
        assert_eq!(
            usecase
                .get_operation("p-1", &operation_id)
                .await
                .unwrap()
                .latest_status,
            SendExecutionStatus::Terminal {
                result: terminal_result.clone(),
            },
            "{boundary}"
        );

        assert_eq!(
            usecase
                .recover_pending_provider_effects_pass()
                .await
                .unwrap(),
            usize::from(reserved),
            "{boundary}"
        );
        assert_eq!(
            usecase
                .recover_pending_provider_effects_pass()
                .await
                .unwrap(),
            0,
            "{boundary}"
        );
        assert_eq!(gate.effect_count(), 0, "{boundary}");
        assert_eq!(
            usecase
                .get_operation("p-1", &operation_id)
                .await
                .unwrap()
                .latest_status,
            SendExecutionStatus::Terminal {
                result: terminal_result,
            },
            "{boundary}"
        );
        repo.with_state(|state| {
            let (obligation, pending, _) = &state.obligations[&obligation_id];
            assert!(!*pending, "{boundary}");
            assert!(matches!(
                obligation,
                ObligationRecord::Send {
                    state: ObligationStateRecord::Completed,
                    ..
                }
            ));
        });
    }
}

#[tokio::test]
async fn queued_send_carries_reserved_turn_identity_into_terminal_participation() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-queued-terminal");
    gate.set_plan(Ok(SendPlan {
        session_id: "s-queued-terminal".to_string(),
        initial_session: None,
        session_projection_guard: RevisionGuard::Absent,
        disposition: SendDisposition::Queued {
            queue_item_id: "queue-terminal".to_string(),
        },
        input_ref: "input-queued".to_string(),
        human_message_id: "human-queued".to_string(),
        prompt: crate::domain::agent_session::events::PromptInput {
            content: "queued".to_string(),
            ..Default::default()
        },
        reserved_turn_id: Some("7".to_string()),
    }));
    let usecase = send_usecase(&repo, &gate);
    expect_accepted(
        usecase
            .send(send_request("send-queued-terminal", "queued"))
            .await
            .unwrap(),
    );
    repo.with_state(|state| {
        let (record, pending, _) = state.obligations.get("send-queued-terminal.exec").unwrap();
        assert!(*pending);
        let ObligationRecord::Send {
            turn_id,
            reserved_turn_id,
            ..
        } = record
        else {
            panic!("expected send obligation");
        };
        assert_eq!(turn_id.as_deref(), Some("7"));
        assert_eq!(reserved_turn_id.as_deref(), Some("7"));
    });
    usecase
        .transition_obligation(ObligationTransition {
            operation_id: "send-queued-terminal",
            obligation_id: "send-queued-terminal.exec",
            expected_kind: "turn_execution",
            expected_state: "pending",
            next_state: "effect_reserved",
            keep_pending: true,
            status: Some(SendExecutionStatus::ProviderStartReserved {
                obligation_id: "send-queued-terminal.exec".to_string(),
            }),
        })
        .await
        .unwrap();
    usecase
        .mark_turn_running("send-queued-terminal", "send-queued-terminal.exec", 7)
        .await
        .unwrap();
    let terminal = TerminalRecordMutation {
        session_id: "s-queued-terminal".to_string(),
        turn_id: "7".to_string(),
        terminal_identity: "queued-terminal-winner".to_string(),
        result: agent_turn_terminal_result(
            "s-queued-terminal",
            "7",
            TurnResult::Completed {
                stop_reason: None,
                token_usage: None,
            },
        ),
        participant_digest: [7; 32],
    };
    let participants = usecase
        .prepare_runtime_terminal_participants(&terminal)
        .await
        .unwrap();
    assert_eq!(participants.mutations.len(), 2);
}

#[tokio::test]
async fn b014_rejected_before_commit_has_zero_effects() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    repo.with_state(|state| {
        state.fail_commit = Some(CommitBatchError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                true,
                "disk gone",
                "corr-storage",
            ),
        });
    });
    let outcome = usecase.send(send_request("op-1", "hello")).await.unwrap();
    assert!(matches!(
        outcome,
        SendCommandOutcome::RejectedBeforeCommit { .. }
    ));
    assert_eq!(gate.effect_count(), 0);
    repo.with_state(|state| {
        assert!(state.records.is_empty());
        assert!(state.events.is_empty());
    });
}

#[tokio::test]
async fn accepted_dispatch_refusal_atomically_exposes_reconciliation_without_provider_io() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    gate.fail_dispatch(SafeOperationFailure::new(
        SessionOperationFailureKind::InvalidEffectIntent,
        false,
        "The accepted provider effect could not be scheduled.",
        "dispatch-refused",
    ));
    let usecase = send_usecase(&repo, &gate);

    let accepted = expect_accepted(
        usecase
            .send(send_request("op-dispatch-refused", "hello"))
            .await
            .unwrap(),
    );

    assert!(matches!(
        accepted.latest_status,
        SendExecutionStatus::ReconciliationRequired { .. }
    ));
    assert_eq!(gate.effect_count(), 0);
    repo.with_state(|state| {
        assert!(matches!(
            &state.obligations["op-dispatch-refused.exec"].0,
            ObligationRecord::Send {
                state: ObligationStateRecord::ReconciliationRequired,
                ..
            }
        ));
        assert!(state.obligations["op-dispatch-refused.exec"].1);
    });
}

#[tokio::test]
async fn accepted_dispatch_reconciliation_keeps_a_background_retry_owner_after_first_commit_failure(
) {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-dispatch-retry");
    let dispatch_failure = SafeOperationFailure::new(
        SessionOperationFailureKind::InvalidEffectIntent,
        false,
        "The accepted provider effect could not be scheduled.",
        "dispatch-retry-owner",
    );
    gate.fail_dispatch(dispatch_failure.clone());
    repo.with_state(|state| {
        state.fail_commit_on_call = Some((
            2,
            CommitBatchError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "The first reconciliation commit is unavailable.",
                    "dispatch-reconciliation-first-commit",
                ),
            },
        ));
    });
    let usecase = send_usecase(&repo, &gate);

    let accepted = expect_accepted(
        usecase
            .send(send_request("op-dispatch-retry", "hello"))
            .await
            .unwrap(),
    );
    assert!(matches!(
        accepted.latest_status,
        SendExecutionStatus::ReconciliationRequired { .. }
    ));

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let converged = repo.with_state(|state| {
                let (_, status, _) =
                    &state.records[&("send".to_string(), "op-dispatch-retry".to_string())];
                let (obligation, pending, _) = &state.obligations["op-dispatch-retry.exec"];
                matches!(
                    &status.value,
                    OperationStatusValue::ReconciliationRequired { failure }
                        if failure == &dispatch_failure
                ) && *pending
                    && matches!(
                        obligation,
                        ObligationRecord::Send {
                            state: ObligationStateRecord::ReconciliationRequired,
                            ..
                        }
                    )
            });
            if converged {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the background owner must durably converge reconciliation");
    assert_eq!(gate.effect_count(), 0);
    repo.with_state(|state| assert!(state.commit_calls >= 3));
}

#[tokio::test]
async fn accepted_readback_failure_wakes_central_recovery_for_the_same_effect() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-readback-retry");
    repo.with_state(|state| {
        state.fail_operation_query_once_after_commit_call = Some(1);
    });
    let usecase = send_usecase(&repo, &gate);
    let recovery_wakeup = usecase.pending_recovery_wakeup();

    let accepted = expect_accepted(
        usecase
            .send(send_request("op-readback-retry", "hello"))
            .await
            .unwrap(),
    );
    assert!(matches!(
        accepted.latest_status,
        SendExecutionStatus::AwaitingProviderStart { .. }
    ));
    assert_eq!(
        gate.effect_count(),
        0,
        "an unreadable acceptance must not dispatch provider I/O"
    );

    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        recovery_wakeup.notified(),
    )
    .await
    .expect("the acceptance readback failure must wake the central recovery owner");
    assert_eq!(
        usecase
            .recover_pending_provider_effects_pass()
            .await
            .unwrap(),
        1,
        "the central owner must reconstruct the effect from durable state"
    );
    assert_eq!(gate.effect_count(), 1);
    repo.with_state(|state| {
        let (_, status, _) = &state.records[&("send".to_string(), "op-readback-retry".to_string())];
        let (obligation, pending, _) = &state.obligations["op-readback-retry.exec"];
        assert!(matches!(
            status.value,
            OperationStatusValue::AwaitingProviderStart { .. }
        ));
        assert!(*pending);
        assert!(matches!(
            obligation,
            ObligationRecord::Send {
                state: ObligationStateRecord::Pending,
                ..
            }
        ));
    });
}

#[tokio::test]
async fn b014_gate_plan_failure_rejects_before_commit() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    gate.set_plan(Err(SafeOperationFailure::new(
        SessionOperationFailureKind::PersistFailure,
        true,
        "input save failed",
        "corr-plan",
    )));
    let usecase = send_usecase(&repo, &gate);
    let outcome = usecase.send(send_request("op-1", "hello")).await.unwrap();
    assert!(matches!(
        outcome,
        SendCommandOutcome::RejectedBeforeCommit { .. }
    ));
    assert_eq!(gate.effect_count(), 0);
    repo.with_state(|state| assert!(state.records.is_empty()));
}

#[tokio::test]
async fn b075_turn_identity_capacity_is_typed_and_changes_nothing() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    gate.set_plan(Err(SafeOperationFailure::new(
        SessionOperationFailureKind::CapacityExceeded,
        false,
        "turn identity capacity exhausted",
        "turn-capacity",
    )));
    let usecase = send_usecase(&repo, &gate);

    assert_eq!(
        usecase.send(send_request("op-turn-max", "hello")).await,
        Err(SendAgentMessageError::CapacityExceeded)
    );
    assert_eq!(gate.effect_count(), 0);
    repo.with_state(|state| {
        assert_eq!(state.commit_calls, 0);
        assert!(state.records.is_empty());
        assert!(state.events.is_empty());
        assert!(state.obligations.is_empty());
    });
}

#[tokio::test]
async fn b017_new_send_uses_only_turn_execution_obligation() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    gate.set_plan(Ok(SendPlan {
        session_id: "s-1".to_string(),
        initial_session: None,
        session_projection_guard: RevisionGuard::Absent,
        disposition: SendDisposition::StartedTurn {
            turn_id: "1".to_string(),
        },
        input_ref: "input-1".to_string(),
        human_message_id: "human-1".to_string(),
        prompt: crate::domain::agent_session::events::PromptInput {
            content: "hello".to_string(),
            ..Default::default()
        },
        reserved_turn_id: None,
    }));
    let usecase = send_usecase(&repo, &gate);
    let accepted = expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());
    match &accepted.latest_status {
        SendExecutionStatus::AwaitingProviderStart {
            dependency_obligation_ids,
        } => assert!(dependency_obligation_ids.is_empty()),
        other => panic!("expected AwaitingProviderStart, got {other:?}"),
    }
    repo.with_state(|state| {
        assert_eq!(state.obligations.len(), 1);
        let (record, pending, revision) = state
            .obligations
            .get("op-1.exec")
            .expect("single turn execution obligation");
        assert!(*pending);
        assert_eq!(*revision, 0);
        assert!(matches!(
            record,
            ObligationRecord::Send {
                kind: SendObligationKindRecord::TurnExecution,
                dependency_obligation_ids,
                state: ObligationStateRecord::Pending,
                ..
            } if dependency_obligation_ids.is_empty()
        ));
    });
    let effect = gate.effects.lock().unwrap()[0].clone();
    assert_eq!(effect.execution_obligation_id, "op-1.exec");
}

#[tokio::test]
async fn b017_unestablished_provider_does_not_block_turn_execution_reservation() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    gate.set_plan(Ok(SendPlan {
        session_id: "s-1".to_string(),
        initial_session: None,
        session_projection_guard: RevisionGuard::Absent,
        disposition: SendDisposition::StartedTurn {
            turn_id: "1".to_string(),
        },
        input_ref: "input-1".to_string(),
        human_message_id: "human-1".to_string(),
        prompt: crate::domain::agent_session::events::PromptInput {
            content: "hello".to_string(),
            ..Default::default()
        },
        reserved_turn_id: None,
    }));
    let usecase = send_usecase(&repo, &gate);
    expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());

    usecase
        .transition_obligation(ObligationTransition {
            operation_id: "op-1",
            obligation_id: "op-1.exec",
            expected_kind: "turn_execution",
            expected_state: "pending",
            next_state: "effect_reserved",
            keep_pending: true,
            status: None,
        })
        .await
        .unwrap();
    repo.with_state(|state| {
        assert!(!state.obligations.contains_key("op-1.establish"));
        assert!(state.obligations["op-1.exec"].1);
        assert!(matches!(
            &state.obligations["op-1.exec"].0,
            ObligationRecord::Send {
                dependency_obligation_ids,
                state: ObligationStateRecord::EffectReserved,
                ..
            } if dependency_obligation_ids.is_empty()
        ));
    });
}

#[tokio::test]
async fn exact_turn_claim_owner_recovers_its_lost_reply_but_rejects_a_concurrent_loser() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    expect_accepted(
        usecase
            .send(send_request("op-exact-claim", "hello"))
            .await
            .unwrap(),
    );
    repo.with_state(|state| {
        state.outcome_unknown_after_commit_on_call = Some(state.commit_calls + 1);
    });

    assert!(usecase
        .claim_turn_execution("op-exact-claim", "op-exact-claim.exec", "exact-owner-a",)
        .await
        .is_err());
    assert_eq!(
        usecase
            .claim_turn_execution("op-exact-claim", "op-exact-claim.exec", "exact-owner-a",)
            .await
            .unwrap(),
        ObligationTransitionOutcome::Applied,
        "the exact owner must resolve its committed claim after losing the reply"
    );
    assert_eq!(
        usecase
            .claim_turn_execution(
                "op-exact-claim",
                "op-exact-claim.exec",
                "concurrent-owner-b",
            )
            .await
            .unwrap(),
        ObligationTransitionOutcome::AlreadyAtTarget,
        "a different owner must not inherit the winner's provider authority"
    );
}

#[tokio::test]
async fn concurrent_turn_claim_allows_only_the_commit_winner_to_handoff() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = Arc::new(send_usecase(&repo, &gate));
    expect_accepted(
        usecase
            .send(send_request("op-claim-race", "hello"))
            .await
            .unwrap(),
    );
    gate.effects.lock().unwrap().clear();
    *repo.commit_barrier.lock().unwrap() = Some(Arc::new(tokio::sync::Barrier::new(2)));

    let claim = || ObligationTransition {
        operation_id: "op-claim-race",
        obligation_id: "op-claim-race.exec",
        expected_kind: "turn_execution",
        expected_state: "pending",
        next_state: "effect_reserved",
        keep_pending: true,
        status: Some(SendExecutionStatus::ProviderStartReserved {
            obligation_id: "op-claim-race.exec".to_string(),
        }),
    };
    let (first, second) = tokio::join!(
        usecase.transition_obligation(claim()),
        usecase.transition_obligation(claim())
    );
    *repo.commit_barrier.lock().unwrap() = None;
    let outcomes = [first.unwrap(), second.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == ObligationTransitionOutcome::Applied)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == ObligationTransitionOutcome::AlreadyAtTarget)
            .count(),
        1
    );

    for outcome in outcomes {
        if outcome == ObligationTransitionOutcome::Applied {
            gate.start_provider_effect(&AcceptedSendEffect {
                operation_id: "op-claim-race".to_string(),
                session_id: "s-1".to_string(),
                human_message_id: "human-1".to_string(),
                assistant_message_id: Some("human-1:agent".to_string()),
                disposition: SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                reserved_turn_id: None,
                execution_obligation_id: "op-claim-race.exec".to_string(),
                canonical_payload: "{}".to_string(),
            })
            .await
            .unwrap();
        }
    }
    assert_eq!(gate.effect_count(), 1);
}

#[tokio::test]
async fn turn_reconciliation_updates_operation_and_obligation_atomically() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());
    usecase
        .transition_obligation(ObligationTransition {
            operation_id: "op-1",
            obligation_id: "op-1.exec",
            expected_kind: "turn_execution",
            expected_state: "pending",
            next_state: "effect_reserved",
            keep_pending: true,
            status: Some(SendExecutionStatus::ProviderStartReserved {
                obligation_id: "op-1.exec".to_string(),
            }),
        })
        .await
        .unwrap();

    let failure = SafeOperationFailure::new(
        SessionOperationFailureKind::OutcomeUnknown,
        true,
        "turn result requires readback",
        "turn-reconciliation-1",
    );
    repo.with_state(|state| {
        state.outcome_unknown_after_commit_on_call = Some(state.commit_calls + 1);
    });
    usecase
        .mark_turn_reconciliation_required("op-1", "op-1.exec", failure.clone())
        .await
        .unwrap();

    let commits_after_first = repo.with_state(|state| {
        assert_eq!(
            state
                .committed_batches
                .last()
                .expect("reconciliation batch")
                .state_mutations
                .len(),
            2
        );
        let (_, status, operation_revision) =
            &state.records[&("send".to_string(), "op-1".to_string())];
        assert_eq!(*operation_revision, 2);
        assert!(matches!(
            &status.value,
            OperationStatusValue::ReconciliationRequired {
                failure: stored_failure,
            } if stored_failure == &failure
        ));
        let (obligation, pending, obligation_revision) = &state.obligations["op-1.exec"];
        assert!(*pending);
        assert_eq!(*obligation_revision, 2);
        assert!(matches!(
            obligation,
            ObligationRecord::Send {
                state: ObligationStateRecord::ReconciliationRequired,
                ..
            }
        ));
        state.commit_calls
    });

    usecase
        .mark_turn_reconciliation_required("op-1", "op-1.exec", failure)
        .await
        .unwrap();
    repo.with_state(|state| assert_eq!(state.commit_calls, commits_after_first));
}

#[tokio::test]
async fn turn_reconciliation_preserves_completed_recovery_history() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    expect_accepted(
        usecase
            .send(send_request("op-recovery-history", "hello"))
            .await
            .unwrap(),
    );
    usecase
        .transition_obligation(ObligationTransition {
            operation_id: "op-recovery-history",
            obligation_id: "op-recovery-history.exec",
            expected_kind: "turn_execution",
            expected_state: "pending",
            next_state: "effect_reserved",
            keep_pending: true,
            status: Some(SendExecutionStatus::ProviderStartReserved {
                obligation_id: "op-recovery-history.exec".to_string(),
            }),
        })
        .await
        .unwrap();
    repo.with_state(|state| {
        let (record, _, revision) = state
            .obligations
            .get_mut("op-recovery-history.exec")
            .unwrap();
        let original = record.clone();
        *record = ObligationRecord::RecoveryTransition {
            original: Box::new(original),
            recovery_action: ObligationRecoveryActionRecord {
                action_id: "completed-readback".to_string(),
                origin_revision: *revision as u64,
                action: RecoveryActionKind::ReadAgain,
                effect_identity: "op-recovery-history.exec".to_string(),
                state: ObligationStateRecord::Completed,
                classification: Some(RecoveryResultClassification::Unchanged),
            },
        };
    });

    usecase
        .mark_turn_reconciliation_required(
            "op-recovery-history",
            "op-recovery-history.exec",
            SafeOperationFailure::new(
                SessionOperationFailureKind::OutcomeUnknown,
                true,
                "turn requires readback",
                "preserve-recovery-history",
            ),
        )
        .await
        .unwrap();

    repo.with_state(|state| {
        assert!(matches!(
            &state.obligations["op-recovery-history.exec"].0,
            ObligationRecord::RecoveryTransition {
                original,
                recovery_action: ObligationRecoveryActionRecord {
                    action_id,
                    state: ObligationStateRecord::Completed,
                    ..
                },
            } if action_id == "completed-readback"
                && matches!(
                    original.as_ref(),
                    ObligationRecord::Send {
                        state: ObligationStateRecord::ReconciliationRequired,
                        ..
                    }
                )
        ));
    });
}

#[tokio::test]
async fn turn_reconciliation_repairs_legacy_operation_only_split_brain() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());
    usecase
        .transition_obligation(ObligationTransition {
            operation_id: "op-1",
            obligation_id: "op-1.exec",
            expected_kind: "turn_execution",
            expected_state: "pending",
            next_state: "effect_reserved",
            keep_pending: true,
            status: None,
        })
        .await
        .unwrap();
    let original_failure = SafeOperationFailure::new(
        SessionOperationFailureKind::OutcomeUnknown,
        true,
        "legacy operation-only reconciliation",
        "legacy-reconciliation",
    );
    usecase
        .record_execution_status(
            "op-1",
            SendExecutionStatus::ReconciliationRequired {
                failure: original_failure.clone(),
            },
        )
        .await
        .unwrap();

    usecase
        .mark_turn_reconciliation_required(
            "op-1",
            "op-1.exec",
            SafeOperationFailure::new(
                SessionOperationFailureKind::ExternalEffectFailed,
                true,
                "new failure must not replace the canonical one",
                "new-reconciliation",
            ),
        )
        .await
        .unwrap();

    repo.with_state(|state| {
        let (_, status, operation_revision) =
            &state.records[&("send".to_string(), "op-1".to_string())];
        assert_eq!(*operation_revision, 1);
        assert!(matches!(
            &status.value,
            OperationStatusValue::ReconciliationRequired { failure }
                if failure == &original_failure
        ));
        let (obligation, pending, obligation_revision) = &state.obligations["op-1.exec"];
        assert!(*pending);
        assert_eq!(*obligation_revision, 2);
        assert!(matches!(
            obligation,
            ObligationRecord::Send {
                state: ObligationStateRecord::ReconciliationRequired,
                ..
            }
        ));
        assert_eq!(
            state
                .committed_batches
                .last()
                .expect("split-brain repair batch")
                .state_mutations
                .len(),
            1
        );
    });
}

#[tokio::test]
async fn turn_reconciliation_cannot_regress_a_terminal_winner() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());
    usecase
        .transition_obligation(ObligationTransition {
            operation_id: "op-1",
            obligation_id: "op-1.exec",
            expected_kind: "turn_execution",
            expected_state: "pending",
            next_state: "effect_reserved",
            keep_pending: true,
            status: None,
        })
        .await
        .unwrap();
    let terminal = TurnResult::Failed {
        error: "terminal winner".to_string(),
        token_usage: None,
    };
    usecase
        .record_execution_status(
            "op-1",
            SendExecutionStatus::Terminal {
                result: terminal.clone(),
            },
        )
        .await
        .unwrap();
    let commits_before = repo.with_state(|state| state.commit_calls);

    usecase
        .mark_turn_reconciliation_required(
            "op-1",
            "op-1.exec",
            SafeOperationFailure::new(
                SessionOperationFailureKind::OutcomeUnknown,
                true,
                "late reconciliation",
                "late-reconciliation",
            ),
        )
        .await
        .unwrap();

    assert_eq!(
        usecase
            .get_operation("p-1", "op-1")
            .await
            .unwrap()
            .latest_status,
        SendExecutionStatus::Terminal { result: terminal }
    );
    repo.with_state(|state| {
        assert_eq!(state.commit_calls, commits_before + 1);
        assert!(matches!(
            &state.obligations["op-1.exec"].0,
            ObligationRecord::Send {
                state: ObligationStateRecord::Completed,
                ..
            }
        ));
        assert!(!state.obligations["op-1.exec"].1);
    });
}

#[tokio::test]
async fn accepted_reserved_provider_effect_is_not_blindly_retried_after_restart() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    let accepted = expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());
    usecase
        .record_execution_status(
            &accepted.receipt.operation_id,
            SendExecutionStatus::ProviderStartReserved {
                obligation_id: "op-1.exec".to_string(),
            },
        )
        .await
        .unwrap();

    repo.with_state(|state| {
        let (_, status, operation_revision) = state
            .records
            .get(&("send".to_string(), "op-1".to_string()))
            .expect("accepted operation record");
        assert_eq!(*operation_revision, 1);
        assert!(matches!(
            &status.value,
            crate::domain::local_event::OperationStatusValue::ProviderStartReserved { .. }
        ));
        let (obligation, pending, obligation_revision) = state
            .obligations
            .get("op-1.exec")
            .expect("accepted execution obligation");
        assert_eq!(*obligation_revision, 1);
        assert!(*pending);
        assert!(matches!(
            obligation,
            ObligationRecord::Send {
                state: ObligationStateRecord::EffectReserved,
                ..
            }
        ));
    });

    let restarted = send_usecase(&repo, &gate);
    assert_eq!(
        restarted
            .recover_pending_provider_effects_pass()
            .await
            .unwrap(),
        1,
        "an already-claimed pending send remains discovered for supervision"
    );

    assert_eq!(gate.effect_count(), 1);
    let current = restarted.get_operation("p-1", "op-1").await.unwrap();
    assert!(matches!(
        current.latest_status,
        SendExecutionStatus::ReconciliationRequired { .. }
    ));
    assert_eq!(
        restarted
            .recover_pending_provider_effects_pass()
            .await
            .unwrap(),
        0,
        "durable reconciliation is a quiescent supervision state"
    );
}

#[tokio::test]
async fn f12_send_matching_pending_decode_and_reference_failures_are_retryable_errors() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    repo.with_state(|state| {
        state.obligations.insert(
            "malformed.exec".to_string(),
            (
                ObligationRecord::WorkflowShutdown {
                    operation_id: "malformed".to_string(),
                    effect_identity: "malformed.exec".to_string(),
                    owner_revision: 0,
                    execution_id: "workflow-1".to_string(),
                    state: ObligationStateRecord::Pending,
                },
                true,
                0,
            ),
        );
    });
    assert!(send_usecase(&repo, &gate)
        .recover_pending_provider_effects_pass()
        .await
        .is_err());

    repo.with_state(|state| {
        state.obligations.clear();
        state.obligations.insert(
            "missing-operation.exec".to_string(),
            (
                pending_send_obligation(
                    "missing-operation.exec",
                    "missing-operation",
                    SendObligationKindRecord::TurnExecution,
                ),
                true,
                0,
            ),
        );
    });
    assert!(send_usecase(&repo, &gate)
        .recover_pending_provider_effects_pass()
        .await
        .is_err());

    repo.with_state(|state| {
        state.obligations.clear();
        state.obligations.insert(
            "missing-establish-operation.establish".to_string(),
            (
                pending_send_obligation(
                    "missing-establish-operation.establish",
                    "missing-establish-operation",
                    SendObligationKindRecord::ProviderEstablish,
                ),
                true,
                0,
            ),
        );
    });
    assert!(send_usecase(&repo, &gate)
        .recover_pending_provider_effects_pass()
        .await
        .is_err());
    assert_eq!(gate.effect_count(), 0);
}

#[tokio::test]
async fn f12_stop_matching_pending_decode_and_required_field_failures_are_retryable_errors() {
    let repo = FakeRepo::new();
    let gate = FakeStopGate::active(0, "1");
    repo.with_state(|state| {
        state.obligations.insert(
            "stop-target-malformed".to_string(),
            (
                pending_send_obligation(
                    "stop-target-malformed",
                    "malformed",
                    SendObligationKindRecord::TurnExecution,
                ),
                true,
                0,
            ),
        );
    });
    assert!(stop_usecase(&repo, &gate)
        .recover_pending_stops_pass()
        .await
        .is_err());

    repo.with_state(|state| {
        state.obligations.clear();
        state.obligations.insert(
            "stop-target-missing-operation".to_string(),
            (
                ObligationRecord::StopInterrupt {
                    operation_id: String::new(),
                    session_id: "s-1".to_string(),
                    turn_id: "1".to_string(),
                    expected_revision: 0,
                    deadline_ms: 0,
                    state: ObligationStateRecord::EffectReserved,
                },
                true,
                0,
            ),
        );
    });
    assert!(stop_usecase(&repo, &gate)
        .recover_pending_stops_pass()
        .await
        .is_err());
    assert_eq!(gate.interrupt_count(), 0);
}

#[tokio::test]
async fn b017_b019_ambiguous_establish_is_reconciled_without_replay_for_create_and_resume() {
    for is_create in [true, false] {
        for completion_failed_after_effect in [false, true] {
            let mode = if is_create { "create" } else { "resume" };
            let crash = if completion_failed_after_effect {
                "completion-failed"
            } else {
                "claim-before-effect"
            };
            let operation_id = format!("op-{mode}-{crash}");
            let session_id = format!("session-{mode}-{crash}");
            let repo = FakeRepo::new();
            let first_gate = FakeSendGate::started_turn(&session_id);
            first_gate.set_plan(Ok(SendPlan {
                session_id: session_id.clone(),
                initial_session: is_create.then(|| {
                    crate::usecase::agent_session::session::build_new_session_with_id(
                        session_id.clone(),
                        "/tmp/send-provider-establish-test",
                        Some("codex".to_string()),
                        crate::domain::agent_session::PermissionMode::Ask,
                        None,
                        false,
                        false,
                        None,
                    )
                }),
                session_projection_guard: RevisionGuard::Absent,
                disposition: SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                input_ref: format!("input-{mode}-{crash}"),
                human_message_id: format!("human-{mode}-{crash}"),
                prompt: crate::domain::agent_session::events::PromptInput {
                    content: mode.to_string(),
                    ..Default::default()
                },
                reserved_turn_id: None,
            }));
            let first = send_usecase(&repo, &first_gate);
            expect_accepted(first.send(send_request(&operation_id, mode)).await.unwrap());
            seed_legacy_provider_dependency(&repo, &operation_id);
            let establish_id = format!("{operation_id}.establish");
            first
                .transition_obligation(ObligationTransition {
                    operation_id: &operation_id,
                    obligation_id: &establish_id,
                    expected_kind: "provider_establish",
                    expected_state: "pending",
                    next_state: "effect_reserved",
                    keep_pending: true,
                    status: None,
                })
                .await
                .unwrap();

            if completion_failed_after_effect {
                repo.with_state(|state| {
                    state.fail_commit_once = Some(CommitBatchError::CapacityExceeded);
                });
                assert!(first
                    .transition_obligation(ObligationTransition {
                        operation_id: &operation_id,
                        obligation_id: &establish_id,
                        expected_kind: "provider_establish",
                        expected_state: "effect_reserved",
                        next_state: "completed",
                        keep_pending: false,
                        status: None,
                    })
                    .await
                    .is_err());
            }

            let restart_gate = FakeSendGate::started_turn(&session_id);
            let restarted = send_usecase(&repo, &restart_gate);
            restarted.recover_pending_provider_effects().await.unwrap();
            assert_eq!(
                restart_gate.effect_count(),
                0,
                "{mode}/{crash} must not repeat provider establishment"
            );
            assert!(matches!(
                restarted
                    .get_operation("p-1", &operation_id)
                    .await
                    .unwrap()
                    .latest_status,
                SendExecutionStatus::ReconciliationRequired { .. }
            ));
            repo.with_state(|state| {
                let (obligation, pending, _) = state
                    .obligations
                    .get(&establish_id)
                    .expect("durable establish obligation");
                assert!(*pending);
                assert!(matches!(
                    obligation,
                    ObligationRecord::Send {
                        state: ObligationStateRecord::ReconciliationRequired,
                        ..
                    }
                ));
            });

            let recovery_executor = FakeRecoveryExecutor::returning(
                RecoveryResultClassification::Pending,
                "provider establishment remains ambiguous",
            );
            let pending = recovery_usecase(&repo, &recovery_executor)
                .pending(super::recovery::PendingRecoveryQuery {
                    limit: 32,
                    partition: None,
                    owner: None,
                    shutdown_plan: None,
                    cursor: None,
                })
                .await
                .unwrap();
            let entry = pending
                .entries
                .iter()
                .find(|entry| entry.obligation_id == establish_id)
                .expect("ambiguous provider establishment remains discoverable");
            assert_eq!(
                entry.category,
                PendingRecoveryCategory::ProviderEstablish,
                "{mode}/{crash}"
            );
            assert_eq!(entry.original_identity, operation_id, "{mode}/{crash}");
            assert_eq!(
                entry.known_status,
                PendingRecoveryKnownStatus::ReconciliationRequired,
                "{mode}/{crash}"
            );
            assert!(
                entry.actions.contains(&RecoveryActionKind::ReadAgain),
                "{mode}/{crash}"
            );
            assert!(
                entry
                    .actions
                    .contains(&RecoveryActionKind::KeepForManualResolution),
                "{mode}/{crash}"
            );
            assert!(
                !entry.actions.contains(&RecoveryActionKind::RetrySameEffect),
                "{mode}/{crash} must not expose blind provider replay"
            );
            assert_eq!(recovery_executor.effect_count(), 0, "{mode}/{crash}");

            // A later restart observes the durable reconciliation state and
            // remains effect-free.
            restarted.recover_pending_provider_effects().await.unwrap();
            assert_eq!(restart_gate.effect_count(), 0);
        }
    }
}

#[tokio::test]
async fn legacy_overlapping_immediate_sends_dispatch_only_the_canonical_current_turn() {
    let stale_operation_id = "op-legacy-overlap-1";
    let current_operation_id = "op-legacy-overlap-2";
    let session_id = "session-legacy-overlap";
    let repo = FakeRepo::new();
    let accepting_gate = FakeSendGate::started_turn(session_id);
    let accepting = send_usecase(&repo, &accepting_gate);

    expect_accepted(
        accepting
            .send(send_request(stale_operation_id, "TEST"))
            .await
            .unwrap(),
    );
    let mut current_plan = accepting_gate
        .plan
        .lock()
        .unwrap()
        .clone()
        .expect("first legacy send plan");
    current_plan.disposition = SendDisposition::StartedTurn {
        turn_id: "2".to_string(),
    };
    // The shipped collision reused the same human and assistant identities.
    // Preserve that exact shape while changing only the canonical turn.
    accepting_gate.set_plan(Ok(current_plan));
    expect_accepted(
        accepting
            .send(send_request(current_operation_id, "TEST"))
            .await
            .unwrap(),
    );

    for operation_id in [stale_operation_id, current_operation_id] {
        seed_legacy_provider_dependency(&repo, operation_id);
        let establish_id = format!("{operation_id}.establish");
        accepting
            .transition_obligation(ObligationTransition {
                operation_id,
                obligation_id: &establish_id,
                expected_kind: "provider_establish",
                expected_state: "pending",
                next_state: "effect_reserved",
                keep_pending: true,
                status: None,
            })
            .await
            .unwrap();
        accepting
            .record_execution_status(
                operation_id,
                SendExecutionStatus::ReconciliationRequired {
                    failure: SafeOperationFailure::new(
                        SessionOperationFailureKind::OutcomeUnknown,
                        true,
                        "legacy provider establishment timed out",
                        format!("{operation_id}-provider-timeout"),
                    ),
                },
            )
            .await
            .unwrap();
    }

    let restart_gate = FakeSendGate::started_turn(session_id);
    restart_gate.allow_legacy_provider_turn_continuation();
    restart_gate.set_canonical_current_turn(2);
    let restarted = Arc::new(send_usecase(&repo, &restart_gate));
    restart_gate.bind_claim_sink(Arc::downgrade(&restarted));
    restarted.recover_pending_provider_effects().await.unwrap();

    let effects = restart_gate.effects();
    assert_eq!(
        effects.len(),
        1,
        "restart recovery must hand off exactly one of the overlapping accepted inputs"
    );
    assert_eq!(effects[0].operation_id, current_operation_id);
    assert_eq!(
        effects[0].disposition,
        SendDisposition::StartedTurn {
            turn_id: "2".to_string()
        }
    );
    assert_eq!(effects[0].human_message_id, "human-1");
    assert_eq!(
        effects[0].assistant_message_id.as_deref(),
        Some("human-1:agent")
    );

    assert!(matches!(
        restarted
            .get_operation("p-1", stale_operation_id)
            .await
            .unwrap()
            .latest_status,
        SendExecutionStatus::Failed { ref failure }
            if failure.kind == SessionOperationFailureKind::InvalidEffectIntent
    ));
    assert!(matches!(
        restarted
            .get_operation("p-1", current_operation_id)
            .await
            .unwrap()
            .latest_status,
        SendExecutionStatus::ProviderStartReserved { .. }
    ));
    repo.with_state(|state| {
        for operation_id in [stale_operation_id, current_operation_id] {
            let (establish, establish_pending, _) =
                &state.obligations[&format!("{operation_id}.establish")];
            assert!(!establish_pending);
            assert!(matches!(
                establish,
                ObligationRecord::Send {
                    state: ObligationStateRecord::Cancelled,
                    ..
                }
            ));
        }
        let (stale_execution, stale_pending, _) =
            &state.obligations[&format!("{stale_operation_id}.exec")];
        assert!(!stale_pending);
        assert!(matches!(
            stale_execution,
            ObligationRecord::Send {
                state: ObligationStateRecord::Cancelled,
                ..
            }
        ));
        let (current_execution, current_pending, _) =
            &state.obligations[&format!("{current_operation_id}.exec")];
        assert!(*current_pending);
        assert!(matches!(
            current_execution,
            ObligationRecord::Send {
                state: ObligationStateRecord::EffectReserved,
                ..
            }
        ));
    });
}

#[tokio::test]
async fn b017_legacy_claude_shape_resumes_the_original_turn_without_manual_recovery() {
    let operation_id = "op-legacy-claude";
    let session_id = "session-legacy-claude";
    let establish_id = format!("{operation_id}.establish");
    let execution_id = format!("{operation_id}.exec");
    let repo = FakeRepo::new();
    let first_gate = FakeSendGate::started_turn(session_id);
    let first = send_usecase(&repo, &first_gate);
    expect_accepted(
        first
            .send(send_request(operation_id, "accepted input"))
            .await
            .unwrap(),
    );
    seed_legacy_provider_dependency(&repo, operation_id);
    first
        .transition_obligation(ObligationTransition {
            operation_id,
            obligation_id: &establish_id,
            expected_kind: "provider_establish",
            expected_state: "pending",
            next_state: "effect_reserved",
            keep_pending: true,
            status: None,
        })
        .await
        .unwrap();
    first
        .record_execution_status(
            operation_id,
            SendExecutionStatus::ReconciliationRequired {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::OutcomeUnknown,
                    true,
                    "legacy provider establishment timed out",
                    "legacy-provider-timeout",
                ),
            },
        )
        .await
        .unwrap();

    let restart_gate = FakeSendGate::started_turn(session_id);
    restart_gate.allow_legacy_provider_turn_continuation();
    let restarted = Arc::new(send_usecase(&repo, &restart_gate));
    restart_gate.bind_claim_sink(Arc::downgrade(&restarted));
    restarted.recover_pending_provider_effects().await.unwrap();

    let effects = restart_gate.effects();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].operation_id, operation_id);
    assert_eq!(effects[0].execution_obligation_id, execution_id);
    repo.with_state(|state| {
        let (establish, establish_pending, _) = &state.obligations[&establish_id];
        assert!(!establish_pending);
        assert!(matches!(
            establish,
            ObligationRecord::Send {
                state: ObligationStateRecord::Cancelled,
                ..
            }
        ));
        let (execution, execution_pending, _) = &state.obligations[&execution_id];
        assert!(*execution_pending);
        assert!(matches!(
            execution,
            ObligationRecord::Send {
                state: ObligationStateRecord::EffectReserved,
                ..
            }
        ));
    });
    assert!(matches!(
        restarted
            .get_operation("p-1", operation_id)
            .await
            .unwrap()
            .latest_status,
        SendExecutionStatus::ProviderStartReserved { .. }
    ));

    let recovery_executor =
        FakeRecoveryExecutor::returning(RecoveryResultClassification::Pending, "unused");
    recovery_executor.own_current_process_effect(&execution_id);
    let pending = recovery_usecase(&repo, &recovery_executor)
        .pending(super::recovery::PendingRecoveryQuery {
            limit: 32,
            partition: None,
            owner: None,
            shutdown_plan: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert!(
        pending.entries.is_empty(),
        "resumed legacy progress must not remain as manual recovery"
    );
}

#[tokio::test]
async fn b017_reserved_legacy_turn_is_reconciled_without_a_second_handoff() {
    let operation_id = "op-legacy-reserved-turn";
    let establish_id = format!("{operation_id}.establish");
    let execution_id = format!("{operation_id}.exec");
    let repo = FakeRepo::new();
    let first_gate = FakeSendGate::started_turn("session-legacy-reserved-turn");
    let first = send_usecase(&repo, &first_gate);
    expect_accepted(
        first
            .send(send_request(operation_id, "accepted input"))
            .await
            .unwrap(),
    );
    seed_legacy_provider_dependency(&repo, operation_id);
    first
        .transition_obligation(ObligationTransition {
            operation_id,
            obligation_id: &establish_id,
            expected_kind: "provider_establish",
            expected_state: "pending",
            next_state: "effect_reserved",
            keep_pending: true,
            status: None,
        })
        .await
        .unwrap();
    first
        .transition_obligation(ObligationTransition {
            operation_id,
            obligation_id: &establish_id,
            expected_kind: "provider_establish",
            expected_state: "effect_reserved",
            next_state: "completed",
            keep_pending: false,
            status: None,
        })
        .await
        .unwrap();
    first
        .transition_obligation(ObligationTransition {
            operation_id,
            obligation_id: &execution_id,
            expected_kind: "turn_execution",
            expected_state: "pending",
            next_state: "effect_reserved",
            keep_pending: true,
            status: Some(SendExecutionStatus::ProviderStartReserved {
                obligation_id: execution_id.clone(),
            }),
        })
        .await
        .unwrap();

    let restart_gate = FakeSendGate::started_turn("session-legacy-reserved-turn");
    restart_gate.allow_legacy_provider_turn_continuation();
    let restarted = send_usecase(&repo, &restart_gate);
    restarted.recover_pending_provider_effects().await.unwrap();

    assert_eq!(restart_gate.effect_count(), 0);
    assert!(matches!(
        restarted
            .get_operation("p-1", operation_id)
            .await
            .unwrap()
            .latest_status,
        SendExecutionStatus::ReconciliationRequired { .. }
    ));
    repo.with_state(|state| {
        assert!(matches!(
            &state.obligations[&execution_id].0,
            ObligationRecord::Send {
                state: ObligationStateRecord::ReconciliationRequired,
                ..
            }
        ));
    });
}

#[tokio::test]
async fn b017_legacy_auto_recovery_never_overrides_a_manual_recovery_claim() {
    let operation_id = "op-legacy-manual-owner";
    let establish_id = format!("{operation_id}.establish");
    let repo = FakeRepo::new();
    let first_gate = FakeSendGate::started_turn("session-legacy-manual-owner");
    let first = send_usecase(&repo, &first_gate);
    expect_accepted(
        first
            .send(send_request(operation_id, "accepted input"))
            .await
            .unwrap(),
    );
    seed_legacy_provider_dependency(&repo, operation_id);
    first
        .transition_obligation(ObligationTransition {
            operation_id,
            obligation_id: &establish_id,
            expected_kind: "provider_establish",
            expected_state: "pending",
            next_state: "effect_reserved",
            keep_pending: true,
            status: Some(SendExecutionStatus::ReconciliationRequired {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::OutcomeUnknown,
                    true,
                    "legacy provider establishment requires manual recovery",
                    "legacy-manual-owner",
                ),
            }),
        })
        .await
        .unwrap();
    repo.with_state(|state| {
        let (record, _, revision) = state.obligations.get_mut(&establish_id).unwrap();
        let original = record.clone();
        *record = ObligationRecord::RecoveryTransition {
            original: Box::new(original),
            recovery_action: ObligationRecoveryActionRecord {
                action_id: "manual-recovery-action".to_string(),
                origin_revision: *revision as u64,
                action: RecoveryActionKind::KeepForManualResolution,
                effect_identity: establish_id.clone(),
                state: ObligationStateRecord::EffectReserved,
                classification: None,
            },
        };
    });

    let restart_gate = FakeSendGate::started_turn("session-legacy-manual-owner");
    restart_gate.allow_legacy_provider_turn_continuation();
    let restarted = send_usecase(&repo, &restart_gate);
    restarted.recover_pending_provider_effects().await.unwrap();

    assert_eq!(restart_gate.effect_count(), 0);
    repo.with_state(|state| {
        assert!(matches!(
            &state.obligations[&establish_id].0,
            ObligationRecord::RecoveryTransition {
                recovery_action: ObligationRecoveryActionRecord {
                    action_id,
                    state: ObligationStateRecord::EffectReserved,
                    ..
                },
                ..
            } if action_id == "manual-recovery-action"
        ));
    });
}

#[tokio::test]
async fn b017_pending_or_completed_establish_resumes_at_the_safe_boundary() {
    for is_create in [true, false] {
        for establish_completed in [false, true] {
            let mode = if is_create { "create" } else { "resume" };
            let boundary = if establish_completed {
                "completed"
            } else {
                "pending"
            };
            let operation_id = format!("op-safe-{mode}-{boundary}");
            let session_id = format!("session-safe-{mode}-{boundary}");
            let repo = FakeRepo::new();
            let first_gate = FakeSendGate::started_turn(&session_id);
            first_gate.set_plan(Ok(SendPlan {
                session_id: session_id.clone(),
                initial_session: is_create.then(|| {
                    crate::usecase::agent_session::session::build_new_session_with_id(
                        session_id.clone(),
                        "/tmp/send-provider-establish-test",
                        Some("codex".to_string()),
                        crate::domain::agent_session::PermissionMode::Ask,
                        None,
                        false,
                        false,
                        None,
                    )
                }),
                session_projection_guard: RevisionGuard::Absent,
                disposition: SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                input_ref: format!("input-safe-{mode}-{boundary}"),
                human_message_id: format!("human-safe-{mode}-{boundary}"),
                prompt: crate::domain::agent_session::events::PromptInput {
                    content: mode.to_string(),
                    ..Default::default()
                },
                reserved_turn_id: None,
            }));
            let first = send_usecase(&repo, &first_gate);
            expect_accepted(first.send(send_request(&operation_id, mode)).await.unwrap());
            seed_legacy_provider_dependency(&repo, &operation_id);
            let establish_id = format!("{operation_id}.establish");
            if establish_completed {
                first
                    .transition_obligation(ObligationTransition {
                        operation_id: &operation_id,
                        obligation_id: &establish_id,
                        expected_kind: "provider_establish",
                        expected_state: "pending",
                        next_state: "effect_reserved",
                        keep_pending: true,
                        status: None,
                    })
                    .await
                    .unwrap();
                first
                    .transition_obligation(ObligationTransition {
                        operation_id: &operation_id,
                        obligation_id: &establish_id,
                        expected_kind: "provider_establish",
                        expected_state: "effect_reserved",
                        next_state: "completed",
                        keep_pending: false,
                        status: None,
                    })
                    .await
                    .unwrap();
            }

            let restart_gate = FakeSendGate::started_turn(&session_id);
            let restarted = send_usecase(&repo, &restart_gate);
            restarted.recover_pending_provider_effects().await.unwrap();
            let effects = restart_gate.effects.lock().unwrap();
            assert_eq!(effects.len(), 1, "{mode}/{boundary}");
            repo.with_state(|state| {
                let (establish, pending, _) = &state.obligations[&establish_id];
                assert!(!*pending);
                assert!(matches!(
                    establish,
                    ObligationRecord::Send {
                        state: ObligationStateRecord::Completed | ObligationStateRecord::Cancelled,
                        ..
                    }
                ));
            });
        }
    }
}

#[tokio::test]
async fn b017_reconciliation_establish_state_never_replays_provider_io() {
    let repo = FakeRepo::new();
    let first_gate = FakeSendGate::started_turn("s-reconcile");
    first_gate.set_plan(Ok(SendPlan {
        session_id: "s-reconcile".to_string(),
        initial_session: None,
        session_projection_guard: RevisionGuard::Absent,
        disposition: SendDisposition::StartedTurn {
            turn_id: "1".to_string(),
        },
        input_ref: "input-reconcile".to_string(),
        human_message_id: "human-reconcile".to_string(),
        prompt: crate::domain::agent_session::events::PromptInput {
            content: "resume".to_string(),
            ..Default::default()
        },
        reserved_turn_id: None,
    }));
    let first = send_usecase(&repo, &first_gate);
    expect_accepted(
        first
            .send(send_request("op-reconcile", "resume"))
            .await
            .unwrap(),
    );
    seed_legacy_provider_dependency(&repo, "op-reconcile");
    first
        .transition_obligation(ObligationTransition {
            operation_id: "op-reconcile",
            obligation_id: "op-reconcile.establish",
            expected_kind: "provider_establish",
            expected_state: "pending",
            next_state: "reconciliation_required",
            keep_pending: true,
            status: None,
        })
        .await
        .unwrap();

    let restart_gate = FakeSendGate::started_turn("s-reconcile");
    let restarted = send_usecase(&repo, &restart_gate);
    restarted.recover_pending_provider_effects().await.unwrap();
    assert_eq!(restart_gate.effect_count(), 0);
    assert!(matches!(
        restarted
            .get_operation("p-1", "op-reconcile")
            .await
            .unwrap()
            .latest_status,
        SendExecutionStatus::ReconciliationRequired { .. }
    ));
}

#[tokio::test]
async fn b021_incompatible_accepted_send_intent_fails_closed_after_restart() {
    for missing_field in ["session_id", "canonical_payload", "human_message_id"] {
        let operation_id = format!("op-missing-{missing_field}");
        let execution_id = format!("{operation_id}.exec");
        let repo = FakeRepo::new();
        let first_gate = FakeSendGate::started_turn("s-incompatible");
        let first = send_usecase(&repo, &first_gate);
        expect_accepted(
            first
                .send(send_request(&operation_id, "accepted payload"))
                .await
                .unwrap(),
        );
        repo.with_state(|state| {
            let (record, _, _) = state
                .obligations
                .get_mut(&execution_id)
                .expect("execution obligation");
            let ObligationRecord::Send {
                session_id,
                canonical_payload,
                human_message_id,
                ..
            } = record
            else {
                panic!("expected send obligation");
            };
            match missing_field {
                "session_id" => session_id.clear(),
                "canonical_payload" => canonical_payload.clear(),
                "human_message_id" => *human_message_id = None,
                _ => unreachable!("closed fixture field"),
            }
        });

        let restart_gate = FakeSendGate::started_turn("s-incompatible");
        let restarted = send_usecase(&repo, &restart_gate);
        restarted.recover_pending_provider_effects().await.unwrap();
        assert_eq!(restart_gate.effect_count(), 0, "missing {missing_field}");
        let status = restarted
            .get_operation("p-1", &operation_id)
            .await
            .unwrap()
            .latest_status;
        match status {
            SendExecutionStatus::ReconciliationRequired { failure } => {
                assert_eq!(
                    failure.kind,
                    SessionOperationFailureKind::InvalidEffectIntent
                );
            }
            other => panic!("missing {missing_field} must fail closed, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn b021_dependency_query_failure_surfaces_without_provider_handoff() {
    let repo = FakeRepo::new();
    let first_gate = FakeSendGate::started_turn("s-dependency-query");
    first_gate.set_plan(Ok(SendPlan {
        session_id: "s-dependency-query".to_string(),
        initial_session: None,
        session_projection_guard: RevisionGuard::Absent,
        disposition: SendDisposition::StartedTurn {
            turn_id: "1".to_string(),
        },
        input_ref: "input-dependency-query".to_string(),
        human_message_id: "human-dependency-query".to_string(),
        prompt: crate::domain::agent_session::events::PromptInput {
            content: "resume".to_string(),
            ..Default::default()
        },
        reserved_turn_id: None,
    }));
    let first = send_usecase(&repo, &first_gate);
    expect_accepted(
        first
            .send(send_request("op-dependency-query", "resume"))
            .await
            .unwrap(),
    );
    seed_legacy_provider_dependency(&repo, "op-dependency-query");
    repo.with_state(|state| {
        state.fail_obligation_query = Some("op-dependency-query.establish".to_string());
    });

    let restart_gate = FakeSendGate::started_turn("s-dependency-query");
    let restarted = send_usecase(&repo, &restart_gate);
    assert!(restarted.recover_pending_provider_effects().await.is_err());
    assert_eq!(restart_gate.effect_count(), 0);
    repo.with_state(|state| state.fail_obligation_query = None);
    assert!(matches!(
        restarted
            .get_operation("p-1", "op-dependency-query")
            .await
            .unwrap()
            .latest_status,
        SendExecutionStatus::AwaitingProviderStart { .. }
    ));
}

#[tokio::test]
async fn b099_other_principal_sees_not_found_with_zero_effects() {
    let repo = FakeRepo::new();
    let gate = FakeSendGate::started_turn("s-1");
    let usecase = send_usecase(&repo, &gate);
    let first = expect_accepted(usecase.send(send_request("op-1", "hello")).await.unwrap());
    let mut foreign = send_request("op-1", "hello");
    foreign.principal = "p-2".to_string();
    assert_eq!(
        usecase.send(foreign).await,
        Err(SendAgentMessageError::NotFound)
    );
    assert_eq!(
        usecase.get_operation("p-2", "op-1").await,
        Err(GetSendOperationError::NotFound)
    );
    let unchanged = usecase.get_operation("p-1", "op-1").await.unwrap();
    assert_eq!(unchanged.receipt, first.receipt);
    assert_eq!(gate.effect_count(), 1);
}

// --- Caller attempt journal -------------------------------------------------

#[tokio::test]
async fn caller_journal_records_then_clears_after_accept() {
    let repo = FakeRepo::new();
    let journal = CallerAttemptJournal::new(
        repo.clone() as Arc<dyn LocalEventTransactionRepository>,
        Arc::new(FakeAuthority),
        GENERATION.to_string(),
    );
    journal
        .record_attempt(
            "p-1",
            crate::domain::local_event::OperationKind::Send,
            "op-1",
            b"exact-command",
        )
        .await
        .unwrap();
    repo.with_state(|state| {
        let entry = state
            .attempts
            .get(&("p-1".to_string(), "send".to_string(), "op-1".to_string()))
            .unwrap();
        assert_eq!(entry.3, "pending");
    });
    journal
        .clear_attempt(
            "p-1",
            crate::domain::local_event::OperationKind::Send,
            "op-1",
            b"exact-command",
            true,
        )
        .await
        .unwrap();
    repo.with_state(|state| {
        let entry = state
            .attempts
            .get(&("p-1".to_string(), "send".to_string(), "op-1".to_string()))
            .unwrap();
        assert_eq!(entry.3, "accepted");
        assert_ne!(entry.1, b"exact-command");
    });
    journal
        .acknowledge_attempt(
            "p-1",
            crate::domain::local_event::OperationKind::Send,
            "op-1",
        )
        .await
        .unwrap();
    repo.with_state(|state| {
        let entry = state
            .attempts
            .get(&("p-1".to_string(), "send".to_string(), "op-1".to_string()))
            .unwrap();
        assert_eq!(entry.3, "cleared");
        assert!(entry.1.is_empty());
    });
}

#[tokio::test]
async fn caller_journal_definitive_replay_retries_resolution_and_absence_is_a_noop() {
    let repo = FakeRepo::new();
    let journal = CallerAttemptJournal::new(
        repo.clone() as Arc<dyn LocalEventTransactionRepository>,
        Arc::new(FakeAuthority),
        GENERATION.to_string(),
    );
    journal
        .record_attempt(
            "p-1",
            OperationKind::Send,
            "op-resolve-replay",
            b"exact-command",
        )
        .await
        .unwrap();
    repo.with_state(|state| {
        state.fail_commit_once = Some(CommitBatchError::CapacityExceeded);
    });
    assert_eq!(
        journal
            .resolve_attempt_if_present(
                "p-1",
                OperationKind::Send,
                "op-resolve-replay",
                b"exact-command",
                true,
            )
            .await,
        Err(CallerJournalError::RejectedBeforeCommit)
    );
    repo.with_state(|state| {
        assert_eq!(
            state.attempts[&(
                "p-1".to_string(),
                "send".to_string(),
                "op-resolve-replay".to_string(),
            )]
                .3,
            "pending"
        );
    });
    assert_eq!(
        journal
            .resolve_attempt_if_present(
                "p-1",
                OperationKind::Send,
                "op-resolve-replay",
                b"exact-command",
                true,
            )
            .await,
        Ok(true)
    );
    repo.with_state(|state| {
        assert_eq!(
            state.attempts[&(
                "p-1".to_string(),
                "send".to_string(),
                "op-resolve-replay".to_string(),
            )]
                .3,
            "accepted"
        );
    });
    assert_eq!(
        journal
            .resolve_attempt_if_present(
                "p-1",
                OperationKind::Send,
                "direct-operation-replay",
                b"exact-command",
                true,
            )
            .await,
        Ok(false)
    );
}

#[tokio::test]
async fn caller_journal_rejects_different_command_under_same_identity() {
    let repo = FakeRepo::new();
    let journal = CallerAttemptJournal::new(
        repo.clone() as Arc<dyn LocalEventTransactionRepository>,
        Arc::new(FakeAuthority),
        GENERATION.to_string(),
    );
    journal
        .record_attempt(
            "p-1",
            crate::domain::local_event::OperationKind::Send,
            "op-1",
            b"exact-command",
        )
        .await
        .unwrap();
    let conflict = journal
        .record_attempt(
            "p-1",
            crate::domain::local_event::OperationKind::Send,
            "op-1",
            b"another-command",
        )
        .await;
    assert_eq!(conflict, Err(CallerJournalError::PayloadConflict));
}

#[tokio::test]
async fn caller_journal_lookup_outage_preserves_the_existing_attempt_identity() {
    let repo = FakeRepo::new();
    let journal = CallerAttemptJournal::new(
        repo.clone() as Arc<dyn LocalEventTransactionRepository>,
        Arc::new(FakeAuthority),
        GENERATION.to_string(),
    );
    journal
        .record_attempt(
            "p-1",
            crate::domain::local_event::OperationKind::Send,
            "op-query-loss",
            b"exact-command",
        )
        .await
        .unwrap();
    let commit_calls = repo.with_state(|state| {
        state.fail_query = true;
        state.commit_calls
    });

    assert_eq!(
        journal
            .record_attempt(
                "p-1",
                crate::domain::local_event::OperationKind::Send,
                "op-query-loss",
                b"exact-command",
            )
            .await,
        Err(CallerJournalError::OutcomeUnknown)
    );
    repo.with_state(|state| assert_eq!(state.commit_calls, commit_calls));
}

#[tokio::test]
async fn caller_journal_pages_past_thirty_two_and_binds_cursor_to_scope() {
    let repo = FakeRepo::new();
    let journal = CallerAttemptJournal::new(
        repo.clone() as Arc<dyn LocalEventTransactionRepository>,
        Arc::new(FakeAuthority),
        GENERATION.to_string(),
    );
    for ordinal in 0..33 {
        journal
            .record_attempt_scoped(
                "p-1",
                crate::domain::local_event::OperationKind::Send,
                &format!("op-{ordinal:02}"),
                format!("exact-{ordinal}").as_bytes(),
                Some("session-1"),
            )
            .await
            .unwrap();
    }
    let first = journal
        .pending_page_for_scope("p-1", "session-1", 32, None)
        .await
        .unwrap();
    assert_eq!(first.entries.len(), 32);
    let cursor = first.next_cursor.expect("full page has a cursor");
    let second = journal
        .pending_page_for_scope("p-1", "session-1", 32, Some(&cursor))
        .await
        .unwrap();
    assert_eq!(second.entries.len(), 1);
    assert_eq!(second.entries[0].caller_request_id, "op-32");
    assert_eq!(
        journal
            .pending_page_for_scope("p-1", "another-session", 32, Some(&cursor))
            .await,
        Err(CallerJournalError::InvalidRequest)
    );
}

#[tokio::test]
async fn caller_journal_maps_storage_outcomes() {
    let repo = FakeRepo::new();
    let journal = CallerAttemptJournal::new(
        repo.clone() as Arc<dyn LocalEventTransactionRepository>,
        Arc::new(FakeAuthority),
        GENERATION.to_string(),
    );
    assert_eq!(
        journal
            .record_attempt(
                "p-1",
                crate::domain::local_event::OperationKind::Send,
                "bad id",
                b"cmd",
            )
            .await,
        Err(CallerJournalError::InvalidRequest)
    );
    repo.with_state(|state| {
        state.fail_commit_once = Some(CommitBatchError::OutcomeUnknown {
            identity: CommitIdentity::parse("x").unwrap(),
        });
    });
    assert_eq!(
        journal
            .record_attempt(
                "p-1",
                crate::domain::local_event::OperationKind::Send,
                "op-1",
                b"cmd",
            )
            .await,
        Err(CallerJournalError::OutcomeUnknown)
    );
    repo.with_state(|state| {
        state.fail_commit_once = Some(CommitBatchError::CapacityExceeded);
    });
    assert_eq!(
        journal
            .record_attempt(
                "p-1",
                crate::domain::local_event::OperationKind::Send,
                "op-1",
                b"cmd",
            )
            .await,
        Err(CallerJournalError::RejectedBeforeCommit)
    );
}

// --- Session lifecycle: B-053..B-056, B-095, B-101..B-103 -------------------

#[tokio::test]
async fn b025_graceful_quit_shutdown_target_commits_terminal_and_queue_pause_atomically() {
    for initially_paused in [false, true] {
        let repo = FakeRepo::new();
        let mut snapshot = open_active_snapshot(4, 7);
        snapshot.queue_paused = initially_paused;
        let gate = FakeLifecycleGate::with_snapshot(snapshot);
        let usecase = lifecycle_usecase(&repo, &gate);
        let result = usecase
            .request_shutdown_target(SessionLifecycleRequest {
                principal: "shutdown:quit-b025".to_string(),
                request_id: format!("shutdown-b025-{initially_paused}"),
                session_id: format!("b025-session-{initially_paused}"),
                expected_session_revision: 4,
                action: SessionLifecycleAction::Close,
            })
            .await
            .unwrap();
        assert!(matches!(
            result,
            SessionLifecycleCommandResult::Accepted {
                state: SessionLifecycleOperationState::Completed,
                ..
            }
        ));
        assert_eq!(gate.execution_count(), 1);

        repo.with_state(|state| {
            let acceptance = state
                .committed_batches
                .iter()
                .find(|batch| !batch.events.is_empty())
                .expect("shutdown-target acceptance batch");
            assert_eq!(
                acceptance.idempotency.operation_kind,
                crate::domain::local_event::CommitOperationKind::ShutdownTarget
            );
            assert!(acceptance.events.iter().any(|event| matches!(
                event.event,
                LocalDomainEvent::AgentSession(AgentSessionDomainEvent::TurnInterrupted {
                    turn_id: 7,
                    reason: InterruptReason::SessionClosed,
                    ..
                })
            )));
            assert!(acceptance
                .state_mutations
                .iter()
                .any(|mutation| matches!(mutation, LocalStateMutation::TerminalRecord(_))));
            assert_eq!(
                acceptance
                    .events
                    .iter()
                    .filter(|event| matches!(
                        event.event,
                        LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueuePaused { .. })
                    ))
                    .count(),
                usize::from(!initially_paused),
                "an existing pause stays intact; an unpaused queue is paused in the terminal batch",
            );
            assert!(!state.events.iter().any(|event| matches!(
                event,
                LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueueResumed { .. })
            )));
            assert_eq!(state.terminals.len(), 1);
        });
    }
}

#[tokio::test]
async fn b053_active_close_commits_terminal_and_queue_pause_atomically() {
    let repo = FakeRepo::new();
    let gate = FakeLifecycleGate::with_snapshot(open_active_snapshot(4, 7));
    let usecase = lifecycle_usecase(&repo, &gate);
    let result = usecase
        .request(lifecycle_request(
            "close-1",
            "s-1",
            4,
            SessionLifecycleAction::Close,
        ))
        .await
        .unwrap();
    let SessionLifecycleCommandResult::Accepted { receipt, state } = result else {
        panic!("expected acceptance");
    };
    assert_eq!(receipt.session_id, "s-1");
    assert_eq!(receipt.first_accepted_revision, 4);
    assert_eq!(state, SessionLifecycleOperationState::Completed);
    assert_eq!(gate.execution_count(), 1);
    repo.with_state(|state| {
        assert!(state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::TurnInterrupted {
                turn_id: 7,
                ..
            })
        )));
        assert!(state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::SessionClosed { .. })
        )));
        assert!(state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueuePaused { .. })
        )));
        assert!(state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::ObligationRecorded {
                kind: crate::domain::agent_session::events::ObligationKind::SessionClose,
                state: crate::domain::agent_session::events::ObligationState::EffectReserved,
                ..
            })
        )));
        assert_eq!(state.terminals.len(), 1);
    });
}

#[tokio::test]
async fn b054_idle_close_and_closed_archive_add_no_synthetic_terminal() {
    let repo = FakeRepo::new();
    let gate = FakeLifecycleGate::with_snapshot(open_idle_snapshot(2));
    let usecase = lifecycle_usecase(&repo, &gate);
    let result = usecase
        .request(lifecycle_request(
            "close-idle",
            "s-1",
            2,
            SessionLifecycleAction::Close,
        ))
        .await
        .unwrap();
    assert!(matches!(
        result,
        SessionLifecycleCommandResult::Accepted {
            state: SessionLifecycleOperationState::Completed,
            ..
        }
    ));
    repo.with_state(|state| {
        assert!(state.terminals.is_empty());
        assert!(!state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::TurnInterrupted { .. })
        )));
        assert!(state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueuePaused { .. })
        )));
    });

    // Closed-session archive: queue untouched, no terminal.
    let repo2 = FakeRepo::new();
    let gate2 = FakeLifecycleGate::with_snapshot(SessionLifecycleSnapshot {
        session_revision: 3,
        lifecycle: SessionLifecycleState::Closed,
        queue_paused: false,
        has_runtime: false,
        has_pending_permission: false,
        has_pending_recovery: false,
        has_pending_provider_operation: false,
    });
    let usecase2 = lifecycle_usecase(&repo2, &gate2);
    let result = usecase2
        .request(lifecycle_request(
            "arch-closed",
            "s-2",
            3,
            SessionLifecycleAction::ArchiveClosed,
        ))
        .await
        .unwrap();
    assert!(matches!(
        result,
        SessionLifecycleCommandResult::Accepted { .. }
    ));
    repo2.with_state(|state| {
        assert!(state.terminals.is_empty());
        assert!(state.obligations.is_empty());
        assert!(!state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueuePaused { .. })
        )));
    });
}

#[tokio::test]
async fn b055_backend_switch_requires_idle_without_pending_work() {
    // Idle without pending work: accepted with queue pause.
    let repo = FakeRepo::new();
    let gate = FakeLifecycleGate::with_snapshot(open_idle_snapshot(1));
    let usecase = lifecycle_usecase(&repo, &gate);
    let result = usecase
        .request(lifecycle_request(
            "sw-1",
            "s-1",
            1,
            SessionLifecycleAction::SwitchBackend {
                backend_id: "codex".to_string(),
            },
        ))
        .await
        .unwrap();
    assert!(matches!(
        result,
        SessionLifecycleCommandResult::Accepted { .. }
    ));
    repo.with_state(|state| {
        assert!(state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueuePaused { .. })
        )));
    });

    // Active turn: Busy, zero effects.
    let repo2 = FakeRepo::new();
    let gate2 = FakeLifecycleGate::with_snapshot(open_active_snapshot(1, 3));
    let usecase2 = lifecycle_usecase(&repo2, &gate2);
    let result = usecase2
        .request(lifecycle_request(
            "sw-2",
            "s-2",
            1,
            SessionLifecycleAction::SwitchBackend {
                backend_id: "codex".to_string(),
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        result,
        SessionLifecycleCommandResult::Rejected(SessionLifecycleRejection::Busy)
    );
    assert_eq!(gate2.execution_count(), 0);
    repo2.with_state(|state| assert!(state.events.is_empty()));

    // Pending permission: InvalidState, zero effects.
    let repo3 = FakeRepo::new();
    let mut snapshot = open_idle_snapshot(1);
    snapshot.has_pending_permission = true;
    let gate3 = FakeLifecycleGate::with_snapshot(snapshot);
    let usecase3 = lifecycle_usecase(&repo3, &gate3);
    let result = usecase3
        .request(lifecycle_request(
            "sw-3",
            "s-3",
            1,
            SessionLifecycleAction::SwitchBackend {
                backend_id: "codex".to_string(),
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        result,
        SessionLifecycleCommandResult::Rejected(SessionLifecycleRejection::InvalidState)
    );
    assert_eq!(gate3.execution_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn b056_b103_hanging_runtime_resolves_within_ten_seconds() {
    let closed_snapshot = SessionLifecycleSnapshot {
        session_revision: 0,
        lifecycle: SessionLifecycleState::Closed,
        queue_paused: true,
        has_runtime: false,
        has_pending_permission: false,
        has_pending_recovery: false,
        has_pending_provider_operation: false,
    };
    let cases = [
        (
            "active-close",
            open_active_snapshot(0, 1),
            SessionLifecycleAction::Close,
            true,
        ),
        (
            "idle-close",
            open_idle_snapshot(0),
            SessionLifecycleAction::Close,
            true,
        ),
        (
            "active-open-archive",
            open_active_snapshot(0, 1),
            SessionLifecycleAction::ArchiveOpen,
            true,
        ),
        (
            "idle-open-archive",
            open_idle_snapshot(0),
            SessionLifecycleAction::ArchiveOpen,
            true,
        ),
        (
            "closed-archive",
            closed_snapshot,
            SessionLifecycleAction::ArchiveClosed,
            false,
        ),
        (
            "idle-backend-switch",
            open_idle_snapshot(0),
            SessionLifecycleAction::SwitchBackend {
                backend_id: "codex".to_string(),
            },
            true,
        ),
    ];

    for (label, snapshot, action, requires_runtime_effect) in cases {
        let repo = FakeRepo::new();
        let gate = FakeLifecycleGate::with_snapshot(snapshot);
        gate.set_mode(LifecycleExecuteMode::Hang);
        let usecase = lifecycle_usecase(&repo, &gate);
        let request_id = format!("b056-{label}");
        let session_id = format!("session-{label}");
        let result = usecase
            .request(lifecycle_request(
                &request_id,
                &session_id,
                0,
                action.clone(),
            ))
            .await
            .unwrap();
        let SessionLifecycleCommandResult::Accepted { receipt, state } = result else {
            panic!("{label}: expected acceptance");
        };
        if requires_runtime_effect {
            assert!(matches!(
                state,
                SessionLifecycleOperationState::ReconciliationRequired { .. }
            ));
        } else {
            assert_eq!(state, SessionLifecycleOperationState::Completed);
        }

        // Stable identity and response-loss readback never execute the late
        // runtime action a second time.
        let (fetched_receipt, fetched_state) = usecase
            .get_operation("p-1", &receipt.operation_id)
            .await
            .unwrap();
        assert_eq!(fetched_receipt, receipt, "{label}");
        assert_eq!(fetched_state, state, "{label}");
        let (caller_receipt, caller_state) =
            usecase.get_operation("p-1", &request_id).await.unwrap();
        assert_eq!(caller_receipt, receipt, "{label}");
        assert_eq!(caller_state, state, "{label}");
        let replay = usecase
            .request(lifecycle_request(&request_id, &session_id, 0, action))
            .await
            .unwrap();
        assert_eq!(
            replay,
            SessionLifecycleCommandResult::Accepted { receipt, state },
            "{label}",
        );
        assert_eq!(
            gate.execution_count(),
            usize::from(requires_runtime_effect),
            "{label}",
        );
    }
}

#[tokio::test(start_paused = true)]
async fn b056_late_runtime_results_are_fenced_for_the_full_lifecycle_matrix() {
    let cases = [
        (
            "active-close",
            open_active_snapshot(0, 1),
            SessionLifecycleAction::Close,
            1,
            1,
        ),
        (
            "idle-close",
            open_idle_snapshot(0),
            SessionLifecycleAction::Close,
            0,
            1,
        ),
        (
            "active-open-archive",
            open_active_snapshot(0, 1),
            SessionLifecycleAction::ArchiveOpen,
            1,
            1,
        ),
        (
            "idle-open-archive",
            open_idle_snapshot(0),
            SessionLifecycleAction::ArchiveOpen,
            0,
            1,
        ),
        (
            "idle-backend-switch",
            open_idle_snapshot(0),
            SessionLifecycleAction::SwitchBackend {
                backend_id: "codex".to_string(),
            },
            0,
            0,
        ),
    ];

    for (label, snapshot, action, terminal_count, session_closed_count) in cases {
        let repo = FakeRepo::new();
        let gate = LateLifecycleGate::with_snapshot(snapshot);
        let usecase = SessionLifecycleOperationUsecase::new(
            repo.clone() as Arc<dyn LocalEventTransactionRepository>,
            Arc::new(FakeAuthority),
            gate.clone() as Arc<dyn SessionLifecycleGate>,
            GENERATION.to_string(),
        );
        let request_id = format!("b056-late-{label}");
        let session_id = format!("b056-late-session-{label}");
        let request = lifecycle_request(&request_id, &session_id, 0, action);
        let SessionLifecycleCommandResult::Accepted { receipt, state } =
            usecase.request(request.clone()).await.unwrap()
        else {
            panic!("{label}: expected acceptance");
        };
        assert!(matches!(
            state,
            SessionLifecycleOperationState::ReconciliationRequired { .. }
        ));
        assert_eq!(gate.execution_count(), 1, "{label}");
        let durable_shape = repo.with_state(|state| {
            (
                state.committed_batches.len(),
                state.events.len(),
                state.terminals.len(),
                state.obligations.len(),
            )
        });

        gate.release_late_result();
        while gate.late_result_count() == 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(gate.late_result_count(), 1, "{label}");
        assert_eq!(
            usecase.request(request).await.unwrap(),
            SessionLifecycleCommandResult::Accepted {
                receipt: receipt.clone(),
                state: state.clone(),
            },
            "{label}",
        );
        assert_eq!(
            usecase
                .get_operation("p-1", &receipt.operation_id)
                .await
                .unwrap(),
            (receipt, state),
            "{label}",
        );
        assert_eq!(gate.execution_count(), 1, "{label}");

        repo.with_state(|state| {
            assert_eq!(
                (
                    state.committed_batches.len(),
                    state.events.len(),
                    state.terminals.len(),
                    state.obligations.len(),
                ),
                durable_shape,
                "{label}: a late runtime result cannot append durable participants",
            );
            assert_eq!(state.terminals.len(), terminal_count, "{label}");
            assert_eq!(
                state
                    .events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        LocalDomainEvent::AgentSession(
                            AgentSessionDomainEvent::SessionClosed { .. }
                        )
                    ))
                    .count(),
                session_closed_count,
                "{label}",
            );
            assert_eq!(
                state
                    .events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueuePaused { .. })
                    ))
                    .count(),
                1,
                "{label}",
            );
            assert!(!state.events.iter().any(|event| matches!(
                event,
                LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueueResumed { .. })
            )));
        });
    }
}

#[tokio::test]
async fn lifecycle_completion_storage_failure_never_publishes_an_unstored_success() {
    let repo = FakeRepo::new();
    let gate = FakeLifecycleGate::with_snapshot(open_idle_snapshot(0));
    let usecase = lifecycle_usecase(&repo, &gate);
    repo.with_state(|state| {
        state.fail_commit_on_call = Some((
            2,
            CommitBatchError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "completion unavailable",
                    "lifecycle-completion",
                ),
            },
        ));
    });

    let result = usecase
        .request(lifecycle_request(
            "close-completion-failure",
            "s-completion-failure",
            0,
            SessionLifecycleAction::Close,
        ))
        .await
        .unwrap();
    let SessionLifecycleCommandResult::Accepted { receipt, state } = result else {
        panic!("acceptance must remain durable");
    };
    assert!(matches!(
        state,
        SessionLifecycleOperationState::ReconciliationRequired { .. }
    ));
    assert_eq!(gate.execution_count(), 1);

    let restarted = lifecycle_usecase(&repo, &gate);
    let (saved_receipt, saved_state) = restarted
        .get_operation("p-1", &receipt.operation_id)
        .await
        .unwrap();
    assert_eq!(saved_receipt, receipt);
    assert!(matches!(
        saved_state,
        SessionLifecycleOperationState::ReconciliationRequired { .. }
    ));
    assert_eq!(gate.execution_count(), 1);
}

#[tokio::test]
async fn lifecycle_completion_unknown_outcome_requires_same_identity_reconciliation() {
    let repo = FakeRepo::new();
    let gate = FakeLifecycleGate::with_snapshot(open_idle_snapshot(0));
    let usecase = lifecycle_usecase(&repo, &gate);
    repo.with_state(|state| {
        state.fail_commit_on_call = Some((
            2,
            CommitBatchError::OutcomeUnknown {
                identity: CommitIdentity::parse("lifecycle-completion-unknown").unwrap(),
            },
        ));
    });
    let request = lifecycle_request(
        "close-completion-unknown",
        "s-completion-unknown",
        0,
        SessionLifecycleAction::Close,
    );

    let result = usecase.request(request.clone()).await.unwrap();
    let SessionLifecycleCommandResult::Accepted { receipt, state } = result else {
        panic!("acceptance must remain durable");
    };
    assert!(matches!(
        state,
        SessionLifecycleOperationState::ReconciliationRequired { .. }
    ));
    assert_eq!(gate.execution_count(), 1);

    let restarted = lifecycle_usecase(&repo, &gate);
    let replay = restarted.request(request).await.unwrap();
    let SessionLifecycleCommandResult::Accepted {
        receipt: replay_receipt,
        state: replay_state,
    } = replay
    else {
        panic!("same caller identity must replay the accepted operation");
    };
    assert_eq!(replay_receipt, receipt);
    assert!(matches!(
        replay_state,
        SessionLifecycleOperationState::ReconciliationRequired { .. }
    ));
    assert_eq!(gate.execution_count(), 1);
}

#[tokio::test]
async fn concurrent_same_lifecycle_identity_converges_after_stale_admission_failure() {
    let repo = FakeRepo::new();
    let gate = Arc::new(RacingLifecycleGate {
        snapshot: open_idle_snapshot(0),
        snapshot_calls: std::sync::atomic::AtomicUsize::new(0),
        first_entered: tokio::sync::Notify::new(),
        release_first: tokio::sync::Notify::new(),
        executions: Mutex::new(Vec::new()),
    });
    let usecase = Arc::new(SessionLifecycleOperationUsecase::new(
        repo.clone() as Arc<dyn LocalEventTransactionRepository>,
        Arc::new(FakeAuthority),
        gate.clone() as Arc<dyn SessionLifecycleGate>,
        GENERATION.to_string(),
    ));
    let request = lifecycle_request(
        "close-concurrent",
        "s-lifecycle-concurrent",
        0,
        SessionLifecycleAction::Close,
    );

    let first = {
        let usecase = usecase.clone();
        let request = request.clone();
        tokio::spawn(async move { usecase.request(request).await })
    };
    gate.first_entered.notified().await;
    let winner = usecase.request(request).await.unwrap();
    gate.release_first.notify_one();
    let stale = first.await.unwrap().unwrap();

    let SessionLifecycleCommandResult::Accepted {
        receipt: winner_receipt,
        state: winner_state,
    } = winner
    else {
        panic!("concurrent winner must be accepted");
    };
    let SessionLifecycleCommandResult::Accepted {
        receipt: stale_receipt,
        state: stale_state,
    } = stale
    else {
        panic!("stale rejection path must converge to the accepted winner");
    };
    assert_eq!(stale_receipt, winner_receipt);
    assert_eq!(stale_state, winner_state);
    assert_eq!(gate.executions.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn lifecycle_retry_query_failure_keeps_the_same_unknown_request_identity() {
    let repo = FakeRepo::new();
    let gate = FakeLifecycleGate::with_snapshot(open_idle_snapshot(0));
    let usecase = lifecycle_usecase(&repo, &gate);
    let request = lifecycle_request(
        "close-query-loss",
        "s-lifecycle-query-loss",
        0,
        SessionLifecycleAction::Close,
    );
    assert!(matches!(
        usecase.request(request.clone()).await.unwrap(),
        SessionLifecycleCommandResult::Accepted { .. }
    ));
    assert_eq!(gate.execution_count(), 1);
    repo.with_state(|state| state.fail_query = true);

    assert_eq!(
        usecase.request(request).await.unwrap(),
        SessionLifecycleCommandResult::OutcomeUnknown {
            request_id: "close-query-loss".to_string(),
        }
    );
    assert_eq!(gate.execution_count(), 1);
}

#[tokio::test]
async fn b101_replay_returns_same_operation_and_other_principal_not_found() {
    let repo = FakeRepo::new();
    let gate = FakeLifecycleGate::with_snapshot(open_active_snapshot(4, 2));
    let usecase = lifecycle_usecase(&repo, &gate);
    let request = lifecycle_request("close-1", "s-1", 4, SessionLifecycleAction::Close);
    let first = usecase.request(request.clone()).await.unwrap();
    let SessionLifecycleCommandResult::Accepted {
        receipt: first_receipt,
        ..
    } = first
    else {
        panic!("expected acceptance");
    };
    // Restart equivalence: fresh usecase over the same durable state.
    let restarted = lifecycle_usecase(&repo, &gate);
    let replay = restarted.request(request).await.unwrap();
    let SessionLifecycleCommandResult::Accepted { receipt, .. } = replay else {
        panic!("expected replay acceptance");
    };
    assert_eq!(receipt, first_receipt);
    assert_eq!(gate.execution_count(), 1);
    assert_eq!(
        restarted
            .get_operation("p-2", &first_receipt.operation_id)
            .await,
        Err(SessionLifecycleOperationError::NotFound)
    );
    repo.with_state(|state| assert_eq!(state.terminals.len(), 1));
}

#[tokio::test]
async fn b102_join_pending_and_payload_conflict() {
    let repo = FakeRepo::new();
    let gate = FakeLifecycleGate::with_snapshot(open_idle_snapshot(1));
    gate.set_mode(LifecycleExecuteMode::Fail); // stays unresolved
    let usecase = lifecycle_usecase(&repo, &gate);
    let first = usecase
        .request(lifecycle_request(
            "arch-1",
            "s-1",
            1,
            SessionLifecycleAction::ArchiveOpen,
        ))
        .await
        .unwrap();
    let SessionLifecycleCommandResult::Accepted {
        receipt: first_receipt,
        state,
    } = first
    else {
        panic!("expected acceptance");
    };
    assert!(matches!(
        state,
        SessionLifecycleOperationState::ReconciliationRequired { .. }
    ));
    // Same principal, same session, same action, different request ID: join.
    let join = usecase
        .request(lifecycle_request(
            "arch-2",
            "s-1",
            1,
            SessionLifecycleAction::ArchiveOpen,
        ))
        .await
        .unwrap();
    let SessionLifecycleCommandResult::Accepted { receipt, .. } = join else {
        panic!("expected join acceptance");
    };
    assert_eq!(receipt.operation_id, first_receipt.operation_id);
    assert_eq!(receipt.first_accepted_revision, 1);
    // Different action against the same unresolved session: PendingOperation.
    let other_action = usecase
        .request(lifecycle_request(
            "close-9",
            "s-1",
            1,
            SessionLifecycleAction::Close,
        ))
        .await
        .unwrap();
    assert_eq!(
        other_action,
        SessionLifecycleCommandResult::Rejected(SessionLifecycleRejection::PendingOperation)
    );
    // Reusing a bound request ID for a different payload: PayloadConflict.
    let conflict = usecase
        .request(lifecycle_request(
            "arch-1",
            "s-1",
            2,
            SessionLifecycleAction::ArchiveOpen,
        ))
        .await;
    assert_eq!(
        conflict,
        Err(SessionLifecycleOperationError::PayloadConflict)
    );
    // A joiner's caller binding remains sufficient after the pending index
    // is resolved and after a process restart; current session state is not
    // used to reconstruct a different operation.
    repo.with_state(|state| {
        for (_, pending, _) in state.obligations.values_mut() {
            *pending = false;
        }
    });
    let restarted = lifecycle_usecase(&repo, &gate);
    let replay = restarted
        .request(lifecycle_request(
            "arch-2",
            "s-1",
            1,
            SessionLifecycleAction::ArchiveOpen,
        ))
        .await
        .unwrap();
    let SessionLifecycleCommandResult::Accepted { receipt, .. } = replay else {
        panic!("expected durable join replay");
    };
    assert_eq!(receipt.operation_id, first_receipt.operation_id);
    assert_eq!(gate.execution_count(), 1);
}

#[tokio::test]
async fn b102_completed_lifecycle_slot_is_reused_by_the_next_legal_action() {
    let repo = FakeRepo::new();
    let gate = FakeLifecycleGate::with_snapshot(open_idle_snapshot(0));
    let usecase = lifecycle_usecase(&repo, &gate);

    let close = usecase
        .request(lifecycle_request(
            "close-sequential",
            "s-sequential",
            0,
            SessionLifecycleAction::Close,
        ))
        .await
        .unwrap();
    assert!(matches!(
        close,
        SessionLifecycleCommandResult::Accepted {
            state: SessionLifecycleOperationState::Completed,
            ..
        }
    ));

    gate.set_snapshot(SessionLifecycleSnapshot {
        session_revision: 1,
        lifecycle: SessionLifecycleState::Closed,
        queue_paused: true,
        has_runtime: false,
        has_pending_permission: false,
        has_pending_recovery: false,
        has_pending_provider_operation: false,
    });
    let archive = usecase
        .request(lifecycle_request(
            "archive-sequential",
            "s-sequential",
            1,
            SessionLifecycleAction::ArchiveClosed,
        ))
        .await
        .unwrap();
    assert!(matches!(
        archive,
        SessionLifecycleCommandResult::Accepted {
            state: SessionLifecycleOperationState::Completed,
            ..
        }
    ));
    // Closed archive is projection-only: it must not manufacture a runtime
    // close effect or recycle the completed close obligation.
    assert_eq!(gate.execution_count(), 1);
    repo.with_state(|state| {
        let (_, pending, revision) = state
            .obligations
            .values()
            .next()
            .expect("reused lifecycle obligation slot");
        assert!(!pending);
        assert_eq!(*revision, 1);
    });
}

#[tokio::test]
async fn b054_runtime_free_close_completes_in_acceptance_without_empty_effect_obligation() {
    let repo = FakeRepo::new();
    let mut snapshot = open_idle_snapshot(0);
    snapshot.has_runtime = false;
    let gate = FakeLifecycleGate::with_snapshot(snapshot);
    let usecase = lifecycle_usecase(&repo, &gate);

    let result = usecase
        .request(lifecycle_request(
            "close-without-runtime",
            "s-runtime-free",
            0,
            SessionLifecycleAction::Close,
        ))
        .await
        .unwrap();

    assert!(matches!(
        result,
        SessionLifecycleCommandResult::Accepted {
            state: SessionLifecycleOperationState::Completed,
            ..
        }
    ));
    assert_eq!(gate.execution_count(), 0);
    repo.with_state(|state| {
        assert!(state.obligations.is_empty());
        assert!(state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::SessionClosed { .. })
        )));
        assert!(state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueuePaused { .. })
        )));
    });
}

#[tokio::test]
async fn b103_pre_acceptance_rejections_change_nothing() {
    // Revision conflict returns the current revision without effects.
    let repo = FakeRepo::new();
    let gate = FakeLifecycleGate::with_snapshot(open_idle_snapshot(5));
    let usecase = lifecycle_usecase(&repo, &gate);
    let result = usecase
        .request(lifecycle_request(
            "close-1",
            "s-1",
            4,
            SessionLifecycleAction::Close,
        ))
        .await
        .unwrap();
    assert_eq!(
        result,
        SessionLifecycleCommandResult::Rejected(SessionLifecycleRejection::RevisionConflict {
            current_revision: 5
        })
    );
    assert_eq!(gate.execution_count(), 0);
    repo.with_state(|state| {
        assert!(state.events.is_empty());
        assert!(state.records.is_empty());
    });

    // Invalid state: closing an archived session.
    let repo2 = FakeRepo::new();
    let gate2 = FakeLifecycleGate::with_snapshot(SessionLifecycleSnapshot {
        session_revision: 0,
        lifecycle: SessionLifecycleState::Archived,
        queue_paused: false,
        has_runtime: false,
        has_pending_permission: false,
        has_pending_recovery: false,
        has_pending_provider_operation: false,
    });
    let usecase2 = lifecycle_usecase(&repo2, &gate2);
    let result = usecase2
        .request(lifecycle_request(
            "close-2",
            "s-2",
            0,
            SessionLifecycleAction::Close,
        ))
        .await
        .unwrap();
    assert_eq!(
        result,
        SessionLifecycleCommandResult::Rejected(SessionLifecycleRejection::InvalidState)
    );
    assert_eq!(gate2.execution_count(), 0);
}

#[tokio::test]
async fn b095_acceptance_save_failure_leaves_open_state_with_zero_effects() {
    let repo = FakeRepo::new();
    let gate = FakeLifecycleGate::with_snapshot(open_active_snapshot(4, 2));
    let usecase = lifecycle_usecase(&repo, &gate);
    repo.with_state(|state| {
        state.fail_commit_once = Some(CommitBatchError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                true,
                "disk gone",
                "corr-close",
            ),
        });
    });
    let result = usecase
        .request(lifecycle_request(
            "close-1",
            "s-1",
            4,
            SessionLifecycleAction::Close,
        ))
        .await
        .unwrap();
    assert!(matches!(
        result,
        SessionLifecycleCommandResult::Rejected(SessionLifecycleRejection::Failed { .. })
    ));
    assert_eq!(gate.execution_count(), 0);
    repo.with_state(|state| {
        assert!(state.events.is_empty());
        assert!(state.terminals.is_empty());
    });
    // After storage recovers, the same request is accepted once.
    let accepted = usecase
        .request(lifecycle_request(
            "close-1",
            "s-1",
            4,
            SessionLifecycleAction::Close,
        ))
        .await
        .unwrap();
    assert!(matches!(
        accepted,
        SessionLifecycleCommandResult::Accepted { .. }
    ));
    assert_eq!(gate.execution_count(), 1);
    repo.with_state(|state| assert_eq!(state.terminals.len(), 1));
}

#[tokio::test]
async fn lifecycle_identity_bounds_are_enforced() {
    let repo = FakeRepo::new();
    let gate = FakeLifecycleGate::with_snapshot(open_idle_snapshot(0));
    let usecase = lifecycle_usecase(&repo, &gate);
    let ok_1 = "a".to_string();
    let ok_128 = "a".repeat(128);
    for (index, id) in [ok_1.as_str(), ok_128.as_str()].iter().enumerate() {
        let session = format!("s-ok-{index}");
        let result = usecase
            .request(lifecycle_request(
                id,
                &session,
                0,
                SessionLifecycleAction::Close,
            ))
            .await
            .unwrap();
        assert!(matches!(
            result,
            SessionLifecycleCommandResult::Accepted { .. }
        ));
    }
    let too_long = "a".repeat(129);
    for invalid in ["", too_long.as_str(), "リクエスト", "bad/char"] {
        let result = usecase
            .request(lifecycle_request(
                invalid,
                "s-1",
                0,
                SessionLifecycleAction::Close,
            ))
            .await;
        assert_eq!(result, Err(SessionLifecycleOperationError::InvalidRequest));
    }
}

// --- Stop: B-028..B-034, B-080, B-087 ------------------------------------

#[tokio::test]
async fn b030_stop_replay_join_and_payload_conflict_preserve_one_effect() {
    let repo = FakeRepo::new();
    let gate = FakeStopGate::active(4, "1");
    gate.set_mode(StopInterruptMode::Fail);
    let usecase = stop_usecase(&repo, &gate);

    let first = usecase
        .request(stop_request("stop-1", "s-1", "1", 4))
        .await
        .unwrap();
    let StopCommandOutcome::Accepted {
        receipt: first_receipt,
        state: first_state,
    } = first
    else {
        panic!("expected accepted Stop");
    };
    assert!(matches!(
        first_state,
        StopOperationState::ReconciliationRequired { .. }
    ));

    // A different caller request identity joins the unresolved target. Its
    // supplied revision does not replace the first acceptance guard.
    let joined = usecase
        .request(stop_request("stop-2", "s-1", "1", 999))
        .await
        .unwrap();
    let StopCommandOutcome::Accepted {
        receipt: joined_receipt,
        state: joined_state,
    } = joined
    else {
        panic!("expected joined Stop");
    };
    assert_eq!(joined_receipt, first_receipt);
    assert_eq!(joined_receipt.accepted_revision, 4);
    assert_eq!(joined_state, first_state);

    for changed in [
        stop_request("stop-1", "s-2", "1", 4),
        stop_request("stop-1", "s-1", "2", 4),
        stop_request("stop-1", "s-1", "1", 5),
    ] {
        assert_eq!(
            usecase.request(changed).await,
            Err(StopOperationError::PayloadConflict)
        );
    }

    let restarted = stop_usecase(&repo, &gate);
    let replay = restarted
        .request(stop_request("stop-1", "s-1", "1", 4))
        .await
        .unwrap();
    assert_eq!(
        replay,
        StopCommandOutcome::Accepted {
            receipt: first_receipt.clone(),
            state: first_state.clone(),
        }
    );
    assert_eq!(
        restarted
            .get_operation("p-2", &first_receipt.operation_id)
            .await,
        Err(StopOperationError::NotFound)
    );
    assert_eq!(gate.interrupt_count(), 1);
    repo.with_state(|state| {
        assert_eq!(state.terminals.len(), 0);
        assert_eq!(state.stop_resolutions.len(), 0);
        assert_eq!(
            state
                .events
                .iter()
                .filter(|event| matches!(
                    event,
                    LocalDomainEvent::AgentSession(AgentSessionDomainEvent::ObligationRecorded {
                        kind:
                            crate::domain::agent_session::events::ObligationKind::ProviderInterrupt,
                        state:
                            crate::domain::agent_session::events::ObligationState::EffectReserved,
                        ..
                    })
                ))
                .count(),
            1,
            "Stop acceptance durably reserves the sole provider effect",
        );
        assert_eq!(
            state
                .obligations
                .values()
                .filter(|(_, pending, _)| *pending)
                .count(),
            1
        );
    });
}

#[tokio::test]
async fn stop_rejects_when_target_revision_changes_after_snapshot_before_acceptance() {
    let repo = FakeRepo::new();
    let gate = FakeStopGate::active(4, "1");
    gate.set_stale_on_acceptance();
    let usecase = stop_usecase(&repo, &gate);

    assert_eq!(
        usecase
            .request(stop_request(
                "stop-stale-during-acceptance",
                "s-stale-during-acceptance",
                "1",
                4,
            ))
            .await,
        Err(StopOperationError::StaleTarget)
    );
    assert_eq!(gate.interrupt_count(), 0);
    repo.with_state(|state| {
        assert_eq!(state.commit_calls, 0);
        assert!(state.events.is_empty());
        assert!(state.records.is_empty());
        assert!(state.obligations.is_empty());
    });
}

#[tokio::test]
async fn stop_classifies_post_preparation_target_drift_as_stale_without_effect() {
    for conflict in [
        CommitBatchError::PayloadConflict,
        CommitBatchError::StreamHeadConflict {
            current: StreamVersion::new(1).unwrap(),
        },
    ] {
        let repo = FakeRepo::new();
        let gate = FakeStopGate::active(4, "1");
        gate.set_revision_after_acceptance(5);
        let usecase = stop_usecase(&repo, &gate);
        repo.with_state(|state| state.fail_commit_once = Some(conflict));

        assert_eq!(
            usecase
                .request(stop_request(
                    "stop-stale-after-preparation",
                    "s-stale-after-preparation",
                    "1",
                    4,
                ))
                .await,
            Err(StopOperationError::StaleTarget)
        );
        assert_eq!(gate.interrupt_count(), 0);
        repo.with_state(|state| {
            assert_eq!(state.commit_calls, 1);
            assert!(state.events.is_empty());
            assert!(state.records.is_empty());
            assert!(state.obligations.is_empty());
        });
    }
}

#[tokio::test]
async fn stop_preserves_snapshot_failure_when_classifying_acceptance_conflict() {
    let repo = FakeRepo::new();
    let gate = FakeStopGate::active(4, "1");
    gate.set_failure_after_acceptance(SafeOperationFailure::new(
        SessionOperationFailureKind::StorageUnavailable,
        true,
        "target snapshot read failed",
        "stop-conflict-snapshot",
    ));
    let usecase = stop_usecase(&repo, &gate);
    repo.with_state(|state| {
        state.fail_commit_once = Some(CommitBatchError::StreamHeadConflict {
            current: StreamVersion::new(1).unwrap(),
        });
    });

    assert!(matches!(
        usecase
            .request(stop_request(
                "stop-conflict-read-failure",
                "s-conflict-read-failure",
                "1",
                4,
            ))
            .await,
        Err(StopOperationError::StorageUnavailable { failure })
            if failure.correlation_id == "stop-conflict-snapshot"
    ));
    assert_eq!(gate.interrupt_count(), 0);
    repo.with_state(|state| {
        assert_eq!(state.commit_calls, 1);
        assert!(state.events.is_empty());
        assert!(state.records.is_empty());
        assert!(state.obligations.is_empty());
    });
}

#[tokio::test]
async fn b031_stop_capacity_rejects_the_thirty_third_distinct_target() {
    let repo = FakeRepo::new();
    let gate = FakeStopGate::active(0, "1");
    gate.set_mode(StopInterruptMode::Fail);
    let usecase = stop_usecase(&repo, &gate);

    for index in 0..32 {
        let outcome = usecase
            .request(stop_request(
                &format!("stop-{index}"),
                &format!("s-{index}"),
                "1",
                0,
            ))
            .await
            .unwrap();
        assert!(matches!(outcome, StopCommandOutcome::Accepted { .. }));
    }
    assert_eq!(
        usecase
            .request(stop_request("stop-32", "s-32", "1", 0))
            .await,
        Err(StopOperationError::CapacityExceeded)
    );
    assert_eq!(gate.interrupt_count(), 32);
    repo.with_state(|state| {
        assert_eq!(state.terminals.len(), 0);
        assert_eq!(state.stop_resolutions.len(), 0);
        assert_eq!(
            state
                .obligations
                .values()
                .filter(|(_, pending, _)| *pending)
                .count(),
            32
        );
    });
}

#[tokio::test(start_paused = true)]
async fn b025_b028_hanging_stop_converges_to_one_timeout_terminal_with_queue_paused() {
    let repo = FakeRepo::new();
    let gate = FakeStopGate::active(0, "1");
    gate.set_mode(StopInterruptMode::Hang);
    let usecase = stop_usecase(&repo, &gate);
    let request = stop_request("stop-timeout", "s-timeout", "1", 0);

    let outcome = usecase.request(request.clone()).await.unwrap();
    let StopCommandOutcome::Accepted { receipt, state } = outcome else {
        panic!("expected accepted Stop");
    };
    assert_eq!(
        state,
        StopOperationState::Completed {
            resolution: StopResolution::Succeeded,
        }
    );
    assert_eq!(gate.interrupt_count(), 1);
    assert_eq!(gate.timeout_terminal_commit_count(), 1);
    assert_eq!(
        usecase.get_operation("p-1", &receipt.operation_id).await,
        Ok((
            receipt.clone(),
            StopOperationState::Completed {
                resolution: StopResolution::Succeeded,
            },
        ))
    );

    // A response-loss/restart replay cannot schedule another interrupt or
    // append another terminal after the timeout winner has been sealed.
    let restarted = stop_usecase(&repo, &gate);
    let replay = restarted.request(request).await.unwrap();
    assert_eq!(
        replay,
        StopCommandOutcome::Accepted {
            receipt,
            state: StopOperationState::Completed {
                resolution: StopResolution::Succeeded,
            },
        }
    );
    assert_eq!(gate.interrupt_count(), 1);
    repo.with_state(|state| {
        assert_eq!(state.terminals.len(), 1);
        assert_eq!(state.stop_resolutions.len(), 1);
        assert_eq!(
            state
                .events
                .iter()
                .filter(|event| matches!(
                    event,
                    LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueuePaused { .. })
                ))
                .count(),
            1,
            "B025: an accepted Stop seals the content-preserving queue pause with its terminal"
        );
        assert!(!state.events.iter().any(|event| matches!(
            event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::QueueResumed { .. })
        )));
        assert_eq!(
            state
                .obligations
                .values()
                .filter(|(_, pending, _)| *pending)
                .count(),
            0
        );
        assert_eq!(
            state
                .events
                .iter()
                .filter(|event| matches!(
                    event,
                    LocalDomainEvent::AgentSession(AgentSessionDomainEvent::TurnInterrupted {
                        reason: InterruptReason::Timeout,
                        ..
                    })
                ))
                .count(),
            1
        );
    });
}

#[tokio::test(start_paused = true)]
async fn stop_timeout_fences_from_commit_proof_when_terminal_readback_is_unavailable() {
    let repo = FakeRepo::new();
    let gate = FakeStopGate::active(0, "1");
    gate.set_mode(StopInterruptMode::Hang);
    repo.with_state(|state| {
        // Acceptance is commit one and the timeout terminal is commit two.
        // Any query of that terminal after commit must not be required to
        // fence the still-live process-local runtime.
        state.fail_terminal_query_after_commit_call = Some(2);
    });
    let usecase = stop_usecase(&repo, &gate);

    let outcome = usecase
        .request(stop_request(
            "stop-timeout-query-outage",
            "s-timeout-query-outage",
            "1",
            0,
        ))
        .await
        .unwrap();

    assert!(matches!(
        outcome,
        StopCommandOutcome::Accepted {
            state: StopOperationState::Completed {
                resolution: StopResolution::Succeeded,
            },
            ..
        }
    ));
    assert_eq!(gate.interrupt_count(), 1);
    assert_eq!(gate.timeout_terminal_commit_count(), 1);
    repo.with_state(|state| {
        assert_eq!(state.terminals.len(), 1);
        assert_eq!(state.stop_resolutions.len(), 1);
        assert_eq!(
            state
                .obligations
                .values()
                .filter(|(_, pending, _)| *pending)
                .count(),
            0
        );
    });
}

#[tokio::test(start_paused = true)]
async fn stop_timeout_result_loss_fences_from_completed_operation_readback() {
    let repo = FakeRepo::new();
    let gate = FakeStopGate::active(0, "1");
    gate.set_mode(StopInterruptMode::Hang);
    repo.with_state(|state| {
        // Apply the timeout batch but lose its reply, then make commit
        // resolution transiently unavailable. The completed Stop readback is
        // still durable proof that this Stop owns the terminal.
        state.outcome_unknown_after_commit_on_call = Some(2);
        state.fail_resolve_commit_once = true;
    });
    let usecase = stop_usecase(&repo, &gate);

    let outcome = usecase
        .request(stop_request(
            "stop-timeout-result-loss",
            "s-timeout-result-loss",
            "1",
            0,
        ))
        .await
        .unwrap();

    assert!(matches!(
        outcome,
        StopCommandOutcome::Accepted {
            state: StopOperationState::Completed {
                resolution: StopResolution::Succeeded,
            },
            ..
        }
    ));
    assert_eq!(gate.interrupt_count(), 1);
    assert_eq!(gate.timeout_terminal_commit_count(), 1);
    repo.with_state(|state| {
        assert_eq!(state.commit_calls, 2);
        assert_eq!(state.terminals.len(), 1);
        assert_eq!(state.stop_resolutions.len(), 1);
        assert_eq!(
            state
                .obligations
                .values()
                .filter(|(_, pending, _)| *pending)
                .count(),
            0
        );
    });
}

#[tokio::test]
async fn stop_timeout_restart_recovery_fences_without_terminal_readback() {
    let repo = FakeRepo::new();
    let gate = FakeStopGate::active(0, "1");
    gate.set_mode(StopInterruptMode::Fail);
    let usecase = stop_usecase(&repo, &gate);

    let outcome = usecase
        .request(stop_request(
            "stop-timeout-restart",
            "s-timeout-restart",
            "1",
            0,
        ))
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        StopCommandOutcome::Accepted {
            state: StopOperationState::ReconciliationRequired { .. },
            ..
        }
    ));
    repo.with_state(|state| {
        let (obligation, pending, _) = state
            .obligations
            .values_mut()
            .next()
            .expect("pending Stop obligation");
        assert!(*pending);
        let ObligationRecord::StopInterrupt { deadline_ms, .. } = obligation else {
            panic!("expected Stop obligation");
        };
        *deadline_ms = 0;
        // Acceptance and reconciliation are commits one and two. Recovery's
        // timeout terminal is commit three.
        state.fail_terminal_query_after_commit_call = Some(3);
    });

    let restarted = stop_usecase(&repo, &gate);
    restarted.recover_pending_stops().await.unwrap();

    assert_eq!(
        gate.interrupt_count(),
        1,
        "restart never reissues interrupt"
    );
    assert_eq!(gate.timeout_terminal_commit_count(), 1);
    repo.with_state(|state| {
        assert_eq!(state.terminals.len(), 1);
        assert_eq!(state.stop_resolutions.len(), 1);
        assert_eq!(
            state
                .obligations
                .values()
                .filter(|(_, pending, _)| *pending)
                .count(),
            0
        );
    });
}

#[tokio::test]
async fn b032_stop_acceptance_storage_failure_has_zero_effects() {
    let repo = FakeRepo::new();
    let gate = FakeStopGate::active(0, "1");
    let usecase = stop_usecase(&repo, &gate);
    repo.with_state(|state| {
        state.fail_commit_once = Some(CommitBatchError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                true,
                "disk unavailable",
                "stop-acceptance",
            ),
        });
    });

    let outcome = usecase
        .request(stop_request("stop-storage", "s-storage", "1", 0))
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        StopCommandOutcome::RejectedBeforeCommit { .. }
    ));
    assert_eq!(gate.interrupt_count(), 0);
    repo.with_state(|state| {
        assert!(state.events.is_empty());
        assert!(state.terminals.is_empty());
        assert!(state.obligations.is_empty());
    });
}

#[tokio::test]
async fn b033_b034_b080_terminal_failure_restart_manual_race_recovers_once_and_reuses_capacity() {
    let repo = FakeRepo::new();
    let gate = FakeStopGate::active(0, "1");
    let usecase = stop_usecase(&repo, &gate);
    repo.with_state(|state| {
        state.fail_commit_on_call = Some((
            2,
            CommitBatchError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "terminal store unavailable",
                    "stop-terminal",
                ),
            },
        ));
    });

    let outcome = usecase
        .request(stop_request("stop-terminal", "s-terminal", "1", 0))
        .await
        .unwrap();
    let StopCommandOutcome::Accepted { receipt, state } = outcome else {
        panic!("expected accepted Stop");
    };
    assert!(matches!(
        state,
        StopOperationState::ReconciliationRequired { .. }
    ));
    let restarted = stop_usecase(&repo, &gate);
    let (saved_receipt, saved_state) = restarted
        .get_operation("p-1", &receipt.operation_id)
        .await
        .unwrap();
    assert_eq!(saved_receipt, receipt);
    assert!(matches!(
        saved_state,
        StopOperationState::ReconciliationRequired { .. }
    ));
    assert_eq!(gate.interrupt_count(), 1);
    repo.with_state(|state| {
        assert_eq!(state.terminals.len(), 0);
        assert_eq!(state.stop_resolutions.len(), 0);
        assert_eq!(
            state
                .obligations
                .values()
                .filter(|(_, pending, _)| *pending)
                .count(),
            1
        );
        let (record, _, _) = state
            .obligations
            .values_mut()
            .next()
            .expect("pending Stop obligation");
        let ObligationRecord::StopInterrupt { deadline_ms, .. } = record else {
            panic!("expected stop obligation");
        };
        *deadline_ms = 0;
    });

    let (startup_recovery, manual_retry) = tokio::join!(
        restarted.recover_pending_stops(),
        restarted.recover_pending_stops(),
    );
    startup_recovery.unwrap();
    manual_retry.unwrap();
    assert_eq!(
        restarted
            .get_operation("p-1", &saved_receipt.operation_id)
            .await,
        Ok((
            saved_receipt,
            StopOperationState::Completed {
                resolution: StopResolution::Succeeded,
            },
        ))
    );
    assert_eq!(
        gate.interrupt_count(),
        1,
        "recovery never reissues interrupt"
    );
    repo.with_state(|state| {
        assert_eq!(state.terminals.len(), 1);
        assert_eq!(state.stop_resolutions.len(), 1);
        assert_eq!(
            state
                .obligations
                .values()
                .filter(|(_, pending, _)| *pending)
                .count(),
            0
        );
    });

    let reused = restarted
        .request(stop_request(
            "stop-after-capacity-release",
            "s-after-capacity-release",
            "1",
            0,
        ))
        .await
        .expect("a resolved Stop releases capacity for another target");
    assert!(matches!(reused, StopCommandOutcome::Accepted { .. }));
    assert_eq!(gate.interrupt_count(), 2);
}

#[tokio::test]
async fn b087_stop_identity_bounds_reject_without_acceptance_or_effect() {
    for valid in ["a".to_string(), "a".repeat(128)] {
        let repo = FakeRepo::new();
        let gate = FakeStopGate::active(0, "1");
        let usecase = stop_usecase(&repo, &gate);
        let outcome = usecase
            .request(stop_request(&valid, "s-valid", "1", 0))
            .await
            .expect("boundary-valid Stop identity");
        assert!(matches!(outcome, StopCommandOutcome::Accepted { .. }));
        assert_eq!(gate.interrupt_count(), 1);
        repo.with_state(|state| assert!(state.commit_calls > 0));
    }

    let repo = FakeRepo::new();
    let gate = FakeStopGate::active(0, "1");
    let usecase = stop_usecase(&repo, &gate);
    let too_long = "a".repeat(129);
    for invalid in ["", too_long.as_str(), "停止", "bad/char", "sp ace"] {
        assert_eq!(
            usecase
                .request(stop_request(invalid, "s-invalid", "1", 0))
                .await,
            Err(StopOperationError::InvalidRequest)
        );
    }
    assert_eq!(gate.interrupt_count(), 0);
    repo.with_state(|state| {
        assert_eq!(state.commit_calls, 0);
        assert!(state.events.is_empty());
        assert!(state.obligations.is_empty());
    });
}

fn permission_obligation(
    state: ObligationStateRecord,
    valid_owner_payload: bool,
) -> ObligationRecord {
    ObligationRecord::PermissionResponse {
        operation_id: "permission-operation-1".to_string(),
        effect_identity: "permission-response:permission-operation-1".to_string(),
        session_id: "permission-session".to_string(),
        turn_id: "7".to_string(),
        response: permission_allow(
            "permission-1",
            r#"{"command":"echo exact"}"#,
            r#"{"question":"yes"}"#,
        ),
        owner_access: valid_owner_payload,
        from_runtime_state: true,
        state,
    }
}

fn observed_stop_obligation(
    obligation_id: &str,
    classification: RecoveryResultClassification,
    cancellable: bool,
) -> ObligationRecord {
    let classification = match classification {
        RecoveryResultClassification::Pending => "pending",
        RecoveryResultClassification::Succeeded => "succeeded",
        RecoveryResultClassification::ConfirmedNoEffect => "confirmed_no_effect",
        RecoveryResultClassification::Ambiguous => "ambiguous",
        RecoveryResultClassification::CancelledBeforeEffect => "cancelled_before_effect",
        RecoveryResultClassification::Unchanged => "unchanged",
    };
    let canonical = serde_json::to_vec(&serde_json::json!({
        "schema": "authoritative_effect_observation_v1",
        "effect_identity": obligation_id,
        "origin_revision": 0,
        "classification": classification,
        "cancellable": cancellable,
        "safe_view": "confirmed absent",
    }))
    .unwrap();
    ObligationRecord::Observed {
        original: Box::new(ObligationRecord::StopInterrupt {
            operation_id: obligation_id.to_string(),
            session_id: "classification-session".to_string(),
            turn_id: "1".to_string(),
            expected_revision: 0,
            deadline_ms: 0,
            state: ObligationStateRecord::ReconciliationRequired,
        }),
        observation: AuthoritativeEffectObservationRecord {
            effect_identity: obligation_id.to_string(),
            origin_revision: 0,
            classification: match classification {
                "pending" => RecoveryResultClassification::Pending,
                "succeeded" => RecoveryResultClassification::Succeeded,
                "confirmed_no_effect" => RecoveryResultClassification::ConfirmedNoEffect,
                "ambiguous" => RecoveryResultClassification::Ambiguous,
                "cancelled_before_effect" => RecoveryResultClassification::CancelledBeforeEffect,
                "unchanged" => RecoveryResultClassification::Unchanged,
                _ => unreachable!("closed classification label"),
            },
            cancellable,
            safe_view: "confirmed absent".to_string(),
            result_sha256: FakeAuthority.digest(&canonical),
            proof_mac: FakeAuthority.mac(&canonical),
        },
    }
}

async fn first_recovery_action(
    usecase: &RecoveryActionUsecase,
    action: RecoveryActionKind,
) -> RecoveryActionRequest {
    let page = usecase
        .pending(super::recovery::PendingRecoveryQuery {
            limit: 32,
            partition: None,
            owner: None,
            shutdown_plan: None,
            cursor: None,
        })
        .await
        .unwrap();
    let entry = page.entries.first().expect("pending recovery entry");
    let identity = entry
        .action_identities
        .iter()
        .find(|identity| identity.action == action)
        .expect("backend-issued recovery action");
    RecoveryActionRequest {
        action_id: identity.action_id.clone(),
        obligation_id: entry.obligation_id.clone(),
        origin_revision: identity.origin_revision,
        action,
    }
}

#[tokio::test]
async fn b035_first_pending_page_describes_every_recovery_category_without_session_reads() {
    let repo = FakeRepo::new();
    let fixtures = [
        (
            "01-turn",
            ObligationRecord::Send {
                obligation_id: "01-turn".to_string(),
                operation_id: "send-1".to_string(),
                session_id: "session-1".to_string(),
                kind: SendObligationKindRecord::TurnExecution,
                disposition: SendObligationDispositionRecord::StartedTurn,
                human_message_id: Some("human-1".to_string()),
                assistant_message_id: None,
                reserved_turn_id: None,
                turn_id: Some("1".to_string()),
                dependency_obligation_ids: Vec::new(),
                canonical_payload: "payload-1".to_string(),
                state: ObligationStateRecord::Pending,
            },
            PendingRecoveryCategory::TurnExecution,
            "send-1",
            PendingRecoveryKnownStatus::Pending,
        ),
        (
            "02-queue",
            ObligationRecord::Send {
                obligation_id: "02-queue".to_string(),
                operation_id: "send-2".to_string(),
                session_id: "session-2".to_string(),
                kind: SendObligationKindRecord::TurnExecution,
                disposition: SendObligationDispositionRecord::Queued,
                human_message_id: Some("human-2".to_string()),
                assistant_message_id: None,
                reserved_turn_id: Some("turn-2".to_string()),
                turn_id: Some("turn-2".to_string()),
                dependency_obligation_ids: Vec::new(),
                canonical_payload: "payload-2".to_string(),
                state: ObligationStateRecord::Pending,
            },
            PendingRecoveryCategory::QueueExecution,
            "send-2",
            PendingRecoveryKnownStatus::Pending,
        ),
        (
            "03-permission",
            ObligationRecord::PermissionResponse {
                operation_id: "permission-operation-1".to_string(),
                effect_identity: "permission-response:permission-operation-1".to_string(),
                session_id: "session-3".to_string(),
                turn_id: "3".to_string(),
                response: PermissionResponse {
                    request_id: "permission-1".to_string(),
                    decision: PermissionResponseDecision::Deny { message: None },
                },
                owner_access: true,
                from_runtime_state: true,
                state: ObligationStateRecord::Pending,
            },
            PendingRecoveryCategory::PermissionDelivery,
            "permission-operation-1",
            PendingRecoveryKnownStatus::Pending,
        ),
        (
            "04-provider",
            ObligationRecord::Send {
                obligation_id: "04-provider".to_string(),
                operation_id: "send-4".to_string(),
                session_id: "session-4".to_string(),
                kind: SendObligationKindRecord::ProviderEstablish,
                disposition: SendObligationDispositionRecord::StartedTurn,
                human_message_id: Some("human-4".to_string()),
                assistant_message_id: None,
                reserved_turn_id: None,
                turn_id: None,
                dependency_obligation_ids: Vec::new(),
                canonical_payload: "payload-4".to_string(),
                state: ObligationStateRecord::Prepared,
            },
            PendingRecoveryCategory::ProviderEstablish,
            "send-4",
            PendingRecoveryKnownStatus::Prepared,
        ),
        (
            "05-terminal",
            ObligationRecord::StopInterrupt {
                operation_id: "stop-5".to_string(),
                session_id: "session-5".to_string(),
                turn_id: "5".to_string(),
                expected_revision: 0,
                deadline_ms: 0,
                state: ObligationStateRecord::EffectReserved,
            },
            PendingRecoveryCategory::TerminalCommit,
            "stop-5",
            PendingRecoveryKnownStatus::EffectReserved,
        ),
        (
            "06-close",
            ObligationRecord::SessionClose {
                obligation_id: "06-close".to_string(),
                operation_id: "close-6".to_string(),
                session_id: "session-6".to_string(),
                action: SessionLifecycleRecordAction::Close,
                state: ObligationStateRecord::EffectReserved,
            },
            PendingRecoveryCategory::SessionClose,
            "close-6",
            PendingRecoveryKnownStatus::EffectReserved,
        ),
        (
            "07-backend-recovery",
            ObligationRecord::BackendSessionRecovery {
                session_id: "session-7".to_string(),
                recovery_id: "recovery-7".to_string(),
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                        old_provider_session_generation: 0,
                        reason: crate::domain::agent_session::events::BackendSessionRecoveryReason::BackendSessionLost,
                        reserved_at_bits: 0,
                    },
                state: ObligationStateRecord::ReconciliationRequired,
            },
            PendingRecoveryCategory::BackendRecovery,
            "recovery-7",
            PendingRecoveryKnownStatus::ReconciliationRequired,
        ),
        (
            "08-workflow-shutdown",
            ObligationRecord::WorkflowShutdown {
                operation_id: "quit-8".to_string(),
                effect_identity: "workflow-effect-8".to_string(),
                owner_revision: 0,
                execution_id: "workflow-8".to_string(),
                state: ObligationStateRecord::EffectReserved,
            },
            PendingRecoveryCategory::WorkflowShutdown,
            "workflow-effect-8",
            PendingRecoveryKnownStatus::EffectReserved,
        ),
        (
            "09-publication",
            ObligationRecord::RecoveryPublication {
                session_id: "session-9".to_string(),
                recovery_id: "recovery-9".to_string(),
                message_id: "message-9".to_string(),
                source_obligation_id: "backend-recovery:session-9:recovery-9".to_string(),
                detail: RecoveryPublicationObligationRecord::Pending {
                    pending_message: RecoveryPublicationMessageRecord {
                        kind: RecoveryPublicationMessageKindRecord::Notice,
                        recovery_id: "recovery-9".to_string(),
                        message_id: "message-9".to_string(),
                        error: None,
                    },
                },
                state: ObligationStateRecord::Pending,
            },
            PendingRecoveryCategory::RecoveryPublication,
            "message-9",
            PendingRecoveryKnownStatus::Pending,
        ),
    ];
    for (obligation_id, record, _, _, _) in &fixtures {
        seed_pending_obligation(&repo, obligation_id, record.clone());
    }
    let executor = FakeRecoveryExecutor::returning(
        RecoveryResultClassification::Pending,
        "must not execute during discovery",
    );
    let page = recovery_usecase(&repo, &executor)
        .pending(super::recovery::PendingRecoveryQuery {
            limit: 32,
            partition: None,
            owner: None,
            shutdown_plan: None,
            cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(page.entries.len(), fixtures.len());
    assert!(page.next_cursor.is_none());
    for (obligation_id, _, category, original_identity, known_status) in fixtures {
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.obligation_id == obligation_id)
            .expect("fixture must be listed on the first pending-index page");
        assert_eq!(entry.category, category);
        assert_eq!(entry.original_identity, original_identity);
        assert_eq!(entry.known_status, known_status);
        assert_eq!(
            entry.partition,
            crate::domain::local_event::PendingPartition::Owner
        );
        assert!(!entry.owner.is_empty());
        assert_eq!(entry.state, RecoveryResourceState::Pending);
        assert!(!entry.actions.is_empty());
        assert_eq!(entry.actions.len(), entry.action_identities.len());
    }
    assert_eq!(executor.effect_count(), 0);
    repo.with_state(|state| {
        assert_eq!(state.pending_page_queries, vec![(None, None)]);
        assert_eq!(state.commit_calls, 0);
    });
}

#[tokio::test]
async fn pending_send_handoff_is_internal_but_claimed_effects_remain_recoverable() {
    let repo = FakeRepo::new();
    let live = [
        (
            "live-turn-operation.exec",
            "live-turn-operation",
            SendObligationKindRecord::TurnExecution,
            ObligationStateRecord::Pending,
            OperationStatusValue::AwaitingProviderStart {
                dependency_obligation_ids: Vec::new(),
            },
        ),
        (
            "reserved-turn-operation.exec",
            "reserved-turn-operation",
            SendObligationKindRecord::TurnExecution,
            ObligationStateRecord::EffectReserved,
            OperationStatusValue::Running {
                turn_id: "1".to_string(),
            },
        ),
    ];
    for (obligation_id, operation_id, kind, state, status) in &live {
        seed_pending_obligation(
            &repo,
            obligation_id,
            send_obligation_in_state(obligation_id, operation_id, *kind, *state),
        );
        seed_send_operation_status(&repo, operation_id, "s-1", status.clone());
    }

    let unresolved = [
        (
            "send-reconciliation-required",
            ObligationStateRecord::ReconciliationRequired,
        ),
        ("send-failed", ObligationStateRecord::Failed),
        (
            "send-outcome-unknown",
            ObligationStateRecord::OutcomeUnknown,
        ),
    ];
    for (obligation_id, state) in unresolved {
        seed_pending_obligation(
            &repo,
            obligation_id,
            send_obligation_in_state(
                obligation_id,
                &format!("{obligation_id}-operation"),
                SendObligationKindRecord::TurnExecution,
                state,
            ),
        );
    }

    let effect_reserved_operation = "effect-reserved-reconciliation-operation";
    let effect_reserved_reconciliation = format!("{effect_reserved_operation}.establish");
    let blocked_turn_execution = format!("{effect_reserved_operation}.exec");
    seed_pending_obligation(
        &repo,
        &effect_reserved_reconciliation,
        send_obligation_in_state(
            &effect_reserved_reconciliation,
            effect_reserved_operation,
            SendObligationKindRecord::ProviderEstablish,
            ObligationStateRecord::EffectReserved,
        ),
    );
    let mut blocked_execution = send_obligation_in_state(
        &blocked_turn_execution,
        effect_reserved_operation,
        SendObligationKindRecord::TurnExecution,
        ObligationStateRecord::Pending,
    );
    let ObligationRecord::Send {
        dependency_obligation_ids,
        ..
    } = &mut blocked_execution
    else {
        unreachable!("send helper must return a send obligation");
    };
    *dependency_obligation_ids = vec![effect_reserved_reconciliation.clone()];
    seed_pending_obligation(&repo, &blocked_turn_execution, blocked_execution);
    seed_send_operation_status(
        &repo,
        effect_reserved_operation,
        "s-1",
        OperationStatusValue::ReconciliationRequired {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::OutcomeUnknown,
                true,
                "provider establishment requires readback",
                "effect-reserved-reconciliation",
            ),
        },
    );

    let executor =
        FakeRecoveryExecutor::returning(RecoveryResultClassification::Pending, "must not execute");
    let usecase = recovery_usecase(&repo, &executor);
    let page = usecase
        .pending(super::recovery::PendingRecoveryQuery {
            limit: 32,
            partition: None,
            owner: None,
            shutdown_plan: None,
            cursor: None,
        })
        .await
        .unwrap();

    assert!(
        page.entries
            .iter()
            .all(|entry| entry.obligation_id != live[0].0),
        "a canonical unclaimed turn handoff is internal send progress"
    );
    assert!(
        page.entries
            .iter()
            .all(|entry| entry.obligation_id != blocked_turn_execution),
        "the pending turn blocked behind an ambiguous establish is internal progress"
    );
    for obligation_id in [
        "send-reconciliation-required",
        "send-failed",
        "send-outcome-unknown",
        live[1].0,
        effect_reserved_reconciliation.as_str(),
    ] {
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.obligation_id == obligation_id)
            .expect("genuine unresolved send remains publicly recoverable");
        assert!(entry.actions.contains(&RecoveryActionKind::ReadAgain));
        assert!(entry
            .actions
            .contains(&RecoveryActionKind::KeepForManualResolution));
    }

    for obligation_id in [live[0].0, blocked_turn_execution.as_str()] {
        let action_id = super::recovery::derive_recovery_action_id(
            &FakeAuthority,
            GENERATION,
            obligation_id,
            0,
            RecoveryActionKind::KeepForManualResolution,
        );
        assert_eq!(
            usecase
                .request(RecoveryActionRequest {
                    action_id: action_id.clone(),
                    obligation_id: obligation_id.to_string(),
                    origin_revision: 0,
                    action: RecoveryActionKind::KeepForManualResolution,
                })
                .await
                .unwrap(),
            RecoveryActionOutcome::Rejected {
                action_id,
                rejection: RecoveryActionRejection::ActionUnavailable,
            }
        );
    }
    assert_eq!(executor.effect_count(), 0);
    repo.with_state(|state| assert_eq!(state.commit_calls, 0));
}

#[tokio::test]
async fn b015_b016_permission_exact_retry_is_closed_and_effect_reserved_never_blind_retries() {
    let repo = FakeRepo::new();
    seed_pending_obligation(
        &repo,
        "permission-response:permission-session:7:permission-1",
        permission_obligation(ObligationStateRecord::Pending, true),
    );
    let executor = FakeRecoveryExecutor::returning(
        RecoveryResultClassification::Succeeded,
        "permission delivered",
    );
    let usecase = recovery_usecase(&repo, &executor);
    let request = first_recovery_action(&usecase, RecoveryActionKind::RetrySameEffect).await;

    let first = usecase.request(request.clone()).await.unwrap();
    assert!(matches!(first, RecoveryActionOutcome::Completed { .. }));
    assert_eq!(executor.effect_count(), 1);
    let effect = executor.effects.lock().unwrap()[0].clone();
    let ObligationRecord::PermissionResponse { response, .. } = &effect.immutable_obligation else {
        panic!("expected exact permission obligation");
    };
    let PermissionResponseDecision::Allow {
        updated_input,
        answers,
    } = &response.decision
    else {
        panic!("expected exact allow response");
    };
    assert_eq!(
        updated_input.as_ref().map(|value| value.as_str()),
        Some(r#"{"command":"echo exact"}"#)
    );
    assert_eq!(
        answers.as_ref().map(|value| value.as_str()),
        Some(r#"{"question":"yes"}"#)
    );

    let replay = usecase.request(request.clone()).await.unwrap();
    assert_eq!(
        executor.effect_count(),
        1,
        "same action replays without a second effect"
    );
    assert_eq!(format!("{replay:?}"), format!("{first:?}"));
    let status = usecase.get_action_status(&request.action_id).await.unwrap();
    let super::recovery::RecoveryActionStatus::Completed { result, .. } = status else {
        panic!("completed permission recovery must have an immutable result");
    };
    assert_eq!(
        result.outcome,
        super::recovery::RecoveryActionResultOutcome::Terminal
    );
    assert_eq!(
        result.classification,
        RecoveryResultClassification::Succeeded
    );
    assert_eq!(result.resource_revision, 2);
    assert_eq!(result.canonical_result_sha256.len(), 64);
    assert!(result
        .canonical_result_sha256
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));

    let deny_repo = FakeRepo::new();
    let deny_obligation_id = "permission-response:permission-operation-deny";
    seed_pending_obligation(
        &deny_repo,
        deny_obligation_id,
        ObligationRecord::PermissionResponse {
            operation_id: "permission-operation-deny".to_string(),
            effect_identity: deny_obligation_id.to_string(),
            session_id: "permission-session".to_string(),
            turn_id: "8".to_string(),
            response: PermissionResponse {
                request_id: "permission-deny".to_string(),
                decision: PermissionResponseDecision::Deny {
                    message: Some("Denied by the exact saved policy response.".to_string()),
                },
            },
            owner_access: true,
            from_runtime_state: true,
            state: ObligationStateRecord::Pending,
        },
    );
    let deny_executor = FakeRecoveryExecutor::returning(
        RecoveryResultClassification::Succeeded,
        "permission denial delivered",
    );
    let deny_usecase = recovery_usecase(&deny_repo, &deny_executor);
    let deny_request =
        first_recovery_action(&deny_usecase, RecoveryActionKind::RetrySameEffect).await;
    let deny_first = deny_usecase.request(deny_request.clone()).await.unwrap();
    assert!(matches!(
        deny_first,
        RecoveryActionOutcome::Completed { .. }
    ));
    let deny_effect = deny_executor.effects.lock().unwrap()[0].clone();
    assert!(matches!(
        &deny_effect.immutable_obligation,
        ObligationRecord::PermissionResponse {
            response: PermissionResponse {
                decision: PermissionResponseDecision::Deny { message: Some(message) },
                ..
            },
            ..
        } if message == "Denied by the exact saved policy response."
    ));
    assert_eq!(
        deny_usecase.request(deny_request).await.unwrap(),
        deny_first
    );
    assert_eq!(deny_executor.effect_count(), 1);

    let mut malformed_payload = permission_obligation(ObligationStateRecord::Pending, true);
    let ObligationRecord::PermissionResponse { response, .. } = &mut malformed_payload else {
        unreachable!("permission helper returns a permission obligation");
    };
    *response = permission_allow("permission-1", "{not-json", "{}");
    let oversized_json = serde_json::to_string(
        &"x".repeat(crate::domain::local_event::mutation::RECOVERY_RESULT_MAX_BYTES),
    )
    .unwrap();
    let mut oversized_payload = permission_obligation(ObligationStateRecord::Pending, true);
    let ObligationRecord::PermissionResponse { response, .. } = &mut oversized_payload else {
        unreachable!("permission helper returns a permission obligation");
    };
    *response = permission_allow("permission-1", &oversized_json, "{}");

    for (case, record, expected_state) in [
        (
            "owner-access",
            permission_obligation(ObligationStateRecord::Pending, false),
            RecoveryResourceState::Failed,
        ),
        (
            "malformed-exact-payload",
            malformed_payload,
            RecoveryResourceState::Failed,
        ),
        (
            "oversized-exact-payload",
            oversized_payload,
            RecoveryResourceState::Failed,
        ),
        (
            "already-effect-reserved",
            permission_obligation(ObligationStateRecord::EffectReserved, true),
            RecoveryResourceState::Pending,
        ),
    ] {
        let repo = FakeRepo::new();
        seed_pending_obligation(&repo, &format!("permission-invalid-{case}"), record);
        let executor = FakeRecoveryExecutor::returning(
            RecoveryResultClassification::Succeeded,
            "must not execute",
        );
        let usecase = recovery_usecase(&repo, &executor);
        let page = usecase
            .pending(super::recovery::PendingRecoveryQuery {
                limit: 32,
                partition: None,
                owner: None,
                shutdown_plan: None,
                cursor: None,
            })
            .await
            .unwrap();
        let entry = &page.entries[0];
        assert_eq!(entry.original_identity, "permission-operation-1", "{case}");
        assert_eq!(entry.state, expected_state, "{case}");
        assert!(
            entry
                .actions
                .contains(&RecoveryActionKind::KeepForManualResolution),
            "{case} must retain an explicit safe manual action"
        );
        assert!(!entry.actions.contains(&RecoveryActionKind::RetrySameEffect));
        if expected_state == RecoveryResourceState::Failed {
            assert_eq!(
                entry.actions,
                vec![RecoveryActionKind::KeepForManualResolution],
                "{case} must not invent another payload or effect action"
            );
        }
        assert_eq!(executor.effect_count(), 0, "{case}");
    }
}

#[tokio::test]
async fn b015_restart_resumes_the_same_pending_permission_action_once() {
    let repo = FakeRepo::new();
    let obligation_id = "permission-response:permission-session:7:permission-1";
    let action_id = super::recovery::derive_recovery_action_id(
        &FakeAuthority,
        GENERATION,
        obligation_id,
        0,
        RecoveryActionKind::RetrySameEffect,
    );
    let record = ObligationRecord::RecoveryTransition {
        original: Box::new(permission_obligation(ObligationStateRecord::Pending, true)),
        recovery_action: ObligationRecoveryActionRecord {
            action_id: action_id.clone(),
            origin_revision: 0,
            action: RecoveryActionKind::RetrySameEffect,
            effect_identity: obligation_id.to_string(),
            state: ObligationStateRecord::EffectReserved,
            classification: None,
        },
    };
    let attempt = RecoveryAttemptRecord::Obligation {
        obligation_id: obligation_id.to_string(),
        origin_revision: 0,
        action: RecoveryActionKind::RetrySameEffect,
        effect_identity: obligation_id.to_string(),
        state: ObligationStateRecord::EffectReserved,
        failure: None,
    };
    repo.with_state(|state| {
        state
            .obligations
            .insert(obligation_id.to_string(), (record, true, 1));
        state
            .recovery_actions
            .insert(action_id.clone(), ([7; 32], attempt, None, 0));
    });
    let executor = FakeRecoveryExecutor::returning(
        RecoveryResultClassification::Succeeded,
        "permission delivered",
    );
    let usecase = recovery_usecase(&repo, &executor);
    let request = RecoveryActionRequest {
        action_id: action_id.clone(),
        obligation_id: obligation_id.to_string(),
        origin_revision: 0,
        action: RecoveryActionKind::RetrySameEffect,
    };

    let first = usecase.request(request.clone()).await.unwrap();
    assert!(matches!(first, RecoveryActionOutcome::Completed { .. }));
    assert_eq!(executor.effect_count(), 1);
    assert_eq!(usecase.request(request).await.unwrap(), first);
    assert_eq!(executor.effect_count(), 1);
}

#[tokio::test]
async fn b081_b083_unavailable_stale_and_target_drift_are_effect_zero_typed_results() {
    // A RetrySameEffect identity was issued while the exact permission was
    // pending, but the provider claim moved to effect_reserved before use.
    let repo = FakeRepo::new();
    seed_pending_obligation(
        &repo,
        "permission-race",
        permission_obligation(ObligationStateRecord::Pending, true),
    );
    let executor = FakeRecoveryExecutor::returning(
        RecoveryResultClassification::Succeeded,
        "must not execute",
    );
    let usecase = recovery_usecase(&repo, &executor);
    let unavailable = first_recovery_action(&usecase, RecoveryActionKind::RetrySameEffect).await;
    let unavailable_id = unavailable.action_id.clone();
    repo.with_state(|state| {
        state.obligations.get_mut("permission-race").unwrap().0 =
            permission_obligation(ObligationStateRecord::EffectReserved, true);
    });
    assert_eq!(
        usecase.request(unavailable).await.unwrap(),
        RecoveryActionOutcome::Rejected {
            action_id: unavailable_id,
            rejection: RecoveryActionRejection::ActionUnavailable,
        }
    );
    assert_eq!(executor.effect_count(), 0);

    // Revision drift is reported with the fresh revision, not collapsed into
    // InvalidRequest and not persisted as another action attempt.
    let repo = FakeRepo::new();
    seed_pending_obligation(
        &repo,
        "stop-stale",
        test_stop_obligation(
            "stop-stale-operation",
            "s-1",
            "1",
            ObligationStateRecord::ReconciliationRequired,
        ),
    );
    let executor =
        FakeRecoveryExecutor::returning(RecoveryResultClassification::Pending, "still pending");
    let usecase = recovery_usecase(&repo, &executor);
    let stale = first_recovery_action(&usecase, RecoveryActionKind::ReadAgain).await;
    let stale_id = stale.action_id.clone();
    repo.with_state(|state| state.obligations.get_mut("stop-stale").unwrap().2 = 1);
    assert_eq!(
        usecase.request(stale).await.unwrap(),
        RecoveryActionOutcome::Rejected {
            action_id: stale_id,
            rejection: RecoveryActionRejection::RevisionConflict {
                current_revision: 1,
            },
        }
    );
    assert_eq!(executor.effect_count(), 0);

    // The final executor handoff guard has its own closed result so an owner
    // or runtime revision race never becomes a provider call.
    let repo = FakeRepo::new();
    seed_pending_obligation(
        &repo,
        "stop-target-drift",
        test_stop_obligation(
            "stop-target-drift-operation",
            "s-2",
            "2",
            ObligationStateRecord::ReconciliationRequired,
        ),
    );
    let executor =
        FakeRecoveryExecutor::returning(RecoveryResultClassification::Pending, "must not execute");
    executor.set_handoff(RecoveryEffectHandoff::TargetRevisionChanged);
    let usecase = recovery_usecase(&repo, &executor);
    let drift = first_recovery_action(&usecase, RecoveryActionKind::ReadAgain).await;
    let drift_id = drift.action_id.clone();
    assert_eq!(
        usecase.request(drift).await.unwrap(),
        RecoveryActionOutcome::Rejected {
            action_id: drift_id,
            rejection: RecoveryActionRejection::TargetRevisionChanged,
        }
    );
    assert_eq!(executor.effect_count(), 0);
    repo.with_state(|state| assert!(state.recovery_actions.is_empty()));
}

#[tokio::test]
async fn b082_b084_completed_action_replays_only_closed_result_pairs() {
    use super::recovery::{RecoveryActionResultOutcome as O, RecoveryActionStatus as S};

    let cases = [
        (
            RecoveryResultClassification::Pending,
            O::Pending,
            RecoveryActionKind::ReadAgain,
        ),
        (
            RecoveryResultClassification::ConfirmedNoEffect,
            O::Pending,
            RecoveryActionKind::ReadAgain,
        ),
        (
            RecoveryResultClassification::Ambiguous,
            O::Pending,
            RecoveryActionKind::ReadAgain,
        ),
        (
            RecoveryResultClassification::Succeeded,
            O::Terminal,
            RecoveryActionKind::ReadAgain,
        ),
        (
            RecoveryResultClassification::CancelledBeforeEffect,
            O::Terminal,
            RecoveryActionKind::CancelIfSafe,
        ),
        (
            RecoveryResultClassification::Unchanged,
            O::Unchanged,
            RecoveryActionKind::KeepForManualResolution,
        ),
    ];
    for (index, (classification, outcome, action)) in cases.into_iter().enumerate() {
        let repo = FakeRepo::new();
        let obligation_id = format!("classification-{index}");
        let operation_id = format!("classification-operation-{index}");
        let record = if action == RecoveryActionKind::CancelIfSafe {
            observed_stop_obligation(
                &obligation_id,
                RecoveryResultClassification::ConfirmedNoEffect,
                true,
            )
        } else {
            test_stop_obligation(
                &operation_id,
                "classification-session",
                &index.to_string(),
                ObligationStateRecord::ReconciliationRequired,
            )
        };
        seed_pending_obligation(&repo, &obligation_id, record);
        if classification == RecoveryResultClassification::Succeeded {
            let turn_id = index.to_string();
            let terminal = TerminalResultRecord::Stop {
                operation_id: operation_id.clone(),
                reason: None,
                exit_code: None,
                result: TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                },
            };
            repo.with_state(|state| {
                state.terminals.insert(
                    ("classification-session".to_string(), turn_id.clone()),
                    (
                        format!("classification-terminal-{index}"),
                        terminal.clone(),
                        fake_hash(5, b"classification-terminal"),
                    ),
                );
                state.stop_resolutions.insert(
                    operation_id.clone(),
                    (StopResolutionKind::Succeeded, terminal),
                );
                state.records.insert(
                    (
                        OperationKind::Stop.label().to_string(),
                        operation_id.clone(),
                    ),
                    (
                        OperationReceiptRecord::Stop {
                            operation_id: operation_id.clone(),
                            session_id: "classification-session".to_string(),
                            turn_id,
                            accepted_revision: 0,
                            authentication: RecordAuthentication {
                                principal_mac: [0; 32],
                                binding_hmac: [0; 32],
                            },
                        },
                        OperationStatusRecord {
                            kind: OperationKind::Stop,
                            value: OperationStatusValue::StopCompleted {
                                resolution: StopResolution::Succeeded,
                            },
                        },
                        1,
                    ),
                );
            });
        }
        let executor = FakeRecoveryExecutor::returning(classification, "canonical safe view");
        let usecase = recovery_usecase(&repo, &executor);
        let request = first_recovery_action(&usecase, action).await;
        let action_id = request.action_id.clone();
        let first = usecase.request(request.clone()).await.unwrap();
        let RecoveryActionOutcome::Completed {
            result: first_result,
            ..
        } = first
        else {
            panic!("classification must produce a durable completed action receipt");
        };
        assert_eq!(first_result.outcome, outcome);
        assert_eq!(first_result.classification, classification);
        assert_eq!(first_result.resource_view, "canonical safe view");
        assert_eq!(executor.effect_count(), 1);

        let replay = usecase.request(request).await.unwrap();
        let RecoveryActionOutcome::Completed {
            result: replay_result,
            ..
        } = replay
        else {
            panic!("same action must replay its completed receipt");
        };
        assert_eq!(replay_result, first_result);
        assert_eq!(executor.effect_count(), 1);
        let S::Completed { result, .. } = usecase.get_action_status(&action_id).await.unwrap()
        else {
            panic!("identity query must return the immutable completed result");
        };
        assert_eq!(result, first_result);
    }
}

#[tokio::test]
async fn b083_b086_unknown_or_modified_identity_and_writer_unknown_create_no_effect() {
    let repo = FakeRepo::new();
    seed_pending_obligation(
        &repo,
        "writer-unknown",
        test_stop_obligation(
            "writer-unknown-operation",
            "unknown-session",
            "1",
            ObligationStateRecord::ReconciliationRequired,
        ),
    );
    let executor =
        FakeRecoveryExecutor::returning(RecoveryResultClassification::Pending, "must not execute");
    let usecase = recovery_usecase(&repo, &executor);
    let request = first_recovery_action(&usecase, RecoveryActionKind::ReadAgain).await;

    let mut modified = request.clone();
    let last = modified.action_id.pop().unwrap();
    modified.action_id.push(if last == 'a' { 'b' } else { 'a' });
    assert_eq!(
        usecase.request(modified).await,
        Err(super::recovery::RecoveryActionError::NotFound)
    );
    assert_eq!(
        usecase
            .request(RecoveryActionRequest {
                action_id: "recovery-unissued-valid".to_string(),
                ..request.clone()
            })
            .await,
        Err(super::recovery::RecoveryActionError::NotFound)
    );
    assert_eq!(executor.effect_count(), 0);

    repo.with_state(|state| {
        state.fail_commit_once = Some(CommitBatchError::OutcomeUnknown {
            identity: CommitIdentity::parse(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
        });
    });
    assert_eq!(
        usecase.request(request.clone()).await.unwrap(),
        RecoveryActionOutcome::ActionOutcomeUnknown {
            action_id: request.action_id.clone(),
        }
    );
    assert_eq!(
        usecase.get_action_status(&request.action_id).await.unwrap(),
        super::recovery::RecoveryActionStatus::OutcomeUnknown {
            action_id: request.action_id,
        }
    );
    assert_eq!(executor.effect_count(), 0);
    repo.with_state(|state| assert!(state.recovery_actions.is_empty()));
}

fn permission_allow(request_id: &str, updated_input: &str, answers: &str) -> PermissionResponse {
    PermissionResponse {
        request_id: request_id.to_string(),
        decision: PermissionResponseDecision::Allow {
            updated_input: Some(
                crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                    updated_input.to_string(),
                ),
            ),
            answers: Some(
                crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                    answers.to_string(),
                ),
            ),
        },
    }
}

fn permission_request(
    operation_id: &str,
    response: PermissionResponse,
) -> PermissionResponseOperationRequest {
    PermissionResponseOperationRequest {
        principal: "local-app".to_string(),
        operation_id: operation_id.to_string(),
        session_id: "permission-session".to_string(),
        response,
    }
}

fn permission_usecase(
    repo: &Arc<FakeRepo>,
    gate: &Arc<FakePermissionGate>,
) -> PermissionResponseOperationUsecase {
    PermissionResponseOperationUsecase::new(
        repo.clone(),
        Arc::new(FakeAuthority),
        gate.clone(),
        GENERATION.to_string(),
    )
}

fn expect_permission_accepted(
    outcome: PermissionResponseCommandOutcome,
) -> super::permission::AcceptedPermissionResponseOperation {
    match outcome {
        PermissionResponseCommandOutcome::Accepted(operation) => operation,
        other => panic!("expected accepted permission response, got {other:?}"),
    }
}

#[tokio::test]
async fn permission_response_acceptance_and_completion_are_atomic_and_replay_effect_once() {
    let response = permission_allow(
        "permission-request-1",
        r#"{"path":"/tmp/exact","nested":{"enabled":true}}"#,
        r#"{"Pick one":["A","B"]}"#,
    );
    let repo = FakeRepo::new();
    let gate = FakePermissionGate::accepting(response.clone());
    let usecase = permission_usecase(&repo, &gate);

    let first = expect_permission_accepted(
        usecase
            .request(permission_request(
                "permission-operation-1",
                response.clone(),
            ))
            .await
            .unwrap(),
    );
    assert!(matches!(
        first.latest_status,
        PermissionResponseExecutionStatus::Completed { .. }
    ));
    assert_eq!(gate.effect_count(), 1);
    assert_eq!(gate.after_completion_count(), 1);

    repo.with_state(|state| {
        let acceptance = state
            .committed_batches
            .iter()
            .find(|batch| batch.idempotency.idempotency_key == "permission-operation-1")
            .expect("permission acceptance batch");
        assert_eq!(acceptance.events.len(), 1);
        assert!(matches!(
            &acceptance.events[0].event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::ObligationRecorded {
                kind: crate::domain::agent_session::events::ObligationKind::PermissionResponse,
                state: ObligationState::Pending,
                ..
            })
        ));
        assert!(acceptance.state_mutations.iter().any(|mutation| matches!(
            mutation,
            LocalStateMutation::OperationBinding(binding)
                if binding.key.kind == OperationKind::PermissionResponse
        )));
        assert!(acceptance.state_mutations.iter().any(|mutation| matches!(
            mutation,
            LocalStateMutation::OperationRecord(record)
                if record.kind == OperationKind::PermissionResponse
        )));
        let obligation = acceptance
            .state_mutations
            .iter()
            .find_map(|mutation| match mutation {
                LocalStateMutation::Obligation(obligation) => Some(obligation),
                _ => None,
            })
            .expect("permission obligation in acceptance batch");
        let ObligationRecord::PermissionResponse { response, .. } = &obligation.record else {
            panic!("expected permission obligation");
        };
        let PermissionResponseDecision::Allow {
            updated_input,
            answers,
        } = &response.decision
        else {
            panic!("expected allow response");
        };
        assert_eq!(
            updated_input.as_ref().map(|value| value.as_str()),
            Some(r#"{"path":"/tmp/exact","nested":{"enabled":true}}"#)
        );
        assert_eq!(
            answers.as_ref().map(|value| value.as_str()),
            Some(r#"{"Pick one":["A","B"]}"#)
        );

        let completion = state
            .committed_batches
            .iter()
            .find(|batch| batch.idempotency.idempotency_key == "permission-operation-1.complete")
            .expect("permission completion batch");
        assert_eq!(completion.events.len(), 2);
        assert!(completion.events.iter().any(|event| matches!(
            &event.event,
            LocalDomainEvent::AgentSession(AgentSessionDomainEvent::PermissionResolved {
                request_id: Some(request_id),
                ..
            }) if request_id == "permission-request-1"
        )));
        assert!(completion.state_mutations.iter().any(|mutation| matches!(
            mutation,
            LocalStateMutation::OperationRecord(record)
                if record.kind == OperationKind::PermissionResponse
                    && record.revision.value() == 2
        )));
        assert!(completion.state_mutations.iter().any(|mutation| matches!(
            mutation,
            LocalStateMutation::Obligation(obligation)
                if obligation.pending.is_none() && obligation.revision.value() == 2
        )));
    });

    let replay = expect_permission_accepted(
        usecase
            .request(permission_request("permission-operation-1", response))
            .await
            .unwrap(),
    );
    assert_eq!(replay, first);
    assert_eq!(gate.effect_count(), 1);
    assert_eq!(gate.after_completion_count(), 1);

    let conflict = usecase
        .request(permission_request(
            "permission-operation-1",
            PermissionResponse {
                request_id: "permission-request-1".to_string(),
                decision: PermissionResponseDecision::Deny {
                    message: Some("No".to_string()),
                },
            },
        ))
        .await;
    assert_eq!(
        conflict,
        Err(PermissionResponseOperationError::PayloadConflict)
    );
    assert_eq!(gate.effect_count(), 1);
}

#[tokio::test]
async fn concurrent_permission_precommit_failure_converges_on_the_winning_same_identity() {
    let response = permission_allow("permission-request-race", "{}", "{}");
    let repo = FakeRepo::new();
    let gate = Arc::new(RacingPermissionGate {
        plan: PermissionResponsePlan {
            session_id: "permission-session".to_string(),
            request_id: response.request_id.clone(),
            turn_id: 9,
            response: response.clone(),
            from_runtime_state: true,
        },
        plan_calls: std::sync::atomic::AtomicUsize::new(0),
        first_entered: tokio::sync::Notify::new(),
        release_first: tokio::sync::Notify::new(),
        effects: Mutex::new(Vec::new()),
    });
    let usecase = Arc::new(PermissionResponseOperationUsecase::new(
        repo as Arc<dyn LocalEventTransactionRepository>,
        Arc::new(FakeAuthority),
        gate.clone() as Arc<dyn PermissionResponseGate>,
        GENERATION.to_string(),
    ));
    let first_usecase = usecase.clone();
    let first_response = response.clone();
    let first = tokio::spawn(async move {
        first_usecase
            .request(permission_request(
                "permission-operation-race",
                first_response,
            ))
            .await
            .unwrap()
    });
    gate.first_entered.notified().await;
    let winner = expect_permission_accepted(
        usecase
            .request(permission_request("permission-operation-race", response))
            .await
            .unwrap(),
    );
    gate.release_first.notify_one();
    let delayed = expect_permission_accepted(first.await.unwrap());
    assert_eq!(delayed.receipt, winner.receipt);
    assert_eq!(gate.effects.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn permission_response_acceptance_faults_never_start_the_provider_effect() {
    let response = permission_allow("permission-request-storage", "{}", "{}");
    let repo = FakeRepo::new();
    let gate = FakePermissionGate::accepting(response.clone());
    let usecase = permission_usecase(&repo, &gate);
    repo.with_state(|state| {
        state.fail_commit_once = Some(CommitBatchError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                true,
                "disk unavailable",
                "permission-acceptance-storage",
            ),
        });
    });
    let outcome = usecase
        .request(permission_request("permission-operation-storage", response))
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        PermissionResponseCommandOutcome::RejectedBeforeCommit { .. }
    ));
    assert_eq!(gate.effect_count(), 0);
    repo.with_state(|state| {
        assert!(state.records.is_empty());
        assert!(state.obligations.is_empty());
        assert!(state.events.is_empty());
    });

    let response = permission_allow("permission-request-unknown", "{}", "{}");
    let repo = FakeRepo::new();
    let gate = FakePermissionGate::accepting(response.clone());
    let usecase = permission_usecase(&repo, &gate);
    repo.with_state(|state| {
        state.fail_commit_once = Some(CommitBatchError::OutcomeUnknown {
            identity: CommitIdentity::parse("permission-acceptance-unknown").unwrap(),
        });
    });
    assert_eq!(
        usecase
            .request(permission_request("permission-operation-unknown", response,))
            .await
            .unwrap(),
        PermissionResponseCommandOutcome::OutcomeUnknown {
            operation_id: "permission-operation-unknown".to_string(),
        }
    );
    assert_eq!(gate.effect_count(), 0);
}

#[tokio::test]
async fn f12_permission_startup_recovery_is_redrivable_after_one_page_query_failure() {
    let response = permission_allow("permission-request-startup-page", "{}", "{}");
    let repo = FakeRepo::new();
    let gate = FakePermissionGate::accepting(response);
    let restarted = permission_usecase(&repo, &gate);

    repo.with_state(|state| state.fail_query = true);
    assert!(restarted
        .recover_pending_permission_responses_pass()
        .await
        .is_err());
    repo.with_state(|state| state.fail_query = false);
    assert_eq!(
        restarted
            .recover_pending_permission_responses_pass()
            .await
            .unwrap(),
        0
    );
    assert_eq!(gate.effect_count(), 0);
}

#[tokio::test]
async fn f12_permission_matching_pending_decode_and_reference_failures_are_retryable_errors() {
    let response = permission_allow("permission-request-corrupt-pending", "{}", "{}");
    let repo = FakeRepo::new();
    let gate = FakePermissionGate::accepting(response);
    repo.with_state(|state| {
        state.obligations.insert(
            "permission-response-malformed".to_string(),
            (
                pending_send_obligation(
                    "permission-response-malformed",
                    "malformed",
                    SendObligationKindRecord::TurnExecution,
                ),
                true,
                0,
            ),
        );
    });
    assert!(permission_usecase(&repo, &gate)
        .recover_pending_permission_responses_pass()
        .await
        .is_err());

    repo.with_state(|state| {
        state.obligations.clear();
        state.obligations.insert(
            "permission-response-missing-operation".to_string(),
            (
                ObligationRecord::PermissionResponse {
                    operation_id: "permission-operation-missing".to_string(),
                    effect_identity: "permission-response:permission-operation-missing".to_string(),
                    session_id: "permission-session".to_string(),
                    turn_id: "1".to_string(),
                    response: permission_allow("permission-request-missing", "{}", "{}"),
                    owner_access: true,
                    from_runtime_state: true,
                    state: ObligationStateRecord::EffectReserved,
                },
                true,
                0,
            ),
        );
    });
    assert!(permission_usecase(&repo, &gate)
        .recover_pending_permission_responses_pass()
        .await
        .is_err());
    assert_eq!(gate.effect_count(), 0);
}

#[tokio::test]
async fn f12_permission_drive_failure_is_recovered_once_with_the_same_identity() {
    let response = permission_allow("permission-request-startup-drive", "{}", "{}");
    let repo = FakeRepo::new();
    let gate = FakePermissionGate::accepting(response.clone());
    repo.with_state(|state| {
        state.fail_commit_on_call = Some((
            2,
            CommitBatchError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "one transient permission claim failure",
                    "permission-startup-drive",
                ),
            },
        ));
    });

    let first = expect_permission_accepted(
        permission_usecase(&repo, &gate)
            .request(permission_request(
                "permission-operation-startup-drive",
                response,
            ))
            .await
            .unwrap(),
    );
    assert!(matches!(
        first.latest_status,
        PermissionResponseExecutionStatus::ReconciliationRequired { .. }
    ));
    assert_eq!(gate.effect_count(), 0);

    let restarted = permission_usecase(&repo, &gate);
    restarted
        .recover_pending_permission_responses()
        .await
        .unwrap();
    let recovered = restarted
        .get_operation("local-app", &first.receipt.operation_id)
        .await
        .unwrap();
    assert!(matches!(
        recovered.latest_status,
        PermissionResponseExecutionStatus::Completed { .. }
    ));
    assert_eq!(gate.effect_count(), 1);
    assert_eq!(gate.after_completion_count(), 1);
}

#[tokio::test]
async fn f12_permission_pending_added_mid_scan_is_found_on_a_fresh_bounded_pass() {
    let repo = FakeRepo::new();
    let first_response = permission_allow("permission-request-mid-scan-1", "{}", "{}");
    let gate = FakePermissionGate::accepting(first_response.clone());
    repo.with_state(|state| {
        state.fail_commit_on_call = Some((2, CommitBatchError::CapacityExceeded));
    });
    let first = expect_permission_accepted(
        permission_usecase(&repo, &gate)
            .request(permission_request(
                "permission-operation-mid-scan-1",
                first_response,
            ))
            .await
            .unwrap(),
    );

    let second_response = permission_allow("permission-request-mid-scan-2", "{}", "{}");
    gate.plan.lock().unwrap().as_mut().unwrap().request_id = second_response.request_id.clone();
    gate.plan.lock().unwrap().as_mut().unwrap().response = second_response.clone();
    repo.with_state(|state| {
        state.fail_commit_on_call = Some((4, CommitBatchError::CapacityExceeded));
    });
    let second = expect_permission_accepted(
        permission_usecase(&repo, &gate)
            .request(permission_request(
                "permission-operation-mid-scan-2",
                second_response,
            ))
            .await
            .unwrap(),
    );

    repo.with_state(|state| {
        let second_pending = state
            .obligations
            .remove(&second.receipt.input_ref)
            .expect("second durable pending permission obligation");
        state.pending_insert_after_page = Some((second.receipt.input_ref.clone(), second_pending));
    });

    let restarted = permission_usecase(&repo, &gate);
    restarted
        .recover_pending_permission_responses()
        .await
        .unwrap();
    for operation_id in [
        first.receipt.operation_id.as_str(),
        second.receipt.operation_id.as_str(),
    ] {
        assert!(matches!(
            restarted
                .get_operation("local-app", operation_id)
                .await
                .unwrap()
                .latest_status,
            PermissionResponseExecutionStatus::Completed { .. }
        ));
    }
    let effects = gate.effects.lock().unwrap();
    assert_eq!(effects.len(), 2);
    assert_ne!(effects[0].operation_id, effects[1].operation_id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn f12_permission_manual_and_startup_recovery_overlap_executes_one_effect() {
    let response = permission_allow("permission-request-manual-startup", "{}", "{}");
    let repo = FakeRepo::new();
    let gate = FakePermissionGate::accepting(response.clone());
    repo.with_state(|state| {
        state.fail_commit_on_call = Some((2, CommitBatchError::CapacityExceeded));
    });
    let pending = expect_permission_accepted(
        permission_usecase(&repo, &gate)
            .request(permission_request(
                "permission-operation-manual-startup",
                response,
            ))
            .await
            .unwrap(),
    );
    assert_eq!(gate.effect_count(), 0);

    let restarted = Arc::new(permission_usecase(&repo, &gate));
    let manual = {
        let restarted = restarted.clone();
        let operation_id = pending.receipt.operation_id.clone();
        tokio::spawn(async move { restarted.resume_operation(&operation_id).await })
    };
    let startup = {
        let restarted = restarted.clone();
        tokio::spawn(async move { restarted.recover_pending_permission_responses().await })
    };
    let _ = manual.await.unwrap().unwrap();
    startup.await.unwrap().unwrap();

    assert_eq!(gate.effect_count(), 1);
    assert!(matches!(
        restarted
            .get_operation("local-app", &pending.receipt.operation_id)
            .await
            .unwrap()
            .latest_status,
        PermissionResponseExecutionStatus::Completed { .. }
    ));
}

#[tokio::test]
async fn permission_response_retry_query_failure_keeps_the_same_unknown_identity() {
    let response = permission_allow("permission-request-query-loss", "{}", "{}");
    let repo = FakeRepo::new();
    let gate = FakePermissionGate::accepting(response.clone());
    let usecase = permission_usecase(&repo, &gate);
    let completed = expect_permission_accepted(
        usecase
            .request(permission_request(
                "permission-operation-query-loss",
                response.clone(),
            ))
            .await
            .unwrap(),
    );
    assert!(matches!(
        completed.latest_status,
        PermissionResponseExecutionStatus::Completed { .. }
    ));
    assert_eq!(gate.effect_count(), 1);

    repo.with_state(|state| state.fail_query = true);
    assert_eq!(
        usecase
            .request(permission_request(
                "permission-operation-query-loss",
                response,
            ))
            .await
            .unwrap(),
        PermissionResponseCommandOutcome::OutcomeUnknown {
            operation_id: "permission-operation-query-loss".to_string(),
        }
    );
    assert_eq!(gate.effect_count(), 1);
}

#[tokio::test]
async fn permission_response_provider_or_completion_ambiguity_is_never_retried() {
    let response = permission_allow("permission-request-provider", "{}", "{}");
    let repo = FakeRepo::new();
    let gate = FakePermissionGate::accepting(response.clone());
    gate.set_mode(PermissionExecuteMode::Fail);
    let usecase = permission_usecase(&repo, &gate);
    let failed = expect_permission_accepted(
        usecase
            .request(permission_request(
                "permission-operation-provider",
                response.clone(),
            ))
            .await
            .unwrap(),
    );
    assert!(matches!(
        failed.latest_status,
        PermissionResponseExecutionStatus::ReconciliationRequired { .. }
    ));
    assert_eq!(gate.effect_count(), 1);
    let restart = permission_usecase(&repo, &gate);
    let replay = expect_permission_accepted(
        restart
            .request(permission_request(
                "permission-operation-provider",
                response,
            ))
            .await
            .unwrap(),
    );
    assert!(matches!(
        replay.latest_status,
        PermissionResponseExecutionStatus::ReconciliationRequired { .. }
    ));
    assert_eq!(gate.effect_count(), 1);
    assert_eq!(
        restart
            .recover_pending_permission_responses_pass()
            .await
            .unwrap(),
        1,
        "a claimed permission response must remain visible to startup supervision"
    );
    assert_eq!(gate.effect_count(), 1, "startup must not replay the claim");

    let response = permission_allow("permission-request-completion", "{}", "{}");
    let repo = FakeRepo::new();
    let gate = FakePermissionGate::accepting(response.clone());
    let usecase = permission_usecase(&repo, &gate);
    repo.with_state(|state| {
        state.fail_commit_on_call = Some((
            3,
            CommitBatchError::OutcomeUnknown {
                identity: CommitIdentity::parse("permission-completion-unknown").unwrap(),
            },
        ));
    });
    let ambiguous = expect_permission_accepted(
        usecase
            .request(permission_request(
                "permission-operation-completion",
                response.clone(),
            ))
            .await
            .unwrap(),
    );
    assert!(matches!(
        ambiguous.latest_status,
        PermissionResponseExecutionStatus::ReconciliationRequired { .. }
    ));
    assert_eq!(gate.effect_count(), 1);
    let restart = permission_usecase(&repo, &gate);
    let replay = expect_permission_accepted(
        restart
            .request(permission_request(
                "permission-operation-completion",
                response,
            ))
            .await
            .unwrap(),
    );
    assert!(matches!(
        replay.latest_status,
        PermissionResponseExecutionStatus::ReconciliationRequired { .. }
    ));
    assert_eq!(gate.effect_count(), 1);
    assert_eq!(gate.after_completion_count(), 0);
}

#[tokio::test]
async fn permission_provider_failure_reconciliation_write_failure_returns_the_confirmed_claim() {
    let response = permission_allow("permission-request-reconcile-write", "{}", "{}");
    let repo = FakeRepo::new();
    let gate = FakePermissionGate::accepting(response.clone());
    gate.set_mode(PermissionExecuteMode::Fail);
    repo.with_state(|state| {
        state.fail_commit_on_call = Some((3, CommitBatchError::CapacityExceeded));
    });

    let failed = expect_permission_accepted(
        permission_usecase(&repo, &gate)
            .request(permission_request(
                "permission-operation-reconcile-write",
                response.clone(),
            ))
            .await
            .unwrap(),
    );
    assert!(matches!(
        failed.latest_status,
        PermissionResponseExecutionStatus::ReconciliationRequired { .. }
    ));

    let replay = expect_permission_accepted(
        permission_usecase(&repo, &gate)
            .request(permission_request(
                "permission-operation-reconcile-write",
                response,
            ))
            .await
            .unwrap(),
    );
    assert_eq!(replay, failed);
    assert_eq!(gate.effect_count(), 1);
}

#[tokio::test]
async fn permission_startup_recovery_pages_past_200_and_uses_only_the_indexed_prefix() {
    let response = permission_allow("unused", "{}", "{}");
    let repo = FakeRepo::new();
    repo.with_state(|state| {
        for ordinal in 0..205 {
            let operation_id = format!("permission-operation-bulk-{ordinal:03}");
            let obligation_id = format!("permission-response-bulk-{ordinal:03}");
            let request_id = format!("permission-request-bulk-{ordinal:03}");
            let exact_response = permission_allow(&request_id, "{}", "{}");
            state.records.insert(
                (
                    OperationKind::PermissionResponse.label().to_string(),
                    operation_id.clone(),
                ),
                (
                    OperationReceiptRecord::PermissionResponse {
                        operation_id: operation_id.clone(),
                        session_id: "permission-session".to_string(),
                        request_id: request_id.clone(),
                        input_ref: obligation_id.clone(),
                        authentication: RecordAuthentication {
                            principal_mac: [0; 32],
                            binding_hmac: [0; 32],
                        },
                    },
                    OperationStatusRecord {
                        kind: OperationKind::PermissionResponse,
                        value: OperationStatusValue::ReconciliationRequired {
                            failure: SafeOperationFailure::new(
                                SessionOperationFailureKind::OutcomeUnknown,
                                false,
                                "Reconciliation required.",
                                format!("permission-bulk-{ordinal:03}"),
                            ),
                        },
                    },
                    0,
                ),
            );
            state.obligations.insert(
                obligation_id,
                (
                    ObligationRecord::PermissionResponse {
                        operation_id: operation_id.clone(),
                        effect_identity: format!("permission-response:{operation_id}"),
                        session_id: "permission-session".to_string(),
                        turn_id: "1".to_string(),
                        response: exact_response,
                        owner_access: true,
                        from_runtime_state: true,
                        state: ObligationStateRecord::EffectReserved,
                    },
                    true,
                    0,
                ),
            );
        }
        for ordinal in 0..75 {
            let obligation_id = format!("stop-target-mixed-{ordinal:03}");
            state.obligations.insert(
                obligation_id.clone(),
                (
                    test_stop_obligation(
                        &format!("stop-mixed-operation-{ordinal:03}"),
                        "mixed-session",
                        "1",
                        ObligationStateRecord::EffectReserved,
                    ),
                    true,
                    0,
                ),
            );
        }
    });
    let gate = FakePermissionGate::accepting(response);
    let usecase = permission_usecase(&repo, &gate);
    assert_eq!(
        usecase
            .recover_pending_permission_responses_pass()
            .await
            .unwrap(),
        205
    );
    repo.with_state(|state| {
        assert_eq!(state.pending_page_queries.len(), 2);
        assert!(state.pending_page_queries.iter().all(|(prefix, _)| {
            prefix.as_deref()
                == Some(PermissionResponseOperationUsecase::recovery_ordered_key_prefix())
        }));
        assert!(state.pending_page_queries[0].1.is_none());
        assert!(state.pending_page_queries[1].1.is_some());
    });
    assert_eq!(gate.effect_count(), 0);
}
