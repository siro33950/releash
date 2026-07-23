//! The `commit_batch` transaction (design "Commit transaction", 9 steps).
//!
//! Executed on the writer thread only. Any failure before SQLite COMMIT
//! rolls back to the pre-batch state; any error or reply loss between the
//! start of COMMIT and the completed fresh readback is `OutcomeUnknown` for
//! the same commit identity.

use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::adaptor::gateway::local_event_store::fault::FaultInjector;
use crate::adaptor::gateway::local_event_store::projection_record_codec::{
    encode_message_projection_update_v1, encode_session_projection_update_v1,
};
use crate::adaptor::gateway::local_event_store::state_record_codec::{
    StoredMigrationCheckpointV1, StoredMigrationParityV1, StoredMigrationQuitV1,
    StoredObligationV1, StoredOperationReceiptV1, StoredOperationStatusV1, StoredRecoveryActionV1,
    StoredRecoveryResultV1, StoredShutdownArchiveV1, StoredShutdownPlanV1, StoredShutdownTargetV1,
    StoredTerminalV1,
};
use crate::adaptor::gateway::local_event_store::writer::{PreparedBatch, PreparedEvent};
use crate::domain::local_event::{
    validate_operation_record, validate_stop_resolution, validate_terminal_record,
    CallerAttemptMutation, CommitBatchError, CommitBatchResult, CommitOperationKind,
    CommitResolution, CommittedBatch, CommittedStreamHead, IdempotencyBinding,
    LocalEventQueryError, LocalStateMutation, MessageProjectionMutation,
    MigrationCheckpointMutation, MigrationParityMutation, MigrationQuitFlightMutation,
    ObligationMutation, ObligationRecord, ObligationStateRecord, OperationBindingMutation,
    OperationKind, OperationRecordMutation, RecoveryActionMutation, RecoveryAttemptRecord,
    RecoveryResourceViewRecord, RecoveryResultRecord, Revision, RevisionGuard,
    SafeOperationFailure, SessionOperationFailureKind, SessionProjectionMutation,
    ShutdownCompactArchiveMutation, ShutdownLatestPointerMutation, ShutdownPlanMutation,
    ShutdownRecoverySnapshotMutation, ShutdownRetiringPointerMutation, ShutdownTargetMutation,
    ShutdownTargetRecord, StopResolutionMutation, StreamId, StreamVersion, TerminalRecordMutation,
};

use crate::adaptor::gateway::local_event_store::envelope::{
    migration_phase_to_label, shutdown_phase_to_label,
};

pub(crate) const SQL_SEAL_EVENT_COUNT: &str = "SELECT COUNT(*) FROM events
     WHERE global_sequence BETWEEN ?1 AND ?2 AND commit_id = ?3";

fn correlation_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn storage_unavailable(error: &rusqlite::Error) -> CommitBatchError {
    let correlation = correlation_id();
    log::warn!("local event store sqlite failure [{correlation}]: {error}");
    CommitBatchError::StorageUnavailable {
        failure: SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            true,
            "local event store write failed",
            correlation,
        ),
    }
}

fn corrupt(context: &str) -> CommitBatchError {
    let correlation = correlation_id();
    log::error!("local event store corrupt state [{correlation}]: {context}");
    CommitBatchError::Corrupt {
        correlation_id: correlation,
    }
}

fn encoded_record<T>(result: Result<T, impl std::fmt::Debug>) -> Result<T, CommitBatchError> {
    result.map_err(|_| CommitBatchError::PayloadConflict)
}

fn conflict(current: i64) -> CommitBatchError {
    CommitBatchError::StreamHeadConflict {
        current: StreamVersion::new(current.max(0)).unwrap_or(StreamVersion::zero()),
    }
}

fn check_guard(existing: Option<i64>, guard: RevisionGuard) -> Result<(), CommitBatchError> {
    match (existing, guard) {
        (None, RevisionGuard::Absent) => Ok(()),
        (Some(revision), RevisionGuard::Expected(expected)) if revision == expected.value() => {
            Ok(())
        }
        (Some(revision), _) => Err(conflict(revision)),
        (None, RevisionGuard::Expected(_)) => Err(conflict(0)),
    }
}

fn operation_progress_is_structurally_valid(mutations: &[LocalStateMutation]) -> bool {
    let mut advances_existing_owner = false;
    let mut inserted_recovery_publications = 0_usize;
    for mutation in mutations {
        match mutation {
            LocalStateMutation::OperationRecord(record)
            | LocalStateMutation::SessionLifecycleOperation(record) => {
                if matches!(record.expected, RevisionGuard::Absent) {
                    return false;
                }
                advances_existing_owner = true;
            }
            LocalStateMutation::Obligation(obligation) => {
                if matches!(obligation.expected, RevisionGuard::Absent) {
                    inserted_recovery_publications += 1;
                    if inserted_recovery_publications > 1
                        || !operation_progress_backend_publication_is_valid(obligation, mutations)
                    {
                        return false;
                    }
                    continue;
                }
                advances_existing_owner = true;
            }
            LocalStateMutation::RecoveryAction(action) => {
                if matches!(action.expected, RevisionGuard::Absent) {
                    return false;
                }
                advances_existing_owner = true;
            }
            LocalStateMutation::CallerAttempt(attempt) => {
                if matches!(attempt.expected, RevisionGuard::Absent) {
                    return false;
                }
                advances_existing_owner = true;
            }
            LocalStateMutation::SessionProjection(session) => {
                if matches!(session.expected, RevisionGuard::Absent) {
                    return false;
                }
            }
            LocalStateMutation::MessageProjection(_)
            | LocalStateMutation::TerminalRecord(_)
            | LocalStateMutation::StopResolution(_) => {}
            LocalStateMutation::OperationBinding(_)
            | LocalStateMutation::SessionProjectionRemoval(_)
            | LocalStateMutation::ShutdownPlan(_)
            | LocalStateMutation::ShutdownTarget(_)
            | LocalStateMutation::ShutdownRecoverySnapshot(_)
            | LocalStateMutation::ShutdownCompactArchive(_)
            | LocalStateMutation::ShutdownLatestPointer(_)
            | LocalStateMutation::ShutdownRetiringPointer(_)
            | LocalStateMutation::MigrationCheckpoint(_)
            | LocalStateMutation::MigrationParity(_)
            | LocalStateMutation::MigrationQuitFlight(_) => return false,
        }
    }
    advances_existing_owner
}

fn operation_progress_backend_publication_is_valid(
    publication: &ObligationMutation,
    mutations: &[LocalStateMutation],
) -> bool {
    let ObligationRecord::RecoveryPublication {
        session_id,
        recovery_id,
        message_id,
        source_obligation_id,
        detail: crate::domain::local_event::RecoveryPublicationObligationRecord::Pending { .. },
        state: ObligationStateRecord::Pending,
    } = &publication.record
    else {
        return false;
    };
    let digest = Sha256::digest(
        format!("recovery-publication/v1\0{session_id}\0{recovery_id}\0{message_id}").as_bytes(),
    );
    if publication.obligation_id != format!("recovery-publication-{}", hex::encode(digest))
        || publication.revision != Revision::new(0).expect("zero revision")
        || publication.pending.as_ref().is_none_or(|pending| {
            pending.owner != *session_id
                || pending.partition != crate::domain::local_event::PendingPartition::Owner
                || pending.shutdown_plan.is_some()
        })
    {
        return false;
    }
    mutations.iter().any(|mutation| {
        let LocalStateMutation::Obligation(source) = mutation else {
            return false;
        };
        if source.obligation_id != *source_obligation_id
            || !matches!(source.expected, RevisionGuard::Expected(_))
            || source.pending.is_some()
        {
            return false;
        }
        fn recovery_transition(
            record: &ObligationRecord,
        ) -> Option<(&ObligationRecord, &crate::domain::local_event::ObligationRecoveryActionRecord)>
        {
            match record {
                ObligationRecord::RecoveryTransition {
                    original,
                    recovery_action,
                } => Some((original, recovery_action)),
                ObligationRecord::Observed { original, .. } => recovery_transition(original),
                _ => None,
            }
        }
        let Some((original, action)) = recovery_transition(&source.record) else {
            return false;
        };
        fn semantic_original(record: &ObligationRecord) -> &ObligationRecord {
            match record {
                ObligationRecord::Observed { original, .. }
                | ObligationRecord::RecoveryTransition { original, .. } => {
                    semantic_original(original)
                }
                record => record,
            }
        }
        let original = semantic_original(original);
        matches!(
            original,
            ObligationRecord::BackendSessionRecovery {
                session_id: stored_session_id,
                recovery_id: stored_recovery_id,
                detail: Some(
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::Completed {
                        old_provider_session_generation,
                        provider_session_generation,
                        backend_session_id,
                        ..
                    },
                ),
                state: ObligationStateRecord::Completed,
            } if stored_session_id == session_id
                && stored_recovery_id == recovery_id
                && !backend_session_id.is_empty()
                && old_provider_session_generation
                    .checked_add(1)
                    == Some(*provider_session_generation)
                && action.effect_identity == *source_obligation_id
                && action.state == ObligationStateRecord::Completed
                && action.classification
                    == Some(
                        crate::domain::agent_session::events::RecoveryResultClassification::Succeeded,
                    )
        )
    })
}

fn operation_progress_only_advances_existing_owners(mutations: &[LocalStateMutation]) -> bool {
    mutations.iter().all(|mutation| {
        !matches!(
            mutation,
            LocalStateMutation::Obligation(ObligationMutation {
                expected: RevisionGuard::Absent,
                ..
            })
        )
    })
}

/// Workflow and projection commits may drain through an active shutdown only
/// when the batch proves that it advances already-admitted work.  In
/// particular, the internal lane label is not authority to mint a new caller
/// operation, binding, obligation, or recovery action.
fn internal_progress_is_anchored_to_existing_owner(prepared: &PreparedBatch) -> bool {
    let mutations = &prepared.batch.state_mutations;
    let mut advances_existing_owner = false;
    for mutation in mutations {
        match mutation {
            LocalStateMutation::OperationBinding(_)
            | LocalStateMutation::CallerAttempt(_)
            | LocalStateMutation::ShutdownPlan(_)
            | LocalStateMutation::ShutdownTarget(_)
            | LocalStateMutation::ShutdownRecoverySnapshot(_)
            | LocalStateMutation::ShutdownCompactArchive(_)
            | LocalStateMutation::ShutdownLatestPointer(_)
            | LocalStateMutation::ShutdownRetiringPointer(_)
            | LocalStateMutation::MigrationCheckpoint(_)
            | LocalStateMutation::MigrationParity(_)
            | LocalStateMutation::MigrationQuitFlight(_) => return false,
            LocalStateMutation::OperationRecord(operation)
            | LocalStateMutation::SessionLifecycleOperation(operation) => {
                if !guard_advances_existing(operation.expected, operation.revision) {
                    return false;
                }
                advances_existing_owner = true;
            }
            LocalStateMutation::Obligation(obligation) => {
                if !guard_advances_existing(obligation.expected, obligation.revision) {
                    return false;
                }
                advances_existing_owner = true;
            }
            LocalStateMutation::RecoveryAction(action) => {
                if !guard_advances_existing(action.expected, action.revision) {
                    return false;
                }
                advances_existing_owner = true;
            }
            LocalStateMutation::SessionProjection(session) => {
                if guard_advances_existing(session.expected, session.revision) {
                    advances_existing_owner = true;
                } else if !guard_inserts_revision_zero(session.expected, session.revision) {
                    return false;
                }
            }
            LocalStateMutation::SessionProjectionRemoval(removal) => {
                if !matches!(removal.expected, RevisionGuard::Expected(_)) {
                    return false;
                }
                advances_existing_owner = true;
            }
            LocalStateMutation::MessageProjection(message) => {
                if !guard_advances_existing(message.expected, message.revision)
                    && !guard_inserts_revision_zero(message.expected, message.revision)
                {
                    return false;
                }
            }
            LocalStateMutation::TerminalRecord(_) | LocalStateMutation::StopResolution(_) => {}
        }
    }
    if !advances_existing_owner {
        return false;
    }
    match prepared.batch.idempotency.operation_kind {
        CommitOperationKind::Projection => projection_progress_has_one_session_scope(prepared),
        CommitOperationKind::Workflow => workflow_progress_has_one_execution_scope(prepared),
        _ => false,
    }
}

fn guard_advances_existing(expected: RevisionGuard, revision: Revision) -> bool {
    let RevisionGuard::Expected(current) = expected else {
        return false;
    };
    current.next() == Some(revision)
}

fn guard_inserts_revision_zero(expected: RevisionGuard, revision: Revision) -> bool {
    matches!(expected, RevisionGuard::Absent) && revision.value() == 0
}

fn receipt_session_id(
    receipt: &crate::domain::local_event::OperationReceiptRecord,
) -> Option<&str> {
    match receipt {
        crate::domain::local_event::OperationReceiptRecord::Send { session_id, .. }
        | crate::domain::local_event::OperationReceiptRecord::PermissionResponse {
            session_id,
            ..
        }
        | crate::domain::local_event::OperationReceiptRecord::Stop { session_id, .. }
        | crate::domain::local_event::OperationReceiptRecord::SessionLifecycle {
            session_id, ..
        } => Some(session_id),
        crate::domain::local_event::OperationReceiptRecord::ApplicationQuit { .. }
        | crate::domain::local_event::OperationReceiptRecord::MigrationApplicationQuit { .. } => {
            None
        }
    }
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
        ObligationRecord::Observed { original, .. }
        | ObligationRecord::RecoveryTransition { original, .. } => obligation_session_id(original),
        ObligationRecord::WorkflowShutdown { .. }
        | ObligationRecord::LegacyReconciliation { .. }
        | ObligationRecord::RecoveryReserved { .. }
        | ObligationRecord::RecoveryCompleted { .. }
        | ObligationRecord::WorkflowExecution { .. } => None,
    }
}

fn projection_progress_has_one_session_scope(prepared: &PreparedBatch) -> bool {
    let mut owner_session = None;
    for mutation in &prepared.batch.state_mutations {
        match mutation {
            LocalStateMutation::SessionProjection(projection)
                if matches!(
                    projection.projection,
                    crate::domain::local_event::SessionProjectionRecord::AgentSession(_)
                ) && guard_advances_existing(projection.expected, projection.revision) =>
            {
                if owner_session
                    .as_ref()
                    .is_some_and(|owner| owner != &projection.session_id)
                {
                    return false;
                }
                owner_session = Some(projection.session_id.clone());
            }
            LocalStateMutation::SessionProjectionRemoval(removal) => {
                if owner_session
                    .as_ref()
                    .is_some_and(|owner| owner != &removal.session_id)
                {
                    return false;
                }
                owner_session = Some(removal.session_id.clone());
            }
            _ => {}
        }
    }
    let Some(owner_session) = owner_session else {
        return false;
    };
    let expected_stream = match StreamId::agent_session(&owner_session) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    if prepared
        .batch
        .expected_heads
        .iter()
        .any(|head| head.stream_id != expected_stream)
        || prepared.batch.events.iter().any(|event| {
            event.stream_id != expected_stream
                || !matches!(
                    event.event,
                    crate::domain::local_event::LocalDomainEvent::AgentSession(_)
                )
        })
    {
        return false;
    }
    for mutation in &prepared.batch.state_mutations {
        let scoped = match mutation {
            LocalStateMutation::OperationRecord(operation)
            | LocalStateMutation::SessionLifecycleOperation(operation) => {
                receipt_session_id(&operation.receipt) == Some(owner_session.as_str())
            }
            LocalStateMutation::SessionProjection(projection) => {
                projection.session_id == owner_session
                    && matches!(
                        &projection.projection,
                        crate::domain::local_event::SessionProjectionRecord::AgentSession(
                            projected
                        ) if projected.meta.id == owner_session
                    )
            }
            LocalStateMutation::MessageProjection(message) => {
                message.session_id == owner_session
                    && matches!(
                        &message.projection,
                        crate::domain::local_event::MessageProjectionRecord::AgentMessage(
                            projected
                        ) if projected.id == message.message_id
                    )
            }
            LocalStateMutation::SessionProjectionRemoval(removal) => {
                removal.session_id == owner_session
            }
            LocalStateMutation::TerminalRecord(terminal) => terminal.session_id == owner_session,
            LocalStateMutation::StopResolution(stop) => {
                prepared.batch.state_mutations.iter().any(|candidate| {
                    matches!(
                        candidate,
                        LocalStateMutation::OperationRecord(operation)
                            if operation.kind == OperationKind::Stop
                                && operation.operation_id == stop.stop_operation_id
                                && receipt_session_id(&operation.receipt)
                                    == Some(owner_session.as_str())
                    )
                })
            }
            LocalStateMutation::Obligation(obligation) => {
                obligation_session_id(&obligation.record) == Some(owner_session.as_str())
            }
            LocalStateMutation::RecoveryAction(_) => false,
            LocalStateMutation::OperationBinding(_)
            | LocalStateMutation::CallerAttempt(_)
            | LocalStateMutation::ShutdownPlan(_)
            | LocalStateMutation::ShutdownTarget(_)
            | LocalStateMutation::ShutdownRecoverySnapshot(_)
            | LocalStateMutation::ShutdownCompactArchive(_)
            | LocalStateMutation::ShutdownLatestPointer(_)
            | LocalStateMutation::ShutdownRetiringPointer(_)
            | LocalStateMutation::MigrationCheckpoint(_)
            | LocalStateMutation::MigrationParity(_)
            | LocalStateMutation::MigrationQuitFlight(_) => false,
        };
        if !scoped {
            return false;
        }
    }
    true
}

fn workflow_progress_has_one_execution_scope(prepared: &PreparedBatch) -> bool {
    let mut execution_id = None;
    for mutation in &prepared.batch.state_mutations {
        if let LocalStateMutation::Obligation(ObligationMutation {
            record: ObligationRecord::WorkflowExecution { execution },
            expected,
            revision,
            ..
        }) = mutation
        {
            if !guard_advances_existing(*expected, *revision)
                || execution_id
                    .as_ref()
                    .is_some_and(|stored| stored != &execution.execution_id)
            {
                return false;
            }
            execution_id = Some(execution.execution_id.clone());
        }
    }
    let Some(execution_id) = execution_id else {
        return false;
    };
    let expected_stream = match StreamId::workflow(&execution_id) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    if prepared
        .batch
        .expected_heads
        .iter()
        .any(|head| head.stream_id != expected_stream)
        || prepared.batch.events.iter().any(|event| {
            event.stream_id != expected_stream
                || !matches!(
                    &event.event,
                    crate::domain::local_event::LocalDomainEvent::Workflow(workflow)
                        if workflow.execution_id() == execution_id
                )
        })
    {
        return false;
    }
    prepared
        .batch
        .state_mutations
        .iter()
        .all(|mutation| match mutation {
            LocalStateMutation::Obligation(ObligationMutation {
                record: ObligationRecord::WorkflowExecution { execution },
                ..
            }) => execution.execution_id == execution_id,
            LocalStateMutation::SessionProjection(projection) => match &projection.projection {
                crate::domain::local_event::SessionProjectionRecord::WorkflowExecution(
                    crate::domain::local_event::WorkflowExecutionProjectionRecord::Present(
                        execution,
                    ),
                ) => {
                    projection.session_id == format!("workflow:{execution_id}")
                        && execution.execution_id == execution_id
                }
                crate::domain::local_event::SessionProjectionRecord::WorkflowExecution(
                    crate::domain::local_event::WorkflowExecutionProjectionRecord::Deleted {
                        execution_id: deleted,
                    },
                ) => {
                    projection.session_id == format!("workflow:{execution_id}")
                        && deleted == &execution_id
                }
                crate::domain::local_event::SessionProjectionRecord::WorkflowWorktreeOwner(
                    owner,
                ) => owner.execution_id == execution_id,
                crate::domain::local_event::SessionProjectionRecord::AgentSession(_) => false,
            },
            _ => false,
        })
}

fn recovery_result_targets_shutdown_target(
    resource_view: &RecoveryResourceViewRecord,
    current_plan_id: &str,
    current_epoch: i64,
    target_ordinal: i64,
    target_key: &str,
    target_state: crate::domain::local_event::ShutdownTargetStateRecord,
) -> bool {
    match resource_view {
        RecoveryResourceViewRecord::ShutdownTarget {
            plan,
            ordinal,
            target_id,
            state,
        } => {
            plan.plan_id == current_plan_id
                && plan.epoch == current_epoch
                && *ordinal == target_ordinal
                && target_id == target_key
                && *state == target_state
        }
        RecoveryResourceViewRecord::SafeSummary(summary) => {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(summary) else {
                return false;
            };
            let state = match target_state {
                crate::domain::local_event::ShutdownTargetStateRecord::Prepared => "prepared",
                crate::domain::local_event::ShutdownTargetStateRecord::EffectReserved => {
                    "effect_reserved"
                }
                crate::domain::local_event::ShutdownTargetStateRecord::Completed => "completed",
                crate::domain::local_event::ShutdownTargetStateRecord::Failed => "failed",
                crate::domain::local_event::ShutdownTargetStateRecord::ReconciliationRequired => {
                    "reconciliation_required"
                }
            };
            value.get("schema").and_then(serde_json::Value::as_str)
                == Some("shutdown_target_recovery_result_v1")
                && value.get("plan_id").and_then(serde_json::Value::as_str) == Some(current_plan_id)
                && value.get("epoch").and_then(serde_json::Value::as_i64) == Some(current_epoch)
                && value.get("ordinal").and_then(serde_json::Value::as_i64) == Some(target_ordinal)
                && value.get("target_key").and_then(serde_json::Value::as_str) == Some(target_key)
                && value.get("state").and_then(serde_json::Value::as_str) == Some(state)
        }
        _ => false,
    }
}

fn shutdown_target_key(
    kind: crate::domain::local_event::ShutdownTargetKindRecord,
    target_id: &str,
) -> String {
    let kind = match kind {
        crate::domain::local_event::ShutdownTargetKindRecord::AgentSession => "agent_session",
        crate::domain::local_event::ShutdownTargetKindRecord::WorkflowExecution => {
            "workflow_execution"
        }
        crate::domain::local_event::ShutdownTargetKindRecord::WorkflowNode => "workflow_node",
    };
    fn push_lp(material: &mut Vec<u8>, value: &str) {
        material.extend_from_slice(&(value.len() as u32).to_be_bytes());
        material.extend_from_slice(value.as_bytes());
    }
    let mut material = Vec::with_capacity(kind.len() + target_id.len() + 64);
    push_lp(&mut material, "application-shutdown-target/v1");
    push_lp(&mut material, kind);
    push_lp(&mut material, target_id);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(material))
}

fn shutdown_effect_request_id(effect_identity: &str) -> String {
    format!(
        "shutdown-{}",
        hex::encode(Sha256::digest(effect_identity.as_bytes()))
    )
}

fn shutdown_target_agent_session_lifecycle_is_bound_to_current_plan(
    connection: &Connection,
    current_plan_id: &str,
    current_epoch: i64,
    current_plan_summary: &str,
    prepared: &PreparedBatch,
) -> Result<bool, CommitBatchError> {
    use crate::domain::agent_session::events::{
        AgentSessionDomainEvent, InterruptReason, ObligationKind, ObligationState,
        SessionLifecycleKind,
    };
    use crate::domain::local_event::{
        LocalDomainEvent, OperationReceiptRecord, OperationStatusValue,
        SessionLifecycleRecordAction, SessionProjectionRecord, TerminalInterruptReasonRecord,
        TerminalResultRecord,
    };

    let batch = &prepared.batch;
    let plan_summary =
        encoded_record(StoredShutdownPlanV1::decode(current_plan_summary))?.into_value();
    if plan_summary.operation_id.is_empty() {
        return Ok(false);
    }

    let mut lifecycle_operation = None;
    for mutation in &batch.state_mutations {
        if let LocalStateMutation::SessionLifecycleOperation(operation) = mutation {
            if lifecycle_operation.replace(operation).is_some() {
                return Ok(false);
            }
        }
    }
    let Some(operation) = lifecycle_operation else {
        return Ok(false);
    };
    let OperationReceiptRecord::SessionLifecycle {
        operation_id,
        session_id,
        action,
        first_accepted_revision,
        commit_operation_kind,
        authentication,
    } = &operation.receipt
    else {
        return Ok(false);
    };
    if operation.kind != OperationKind::SessionLifecycle
        || operation.operation_id != *operation_id
        || *action != SessionLifecycleRecordAction::Close
        || *first_accepted_revision < 0
        || *commit_operation_kind != CommitOperationKind::ShutdownTarget
        || operation.latest_status.kind != OperationKind::SessionLifecycle
        || operation.latest_status.migration_quit
        || session_id.is_empty()
        || !match operation.expected {
            RevisionGuard::Absent => {
                guard_inserts_revision_zero(operation.expected, operation.revision)
            }
            RevisionGuard::Expected(_) => {
                guard_advances_existing(operation.expected, operation.revision)
            }
        }
    {
        return Ok(false);
    }
    let accepting = matches!(operation.expected, RevisionGuard::Absent);

    let mut matching_target = None;
    let mut statement = connection
        .prepare(
            "SELECT detail FROM shutdown_targets
             WHERE plan_id = ?1 AND epoch = ?2
             ORDER BY ordinal LIMIT 4097",
        )
        .map_err(|error| storage_unavailable(&error))?;
    let rows = statement
        .query_map(params![current_plan_id, current_epoch], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| storage_unavailable(&error))?;
    for row in rows {
        let raw = row.map_err(|error| storage_unavailable(&error))?;
        let detail = encoded_record(StoredShutdownTargetV1::decode(&raw))?.into_value();
        let ShutdownTargetRecord::Target {
            target_id,
            kind,
            state,
            effect_identity,
            owner_operation_id,
            recovery_action,
            ..
        } = detail
        else {
            return Ok(false);
        };
        if target_id != *session_id {
            continue;
        }
        if matching_target.is_some()
            || kind != crate::domain::local_event::ShutdownTargetKindRecord::AgentSession
            || effect_identity.is_empty()
        {
            return Ok(false);
        }
        let normal_reservation = state
            == crate::domain::local_event::ShutdownTargetStateRecord::EffectReserved
            && owner_operation_id.as_deref() == Some(plan_summary.operation_id.as_str());
        let recovery_reservation = recovery_action
            .as_ref()
            .is_some_and(|action| action.state == ObligationStateRecord::EffectReserved)
            && owner_operation_id
                .as_deref()
                .is_none_or(|owner| owner == plan_summary.operation_id);
        if !normal_reservation && !recovery_reservation {
            return Ok(false);
        }
        matching_target = Some(effect_identity);
    }
    let Some(effect_identity) = matching_target else {
        return Ok(false);
    };
    let principal = format!("shutdown:{}", plan_summary.operation_id);
    let caller_request_id = shutdown_effect_request_id(&effect_identity);

    let binding_matches = |binding: &OperationBindingMutation| {
        binding.key.principal == principal
            && binding.key.generation_id == batch.idempotency.generation_id
            && binding.key.kind == OperationKind::SessionLifecycle
            && binding.key.caller_request_id == caller_request_id
            && binding.operation_id == *operation_id
            && binding.binding_hmac == authentication.binding_hmac
    };
    if accepting {
        let mut binding = None;
        for mutation in &batch.state_mutations {
            if let LocalStateMutation::OperationBinding(candidate) = mutation {
                if binding.replace(candidate).is_some() {
                    return Ok(false);
                }
            }
        }
        if binding.is_none_or(|binding| !binding_matches(binding))
            || batch.idempotency.idempotency_key != *operation_id
            || batch.idempotency.payload_hash != authentication.binding_hmac
        {
            return Ok(false);
        }
    } else {
        if batch
            .state_mutations
            .iter()
            .any(|mutation| matches!(mutation, LocalStateMutation::OperationBinding(_)))
            || batch.idempotency.idempotency_key
                != format!("{operation_id}.st{}", operation.revision.value())
        {
            return Ok(false);
        }
        let bindings = connection
            .prepare(
                "SELECT principal, generation_id, caller_request_id, binding_hmac
                 FROM operation_bindings
                 WHERE kind = ?1 AND operation_id = ?2",
            )
            .and_then(|mut statement| {
                statement
                    .query_map(
                        params![OperationKind::SessionLifecycle.label(), operation_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, Vec<u8>>(3)?,
                            ))
                        },
                    )?
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|error| storage_unavailable(&error))?;
        if bindings.len() != 1
            || bindings[0].0 != principal
            || bindings[0].1 != batch.idempotency.generation_id
            || bindings[0].2 != caller_request_id
            || !bool::from(
                bindings[0]
                    .3
                    .as_slice()
                    .ct_eq(authentication.binding_hmac.as_slice()),
            )
        {
            return Ok(false);
        }
    }

    let expected_stream =
        StreamId::agent_session(session_id).map_err(|_| CommitBatchError::PayloadConflict)?;
    if accepting {
        if batch.expected_heads.len() != 1
            || batch.expected_heads[0].stream_id != expected_stream
            || batch.events.is_empty()
        {
            return Ok(false);
        }
    } else if !batch.expected_heads.is_empty() || !batch.events.is_empty() {
        return Ok(false);
    }

    let mut session_projection_count = 0_usize;
    let mut close_obligation_count = 0_usize;
    let mut terminal_count = 0_usize;
    let mut participant_operation_count = 0_usize;
    let mut participant_obligation_count = 0_usize;
    let mut stop_resolution_count = 0_usize;
    for mutation in &batch.state_mutations {
        match mutation {
            LocalStateMutation::OperationBinding(_) => {
                if !accepting {
                    return Ok(false);
                }
            }
            LocalStateMutation::SessionLifecycleOperation(candidate) => {
                if !std::ptr::eq(candidate, operation) {
                    return Ok(false);
                }
            }
            LocalStateMutation::SessionProjection(projection) => {
                let SessionProjectionRecord::AgentSession(projected) = &projection.projection
                else {
                    return Ok(false);
                };
                session_projection_count += 1;
                if !accepting
                    || session_projection_count > 1
                    || projection.session_id != *session_id
                    || projected.meta.id != *session_id
                    || !guard_advances_existing(projection.expected, projection.revision)
                {
                    return Ok(false);
                }
            }
            LocalStateMutation::MessageProjection(message) => {
                let crate::domain::local_event::MessageProjectionRecord::AgentMessage(projected) =
                    &message.projection
                else {
                    return Ok(false);
                };
                if !accepting
                    || message.session_id != *session_id
                    || message.message_id != projected.id
                    || (!guard_advances_existing(message.expected, message.revision)
                        && !guard_inserts_revision_zero(message.expected, message.revision))
                {
                    return Ok(false);
                }
            }
            LocalStateMutation::TerminalRecord(terminal) => {
                terminal_count += 1;
                if !accepting
                    || terminal_count > 1
                    || terminal.session_id != *session_id
                    || terminal.terminal_identity != *operation_id
                    || !matches!(
                        &terminal.result,
                        TerminalResultRecord::SessionClosed {
                            operation_id: terminal_operation_id,
                            reason: TerminalInterruptReasonRecord::SessionClosed,
                            ..
                        } if terminal_operation_id == operation_id
                    )
                {
                    return Ok(false);
                }
            }
            LocalStateMutation::OperationRecord(participant) => {
                participant_operation_count += 1;
                if !accepting
                    || !guard_advances_existing(participant.expected, participant.revision)
                {
                    return Ok(false);
                }
                let scoped = match (&participant.receipt, &participant.latest_status.value) {
                    (
                        OperationReceiptRecord::Send {
                            session_id: stored_session,
                            ..
                        },
                        OperationStatusValue::Terminal { .. },
                    )
                    | (
                        OperationReceiptRecord::Stop {
                            session_id: stored_session,
                            ..
                        },
                        OperationStatusValue::StopCompleted { .. },
                    ) => stored_session == session_id,
                    _ => false,
                };
                if !scoped {
                    return Ok(false);
                }
            }
            LocalStateMutation::Obligation(obligation) => match &obligation.record {
                ObligationRecord::SessionClose {
                    obligation_id,
                    operation_id: stored_operation_id,
                    session_id: stored_session_id,
                    action: stored_action,
                    state,
                } => {
                    close_obligation_count += 1;
                    let expected_obligation_id = format!(
                        "session-lifecycle-target-{}",
                        hex::encode(Sha256::digest(
                            format!("session-lifecycle-target/v1\0{session_id}").as_bytes()
                        ))
                    );
                    let pending_matches = obligation.pending.as_ref().is_some_and(|pending| {
                        pending.owner == *session_id
                            && pending.partition
                                == crate::domain::local_event::PendingPartition::Owner
                            && pending.shutdown_plan.is_none()
                            && pending
                                .ordered_key
                                .ends_with(&format!("-{expected_obligation_id}"))
                    });
                    let state_matches = match &operation.latest_status.value {
                        OperationStatusValue::Accepted if accepting => {
                            *state == ObligationStateRecord::EffectReserved && pending_matches
                        }
                        OperationStatusValue::Completed if !accepting => {
                            *state == ObligationStateRecord::Completed
                                && obligation.pending.is_none()
                        }
                        OperationStatusValue::ReconciliationRequired { .. } if !accepting => {
                            *state == ObligationStateRecord::ReconciliationRequired
                                && pending_matches
                        }
                        _ => false,
                    };
                    if close_obligation_count > 1
                        || obligation.obligation_id != expected_obligation_id
                        || obligation_id != &expected_obligation_id
                        || stored_operation_id != operation_id
                        || stored_session_id != session_id
                        || *stored_action != SessionLifecycleRecordAction::Close
                        || (!guard_advances_existing(obligation.expected, obligation.revision)
                            && !guard_inserts_revision_zero(
                                obligation.expected,
                                obligation.revision,
                            ))
                        || !state_matches
                    {
                        return Ok(false);
                    }
                }
                ObligationRecord::Send {
                    session_id: stored_session,
                    state,
                    ..
                }
                | ObligationRecord::StopInterrupt {
                    session_id: stored_session,
                    state,
                    ..
                } => {
                    participant_obligation_count += 1;
                    if !accepting
                        || stored_session != session_id
                        || *state != ObligationStateRecord::Completed
                        || obligation.pending.is_some()
                        || !guard_advances_existing(obligation.expected, obligation.revision)
                    {
                        return Ok(false);
                    }
                }
                _ => return Ok(false),
            },
            LocalStateMutation::StopResolution(_) => {
                stop_resolution_count += 1;
                if !accepting || stop_resolution_count > 1 {
                    return Ok(false);
                }
            }
            LocalStateMutation::CallerAttempt(_)
            | LocalStateMutation::RecoveryAction(_)
            | LocalStateMutation::SessionProjectionRemoval(_)
            | LocalStateMutation::ShutdownPlan(_)
            | LocalStateMutation::ShutdownTarget(_)
            | LocalStateMutation::ShutdownRecoverySnapshot(_)
            | LocalStateMutation::ShutdownCompactArchive(_)
            | LocalStateMutation::ShutdownLatestPointer(_)
            | LocalStateMutation::ShutdownRetiringPointer(_)
            | LocalStateMutation::MigrationCheckpoint(_)
            | LocalStateMutation::MigrationParity(_)
            | LocalStateMutation::MigrationQuitFlight(_) => return Ok(false),
        }
    }

    if accepting {
        if session_projection_count != 1 {
            return Ok(false);
        }
        match operation.latest_status.value {
            OperationStatusValue::Accepted if close_obligation_count == 1 => {}
            OperationStatusValue::Completed if close_obligation_count == 0 => {}
            _ => return Ok(false),
        }
        if terminal_count == 0
            && (participant_operation_count != 0
                || participant_obligation_count != 0
                || stop_resolution_count != 0)
        {
            return Ok(false);
        }
    } else if close_obligation_count != 1
        || session_projection_count != 0
        || terminal_count != 0
        || participant_operation_count != 0
        || participant_obligation_count != 0
        || stop_resolution_count != 0
    {
        return Ok(false);
    }

    let mut lifecycle_accepted_count = 0_usize;
    let mut session_closed_count = 0_usize;
    let mut interrupted_count = 0_usize;
    let mut queue_paused_count = 0_usize;
    let mut obligation_recorded_count = 0_usize;
    let mut stop_resolution_event_count = 0_usize;
    for event in &batch.events {
        if event.stream_id != expected_stream {
            return Ok(false);
        }
        let LocalDomainEvent::AgentSession(event) = &event.event else {
            return Ok(false);
        };
        match event {
            AgentSessionDomainEvent::SessionLifecycleOperationAccepted {
                operation_id: accepted_operation_id,
                kind: SessionLifecycleKind::Close,
                ..
            } if accepted_operation_id == operation_id => lifecycle_accepted_count += 1,
            AgentSessionDomainEvent::SessionClosed { .. } => session_closed_count += 1,
            AgentSessionDomainEvent::TurnInterrupted {
                reason: InterruptReason::SessionClosed,
                ..
            } => interrupted_count += 1,
            AgentSessionDomainEvent::QueuePaused { .. } => queue_paused_count += 1,
            AgentSessionDomainEvent::ObligationRecorded {
                obligation_id,
                kind: ObligationKind::SessionClose,
                state: ObligationState::EffectReserved,
                ..
            } if batch.state_mutations.iter().any(|mutation| {
                matches!(
                    mutation,
                    LocalStateMutation::Obligation(ObligationMutation {
                        obligation_id: stored,
                        ..
                    }) if stored == obligation_id
                )
            }) =>
            {
                obligation_recorded_count += 1;
            }
            AgentSessionDomainEvent::StopResolutionRecorded { .. } => {
                stop_resolution_event_count += 1
            }
            _ => return Ok(false),
        }
    }
    Ok(!accepting
        || (lifecycle_accepted_count == 1
            && session_closed_count == 1
            && interrupted_count == terminal_count
            && queue_paused_count <= 1
            && obligation_recorded_count == close_obligation_count
            && stop_resolution_event_count == stop_resolution_count))
}

fn application_quit_progress_is_bound_to_current_plan(
    connection: &Connection,
    current_plan_id: &str,
    current_epoch: i64,
    current_plan_summary: &str,
    prepared: &PreparedBatch,
) -> Result<bool, CommitBatchError> {
    use crate::domain::local_event::{
        ApplicationDomainEvent, OperationReceiptRecord, ShutdownPlanKey, ShutdownTargetKindRecord,
        ShutdownTargetStateRecord,
    };

    let batch = &prepared.batch;
    let plan_summary =
        encoded_record(StoredShutdownPlanV1::decode(current_plan_summary))?.into_value();
    let operation_id = plan_summary.operation_id.as_str();
    if operation_id.is_empty() || batch.state_mutations.is_empty() {
        return Ok(false);
    }

    // A caller joining the already accepted flight may add exactly one
    // immutable binding to the current operation. It cannot smuggle any
    // state or stream participant into that join commit.
    if let [LocalStateMutation::OperationBinding(binding)] = batch.state_mutations.as_slice() {
        return Ok(batch.expected_heads.is_empty()
            && batch.events.is_empty()
            && binding.key.kind == OperationKind::ApplicationQuit
            && binding.key.generation_id == batch.idempotency.generation_id
            && !binding.key.principal.is_empty()
            && !binding.key.caller_request_id.is_empty()
            && binding.operation_id == operation_id
            && batch.idempotency.idempotency_key
                == format!("{operation_id}.join.{}", binding.key.caller_request_id));
    }

    // The workflow executor's shutdown reservation is part of the fixed
    // target effect. Its exact effect/session identity must already be in the
    // current plan; an arbitrary obligation relabeled ApplicationQuit is not
    // an admission capability.
    if let [LocalStateMutation::Obligation(obligation)] = batch.state_mutations.as_slice() {
        let ObligationRecord::WorkflowShutdown {
            operation_id: stored_operation_id,
            effect_identity,
            owner_revision,
            execution_id,
            state,
        } = &obligation.record
        else {
            return Ok(false);
        };
        let mut target_matches = false;
        let mut statement = connection
            .prepare(
                "SELECT detail, revision FROM shutdown_targets
                 WHERE plan_id = ?1 AND epoch = ?2
                 ORDER BY ordinal LIMIT 4097",
            )
            .map_err(|error| storage_unavailable(&error))?;
        let rows = statement
            .query_map(params![current_plan_id, current_epoch], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| storage_unavailable(&error))?;
        for row in rows {
            let (raw, revision) = row.map_err(|error| storage_unavailable(&error))?;
            let detail = encoded_record(StoredShutdownTargetV1::decode(&raw))?.into_value();
            if matches!(
                detail,
                ShutdownTargetRecord::Target {
                    target_id,
                    kind: ShutdownTargetKindRecord::WorkflowExecution,
                    state: ShutdownTargetStateRecord::EffectReserved,
                    effect_identity: stored_effect,
                    owner_operation_id: Some(owner),
                    ..
                } if target_id == *execution_id
                    && stored_effect == *effect_identity
                    && owner == operation_id
                    && revision == *owner_revision
            ) {
                if target_matches {
                    return Ok(false);
                }
                target_matches = true;
            }
        }
        let digest = hex::encode(Sha256::digest(effect_identity.as_bytes()));
        let expected_obligation_id = format!("workflow-shutdown-{}", &digest[..32]);
        let guard_matches = match (obligation.expected, *state) {
            (RevisionGuard::Absent, ObligationStateRecord::EffectReserved) => {
                guard_inserts_revision_zero(obligation.expected, obligation.revision)
                    && obligation.pending.as_ref().is_some_and(|pending| {
                        pending.owner == *execution_id
                            && pending.partition
                                == crate::domain::local_event::PendingPartition::Owner
                            && pending.shutdown_plan.is_none()
                            && pending.ordered_key == format!("workflow-shutdown-{effect_identity}")
                    })
            }
            (RevisionGuard::Expected(_), ObligationStateRecord::Completed) => {
                guard_advances_existing(obligation.expected, obligation.revision)
                    && obligation.pending.is_none()
            }
            _ => false,
        };
        return Ok(target_matches
            && batch.expected_heads.is_empty()
            && batch.events.is_empty()
            && stored_operation_id == operation_id
            && obligation.obligation_id == expected_obligation_id
            && batch.idempotency.idempotency_key
                == format!("{expected_obligation_id}.{}", obligation.revision.value())
            && guard_matches);
    }

    let current_plan = ShutdownPlanKey {
        plan_id: current_plan_id.to_string(),
        epoch: current_epoch,
    };
    let mut operation_revision = None;
    let mut operation_status = None;
    let mut plan_phase = None;
    let mut target_transition = None;
    let mut saw_plan = false;
    let mut saw_pointer = false;
    for mutation in &batch.state_mutations {
        match mutation {
            LocalStateMutation::OperationRecord(operation) => {
                let OperationReceiptRecord::ApplicationQuit {
                    operation_id: receipt_operation_id,
                    plan,
                    ..
                } = &operation.receipt
                else {
                    return Ok(false);
                };
                if operation_revision.replace(operation.revision).is_some()
                    || operation.kind != OperationKind::ApplicationQuit
                    || operation.operation_id != operation_id
                    || receipt_operation_id != operation_id
                    || plan != &current_plan
                    || operation.latest_status.kind != OperationKind::ApplicationQuit
                    || operation.latest_status.migration_quit
                    || !guard_advances_existing(operation.expected, operation.revision)
                {
                    return Ok(false);
                }
                operation_status = Some(&operation.latest_status.value);
            }
            LocalStateMutation::ShutdownPlan(plan) => {
                if saw_plan
                    || plan.key != current_plan
                    || plan.summary.operation_id != operation_id
                    || !guard_advances_existing(plan.expected, plan.revision)
                {
                    return Ok(false);
                }
                saw_plan = true;
                plan_phase = Some(plan.phase);
            }
            LocalStateMutation::ShutdownTarget(target) => {
                if target_transition.replace(target).is_some()
                    || target.key != current_plan
                    || !guard_advances_existing(target.expected, target.revision)
                {
                    return Ok(false);
                }
                let existing: Option<(String, i64)> = connection
                    .query_row(
                        "SELECT detail, revision FROM shutdown_targets
                         WHERE plan_id = ?1 AND epoch = ?2 AND ordinal = ?3",
                        params![current_plan_id, current_epoch, target.ordinal],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                    .map_err(|error| storage_unavailable(&error))?;
                let Some((raw, revision)) = existing else {
                    return Ok(false);
                };
                let RevisionGuard::Expected(expected) = target.expected else {
                    return Ok(false);
                };
                if expected.value() != revision {
                    return Ok(false);
                }
                let existing = encoded_record(StoredShutdownTargetV1::decode(&raw))?.into_value();
                let (
                    ShutdownTargetRecord::Target {
                        target_id: old_target_id,
                        kind: old_kind,
                        state: old_state,
                        effect_identity: old_effect,
                        owner_operation_id: old_owner,
                        recovery_action: old_recovery,
                        ..
                    },
                    ShutdownTargetRecord::Target {
                        target_id,
                        kind,
                        state,
                        effect_identity,
                        owner_operation_id,
                        failure,
                        recovery_action,
                    },
                ) = (&existing, &target.detail)
                else {
                    return Ok(false);
                };
                let transition_matches = matches!(
                    (*old_state, *state),
                    (
                        ShutdownTargetStateRecord::Prepared,
                        ShutdownTargetStateRecord::EffectReserved
                    ) | (
                        ShutdownTargetStateRecord::EffectReserved,
                        ShutdownTargetStateRecord::Completed
                    ) | (
                        ShutdownTargetStateRecord::EffectReserved,
                        ShutdownTargetStateRecord::ReconciliationRequired
                    )
                );
                if target_id != old_target_id
                    || kind != old_kind
                    || effect_identity != old_effect
                    || old_owner
                        .as_deref()
                        .is_some_and(|owner| owner != operation_id)
                    || owner_operation_id.as_deref() != Some(operation_id)
                    || recovery_action != old_recovery
                    || !transition_matches
                    || (*state == ShutdownTargetStateRecord::ReconciliationRequired)
                        != failure.is_some()
                {
                    return Ok(false);
                }
            }
            LocalStateMutation::ShutdownLatestPointer(pointer) => {
                if saw_pointer
                    || pointer.expected.as_ref() != Some(&current_plan)
                    || pointer.new.is_some()
                {
                    return Ok(false);
                }
                saw_pointer = true;
            }
            LocalStateMutation::OperationBinding(_)
            | LocalStateMutation::CallerAttempt(_)
            | LocalStateMutation::SessionProjection(_)
            | LocalStateMutation::MessageProjection(_)
            | LocalStateMutation::SessionProjectionRemoval(_)
            | LocalStateMutation::TerminalRecord(_)
            | LocalStateMutation::StopResolution(_)
            | LocalStateMutation::Obligation(_)
            | LocalStateMutation::RecoveryAction(_)
            | LocalStateMutation::SessionLifecycleOperation(_)
            | LocalStateMutation::ShutdownRecoverySnapshot(_)
            | LocalStateMutation::ShutdownCompactArchive(_)
            | LocalStateMutation::ShutdownRetiringPointer(_)
            | LocalStateMutation::MigrationCheckpoint(_)
            | LocalStateMutation::MigrationParity(_)
            | LocalStateMutation::MigrationQuitFlight(_) => return Ok(false),
        }
    }

    if let Some(target) = target_transition {
        if batch.state_mutations.len() != 1
            || !batch.expected_heads.is_empty()
            || !batch.events.is_empty()
            || batch.idempotency.idempotency_key
                != format!(
                    "{operation_id}.target.{}.{}",
                    target.ordinal,
                    target.revision.value()
                )
        {
            return Ok(false);
        }
        return Ok(true);
    }
    let Some(operation_revision) = operation_revision else {
        return Ok(false);
    };
    if !saw_plan
        || batch.expected_heads.len() != 1
        || batch.expected_heads[0].stream_id != StreamId::application()
        || batch.events.len() != 1
    {
        return Ok(false);
    }
    let event = &batch.events[0];
    let event_matches = event.stream_id == StreamId::application()
        && matches!(
            &event.event,
            crate::domain::local_event::LocalDomainEvent::Application(
                ApplicationDomainEvent::ShutdownPhaseAdvanced {
                    plan_id,
                    epoch,
                    phase,
                    ..
                }
            ) if plan_id == current_plan_id
                && *epoch == current_epoch
                && Some(*phase) == plan_phase
        );
    let activation = batch.idempotency.idempotency_key == format!("{operation_id}.activate")
        && operation_status == Some(&crate::domain::local_event::OperationStatusValue::Activated)
        && plan_phase == Some(crate::domain::local_event::ApplicationShutdownPhase::Activated)
        && !saw_pointer
        && batch.state_mutations.len() == 2;
    let finish_identity = batch.idempotency.idempotency_key
        == format!("{operation_id}.finish.{}", operation_revision.value());
    let finish = finish_identity
        && match (operation_status, plan_phase) {
            (
                Some(crate::domain::local_event::OperationStatusValue::Completed),
                Some(crate::domain::local_event::ApplicationShutdownPhase::Completed),
            ) => saw_pointer && batch.state_mutations.len() == 3,
            (
                Some(crate::domain::local_event::OperationStatusValue::FailedBeforeActivation {
                    ..
                }),
                Some(crate::domain::local_event::ApplicationShutdownPhase::Failed),
            )
            | (
                Some(crate::domain::local_event::OperationStatusValue::ReconciliationRequired {
                    ..
                }),
                Some(crate::domain::local_event::ApplicationShutdownPhase::ReconciliationRequired),
            ) => !saw_pointer && batch.state_mutations.len() == 2,
            _ => false,
        };
    Ok(event_matches && (activation || finish))
}

fn shutdown_target_recovery_is_bound_to_current_plan(
    connection: &Connection,
    current_plan_id: &str,
    current_epoch: i64,
    current_plan_summary: &str,
    mutations: &[LocalStateMutation],
) -> Result<bool, CommitBatchError> {
    let plan_summary =
        encoded_record(StoredShutdownPlanV1::decode(current_plan_summary))?.into_value();
    let mut recovery_action = None;
    let mut shutdown_target = None;
    let mut closure_operation = None;
    let mut closure_plan = None;
    let mut closure_pointer = None;
    for mutation in mutations {
        match mutation {
            LocalStateMutation::RecoveryAction(action) => {
                if recovery_action.replace(action).is_some() {
                    return Ok(false);
                }
            }
            LocalStateMutation::ShutdownTarget(target) => {
                if shutdown_target.replace(target).is_some() {
                    return Ok(false);
                }
            }
            LocalStateMutation::OperationRecord(operation)
                if operation.kind == OperationKind::ApplicationQuit =>
            {
                if closure_operation.replace(operation).is_some() {
                    return Ok(false);
                }
            }
            LocalStateMutation::ShutdownPlan(plan) => {
                if closure_plan.replace(plan).is_some() {
                    return Ok(false);
                }
            }
            LocalStateMutation::ShutdownLatestPointer(pointer) => {
                if closure_pointer.replace(pointer).is_some() {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
    }
    let (Some(action), Some(target)) = (recovery_action, shutdown_target) else {
        return Ok(false);
    };
    let has_plan_closure = match (closure_operation, closure_plan, closure_pointer) {
        (None, None, None) => false,
        (Some(operation), Some(plan), Some(pointer))
            if operation.operation_id == plan_summary.operation_id
                && matches!(operation.expected, RevisionGuard::Expected(_))
                && plan.key.plan_id == current_plan_id
                && plan.key.epoch == current_epoch
                && plan.phase
                    == crate::domain::local_event::ApplicationShutdownPhase::Completed
                && plan.summary.operation_id == plan_summary.operation_id
                && plan.summary.intent == plan_summary.intent
                && matches!(plan.expected, RevisionGuard::Expected(_))
                && pointer.expected.as_ref().is_some_and(|key| {
                    key.plan_id == current_plan_id && key.epoch == current_epoch
                })
                && pointer.new.is_none() =>
        {
            true
        }
        _ => return Ok(false),
    };
    if target.key.plan_id != current_plan_id
        || target.key.epoch != current_epoch
        || target.ordinal < 0
    {
        return Ok(false);
    }
    let RevisionGuard::Expected(expected_target_revision) = target.expected else {
        // Shutdown recovery may only mutate a member of the plan's fixed
        // target set. In particular, Recovery is never an insert path.
        return Ok(false);
    };
    let existing_target: Option<(String, i64)> = connection
        .query_row(
            "SELECT detail, revision FROM shutdown_targets
             WHERE plan_id = ?1 AND epoch = ?2 AND ordinal = ?3",
            params![current_plan_id, current_epoch, target.ordinal],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    let Some((existing_target, existing_target_revision)) = existing_target else {
        return Ok(false);
    };
    if expected_target_revision.value() != existing_target_revision
        || target.revision.value()
            != match existing_target_revision.checked_add(1) {
                Some(next) => next,
                None => return Ok(false),
            }
    {
        return Ok(false);
    }
    let existing_target =
        encoded_record(StoredShutdownTargetV1::decode(&existing_target))?.into_value();
    let (
        ShutdownTargetRecord::Target {
            target_id: existing_target_id,
            kind: existing_kind,
            state: existing_state,
            effect_identity: existing_effect_identity,
            owner_operation_id: existing_owner_operation_id,
            failure: existing_failure,
            recovery_action: existing_target_recovery,
        },
        ShutdownTargetRecord::Target {
            target_id,
            kind,
            state,
            effect_identity,
            owner_operation_id,
            failure,
            recovery_action: Some(target_recovery),
        },
    ) = (&existing_target, &target.detail)
    else {
        return Ok(false);
    };
    if target_id != existing_target_id
        || kind != existing_kind
        || effect_identity != existing_effect_identity
        || owner_operation_id != existing_owner_operation_id
    {
        // A recovery transition may change target state/failure, but never
        // the executor-owned identity at the admitted ordinal.
        return Ok(false);
    }

    let RecoveryAttemptRecord::ShutdownTarget {
        resource_ref,
        plan,
        ordinal,
        target_key,
        origin_revision,
        action: attempted_action,
        effect_identity_sha256,
        intent,
        state: attempt_state,
        failure: attempt_failure,
    } = &action.attempt
    else {
        return Ok(false);
    };
    let expected_resource_ref = format!(
        "shutdown-target:{current_plan_id}:{current_epoch}:{}:{target_key}",
        target.ordinal
    );
    if plan.plan_id != current_plan_id
        || plan.epoch != current_epoch
        || *ordinal != target.ordinal
        || target_key != &shutdown_target_key(*existing_kind, existing_target_id)
        || resource_ref != &expected_resource_ref
        || target_recovery.action_id != action.action_id
        || target_recovery.origin_revision != *origin_revision
        || target_recovery.action != *attempted_action
        || target_recovery.state != *attempt_state
        || *effect_identity_sha256
            != <[u8; 32]>::from(Sha256::digest(existing_effect_identity.as_bytes()))
        || *intent != plan_summary.intent
        || attempt_failure.is_some()
    {
        return Ok(false);
    }

    match action.expected {
        RevisionGuard::Absent => Ok(action.revision.value() == 0
            && action.completed.is_none()
            && *origin_revision == existing_target_revision as u64
            && *attempt_state == ObligationStateRecord::EffectReserved
            && *state == *existing_state
            && *failure == *existing_failure),
        RevisionGuard::Expected(expected_action_revision) => {
            let existing_action: Option<(String, Option<String>, i64)> = connection
                .query_row(
                    "SELECT attempt, completed, revision FROM recovery_action_attempts
                     WHERE action_id = ?1",
                    params![action.action_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(|error| storage_unavailable(&error))?;
            let Some((existing_attempt, existing_completed, existing_action_revision)) =
                existing_action
            else {
                return Ok(false);
            };
            if expected_action_revision.value() != existing_action_revision
                || action.revision.value()
                    != match existing_action_revision.checked_add(1) {
                        Some(next) => next,
                        None => return Ok(false),
                    }
                || existing_completed.is_some()
            {
                return Ok(false);
            }
            let existing_attempt =
                encoded_record(StoredRecoveryActionV1::decode(&existing_attempt))?.into_value();
            let RecoveryAttemptRecord::ShutdownTarget {
                resource_ref: existing_resource_ref,
                plan: existing_plan,
                ordinal: existing_ordinal,
                target_key: existing_target_key,
                origin_revision: existing_origin_revision,
                action: existing_action_kind,
                effect_identity_sha256: existing_effect_identity_sha256,
                intent: existing_intent,
                state: existing_attempt_state,
                failure: existing_attempt_failure,
            } = existing_attempt
            else {
                return Ok(false);
            };
            let Some(existing_target_recovery) = existing_target_recovery else {
                return Ok(false);
            };
            if existing_resource_ref != *resource_ref
                || existing_plan != *plan
                || existing_ordinal != *ordinal
                || existing_target_key != *target_key
                || existing_origin_revision != *origin_revision
                || existing_action_kind != *attempted_action
                || existing_effect_identity_sha256 != *effect_identity_sha256
                || existing_intent != *intent
                || existing_attempt_state != ObligationStateRecord::EffectReserved
                || existing_attempt_failure.is_some()
                || existing_target_recovery.action_id != action.action_id
                || existing_target_recovery.origin_revision != *origin_revision
                || existing_target_recovery.action != *attempted_action
                || existing_target_recovery.state != ObligationStateRecord::EffectReserved
                || *attempt_state != ObligationStateRecord::Completed
                || target_recovery.state != ObligationStateRecord::Completed
            {
                return Ok(false);
            }
            let Some(RecoveryResultRecord::Action(completed)) = action.completed.as_ref() else {
                return Ok(false);
            };
            Ok((!has_plan_closure
                || *state == crate::domain::local_event::ShutdownTargetStateRecord::Completed)
                && completed.resource_revision == target.revision.value() as u64
                && recovery_result_targets_shutdown_target(
                    &completed.resource_view,
                    current_plan_id,
                    current_epoch,
                    target.ordinal,
                    target_key,
                    *state,
                ))
        }
    }
}

fn encode_stream_heads(heads: &[CommittedStreamHead]) -> String {
    let entries: Vec<(String, i64)> = heads
        .iter()
        .map(|head| (head.stream_id.as_str().to_string(), head.head.value()))
        .collect();
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string())
}

fn decode_stream_heads(raw: &str) -> Result<Vec<CommittedStreamHead>, CommitBatchError> {
    let entries: Vec<(String, i64)> =
        serde_json::from_str(raw).map_err(|_| corrupt("stream_heads_json parse failed"))?;
    entries
        .into_iter()
        .map(|(stream_id, head)| {
            Ok(CommittedStreamHead {
                stream_id: StreamId::parse(&stream_id)
                    .map_err(|_| corrupt("stream_heads_json stream id invalid"))?,
                head: StreamVersion::new(head)
                    .map_err(|_| corrupt("stream_heads_json head invalid"))?,
            })
        })
        .collect()
}

struct SealedCommitRow {
    commit_id: String,
    payload_hash: Vec<u8>,
    state: String,
    first_global_sequence: Option<i64>,
    last_global_sequence: Option<i64>,
    event_count: i64,
    mutation_count: i64,
    stream_heads_json: String,
    result_hash: Option<Vec<u8>>,
}

fn lookup_idempotency(
    connection: &Connection,
    idempotency: &IdempotencyBinding,
) -> Result<Option<SealedCommitRow>, rusqlite::Error> {
    connection
        .query_row(
            "SELECT commit_id, payload_hash, state, first_global_sequence, last_global_sequence,
                    event_count, mutation_count, stream_heads_json, result_hash
             FROM logical_commits
             WHERE generation_id = ?1 AND operation_kind = ?2 AND idempotency_key = ?3",
            params![
                idempotency.generation_id,
                idempotency.operation_kind.label(),
                idempotency.idempotency_key
            ],
            |row| {
                Ok(SealedCommitRow {
                    commit_id: row.get(0)?,
                    payload_hash: row.get(1)?,
                    state: row.get(2)?,
                    first_global_sequence: row.get(3)?,
                    last_global_sequence: row.get(4)?,
                    event_count: row.get(5)?,
                    mutation_count: row.get(6)?,
                    stream_heads_json: row.get(7)?,
                    result_hash: row.get(8)?,
                })
            },
        )
        .optional()
}

fn committed_batch_from_row(
    prepared_commit_id: &crate::domain::local_event::CommitIdentity,
    row: &SealedCommitRow,
) -> Result<CommittedBatch, CommitBatchError> {
    use crate::domain::local_event::GlobalSequence;
    if row.state != "sealed" {
        return Err(corrupt("visible logical commit is not sealed"));
    }
    let result_hash_bytes = row
        .result_hash
        .as_ref()
        .ok_or_else(|| corrupt("sealed commit without result hash"))?;
    let result_hash: [u8; 32] = result_hash_bytes
        .as_slice()
        .try_into()
        .map_err(|_| corrupt("sealed commit result hash length"))?;
    let sequence_range = match (row.first_global_sequence, row.last_global_sequence) {
        (Some(first), Some(last)) => Some((
            GlobalSequence::new(first).map_err(|_| corrupt("sealed commit first sequence"))?,
            GlobalSequence::new(last).map_err(|_| corrupt("sealed commit last sequence"))?,
        )),
        (None, None) => None,
        _ => return Err(corrupt("sealed commit half-open sequence range")),
    };
    Ok(CommittedBatch {
        commit_id: prepared_commit_id.clone(),
        sequence_range,
        stream_heads: decode_stream_heads(&row.stream_heads_json)?,
        event_count: row.event_count,
        mutation_count: row.mutation_count,
        result_hash,
    })
}

fn result_hash_of(
    commit_id: &str,
    sequence_range: Option<(i64, i64)>,
    event_ids: &[String],
    heads: &[CommittedStreamHead],
    mutation_count: i64,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(commit_id.as_bytes());
    if let Some((first, last)) = sequence_range {
        hasher.update(first.to_be_bytes());
        hasher.update(last.to_be_bytes());
    }
    for event_id in event_ids {
        hasher.update((event_id.len() as u64).to_be_bytes());
        hasher.update(event_id.as_bytes());
    }
    for head in heads {
        hasher.update(head.stream_id.as_str().as_bytes());
        hasher.update(head.head.value().to_be_bytes());
    }
    hasher.update(mutation_count.to_be_bytes());
    hasher.finalize().into()
}

/// Resolve a commit identity to its sealed result or proven absence.
/// Used by `resolve_commit` and the `CommitByIdentity` query.
pub(crate) fn resolve_commit_row(
    connection: &Connection,
    commit_id: &crate::domain::local_event::CommitIdentity,
) -> Result<CommitResolution, LocalEventQueryError> {
    let row = connection
        .query_row(
            "SELECT commit_id, payload_hash, state, first_global_sequence, last_global_sequence,
                    event_count, mutation_count, stream_heads_json, result_hash
             FROM logical_commits WHERE commit_id = ?1",
            params![commit_id.as_str()],
            |row| {
                Ok(SealedCommitRow {
                    commit_id: row.get(0)?,
                    payload_hash: row.get(1)?,
                    state: row.get(2)?,
                    first_global_sequence: row.get(3)?,
                    last_global_sequence: row.get(4)?,
                    event_count: row.get(5)?,
                    mutation_count: row.get(6)?,
                    stream_heads_json: row.get(7)?,
                    result_hash: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(|error| {
            let correlation = correlation_id();
            log::warn!("local event store read failure [{correlation}]: {error}");
            LocalEventQueryError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "local event store read failed",
                    correlation,
                ),
            }
        })?;
    match row {
        None => Ok(CommitResolution::NotCommitted),
        Some(row) => {
            let batch = committed_batch_from_row(commit_id, &row).map_err(|error| match error {
                CommitBatchError::Corrupt { correlation_id } => {
                    LocalEventQueryError::Corrupt { correlation_id }
                }
                _ => LocalEventQueryError::Internal {
                    correlation_id: correlation_id(),
                },
            })?;
            Ok(CommitResolution::Committed(batch))
        }
    }
}

/// Execute one prepared batch on the writer connection.
///
/// Returns the commit result. The reply-drop fault is handled by the caller;
/// this function only distinguishes rollback errors from `OutcomeUnknown`.
pub fn execute_commit(
    connection: &Connection,
    prepared: &PreparedBatch,
    now_ms: i64,
    fault: &FaultInjector,
) -> Result<CommitBatchResult, CommitBatchError> {
    let batch = &prepared.batch;
    let commit_id = batch.commit_id.as_str().to_string();

    // Step 1 (batch shape) was validated before queue admission.
    if fault.take_fail_before_begin() {
        return Err(CommitBatchError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                true,
                "injected failure before transaction begin",
                correlation_id(),
            ),
        });
    }

    // Step 2: BEGIN IMMEDIATE.
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| storage_unavailable(&error))?;

    let outcome = match execute_in_transaction(connection, prepared, now_ms, fault) {
        Ok(outcome) => outcome,
        Err(error @ CommitBatchError::OutcomeUnknown { .. }) => {
            // COMMIT already started; do not roll back.
            return Err(error);
        }
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            return Err(error);
        }
    };

    // Step 8 second half: fresh readback with a separate statement.
    if fault.take_crash_after_commit_before_readback() {
        return Err(CommitBatchError::OutcomeUnknown {
            identity: batch.commit_id.clone(),
        });
    }
    let readback = lookup_idempotency(connection, &batch.idempotency).map_err(|_| {
        CommitBatchError::OutcomeUnknown {
            identity: batch.commit_id.clone(),
        }
    })?;
    let Some(row) = readback else {
        return Err(CommitBatchError::OutcomeUnknown {
            identity: batch.commit_id.clone(),
        });
    };
    if row.commit_id != commit_id {
        return Err(corrupt("readback returned a different commit identity"));
    }
    let committed = committed_batch_from_row(&batch.commit_id, &row)?;
    match outcome {
        CommitOutcome::Committed => Ok(CommitBatchResult::Committed(committed)),
        CommitOutcome::Replayed => Ok(CommitBatchResult::Replayed(committed)),
    }
}

enum CommitOutcome {
    Committed,
    Replayed,
}

fn execute_in_transaction(
    connection: &Connection,
    prepared: &PreparedBatch,
    now_ms: i64,
    fault: &FaultInjector,
) -> Result<CommitOutcome, CommitBatchError> {
    let batch = &prepared.batch;
    let commit_id = batch.commit_id.as_str();

    // Step 3: idempotency point lookup.
    if let Some(row) = lookup_idempotency(connection, &batch.idempotency)
        .map_err(|error| storage_unavailable(&error))?
    {
        if row.state != "sealed" {
            return Err(corrupt("existing logical commit is not sealed"));
        }
        let payload_matches =
            row.payload_hash.as_slice() == batch.idempotency.payload_hash.as_slice();
        if !payload_matches || row.commit_id != commit_id {
            return Err(CommitBatchError::PayloadConflict);
        }
        // Same binding: the transaction commits nothing; COMMIT below is a
        // no-op and the readback returns the saved result.
        connection
            .execute_batch("COMMIT")
            .map_err(|_| CommitBatchError::OutcomeUnknown {
                identity: batch.commit_id.clone(),
            })?;
        return Ok(CommitOutcome::Replayed);
    }
    // The commit identity itself must also be new.
    let existing_commit: Option<String> = connection
        .query_row(
            "SELECT commit_id FROM logical_commits WHERE commit_id = ?1",
            params![commit_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    if existing_commit.is_some() {
        return Err(CommitBatchError::PayloadConflict);
    }

    // A current shutdown plan is the global admission authority. Saved
    // idempotent results above remain readable/replayable, while every new
    // user mutation is closed before state or external-effect intent can be
    // recorded. Workflow/projection commits remain available to quiesce the
    // fixed target set owned by the active plan.
    let (
        current_shutdown,
        current_shutdown_epoch,
        current_shutdown_phase,
        current_shutdown_summary,
    ): (Option<String>, Option<i64>, Option<String>, Option<String>) = connection
        .query_row(
            "SELECT m.current_shutdown_plan_id, m.current_shutdown_epoch, p.phase, p.summary
             FROM store_metadata m
             LEFT JOIN shutdown_plans p
               ON p.plan_id = m.current_shutdown_plan_id
              AND p.epoch = m.current_shutdown_epoch
             WHERE m.id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|error| storage_unavailable(&error))?;
    let shutdown_admission_closed = match (
        current_shutdown.as_ref(),
        current_shutdown_epoch,
        current_shutdown_phase.as_deref(),
        current_shutdown_summary.as_deref(),
    ) {
        (None, None, None, None) => false,
        (Some(_), Some(_), Some("failed" | "cancelled" | "completed"), Some(_)) => false,
        (Some(_), Some(_), Some(_), Some(_)) => true,
        _ => return Err(corrupt("current shutdown pointer has no exact plan phase")),
    };
    let shutdown_target_recovery = if shutdown_admission_closed
        && batch.idempotency.operation_kind == CommitOperationKind::Recovery
    {
        shutdown_target_recovery_is_bound_to_current_plan(
            connection,
            current_shutdown
                .as_deref()
                .expect("closed shutdown has a plan id"),
            current_shutdown_epoch.expect("closed shutdown has an epoch"),
            current_shutdown_summary
                .as_deref()
                .expect("closed shutdown has a summary"),
            &batch.state_mutations,
        )?
    } else {
        false
    };
    let operation_progress = batch.idempotency.operation_kind
        == CommitOperationKind::OperationProgress
        && operation_progress_is_structurally_valid(&batch.state_mutations)
        && (!shutdown_admission_closed
            || operation_progress_only_advances_existing_owners(&batch.state_mutations));
    let internal_progress = shutdown_admission_closed
        && matches!(
            batch.idempotency.operation_kind,
            CommitOperationKind::Workflow | CommitOperationKind::Projection
        )
        && internal_progress_is_anchored_to_existing_owner(prepared);
    let shutdown_target_lifecycle = if shutdown_admission_closed
        && batch.idempotency.operation_kind == CommitOperationKind::ShutdownTarget
    {
        shutdown_target_agent_session_lifecycle_is_bound_to_current_plan(
            connection,
            current_shutdown
                .as_deref()
                .expect("closed shutdown has a plan id"),
            current_shutdown_epoch.expect("closed shutdown has an epoch"),
            current_shutdown_summary
                .as_deref()
                .expect("closed shutdown has a summary"),
            prepared,
        )?
    } else {
        false
    };
    let application_quit_progress = if shutdown_admission_closed
        && batch.idempotency.operation_kind == CommitOperationKind::ApplicationQuit
    {
        application_quit_progress_is_bound_to_current_plan(
            connection,
            current_shutdown
                .as_deref()
                .expect("closed shutdown has a plan id"),
            current_shutdown_epoch.expect("closed shutdown has an epoch"),
            current_shutdown_summary
                .as_deref()
                .expect("closed shutdown has a summary"),
            prepared,
        )?
    } else {
        false
    };
    if batch.idempotency.operation_kind == CommitOperationKind::OperationProgress
        && !operation_progress
    {
        if shutdown_admission_closed {
            return Err(CommitBatchError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::PreviousShutdownReconciliationRequired,
                    true,
                    "Application shutdown is in progress.",
                    correlation_id(),
                ),
            });
        }
        return Err(corrupt(
            "operation-progress batch violated its existing-owner structural guard",
        ));
    }
    if shutdown_admission_closed
        && (matches!(
            batch.idempotency.operation_kind,
            CommitOperationKind::Send
                | CommitOperationKind::PermissionResponse
                | CommitOperationKind::Stop
                | CommitOperationKind::SessionLifecycle
                | CommitOperationKind::UserMutation
                | CommitOperationKind::Recovery
                | CommitOperationKind::Workflow
                | CommitOperationKind::Projection
                | CommitOperationKind::ShutdownTarget
                | CommitOperationKind::ApplicationQuit
                | CommitOperationKind::Migration
        ) && !shutdown_target_recovery
            && !internal_progress
            && !shutdown_target_lifecycle
            && !application_quit_progress)
    {
        return Err(CommitBatchError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::PreviousShutdownReconciliationRequired,
                true,
                "Application shutdown is in progress.",
                correlation_id(),
            ),
        });
    }

    // Step 4: expected stream heads and mutation revision guards.
    let mut current_heads: Vec<(StreamId, i64)> = Vec::with_capacity(batch.expected_heads.len());
    for expected in &batch.expected_heads {
        let current: Option<i64> = connection
            .query_row(
                "SELECT head FROM stream_heads WHERE stream_id = ?1",
                params![expected.stream_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| storage_unavailable(&error))?;
        let current = current.unwrap_or(0);
        if current != expected.expected.value() {
            return Err(conflict(current));
        }
        current_heads.push((expected.stream_id.clone(), current));
    }
    validate_mutation_guards(connection, &batch.state_mutations)?;

    // Sequence allocation bounds (fail typed before overflow).
    let next_global: i64 = connection
        .query_row(
            "SELECT next_global_sequence FROM store_metadata WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_unavailable(&error))?;
    let event_count = prepared.events.len() as i64;
    if event_count > 0 && next_global.checked_add(event_count - 1).is_none() {
        return Err(CommitBatchError::SequenceExhausted);
    }
    for (stream_id, current) in &current_heads {
        let events_for_stream = prepared
            .events
            .iter()
            .filter(|event| &event.stream_id == stream_id)
            .count() as i64;
        if current.checked_add(events_for_stream).is_none() {
            return Err(CommitBatchError::SequenceExhausted);
        }
    }

    // Step 5: insert the preparing logical commit, then events with
    // contiguous global / stream sequences.
    let mut new_heads: Vec<CommittedStreamHead> = Vec::with_capacity(current_heads.len());
    for (stream_id, current) in &current_heads {
        let events_for_stream = prepared
            .events
            .iter()
            .filter(|event| &event.stream_id == stream_id)
            .count() as i64;
        new_heads.push(CommittedStreamHead {
            stream_id: stream_id.clone(),
            head: StreamVersion::new(current + events_for_stream)
                .map_err(|_| CommitBatchError::SequenceExhausted)?,
        });
    }
    connection
        .execute(
            "INSERT INTO logical_commits (
                commit_id, generation_id, operation_kind, idempotency_key, payload_hash,
                state, first_global_sequence, last_global_sequence, event_count,
                mutation_count, stream_heads_json, result_hash, committed_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'preparing', NULL, NULL, ?6, ?7, ?8, NULL, ?9)",
            params![
                commit_id,
                batch.idempotency.generation_id,
                batch.idempotency.operation_kind.label(),
                batch.idempotency.idempotency_key,
                batch.idempotency.payload_hash.as_slice(),
                event_count,
                batch.state_mutations.len() as i64,
                encode_stream_heads(&new_heads),
                now_ms
            ],
        )
        .map_err(|error| storage_unavailable(&error))?;

    let mut event_ids: Vec<String> = Vec::with_capacity(prepared.events.len());
    let mut stream_positions: Vec<(StreamId, i64)> = current_heads.clone();
    for (offset, event) in prepared.events.iter().enumerate() {
        let global = next_global + offset as i64;
        let position = stream_positions
            .iter_mut()
            .find(|(stream_id, _)| stream_id == &event.stream_id)
            .ok_or_else(|| corrupt("event stream missing from expected heads"))?;
        position.1 += 1;
        let event_id = format!("{commit_id}.{global}");
        insert_event(connection, commit_id, &event_id, event, position.1, global)?;
        event_ids.push(event_id);
        fail_after_participant_write_if_armed(fault)?;
    }
    for head in &new_heads {
        connection
            .execute(
                "INSERT INTO stream_heads (stream_id, head, updated_commit_id)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (stream_id) DO UPDATE SET
                     head = excluded.head,
                     updated_commit_id = excluded.updated_commit_id",
                params![head.stream_id.as_str(), head.head.value(), commit_id],
            )
            .map_err(|error| storage_unavailable(&error))?;
    }

    // Step 6: state mutations, direct indexes, projections.
    for mutation in &batch.state_mutations {
        apply_mutation(connection, commit_id, mutation)?;
        fail_after_participant_write_if_armed(fault)?;
    }

    // Step 7: verify counts / range and seal.
    let sequence_range = if event_count > 0 {
        Some((next_global, next_global + event_count - 1))
    } else {
        None
    };
    let stored_event_count: i64 = match sequence_range {
        Some((first, last)) => connection
            .query_row(
                SQL_SEAL_EVENT_COUNT,
                params![first, last, commit_id],
                |row| row.get(0),
            )
            .map_err(|error| storage_unavailable(&error))?,
        None => 0,
    };
    if stored_event_count != event_count {
        return Err(corrupt("event count mismatch at seal"));
    }
    let result_hash = result_hash_of(
        commit_id,
        sequence_range,
        &event_ids,
        &new_heads,
        batch.state_mutations.len() as i64,
    );
    connection
        .execute(
            "UPDATE logical_commits SET
                state = 'sealed',
                first_global_sequence = ?2,
                last_global_sequence = ?3,
                result_hash = ?4
             WHERE commit_id = ?1",
            params![
                commit_id,
                sequence_range.map(|range| range.0),
                sequence_range.map(|range| range.1),
                result_hash.as_slice()
            ],
        )
        .map_err(|error| storage_unavailable(&error))?;
    if event_count > 0 {
        connection
            .execute(
                "UPDATE store_metadata SET next_global_sequence = ?1 WHERE id = 1",
                params![next_global + event_count],
            )
            .map_err(|error| storage_unavailable(&error))?;
    }

    if fault.take_fail_before_commit() {
        return Err(CommitBatchError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                true,
                "injected failure before COMMIT",
                correlation_id(),
            ),
        });
    }

    // Step 8 first half: COMMIT. From here every failure is OutcomeUnknown.
    connection
        .execute_batch("COMMIT")
        .map_err(|_| CommitBatchError::OutcomeUnknown {
            identity: batch.commit_id.clone(),
        })?;
    Ok(CommitOutcome::Committed)
}

fn fail_after_participant_write_if_armed(fault: &FaultInjector) -> Result<(), CommitBatchError> {
    if !fault.take_fail_after_participant_write() {
        return Ok(());
    }
    Err(CommitBatchError::StorageUnavailable {
        failure: SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            true,
            "injected failure after participant write",
            correlation_id(),
        ),
    })
}

fn insert_event(
    connection: &Connection,
    commit_id: &str,
    event_id: &str,
    event: &PreparedEvent,
    stream_sequence: i64,
    global_sequence: i64,
) -> Result<(), CommitBatchError> {
    connection
        .execute(
            "INSERT INTO events (
                global_sequence, event_id, commit_id, stream_id, stream_sequence,
                event_type, payload_version, occurred_at, payload, payload_sha256
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                global_sequence,
                event_id,
                commit_id,
                event.stream_id.as_str(),
                stream_sequence,
                event.payload.event_type,
                event.payload.payload_version,
                event.occurred_at_ms.to_string(),
                event.payload.payload,
                event.payload_sha256.as_slice()
            ],
        )
        .map_err(|error| storage_unavailable(&error))?;
    Ok(())
}

// --- Mutation guard validation (step 4) ---

fn validate_mutation_guards(
    connection: &Connection,
    mutations: &[LocalStateMutation],
) -> Result<(), CommitBatchError> {
    validate_pending_capacity(connection, mutations, "feedback-", 512)?;
    validate_pending_capacity(connection, mutations, "stop-target-", 32)?;
    for mutation in mutations {
        validate_one_guard(connection, mutation)?;
    }
    Ok(())
}

fn validate_pending_capacity(
    connection: &Connection,
    mutations: &[LocalStateMutation],
    obligation_prefix: &str,
    maximum: i64,
) -> Result<(), CommitBatchError> {
    let pattern = format!("{obligation_prefix}%");
    let current: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pending_obligations WHERE obligation_id LIKE ?1",
            params![pattern],
            |row| row.get(0),
        )
        .map_err(|error| storage_unavailable(&error))?;
    let mut additions = 0i64;
    for mutation in mutations {
        let LocalStateMutation::Obligation(obligation) = mutation else {
            continue;
        };
        if obligation.pending.is_none() || !obligation.obligation_id.starts_with(obligation_prefix)
        {
            continue;
        }
        let exists: i64 = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM obligations WHERE obligation_id = ?1)",
                params![obligation.obligation_id],
                |row| row.get(0),
            )
            .map_err(|error| storage_unavailable(&error))?;
        if exists == 0 {
            additions += 1;
        }
    }
    if current.saturating_add(additions) > maximum {
        return Err(CommitBatchError::CapacityExceeded);
    }
    Ok(())
}

fn read_revision(
    connection: &Connection,
    sql: &str,
    parameters: impl rusqlite::Params,
) -> Result<Option<i64>, CommitBatchError> {
    connection
        .query_row(sql, parameters, |row| row.get::<_, i64>(0))
        .optional()
        .map_err(|error| storage_unavailable(&error))
}

fn validate_one_guard(
    connection: &Connection,
    mutation: &LocalStateMutation,
) -> Result<(), CommitBatchError> {
    match mutation {
        LocalStateMutation::OperationBinding(m) => validate_operation_binding(connection, m),
        LocalStateMutation::CallerAttempt(m) => validate_caller_attempt(connection, m),
        LocalStateMutation::OperationRecord(m)
        | LocalStateMutation::SessionLifecycleOperation(m) => {
            validate_operation_record(m.kind, &m.operation_id, &m.receipt, &m.latest_status)
                .map_err(|_| CommitBatchError::PayloadConflict)?;
            let existing = read_revision(
                connection,
                "SELECT revision FROM operation_records WHERE kind = ?1 AND operation_id = ?2",
                params![m.kind.label(), m.operation_id],
            )?;
            check_guard(existing, m.expected)
        }
        LocalStateMutation::SessionProjection(m) => {
            let existing = read_revision(
                connection,
                "SELECT revision FROM session_projection WHERE session_id = ?1",
                params![m.session_id],
            )?;
            check_guard(existing, m.expected)
        }
        LocalStateMutation::MessageProjection(m) => {
            let existing = read_revision(
                connection,
                "SELECT revision FROM message_projection
                 WHERE session_id = ?1 AND message_id = ?2",
                params![m.session_id, m.message_id],
            )?;
            check_guard(existing, m.expected)
        }
        LocalStateMutation::SessionProjectionRemoval(m) => {
            let existing = read_revision(
                connection,
                "SELECT revision FROM session_projection WHERE session_id = ?1",
                params![m.session_id],
            )?;
            check_guard(existing, m.expected)
        }
        LocalStateMutation::TerminalRecord(m) => {
            validate_terminal_record(m).map_err(|_| CommitBatchError::PayloadConflict)?;
            let existing: Option<(String, String, Vec<u8>)> = connection
                .query_row(
                    "SELECT terminal_identity, result, participant_digest FROM terminal_records
                     WHERE session_id = ?1 AND turn_id = ?2",
                    params![m.session_id, m.turn_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(|error| storage_unavailable(&error))?;
            match existing {
                None => Ok(()),
                Some((identity, result, digest))
                    if identity == m.terminal_identity
                        && encoded_record(StoredTerminalV1::decode(&result))?.value()
                            == &m.result
                        && digest.as_slice() == m.participant_digest.as_slice() =>
                {
                    Ok(())
                }
                Some(_) => Err(CommitBatchError::PayloadConflict),
            }
        }
        LocalStateMutation::StopResolution(m) => {
            validate_stop_resolution(m).map_err(|_| CommitBatchError::PayloadConflict)?;
            let existing: Option<(String, String)> = connection
                .query_row(
                    "SELECT resolution, detail FROM stop_resolutions WHERE stop_operation_id = ?1",
                    params![m.stop_operation_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|error| storage_unavailable(&error))?;
            match existing {
                None => Ok(()),
                Some((resolution, detail))
                    if resolution == m.resolution.label()
                        && encoded_record(StoredTerminalV1::decode(&detail))?.value()
                            == &m.detail =>
                {
                    Ok(())
                }
                Some(_) => Err(CommitBatchError::PayloadConflict),
            }
        }
        LocalStateMutation::Obligation(m) => {
            let existing = read_revision(
                connection,
                "SELECT revision FROM obligations WHERE obligation_id = ?1",
                params![m.obligation_id],
            )?;
            check_guard(existing, m.expected)
        }
        LocalStateMutation::RecoveryAction(m) => validate_recovery_action(connection, m),
        LocalStateMutation::ShutdownPlan(m) => {
            let existing: Option<(i64, String)> = connection
                .query_row(
                    "SELECT revision, details_state FROM shutdown_plans
                     WHERE plan_id = ?1 AND epoch = ?2",
                    params![m.key.plan_id, m.key.epoch],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|error| storage_unavailable(&error))?;
            if existing
                .as_ref()
                .is_some_and(|(_, details)| details == "compacted")
            {
                // ArchiveSwitch is one-way and seals the plan root. Logical
                // commit replay was handled before guard validation, so any
                // new mutation here would be a forbidden reverse/rewrite.
                return Err(CommitBatchError::PayloadConflict);
            }
            check_guard(existing.map(|(revision, _)| revision), m.expected)
        }
        LocalStateMutation::ShutdownTarget(m) => {
            let details_state: Option<String> = connection
                .query_row(
                    "SELECT details_state FROM shutdown_plans
                     WHERE plan_id = ?1 AND epoch = ?2",
                    params![m.key.plan_id, m.key.epoch],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| storage_unavailable(&error))?;
            if details_state.as_deref() == Some("compacted") {
                return Err(CommitBatchError::PayloadConflict);
            }
            let existing = read_revision(
                connection,
                "SELECT revision FROM shutdown_targets
                 WHERE plan_id = ?1 AND epoch = ?2 AND ordinal = ?3",
                params![m.key.plan_id, m.key.epoch, m.ordinal],
            )?;
            check_guard(existing, m.expected)
        }
        LocalStateMutation::ShutdownRecoverySnapshot(m) => {
            let details_state: Option<String> = connection
                .query_row(
                    "SELECT details_state FROM shutdown_plans
                     WHERE plan_id = ?1 AND epoch = ?2",
                    params![m.key.plan_id, m.key.epoch],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| storage_unavailable(&error))?;
            if details_state.as_deref() == Some("compacted") {
                return Err(CommitBatchError::PayloadConflict);
            }
            let existing: Option<String> = connection
                .query_row(
                    "SELECT detail FROM shutdown_recovery_snapshots
                     WHERE plan_id = ?1 AND epoch = ?2 AND partition = ?3 AND ordinal = ?4",
                    params![m.key.plan_id, m.key.epoch, m.partition.label(), m.ordinal],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| storage_unavailable(&error))?;
            match existing {
                None => Ok(()),
                Some(detail) => {
                    let saved = encoded_record(StoredShutdownTargetV1::decode(&detail))?;
                    if saved.value() == &m.detail {
                        Ok(())
                    } else {
                        Err(CommitBatchError::PayloadConflict)
                    }
                }
            }
        }
        LocalStateMutation::ShutdownCompactArchive(m) => {
            let existing: Option<String> = connection
                .query_row(
                    "SELECT archive FROM shutdown_compact_archives
                     WHERE plan_id = ?1 AND epoch = ?2",
                    params![m.key.plan_id, m.key.epoch],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| storage_unavailable(&error))?;
            match existing {
                None => Ok(()),
                Some(archive) => {
                    let saved = encoded_record(StoredShutdownArchiveV1::decode(&archive))?;
                    if saved.value() == &m.archive {
                        Ok(())
                    } else {
                        Err(CommitBatchError::PayloadConflict)
                    }
                }
            }
        }
        LocalStateMutation::ShutdownLatestPointer(m) => validate_shutdown_pointer(connection, m),
        LocalStateMutation::ShutdownRetiringPointer(m) => {
            validate_shutdown_retiring_pointer(connection, m)
        }
        LocalStateMutation::MigrationCheckpoint(m) => validate_migration_checkpoint(connection, m),
        LocalStateMutation::MigrationParity(m) => {
            let existing = read_revision(
                connection,
                "SELECT revision FROM local_store_migrations WHERE migration_id = ?1",
                params![m.migration_id],
            )?;
            check_guard(existing, m.expected)
        }
        LocalStateMutation::MigrationQuitFlight(m) => validate_migration_quit_flight(connection, m),
    }
}

fn validate_operation_binding(
    connection: &Connection,
    m: &OperationBindingMutation,
) -> Result<(), CommitBatchError> {
    let existing: Option<(String, Vec<u8>)> = connection
        .query_row(
            "SELECT operation_id, binding_hmac FROM operation_bindings
             WHERE principal = ?1 AND generation_id = ?2 AND kind = ?3 AND caller_request_id = ?4",
            params![
                m.key.principal,
                m.key.generation_id,
                m.key.kind.label(),
                m.key.caller_request_id
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    match existing {
        None => Ok(()),
        Some((operation_id, hmac))
            if operation_id == m.operation_id && hmac.as_slice() == m.binding_hmac.as_slice() =>
        {
            Ok(())
        }
        Some(_) => Err(CommitBatchError::PayloadConflict),
    }
}

fn validate_caller_attempt(
    connection: &Connection,
    m: &CallerAttemptMutation,
) -> Result<(), CommitBatchError> {
    let existing: Option<(Option<String>, Vec<u8>, i64)> = connection
        .query_row(
            "SELECT scope_id, command_hash, revision FROM caller_attempts
             WHERE principal = ?1 AND generation_id = ?2 AND kind = ?3 AND caller_request_id = ?4",
            params![
                m.key.principal,
                m.key.generation_id,
                m.key.kind.label(),
                m.key.caller_request_id
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    match existing {
        None => check_guard(None, m.expected),
        Some((scope_id, hash, revision)) => {
            if scope_id.as_deref() != m.scope_id.as_deref()
                || hash.as_slice() != m.command_hash.as_slice()
            {
                return Err(CommitBatchError::PayloadConflict);
            }
            check_guard(Some(revision), m.expected)
        }
    }
}

fn validate_recovery_action(
    connection: &Connection,
    m: &RecoveryActionMutation,
) -> Result<(), CommitBatchError> {
    let existing: Option<(Vec<u8>, Option<String>, i64)> = connection
        .query_row(
            "SELECT binding_hash, completed, revision FROM recovery_action_attempts
             WHERE action_id = ?1",
            params![m.action_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    match existing {
        None => check_guard(None, m.expected),
        Some((hash, completed, revision)) => {
            if !bool::from(hash.as_slice().ct_eq(m.binding_hash.as_slice())) {
                return Err(CommitBatchError::PayloadConflict);
            }
            if let Some(saved) = completed {
                // The completed result is immutable once saved.
                match &m.completed {
                    Some(new)
                        if encoded_record(StoredRecoveryResultV1::decode(&saved))?.value()
                            == new => {}
                    _ => return Err(CommitBatchError::PayloadConflict),
                }
            }
            check_guard(Some(revision), m.expected)
        }
    }
}

fn validate_shutdown_pointer(
    connection: &Connection,
    m: &ShutdownLatestPointerMutation,
) -> Result<(), CommitBatchError> {
    let (plan_id, epoch, revision): (Option<String>, Option<i64>, i64) = connection
        .query_row(
            "SELECT current_shutdown_plan_id, current_shutdown_epoch, shutdown_pointer_revision
             FROM store_metadata WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| storage_unavailable(&error))?;
    let current = match (plan_id, epoch) {
        (Some(plan_id), Some(epoch)) => Some((plan_id, epoch)),
        _ => None,
    };
    let expected = m
        .expected
        .as_ref()
        .map(|key| (key.plan_id.clone(), key.epoch));
    if current != expected {
        return Err(conflict(revision));
    }
    Ok(())
}

fn validate_shutdown_retiring_pointer(
    connection: &Connection,
    m: &ShutdownRetiringPointerMutation,
) -> Result<(), CommitBatchError> {
    let (plan_id, epoch, revision): (Option<String>, Option<i64>, i64) = connection
        .query_row(
            "SELECT retiring_shutdown_plan_id, retiring_shutdown_epoch,
                    shutdown_retiring_revision
             FROM store_metadata WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| storage_unavailable(&error))?;
    let current = match (plan_id, epoch) {
        (Some(plan_id), Some(epoch)) => Some((plan_id, epoch)),
        (None, None) => None,
        _ => return Err(corrupt("partial shutdown retiring selector")),
    };
    let expected = m
        .expected
        .as_ref()
        .map(|key| (key.plan_id.clone(), key.epoch));
    if current != expected {
        return Err(conflict(revision));
    }
    Ok(())
}

fn validate_migration_checkpoint(
    connection: &Connection,
    m: &MigrationCheckpointMutation,
) -> Result<(), CommitBatchError> {
    let existing: Option<(Vec<u8>, i64)> = connection
        .query_row(
            "SELECT source_inventory_hash, revision FROM local_store_migrations
             WHERE migration_id = ?1",
            params![m.migration_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    match existing {
        None => check_guard(None, m.expected),
        Some((hash, revision)) => {
            if hash.as_slice() != m.source_inventory_hash.as_slice() {
                return Err(CommitBatchError::PayloadConflict);
            }
            check_guard(Some(revision), m.expected)
        }
    }
}

// --- Mutation apply (step 6); guards were validated in step 4 ---

fn apply_mutation(
    connection: &Connection,
    commit_id: &str,
    mutation: &LocalStateMutation,
) -> Result<(), CommitBatchError> {
    let run = |result: Result<usize, rusqlite::Error>| -> Result<(), CommitBatchError> {
        result
            .map(|_| ())
            .map_err(|error| storage_unavailable(&error))
    };
    match mutation {
        LocalStateMutation::OperationBinding(m) => {
            apply_operation_binding(connection, commit_id, m)
        }
        LocalStateMutation::CallerAttempt(m) => apply_caller_attempt(connection, commit_id, m),
        LocalStateMutation::OperationRecord(m)
        | LocalStateMutation::SessionLifecycleOperation(m) => {
            apply_operation_record(connection, commit_id, m)
        }
        LocalStateMutation::SessionProjection(m) => {
            apply_session_projection(connection, commit_id, m)
        }
        LocalStateMutation::MessageProjection(m) => {
            apply_message_projection(connection, commit_id, m)
        }
        LocalStateMutation::SessionProjectionRemoval(m) => {
            run(connection.execute(
                "DELETE FROM message_projection WHERE session_id = ?1",
                params![m.session_id],
            ))?;
            run(connection.execute(
                "DELETE FROM session_projection WHERE session_id = ?1",
                params![m.session_id],
            ))
        }
        LocalStateMutation::TerminalRecord(m) => apply_terminal_record(connection, commit_id, m),
        LocalStateMutation::StopResolution(m) => apply_stop_resolution(connection, commit_id, m),
        LocalStateMutation::Obligation(m) => apply_obligation(connection, commit_id, m),
        LocalStateMutation::RecoveryAction(m) => {
            let existing: Option<(String, Option<String>)> = connection
                .query_row(
                    "SELECT attempt, completed FROM recovery_action_attempts WHERE action_id = ?1",
                    params![m.action_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|error| storage_unavailable(&error))?;
            let attempt = match existing.as_ref() {
                Some((raw, _)) => {
                    let stored = encoded_record(StoredRecoveryActionV1::decode(raw))?;
                    encoded_record(stored.encode_update(&m.attempt))?
                }
                None => encoded_record(StoredRecoveryActionV1::encode_new(&m.attempt))?,
            };
            let completed = match (existing.and_then(|(_, completed)| completed), &m.completed) {
                (Some(raw), Some(_)) => Some(raw),
                (Some(_), None) => return Err(CommitBatchError::PayloadConflict),
                (None, Some(value)) => {
                    Some(encoded_record(StoredRecoveryResultV1::encode_new(value))?)
                }
                (None, None) => None,
            };
            run(connection.execute(
                "INSERT INTO recovery_action_attempts
                (action_id, binding_hash, attempt, completed, revision, commit_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (action_id) DO UPDATE SET
                attempt = excluded.attempt,
                completed = excluded.completed,
                revision = excluded.revision,
                commit_id = excluded.commit_id",
                params![
                    m.action_id,
                    m.binding_hash.as_slice(),
                    attempt,
                    completed,
                    m.revision.value(),
                    commit_id
                ],
            ))
        }
        LocalStateMutation::ShutdownPlan(m) => apply_shutdown_plan(connection, commit_id, m),
        LocalStateMutation::ShutdownTarget(m) => apply_shutdown_target(connection, commit_id, m),
        LocalStateMutation::ShutdownRecoverySnapshot(m) => {
            apply_shutdown_snapshot(connection, commit_id, m)
        }
        LocalStateMutation::ShutdownCompactArchive(m) => {
            apply_shutdown_archive(connection, commit_id, m)
        }
        LocalStateMutation::ShutdownLatestPointer(m) => apply_shutdown_pointer(connection, m),
        LocalStateMutation::ShutdownRetiringPointer(m) => {
            apply_shutdown_retiring_pointer(connection, m)
        }
        LocalStateMutation::MigrationCheckpoint(m) => {
            apply_migration_checkpoint(connection, commit_id, m)
        }
        LocalStateMutation::MigrationParity(m) => apply_migration_parity(connection, commit_id, m),
        LocalStateMutation::MigrationQuitFlight(m) => {
            apply_migration_quit_flight(connection, commit_id, m)
        }
    }
}

fn run(result: Result<usize, rusqlite::Error>) -> Result<(), CommitBatchError> {
    result
        .map(|_| ())
        .map_err(|error| storage_unavailable(&error))
}

fn apply_operation_binding(
    connection: &Connection,
    commit_id: &str,
    m: &OperationBindingMutation,
) -> Result<(), CommitBatchError> {
    run(connection.execute(
        "INSERT INTO operation_bindings
            (principal, generation_id, kind, caller_request_id, operation_id, binding_hmac, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (principal, generation_id, kind, caller_request_id) DO NOTHING",
        params![
            m.key.principal,
            m.key.generation_id,
            m.key.kind.label(),
            m.key.caller_request_id,
            m.operation_id,
            m.binding_hmac.as_slice(),
            commit_id
        ],
    ))
}

fn apply_caller_attempt(
    connection: &Connection,
    commit_id: &str,
    m: &CallerAttemptMutation,
) -> Result<(), CommitBatchError> {
    run(connection.execute(
        "INSERT INTO caller_attempts
            (principal, generation_id, kind, caller_request_id, scope_id, command_hash,
             sealed_command, resolution, revision, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (principal, generation_id, kind, caller_request_id) DO UPDATE SET
            scope_id = excluded.scope_id,
            sealed_command = excluded.sealed_command,
            resolution = excluded.resolution,
            revision = excluded.revision,
            commit_id = excluded.commit_id",
        params![
            m.key.principal,
            m.key.generation_id,
            m.key.kind.label(),
            m.key.caller_request_id,
            m.scope_id,
            m.command_hash.as_slice(),
            m.sealed_command,
            m.resolution.label(),
            m.revision.value(),
            commit_id
        ],
    ))
}

fn apply_operation_record(
    connection: &Connection,
    commit_id: &str,
    m: &OperationRecordMutation,
) -> Result<(), CommitBatchError> {
    let existing: Option<(String, String)> = connection
        .query_row(
            "SELECT receipt, latest_status FROM operation_records WHERE kind = ?1 AND operation_id = ?2",
            params![m.kind.label(), m.operation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    let receipt = match existing.as_ref() {
        Some((raw, _)) => {
            let stored = encoded_record(StoredOperationReceiptV1::decode(raw))?;
            if stored.value() != &m.receipt {
                return Err(CommitBatchError::PayloadConflict);
            }
            raw.clone()
        }
        None => encoded_record(StoredOperationReceiptV1::encode_new(&m.receipt))?,
    };
    let latest_status = match existing.as_ref() {
        Some((_, raw)) => {
            let stored = encoded_record(StoredOperationStatusV1::decode(raw))?;
            encoded_record(stored.encode_update(&m.latest_status))?
        }
        None => encoded_record(StoredOperationStatusV1::encode_new(&m.latest_status))?,
    };
    // The receipt column stays immutable: only the insert writes it.
    run(connection.execute(
        "INSERT INTO operation_records
            (kind, operation_id, receipt, latest_status, revision, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (kind, operation_id) DO UPDATE SET
            latest_status = excluded.latest_status,
            revision = excluded.revision,
            commit_id = excluded.commit_id",
        params![
            m.kind.label(),
            m.operation_id,
            receipt,
            latest_status,
            m.revision.value(),
            commit_id
        ],
    ))
}

fn apply_session_projection(
    connection: &Connection,
    commit_id: &str,
    m: &SessionProjectionMutation,
) -> Result<(), CommitBatchError> {
    let existing = connection
        .query_row(
            "SELECT projection FROM session_projection WHERE session_id = ?1",
            params![m.session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    let encoded =
        encode_session_projection_update_v1(existing.as_deref(), &m.projection, &m.session_id)
            .map_err(|_| CommitBatchError::PayloadConflict)?;
    run(connection.execute(
        "INSERT INTO session_projection (session_id, projection, revision, commit_id)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (session_id) DO UPDATE SET
            projection = excluded.projection,
            revision = excluded.revision,
            commit_id = excluded.commit_id",
        params![m.session_id, encoded, m.revision.value(), commit_id],
    ))
}

fn apply_message_projection(
    connection: &Connection,
    commit_id: &str,
    m: &MessageProjectionMutation,
) -> Result<(), CommitBatchError> {
    let existing = connection
        .query_row(
            "SELECT message_ordinal, projection FROM message_projection
             WHERE session_id = ?1 AND message_id = ?2",
            params![m.session_id, m.message_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    let message_ordinal = match existing.as_ref() {
        Some((ordinal, _)) if *ordinal > 0 => *ordinal,
        Some(_) => return Err(corrupt("message projection ordinal")),
        None => connection
            .query_row(
                "SELECT COALESCE(MAX(message_ordinal), 0) + 1
                 FROM message_projection WHERE session_id = ?1",
                params![m.session_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| storage_unavailable(&error))?,
    };
    let encoded = encode_message_projection_update_v1(
        existing.as_ref().map(|(_, raw)| raw.as_str()),
        &m.projection,
        &m.session_id,
        &m.message_id,
    )
    .map_err(|_| CommitBatchError::PayloadConflict)?;
    run(connection.execute(
        "INSERT INTO message_projection
            (session_id, message_id, message_ordinal, projection, revision, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (session_id, message_id) DO UPDATE SET
            projection = excluded.projection,
            revision = excluded.revision,
            commit_id = excluded.commit_id",
        params![
            m.session_id,
            m.message_id,
            message_ordinal,
            encoded,
            m.revision.value(),
            commit_id
        ],
    ))
}

fn apply_terminal_record(
    connection: &Connection,
    commit_id: &str,
    m: &TerminalRecordMutation,
) -> Result<(), CommitBatchError> {
    let result = encoded_record(StoredTerminalV1::encode_new(&m.result))?;
    run(connection.execute(
        "INSERT INTO terminal_records
            (session_id, turn_id, terminal_identity, result, participant_digest, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (session_id, turn_id) DO NOTHING",
        params![
            m.session_id,
            m.turn_id,
            m.terminal_identity,
            result,
            m.participant_digest.as_slice(),
            commit_id
        ],
    ))
}

fn apply_stop_resolution(
    connection: &Connection,
    commit_id: &str,
    m: &StopResolutionMutation,
) -> Result<(), CommitBatchError> {
    let detail = encoded_record(StoredTerminalV1::encode_new(&m.detail))?;
    run(connection.execute(
        "INSERT INTO stop_resolutions (stop_operation_id, resolution, detail, commit_id)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (stop_operation_id) DO NOTHING",
        params![m.stop_operation_id, m.resolution.label(), detail, commit_id],
    ))
}

fn apply_obligation(
    connection: &Connection,
    commit_id: &str,
    m: &ObligationMutation,
) -> Result<(), CommitBatchError> {
    let existing: Option<String> = connection
        .query_row(
            "SELECT record FROM obligations WHERE obligation_id = ?1",
            params![m.obligation_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    let record = match existing {
        Some(raw) => {
            let stored = encoded_record(StoredObligationV1::decode(&raw))?;
            encoded_record(stored.encode_update(&m.record))?
        }
        None => encoded_record(StoredObligationV1::encode_new(&m.record))?,
    };
    run(connection.execute(
        "INSERT INTO obligations (obligation_id, record, pending, revision, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (obligation_id) DO UPDATE SET
            record = excluded.record,
            pending = excluded.pending,
            revision = excluded.revision,
            commit_id = excluded.commit_id",
        params![
            m.obligation_id,
            record,
            i64::from(m.pending.is_some()),
            m.revision.value(),
            commit_id
        ],
    ))?;
    // Keep the pending index in exact parity within the same transaction.
    run(connection.execute(
        "DELETE FROM pending_obligations WHERE obligation_id = ?1",
        params![m.obligation_id],
    ))?;
    if let Some(pending) = &m.pending {
        run(connection.execute(
            "INSERT INTO pending_obligations
                (ordered_key, obligation_id, owner, partition,
                 shutdown_plan_id, shutdown_epoch, commit_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                pending.ordered_key,
                m.obligation_id,
                pending.owner,
                pending.partition.label(),
                pending
                    .shutdown_plan
                    .as_ref()
                    .map(|plan| plan.plan_id.clone()),
                pending.shutdown_plan.as_ref().map(|plan| plan.epoch),
                commit_id
            ],
        ))?;
    }
    Ok(())
}

fn apply_shutdown_plan(
    connection: &Connection,
    commit_id: &str,
    m: &ShutdownPlanMutation,
) -> Result<(), CommitBatchError> {
    let existing: Option<String> = connection
        .query_row(
            "SELECT summary FROM shutdown_plans WHERE plan_id = ?1 AND epoch = ?2",
            params![m.key.plan_id, m.key.epoch],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    let summary = match existing {
        Some(raw) => {
            let stored = encoded_record(StoredShutdownPlanV1::decode(&raw))?;
            encoded_record(stored.encode_update(&m.summary))?
        }
        None => encoded_record(StoredShutdownPlanV1::encode_new(&m.summary))?,
    };
    run(connection.execute(
        "INSERT INTO shutdown_plans
            (plan_id, epoch, phase, summary, details_state, revision, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (plan_id, epoch) DO UPDATE SET
            phase = excluded.phase,
            summary = excluded.summary,
            details_state = excluded.details_state,
            revision = excluded.revision,
            commit_id = excluded.commit_id",
        params![
            m.key.plan_id,
            m.key.epoch,
            shutdown_phase_to_label(m.phase),
            summary,
            m.details_state.label(),
            m.revision.value(),
            commit_id
        ],
    ))
}

fn apply_shutdown_target(
    connection: &Connection,
    commit_id: &str,
    m: &ShutdownTargetMutation,
) -> Result<(), CommitBatchError> {
    let existing: Option<String> = connection
        .query_row(
            "SELECT detail FROM shutdown_targets WHERE plan_id = ?1 AND epoch = ?2 AND ordinal = ?3",
            params![m.key.plan_id, m.key.epoch, m.ordinal],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    let detail = match existing {
        Some(raw) => {
            let stored = encoded_record(StoredShutdownTargetV1::decode(&raw))?;
            encoded_record(stored.encode_update(&m.detail))?
        }
        None => encoded_record(StoredShutdownTargetV1::encode_new(&m.detail))?,
    };
    run(connection.execute(
        "INSERT INTO shutdown_targets (plan_id, epoch, ordinal, detail, revision, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (plan_id, epoch, ordinal) DO UPDATE SET
            detail = excluded.detail,
            revision = excluded.revision,
            commit_id = excluded.commit_id",
        params![
            m.key.plan_id,
            m.key.epoch,
            m.ordinal,
            detail,
            m.revision.value(),
            commit_id
        ],
    ))
}

fn apply_shutdown_snapshot(
    connection: &Connection,
    commit_id: &str,
    m: &ShutdownRecoverySnapshotMutation,
) -> Result<(), CommitBatchError> {
    let detail = encoded_record(StoredShutdownTargetV1::encode_new(&m.detail))?;
    run(connection.execute(
        "INSERT INTO shutdown_recovery_snapshots
            (plan_id, epoch, partition, ordinal, detail, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (plan_id, epoch, partition, ordinal) DO NOTHING",
        params![
            m.key.plan_id,
            m.key.epoch,
            m.partition.label(),
            m.ordinal,
            detail,
            commit_id
        ],
    ))
}

fn apply_shutdown_archive(
    connection: &Connection,
    commit_id: &str,
    m: &ShutdownCompactArchiveMutation,
) -> Result<(), CommitBatchError> {
    let archive = encoded_record(StoredShutdownArchiveV1::encode_new(&m.archive))?;
    run(connection.execute(
        "INSERT INTO shutdown_compact_archives (plan_id, epoch, archive, commit_id)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (plan_id, epoch) DO NOTHING",
        params![m.key.plan_id, m.key.epoch, archive, commit_id],
    ))
}

/// Remove obsolete detail rows only after their plan is durably compacted.
/// This is physical maintenance, not a semantic mutation: readers switch to
/// the archive as soon as the plan CAS commits. Each invocation is bounded to
/// 64 rows, 4 MiB and 50 ms and is safe to repeat after a crash.
pub(crate) fn cleanup_compacted_shutdown_details(
    connection: &Connection,
) -> Result<usize, rusqlite::Error> {
    const MAX_ROWS: usize = 64;
    const MAX_BYTES: usize = 4 * 1024 * 1024;
    let started = std::time::Instant::now();
    let selector_present: i64 = connection.query_row(
        "SELECT retiring_shutdown_plan_id IS NOT NULL
                AND retiring_shutdown_epoch IS NOT NULL
         FROM store_metadata WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    if selector_present == 0 {
        return Ok(0);
    }
    connection.execute_batch("BEGIN IMMEDIATE")?;
    let cleanup = (|| -> Result<usize, rusqlite::Error> {
        let pointer: (Option<String>, Option<i64>) = connection.query_row(
            "SELECT retiring_shutdown_plan_id, retiring_shutdown_epoch
             FROM store_metadata WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let (Some(plan_id), Some(epoch)) = pointer else {
            return Ok(0);
        };

        let mut statement = connection.prepare(
            "SELECT detail_kind, detail_rowid, detail_bytes FROM (
                 SELECT 0 AS detail_kind, t.rowid AS detail_rowid,
                        length(CAST(t.detail AS BLOB)) AS detail_bytes
                 FROM shutdown_targets t
                 JOIN shutdown_plans p
                   ON p.plan_id = t.plan_id AND p.epoch = t.epoch
                 WHERE t.plan_id = ?1 AND t.epoch = ?2
                   AND p.details_state = 'compacted'
                 UNION ALL
                 SELECT 1 AS detail_kind, s.rowid AS detail_rowid,
                        length(CAST(s.detail AS BLOB)) AS detail_bytes
                 FROM shutdown_recovery_snapshots s
                 JOIN shutdown_plans p
                   ON p.plan_id = s.plan_id AND p.epoch = s.epoch
                 WHERE s.plan_id = ?1 AND s.epoch = ?2
                   AND p.details_state = 'compacted'
             )
             ORDER BY detail_kind, detail_rowid
             LIMIT ?3",
        )?;
        let rows = statement.query_map(params![plan_id, epoch, MAX_ROWS as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut selected = Vec::new();
        let mut selected_bytes = 0usize;
        for row in rows {
            let (kind, rowid, bytes) = row?;
            let bytes = usize::try_from(bytes).unwrap_or(usize::MAX);
            if selected_bytes.saturating_add(bytes) > MAX_BYTES
                || started.elapsed() >= std::time::Duration::from_millis(50)
            {
                break;
            }
            selected_bytes = selected_bytes.saturating_add(bytes);
            selected.push((kind, rowid));
        }
        drop(statement);

        let mut deleted = 0usize;
        for (kind, rowid) in selected {
            if started.elapsed() >= std::time::Duration::from_millis(50) {
                break;
            }
            let table = if kind == 0 {
                "shutdown_targets"
            } else {
                "shutdown_recovery_snapshots"
            };
            deleted = deleted.saturating_add(connection.execute(
                &format!("DELETE FROM {table} WHERE rowid = ?1"),
                params![rowid],
            )?);
        }

        let remaining: i64 = connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM shutdown_targets
                 WHERE plan_id = ?1 AND epoch = ?2
                 UNION ALL
                 SELECT 1 FROM shutdown_recovery_snapshots
                 WHERE plan_id = ?1 AND epoch = ?2
             )",
            params![plan_id, epoch],
            |row| row.get(0),
        )?;
        if remaining == 0 {
            connection.execute(
                "UPDATE store_metadata SET
                    retiring_shutdown_plan_id = NULL,
                    retiring_shutdown_epoch = NULL,
                    shutdown_retiring_revision = shutdown_retiring_revision + 1
                 WHERE id = 1
                   AND retiring_shutdown_plan_id = ?1
                   AND retiring_shutdown_epoch = ?2",
                params![plan_id, epoch],
            )?;
        }
        Ok(deleted)
    })();
    match cleanup {
        Ok(deleted) => {
            connection.execute_batch("COMMIT")?;
            Ok(deleted)
        }
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn apply_shutdown_pointer(
    connection: &Connection,
    m: &ShutdownLatestPointerMutation,
) -> Result<(), CommitBatchError> {
    run(connection.execute(
        "UPDATE store_metadata SET
            current_shutdown_plan_id = ?1,
            current_shutdown_epoch = ?2,
            shutdown_pointer_revision = shutdown_pointer_revision + 1
         WHERE id = 1",
        params![
            m.new.as_ref().map(|key| key.plan_id.clone()),
            m.new.as_ref().map(|key| key.epoch)
        ],
    ))
}

fn apply_shutdown_retiring_pointer(
    connection: &Connection,
    m: &ShutdownRetiringPointerMutation,
) -> Result<(), CommitBatchError> {
    if let Some(plan) = &m.new {
        let details_state: Option<String> = connection
            .query_row(
                "SELECT details_state FROM shutdown_plans
                 WHERE plan_id = ?1 AND epoch = ?2",
                params![plan.plan_id, plan.epoch],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| storage_unavailable(&error))?;
        if details_state.as_deref() != Some("compacted") {
            return Err(CommitBatchError::PayloadConflict);
        }
    }
    run(connection.execute(
        "UPDATE store_metadata SET
            retiring_shutdown_plan_id = ?1,
            retiring_shutdown_epoch = ?2,
            shutdown_retiring_revision = shutdown_retiring_revision + 1
         WHERE id = 1",
        params![
            m.new.as_ref().map(|key| key.plan_id.clone()),
            m.new.as_ref().map(|key| key.epoch)
        ],
    ))
}

fn apply_migration_checkpoint(
    connection: &Connection,
    commit_id: &str,
    m: &MigrationCheckpointMutation,
) -> Result<(), CommitBatchError> {
    let existing: Option<String> = connection
        .query_row(
            "SELECT checkpoint FROM local_store_migrations WHERE migration_id = ?1",
            params![m.migration_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    let checkpoint = match existing {
        Some(raw) => {
            let stored = encoded_record(StoredMigrationCheckpointV1::decode(&raw))?;
            encoded_record(stored.encode_update(&m.checkpoint))?
        }
        None => encoded_record(StoredMigrationCheckpointV1::encode_new(&m.checkpoint))?,
    };
    run(connection.execute(
        "INSERT INTO local_store_migrations
            (migration_id, phase, source_inventory_hash, checkpoint, parity, revision, commit_id)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6)
         ON CONFLICT (migration_id) DO UPDATE SET
            phase = excluded.phase,
            checkpoint = excluded.checkpoint,
            revision = excluded.revision,
            commit_id = excluded.commit_id",
        params![
            m.migration_id,
            migration_phase_to_label(m.phase),
            m.source_inventory_hash.as_slice(),
            checkpoint,
            m.revision.value(),
            commit_id
        ],
    ))
}

fn apply_migration_parity(
    connection: &Connection,
    commit_id: &str,
    m: &MigrationParityMutation,
) -> Result<(), CommitBatchError> {
    let existing: Option<Option<String>> = connection
        .query_row(
            "SELECT parity FROM local_store_migrations WHERE migration_id = ?1",
            params![m.migration_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    let parity = match existing.flatten() {
        Some(raw) => {
            let stored = encoded_record(StoredMigrationParityV1::decode(&raw))?;
            encoded_record(stored.encode_update(&m.parity))?
        }
        None => encoded_record(StoredMigrationParityV1::encode_new(&m.parity))?,
    };
    run(connection.execute(
        "UPDATE local_store_migrations SET
            parity = ?2,
            revision = ?3,
            commit_id = ?4
         WHERE migration_id = ?1",
        params![m.migration_id, parity, m.revision.value(), commit_id],
    ))
}

fn validate_migration_quit_flight(
    connection: &Connection,
    m: &MigrationQuitFlightMutation,
) -> Result<(), CommitBatchError> {
    let existing: Option<(String, String, i64)> = connection
        .query_row(
            "SELECT migration_id, accepted_boot_id, revision
             FROM migration_quit_flights WHERE operation_id = ?1",
            params![m.operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    match existing {
        None => check_guard(None, m.expected)?,
        Some((migration_id, accepted_boot_id, revision)) => {
            if migration_id != m.migration_id || accepted_boot_id != m.accepted_boot_id {
                return Err(CommitBatchError::PayloadConflict);
            }
            check_guard(Some(revision), m.expected)?;
        }
    }
    let migration_exists: i64 = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM local_store_migrations WHERE migration_id = ?1)",
            params![m.migration_id],
            |row| row.get(0),
        )
        .map_err(|error| storage_unavailable(&error))?;
    if migration_exists == 0 {
        return Err(CommitBatchError::PayloadConflict);
    }
    let other_operation: Option<String> = connection
        .query_row(
            "SELECT operation_id FROM migration_quit_flights
             WHERE migration_id = ?1 AND operation_id <> ?2",
            params![m.migration_id, m.operation_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    if other_operation.is_some() {
        return Err(CommitBatchError::PayloadConflict);
    }
    Ok(())
}

fn apply_migration_quit_flight(
    connection: &Connection,
    commit_id: &str,
    m: &MigrationQuitFlightMutation,
) -> Result<(), CommitBatchError> {
    let quit_receipt: String = connection
        .query_row(
            "SELECT receipt FROM operation_records
             WHERE kind = 'application_quit' AND operation_id = ?1",
            params![m.operation_id],
            |row| row.get(0),
        )
        .map_err(|error| storage_unavailable(&error))?;
    encoded_record(StoredMigrationQuitV1::decode(&quit_receipt))?;
    let changed = connection
        .execute(
            "INSERT OR IGNORE INTO migration_quit_flights
                (operation_id, migration_id, migration_revision, checkpoint,
                 accepted_boot_id, revision, commit_id)
             SELECT ?1, migration_id, revision, checkpoint, ?3, ?4, ?5
             FROM local_store_migrations WHERE migration_id = ?2",
            params![
                m.operation_id,
                m.migration_id,
                m.accepted_boot_id,
                m.revision.value(),
                commit_id
            ],
        )
        .map_err(|error| storage_unavailable(&error))?;
    if changed == 0 {
        let saved: Option<(String, String, i64)> = connection
            .query_row(
                "SELECT migration_id, accepted_boot_id, revision
                 FROM migration_quit_flights WHERE operation_id = ?1",
                params![m.operation_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| storage_unavailable(&error))?;
        if saved
            .as_ref()
            .map(|(migration_id, accepted_boot_id, revision)| {
                (migration_id.as_str(), accepted_boot_id.as_str(), *revision)
            })
            == Some((
                m.migration_id.as_str(),
                m.accepted_boot_id.as_str(),
                m.revision.value(),
            ))
        {
            return Ok(());
        }
        return Err(CommitBatchError::PayloadConflict);
    }
    Ok(())
}

#[cfg(test)]
mod shutdown_compaction_tests {
    use super::*;
    use crate::adaptor::gateway::local_event_store::schema::apply_schema;
    use crate::domain::local_event::{
        ApplicationShutdownPhase, QuitIntent, Revision, ShutdownDetailsState, ShutdownPlanKey,
        ShutdownPlanMutation, ShutdownPlanRecord,
    };

    fn seed_retiring_plan(connection: &Connection, target_count: usize) {
        apply_schema(connection).expect("schema");
        connection
            .execute(
                "INSERT INTO store_metadata (
                    id, schema_version, store_id, generation_id, created_at_ms,
                    cursor_hmac_key, operation_binding_hmac_key, boot_id,
                    next_global_sequence, health, current_shutdown_plan_id,
                    current_shutdown_epoch, shutdown_pointer_revision,
                    retiring_shutdown_plan_id, retiring_shutdown_epoch,
                    shutdown_retiring_revision
                 ) VALUES (
                    1, 1, 'store', 'generation', 0, zeroblob(32), zeroblob(32),
                    'boot', 1, 'ok', NULL, NULL, 0, 'plan', 0, 1
                 )",
                [],
            )
            .expect("metadata");
        connection
            .execute(
                "INSERT INTO logical_commits (
                    commit_id, generation_id, operation_kind, idempotency_key,
                    payload_hash, state, first_global_sequence,
                    last_global_sequence, event_count, mutation_count,
                    stream_heads_json, result_hash, committed_at_ms
                 ) VALUES (
                    'seed', 'generation', 'application_quit', 'seed', zeroblob(32),
                    'sealed', NULL, NULL, 0, 0, '[]', zeroblob(32), 0
                 )",
                [],
            )
            .expect("logical commit");
        connection
            .execute(
                "INSERT INTO shutdown_plans
                    (plan_id, epoch, phase, summary, details_state, revision, commit_id)
                 VALUES ('plan', 0, 'completed', '{}', 'compacted', 1, 'seed')",
                [],
            )
            .expect("plan");
        for ordinal in 0..target_count {
            connection
                .execute(
                    "INSERT INTO shutdown_targets
                        (plan_id, epoch, ordinal, detail, revision, commit_id)
                     VALUES ('plan', 0, ?1, '{}', 0, 'seed')",
                    params![ordinal as i64],
                )
                .expect("target");
        }
    }

    #[test]
    fn retiring_cleanup_is_sixty_four_rows_and_restart_resumable() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let database = directory.path().join("shutdown-cleanup.sqlite3");
        let connection = Connection::open(&database).expect("open fixture");
        seed_retiring_plan(&connection, 130);

        assert_eq!(
            cleanup_compacted_shutdown_details(&connection).expect("first cleanup"),
            64
        );
        let remaining: i64 = connection
            .query_row("SELECT COUNT(*) FROM shutdown_targets", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 66);
        let retiring: Option<String> = connection
            .query_row(
                "SELECT retiring_shutdown_plan_id FROM store_metadata WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retiring.as_deref(), Some("plan"));
        drop(connection);

        let reopened = Connection::open(&database).expect("reopen fixture");
        apply_schema(&reopened).expect("reopen schema");
        assert_eq!(
            cleanup_compacted_shutdown_details(&reopened).expect("second cleanup"),
            64
        );
        assert_eq!(
            cleanup_compacted_shutdown_details(&reopened).expect("final cleanup"),
            2
        );
        let remaining: i64 = reopened
            .query_row("SELECT COUNT(*) FROM shutdown_targets", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 0);
        let pointer: (Option<String>, Option<i64>) = reopened
            .query_row(
                "SELECT retiring_shutdown_plan_id, retiring_shutdown_epoch
                 FROM store_metadata WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(pointer, (None, None));
    }

    #[test]
    fn compacted_plan_cannot_return_to_available() {
        let connection = Connection::open_in_memory().expect("open fixture");
        seed_retiring_plan(&connection, 0);
        let mutation = LocalStateMutation::ShutdownPlan(ShutdownPlanMutation {
            key: ShutdownPlanKey {
                plan_id: "plan".to_string(),
                epoch: 0,
            },
            phase: ApplicationShutdownPhase::Completed,
            summary: ShutdownPlanRecord {
                operation_id: "quit".to_string(),
                intent: QuitIntent::Exit { code: 0 },
                t0_ms: 0,
                preparation_cutoff_ms: None,
                deadline_ms: 15_000,
                target_count: None,
                prepared_count: None,
                effect_reserved_count: None,
                terminal_count: None,
                completed_count: None,
                unresolved_count: None,
                recovery_snapshot_count: None,
                recovery_snapshot_id: None,
                boot_id: "boot".to_string(),
                outcome: None,
                failure: None,
                shutdown_effect_count: None,
                admission_open: None,
                retry_quit_same_boot: None,
            },
            details_state: ShutdownDetailsState::Available,
            expected: RevisionGuard::Expected(Revision::new(1).unwrap()),
            revision: Revision::new(2).unwrap(),
        });
        assert_eq!(
            validate_one_guard(&connection, &mutation),
            Err(CommitBatchError::PayloadConflict)
        );
    }
}
