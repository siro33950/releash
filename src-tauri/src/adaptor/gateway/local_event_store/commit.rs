//! The `commit_batch` transaction (design "Commit transaction", 9 steps).
//!
//! Executed on the writer thread only. Any failure before SQLite COMMIT
//! rolls back to the pre-batch state; any error or reply loss between the
//! start of COMMIT and the completed fresh readback is `OutcomeUnknown` for
//! the same commit identity.

use std::collections::HashSet;

use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::adaptor::gateway::local_event_store::fault::FaultInjector;
use crate::adaptor::gateway::local_event_store::indexed_projection_codec::{
    indexed_execution_node_row, indexed_execution_row, indexed_session_public_columns,
    IndexedExecutionNodeRow, IndexedExecutionRow, EXECUTION_NODE_RECORD_SCHEMA,
    EXECUTION_RECORD_SCHEMA,
};
use crate::adaptor::gateway::local_event_store::projection_record_codec::encode_session_projection_update_v1;
use crate::adaptor::gateway::local_event_store::state_record_codec::{
    StoredObligationV1, StoredOperationReceiptV1, StoredOperationStatusV1, StoredRecoveryActionV1,
    StoredRecoveryResultV1, StoredShutdownPlanV1, StoredShutdownTargetV1,
};
use crate::adaptor::gateway::local_event_store::writer::{PreparedBatch, PreparedEvent};
use crate::domain::local_event::{
    validate_operation_record, CallerAttemptMutation, CommitBatchError, CommitBatchResult,
    CommitOperationKind, CommitResolution, CommittedBatch, CommittedStreamHead, IdempotencyBinding,
    LocalEventQueryError, LocalStateMutation, ObligationMutation, ObligationRecord,
    ObligationStateRecord, OperationBindingMutation, OperationKind, OperationRecordMutation,
    RecoveryActionMutation, RecoveryAttemptRecord, RecoveryResourceViewRecord,
    RecoveryResultRecord, Revision, RevisionGuard, SafeOperationFailure,
    SessionOperationFailureKind, SessionProjectionMutation, ShutdownDetailsCompactionMutation,
    ShutdownLatestPointerMutation, ShutdownPlanMutation, ShutdownRecoverySnapshotMutation,
    ShutdownTargetMutation, ShutdownTargetRecord, StreamId, StreamVersion,
    WorkflowExecutionNodeProjectionMutation, WorkflowExecutionProjectionMutation,
};
use crate::domain::workspace_tree::WorkspaceTree;

use crate::adaptor::gateway::local_event_store::envelope::shutdown_phase_to_label;

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
    for mutation in mutations {
        match mutation {
            LocalStateMutation::OperationRecord(record) => {
                if matches!(record.expected, RevisionGuard::Absent) {
                    return false;
                }
                advances_existing_owner = true;
            }
            LocalStateMutation::Obligation(obligation) => {
                if matches!(obligation.expected, RevisionGuard::Absent) {
                    return false;
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
            LocalStateMutation::WorkflowExecutionProjection(_)
            | LocalStateMutation::WorkflowExecutionNodeProjection(_) => {}
            LocalStateMutation::OperationBinding(_)
            | LocalStateMutation::SessionProjectionRemoval(_)
            | LocalStateMutation::AgentSessionRemoval(_)
            | LocalStateMutation::ShutdownPlan(_)
            | LocalStateMutation::ShutdownTarget(_)
            | LocalStateMutation::ShutdownRecoverySnapshot(_)
            | LocalStateMutation::ShutdownDetailsCompaction(_)
            | LocalStateMutation::ShutdownLatestPointer(_) => return false,
        }
    }
    advances_existing_owner
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
            | LocalStateMutation::AgentSessionRemoval(_)
            | LocalStateMutation::CallerAttempt(_)
            | LocalStateMutation::ShutdownPlan(_)
            | LocalStateMutation::ShutdownTarget(_)
            | LocalStateMutation::ShutdownRecoverySnapshot(_)
            | LocalStateMutation::ShutdownDetailsCompaction(_)
            | LocalStateMutation::ShutdownLatestPointer(_) => return false,
            LocalStateMutation::OperationRecord(operation) => {
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
            LocalStateMutation::WorkflowExecutionProjection(workflow) => {
                if !guard_advances_existing(workflow.expected, workflow.revision)
                    && !guard_inserts_revision_zero(workflow.expected, workflow.revision)
                {
                    return false;
                }
            }
            LocalStateMutation::WorkflowExecutionNodeProjection(nodes) => {
                if !guard_advances_existing(nodes.expected, nodes.revision)
                    && !guard_inserts_revision_zero(nodes.expected, nodes.revision)
                {
                    return false;
                }
            }
        }
    }
    if !advances_existing_owner {
        return false;
    }
    match prepared.batch.idempotency.operation_kind {
        CommitOperationKind::Projection => false,
        CommitOperationKind::Workflow => workflow_progress_has_one_execution_scope(prepared),
        CommitOperationKind::ApplicationQuit
        | CommitOperationKind::Recovery
        | CommitOperationKind::UserMutation
        | CommitOperationKind::OperationProgress => false,
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
                crate::domain::local_event::SessionProjectionRecord::AgentSession(_)
                | crate::domain::local_event::SessionProjectionRecord::ProviderSessionOwnership(
                    _,
                )
                | crate::domain::local_event::SessionProjectionRecord::ProviderHookHealth(_) => {
                    false
                }
            },
            LocalStateMutation::WorkflowExecutionProjection(projection) => {
                prepared.batch.state_mutations.iter().any(|candidate| {
                    matches!(
                        candidate,
                        LocalStateMutation::SessionProjection(source)
                            if source.session_id == format!("workflow:{execution_id}")
                                && source.expected == projection.expected
                                && source.revision == projection.revision
                    )
                }) && match &projection.projection {
                    crate::domain::local_event::WorkflowExecutionProjectionRecord::Present(
                        execution,
                    ) => execution.execution_id == execution_id,
                    crate::domain::local_event::WorkflowExecutionProjectionRecord::Deleted {
                        execution_id: deleted,
                    } => deleted == &execution_id,
                }
            }
            LocalStateMutation::WorkflowExecutionNodeProjection(nodes) => {
                nodes.execution_id == execution_id
                    && nodes
                        .nodes
                        .iter()
                        .all(|node| node.execution_id.as_deref() == Some(execution_id.as_str()))
                    && prepared.batch.state_mutations.iter().any(|candidate| {
                        matches!(
                            candidate,
                            LocalStateMutation::WorkflowExecutionProjection(projection)
                                if projection.expected == nodes.expected
                                    && projection.revision == nodes.revision
                        )
                    })
            }
            _ => false,
        })
}

fn recovery_result_targets_shutdown_target(
    resource_view: &RecoveryResourceViewRecord,
    current_plan_id: &str,
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
            plan.shutdown_id == current_plan_id
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
                && value.get("shutdown_id").and_then(serde_json::Value::as_str)
                    == Some(current_plan_id)
                && value.get("ordinal").and_then(serde_json::Value::as_i64) == Some(target_ordinal)
                && value.get("target_key").and_then(serde_json::Value::as_str) == Some(target_key)
                && value.get("state").and_then(serde_json::Value::as_str) == Some(state)
        }
    }
}

fn shutdown_target_key(
    kind: crate::domain::local_event::ShutdownTargetKindRecord,
    target_id: &str,
) -> String {
    let kind = match kind {
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

fn application_quit_progress_is_bound_to_current_plan(
    connection: &Connection,
    current_plan_id: &str,
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
            && binding.key.installation_id == batch.idempotency.installation_id
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
                 WHERE shutdown_id = ?1
                 ORDER BY ordinal LIMIT 4097",
            )
            .map_err(|error| storage_unavailable(&error))?;
        let rows = statement
            .query_map(params![current_plan_id], |row| {
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
        shutdown_id: current_plan_id.to_string(),
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
                } = &operation.receipt;
                if operation_revision.replace(operation.revision).is_some()
                    || operation.kind != OperationKind::ApplicationQuit
                    || operation.operation_id != operation_id
                    || receipt_operation_id != operation_id
                    || plan != &current_plan
                    || operation.latest_status.kind != OperationKind::ApplicationQuit
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
                         WHERE shutdown_id = ?1 AND ordinal = ?2",
                        params![current_plan_id, target.ordinal],
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
            | LocalStateMutation::SessionProjectionRemoval(_)
            | LocalStateMutation::AgentSessionRemoval(_)
            | LocalStateMutation::Obligation(_)
            | LocalStateMutation::RecoveryAction(_)
            | LocalStateMutation::WorkflowExecutionProjection(_)
            | LocalStateMutation::WorkflowExecutionNodeProjection(_)
            | LocalStateMutation::ShutdownRecoverySnapshot(_)
            | LocalStateMutation::ShutdownDetailsCompaction(_) => return Ok(false),
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
                    shutdown_id,
                    phase,
                    ..
                }
            ) if shutdown_id == current_plan_id
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
                && plan.key.shutdown_id == current_plan_id
                && plan.phase
                    == crate::domain::local_event::ApplicationShutdownPhase::Completed
                && plan.summary.operation_id == plan_summary.operation_id
                && plan.summary.intent == plan_summary.intent
                && matches!(plan.expected, RevisionGuard::Expected(_))
                && pointer
                    .expected
                    .as_ref()
                    .is_some_and(|key| key.shutdown_id == current_plan_id)
                && pointer.new.is_none() =>
        {
            true
        }
        _ => return Ok(false),
    };
    if target.key.shutdown_id != current_plan_id || target.ordinal < 0 {
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
             WHERE shutdown_id = ?1 AND ordinal = ?2",
            params![current_plan_id, target.ordinal],
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
    } = &action.attempt;
    let expected_resource_ref = format!(
        "shutdown-target:{current_plan_id}:{}:{target_key}",
        target.ordinal
    );
    if plan.shutdown_id != current_plan_id
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
            } = existing_attempt;
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
             WHERE installation_id = ?1 AND operation_kind = ?2 AND idempotency_key = ?3",
            params![
                idempotency.installation_id,
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
    let (current_shutdown, current_shutdown_phase, current_shutdown_summary): (
        Option<String>,
        Option<String>,
        Option<String>,
    ) = connection
        .query_row(
            "SELECT m.current_shutdown_id, p.phase, p.summary
             FROM store_metadata m
             LEFT JOIN shutdown_plans p
               ON p.shutdown_id = m.current_shutdown_id
             WHERE m.id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| storage_unavailable(&error))?;
    let shutdown_admission_closed = match (
        current_shutdown.as_ref(),
        current_shutdown_phase.as_deref(),
        current_shutdown_summary.as_deref(),
    ) {
        (None, None, None) => false,
        (Some(_), Some("failed" | "cancelled" | "completed"), Some(_)) => false,
        (Some(_), Some(_), Some(_)) => true,
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
        && operation_progress_is_structurally_valid(&batch.state_mutations);
    let internal_progress = shutdown_admission_closed
        && matches!(
            batch.idempotency.operation_kind,
            CommitOperationKind::Workflow | CommitOperationKind::Projection
        )
        && internal_progress_is_anchored_to_existing_owner(prepared);
    let application_quit_progress = if shutdown_admission_closed
        && batch.idempotency.operation_kind == CommitOperationKind::ApplicationQuit
    {
        application_quit_progress_is_bound_to_current_plan(
            connection,
            current_shutdown
                .as_deref()
                .expect("closed shutdown has a plan id"),
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
            CommitOperationKind::UserMutation
                | CommitOperationKind::Recovery
                | CommitOperationKind::Workflow
                | CommitOperationKind::Projection
                | CommitOperationKind::ApplicationQuit
        ) && !shutdown_target_recovery
            && !internal_progress
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
                commit_id, installation_id, operation_kind, idempotency_key, payload_hash,
                state, first_global_sequence, last_global_sequence, event_count,
                mutation_count, stream_heads_json, result_hash, committed_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'preparing', NULL, NULL, ?6, ?7, ?8, NULL, ?9)",
            params![
                commit_id,
                batch.idempotency.installation_id,
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
    for mutation in mutations {
        validate_one_guard(connection, mutation)?;
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
        LocalStateMutation::OperationRecord(m) => {
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
            indexed_session_public_columns(&m.projection)
                .map_err(|_| CommitBatchError::PayloadConflict)?;
            let existing = read_revision(
                connection,
                "SELECT revision FROM session_projection WHERE session_id = ?1",
                params![m.session_id],
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
        LocalStateMutation::AgentSessionRemoval(m) => {
            let current_head = read_revision(
                connection,
                "SELECT head FROM stream_heads WHERE stream_id = ?1",
                params![m.agent_session_stream.as_str()],
            )?
            .ok_or_else(|| conflict(0))?;
            if current_head.checked_add(1) != Some(m.retained_tombstone_sequence.value()) {
                return Err(conflict(current_head));
            }
            match (
                &m.ownership_projection_id,
                &m.ownership_stream,
                m.ownership_expected,
            ) {
                (Some(projection_id), Some(stream), Some(expected)) => {
                    let projection_revision = read_revision(
                        connection,
                        "SELECT revision FROM session_projection WHERE session_id = ?1",
                        params![projection_id],
                    )?;
                    check_guard(projection_revision, RevisionGuard::Expected(expected))?;
                    let stream_head = read_revision(
                        connection,
                        "SELECT head FROM stream_heads WHERE stream_id = ?1",
                        params![stream.as_str()],
                    )?;
                    check_guard(stream_head, RevisionGuard::Expected(expected))
                }
                (None, None, None) => Ok(()),
                _ => Err(CommitBatchError::PayloadConflict),
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
                     WHERE shutdown_id = ?1",
                    params![m.key.shutdown_id],
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
                     WHERE shutdown_id = ?1",
                    params![m.key.shutdown_id],
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
                 WHERE shutdown_id = ?1 AND ordinal = ?2",
                params![m.key.shutdown_id, m.ordinal],
            )?;
            check_guard(existing, m.expected)
        }
        LocalStateMutation::ShutdownRecoverySnapshot(m) => {
            let details_state: Option<String> = connection
                .query_row(
                    "SELECT details_state FROM shutdown_plans
                     WHERE shutdown_id = ?1",
                    params![m.key.shutdown_id],
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
                     WHERE shutdown_id = ?1 AND partition = ?2 AND ordinal = ?3",
                    params![m.key.shutdown_id, m.partition.label(), m.ordinal],
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
        LocalStateMutation::ShutdownDetailsCompaction(m) => {
            let existing: Option<(String, String, i64)> = connection
                .query_row(
                    "SELECT phase, details_state, revision FROM shutdown_plans
                     WHERE shutdown_id = ?1",
                    params![m.key.shutdown_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(|error| storage_unavailable(&error))?;
            match existing {
                Some((phase, details, revision))
                    if matches!(phase.as_str(), "completed" | "failed" | "cancelled")
                        && details == "available"
                        && revision == m.expected.value()
                        && m.revision
                            == m.expected.next().ok_or_else(|| {
                                corrupt("shutdown details compaction revision overflow")
                            })? =>
                {
                    Ok(())
                }
                Some((_, details, revision))
                    if details == "compacted" && revision == m.revision.value() =>
                {
                    Ok(())
                }
                Some((_, _, revision)) => Err(conflict(revision)),
                None => Err(CommitBatchError::PayloadConflict),
            }
        }
        LocalStateMutation::ShutdownLatestPointer(m) => validate_shutdown_pointer(connection, m),
        LocalStateMutation::WorkflowExecutionProjection(m) => {
            validate_workflow_execution_projection(connection, m)
        }
        LocalStateMutation::WorkflowExecutionNodeProjection(m) => {
            validate_workflow_execution_node_projection(connection, m)
        }
    }
}

fn valid_projection_revision(expected: RevisionGuard, revision: Revision) -> bool {
    guard_inserts_revision_zero(expected, revision) || guard_advances_existing(expected, revision)
}

fn validate_workflow_execution_projection(
    connection: &Connection,
    mutation: &WorkflowExecutionProjectionMutation,
) -> Result<(), CommitBatchError> {
    if !valid_projection_revision(mutation.expected, mutation.revision) {
        return Err(CommitBatchError::PayloadConflict);
    }
    let execution_id = match &mutation.projection {
        crate::domain::local_event::WorkflowExecutionProjectionRecord::Present(execution) => {
            indexed_execution_row(execution).map_err(|_| CommitBatchError::PayloadConflict)?;
            execution.execution_id.as_str()
        }
        crate::domain::local_event::WorkflowExecutionProjectionRecord::Deleted { execution_id } => {
            if execution_id.is_empty() {
                return Err(CommitBatchError::PayloadConflict);
            }
            execution_id
        }
    };
    let existing = read_revision(
        connection,
        "SELECT source_revision FROM workflow_executions WHERE execution_id = ?1",
        params![execution_id],
    )?;
    check_guard(existing, mutation.expected)
}

fn validate_workflow_execution_node_projection(
    connection: &Connection,
    mutation: &WorkflowExecutionNodeProjectionMutation,
) -> Result<(), CommitBatchError> {
    if mutation.execution_id.is_empty()
        || !valid_projection_revision(mutation.expected, mutation.revision)
    {
        return Err(CommitBatchError::PayloadConflict);
    }
    let mut node_ids = HashSet::new();
    for node in &mutation.nodes {
        if !node_ids.insert(node.id.as_str()) {
            return Err(CommitBatchError::PayloadConflict);
        }
        indexed_execution_node_row(&mutation.execution_id, node)
            .map_err(|_| CommitBatchError::PayloadConflict)?;
    }
    if !mutation.nodes.is_empty() {
        let restored = WorkspaceTree::restore("/workflow-execution", mutation.nodes.clone())
            .map_err(|_| CommitBatchError::PayloadConflict)?;
        if restored
            .nodes()
            .iter()
            .any(|node| node.execution_id.as_deref() != Some(mutation.execution_id.as_str()))
        {
            return Err(CommitBatchError::PayloadConflict);
        }
    }
    let existing = read_revision(
        connection,
        "SELECT source_revision FROM workflow_executions WHERE execution_id = ?1",
        params![mutation.execution_id],
    )?;
    check_guard(existing, mutation.expected)
}

fn validate_operation_binding(
    connection: &Connection,
    m: &OperationBindingMutation,
) -> Result<(), CommitBatchError> {
    let existing: Option<(String, Vec<u8>)> = connection
        .query_row(
            "SELECT operation_id, binding_hmac FROM operation_bindings
             WHERE principal = ?1 AND installation_id = ?2 AND kind = ?3 AND caller_request_id = ?4",
            params![
                m.key.principal,
                m.key.installation_id,
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
             WHERE principal = ?1 AND installation_id = ?2 AND kind = ?3 AND caller_request_id = ?4",
            params![
                m.key.principal,
                m.key.installation_id,
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
    let (shutdown_id, revision): (Option<String>, i64) = connection
        .query_row(
            "SELECT current_shutdown_id, shutdown_pointer_revision
             FROM store_metadata WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| storage_unavailable(&error))?;
    let current = shutdown_id;
    let expected = m.expected.as_ref().map(|key| key.shutdown_id.clone());
    if current != expected {
        return Err(conflict(revision));
    }
    Ok(())
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
        LocalStateMutation::OperationRecord(m) => apply_operation_record(connection, commit_id, m),
        LocalStateMutation::SessionProjection(m) => {
            apply_session_projection(connection, commit_id, m)
        }
        LocalStateMutation::SessionProjectionRemoval(m) => run(connection.execute(
            "DELETE FROM session_projection WHERE session_id = ?1",
            params![m.session_id],
        )),
        LocalStateMutation::AgentSessionRemoval(m) => {
            run(connection.execute(
                "DELETE FROM events WHERE stream_id = ?1 AND stream_sequence < ?2",
                params![
                    m.agent_session_stream.as_str(),
                    m.retained_tombstone_sequence.value()
                ],
            ))?;
            if let (Some(projection_id), Some(stream), Some(_)) = (
                &m.ownership_projection_id,
                &m.ownership_stream,
                m.ownership_expected,
            ) {
                run(connection.execute(
                    "DELETE FROM session_projection WHERE session_id = ?1",
                    params![projection_id],
                ))?;
                run(connection.execute(
                    "DELETE FROM events WHERE stream_id = ?1",
                    params![stream.as_str()],
                ))?;
                run(connection.execute(
                    "DELETE FROM stream_heads WHERE stream_id = ?1",
                    params![stream.as_str()],
                ))?;
            }
            Ok(())
        }
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
        LocalStateMutation::ShutdownDetailsCompaction(m) => {
            apply_shutdown_details_compaction(connection, commit_id, m)
        }
        LocalStateMutation::ShutdownLatestPointer(m) => apply_shutdown_pointer(connection, m),
        LocalStateMutation::WorkflowExecutionProjection(m) => {
            apply_workflow_execution_projection(connection, commit_id, m)
        }
        LocalStateMutation::WorkflowExecutionNodeProjection(m) => {
            apply_workflow_execution_node_projection(connection, commit_id, m)
        }
    }
}

fn apply_workflow_execution_projection(
    connection: &Connection,
    commit_id: &str,
    mutation: &WorkflowExecutionProjectionMutation,
) -> Result<(), CommitBatchError> {
    match &mutation.projection {
        crate::domain::local_event::WorkflowExecutionProjectionRecord::Present(execution) => {
            let row =
                indexed_execution_row(execution).map_err(|_| CommitBatchError::PayloadConflict)?;
            run(upsert_indexed_execution_row(
                connection,
                commit_id,
                mutation.revision.value(),
                row,
            ))
        }
        crate::domain::local_event::WorkflowExecutionProjectionRecord::Deleted { execution_id } => {
            run(connection.execute(
                "DELETE FROM workflow_executions WHERE execution_id = ?1",
                params![execution_id],
            ))
        }
    }
}

fn apply_workflow_execution_node_projection(
    connection: &Connection,
    commit_id: &str,
    mutation: &WorkflowExecutionNodeProjectionMutation,
) -> Result<(), CommitBatchError> {
    run(connection.execute(
        "DELETE FROM workflow_execution_nodes WHERE execution_id = ?1",
        params![mutation.execution_id],
    ))?;
    for node in &mutation.nodes {
        let row = indexed_execution_node_row(&mutation.execution_id, node)
            .map_err(|_| CommitBatchError::PayloadConflict)?;
        run(insert_indexed_execution_node_row(
            connection,
            commit_id,
            mutation.revision.value(),
            row,
        ))?;
    }
    Ok(())
}

pub(super) fn upsert_indexed_execution_row(
    connection: &Connection,
    commit_id: &str,
    source_revision: i64,
    row: IndexedExecutionRow,
) -> Result<usize, rusqlite::Error> {
    connection.execute(
        "INSERT INTO workflow_executions
            (execution_id, workspace_identity, status, list_kind, sort_at_bits,
             record_schema, record, source_revision, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (execution_id) DO UPDATE SET
            workspace_identity = excluded.workspace_identity,
            status = excluded.status,
            list_kind = excluded.list_kind,
            sort_at_bits = excluded.sort_at_bits,
            record_schema = excluded.record_schema,
            record = excluded.record,
            source_revision = excluded.source_revision,
            commit_id = excluded.commit_id",
        params![
            row.execution_id,
            row.workspace_identity,
            row.status,
            row.list_kind,
            row.sort_at_bits,
            EXECUTION_RECORD_SCHEMA,
            row.record,
            source_revision,
            commit_id
        ],
    )
}

pub(super) fn insert_indexed_execution_node_row(
    connection: &Connection,
    commit_id: &str,
    source_revision: i64,
    row: IndexedExecutionNodeRow,
) -> Result<usize, rusqlite::Error> {
    connection.execute(
        "INSERT INTO workflow_execution_nodes
            (execution_id, node_id, parent_id, sibling_order, session_id,
             node_execution_id, record_schema, tree_record, detail_record,
             source_revision, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            row.execution_id,
            row.node_id,
            row.parent_id,
            row.sibling_order,
            row.session_id,
            row.node_execution_id,
            EXECUTION_NODE_RECORD_SCHEMA,
            row.tree_record,
            row.detail_record,
            source_revision,
            commit_id
        ],
    )
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
            (principal, installation_id, kind, caller_request_id, operation_id, binding_hmac, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (principal, installation_id, kind, caller_request_id) DO NOTHING",
        params![
            m.key.principal,
            m.key.installation_id,
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
            (principal, installation_id, kind, caller_request_id, scope_id, command_hash,
             sealed_command, resolution, revision, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (principal, installation_id, kind, caller_request_id) DO UPDATE SET
            scope_id = excluded.scope_id,
            sealed_command = excluded.sealed_command,
            resolution = excluded.resolution,
            revision = excluded.revision,
            commit_id = excluded.commit_id",
        params![
            m.key.principal,
            m.key.installation_id,
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
    let public = indexed_session_public_columns(&m.projection)
        .map_err(|_| CommitBatchError::PayloadConflict)?;
    run(connection.execute(
        "INSERT INTO session_projection
            (session_id, projection, revision, commit_id, workspace_identity,
             public_list_kind, public_sort_key_bits, public_summary)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (session_id) DO UPDATE SET
            projection = excluded.projection,
            revision = excluded.revision,
            commit_id = excluded.commit_id,
            workspace_identity = excluded.workspace_identity,
            public_list_kind = excluded.public_list_kind,
            public_sort_key_bits = excluded.public_sort_key_bits,
            public_summary = excluded.public_summary",
        params![
            m.session_id,
            encoded,
            m.revision.value(),
            commit_id,
            public.workspace_identity,
            public.list_kind,
            public.sort_key_bits,
            public.summary
        ],
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
                 shutdown_id, commit_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                pending.ordered_key,
                m.obligation_id,
                pending.owner,
                pending.partition.label(),
                pending
                    .shutdown_plan
                    .as_ref()
                    .map(|plan| plan.shutdown_id.clone()),
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
            "SELECT summary FROM shutdown_plans WHERE shutdown_id = ?1",
            params![m.key.shutdown_id],
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
            (shutdown_id, phase, summary, details_state, revision, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (shutdown_id) DO UPDATE SET
            phase = excluded.phase,
            summary = excluded.summary,
            details_state = excluded.details_state,
            revision = excluded.revision,
            commit_id = excluded.commit_id",
        params![
            m.key.shutdown_id,
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
            "SELECT detail FROM shutdown_targets WHERE shutdown_id = ?1 AND ordinal = ?2",
            params![m.key.shutdown_id, m.ordinal],
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
        "INSERT INTO shutdown_targets (shutdown_id, ordinal, detail, revision, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (shutdown_id, ordinal) DO UPDATE SET
            detail = excluded.detail,
            revision = excluded.revision,
            commit_id = excluded.commit_id",
        params![
            m.key.shutdown_id,
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
            (shutdown_id, partition, ordinal, detail, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (shutdown_id, ordinal) DO NOTHING",
        params![
            m.key.shutdown_id,
            m.partition.label(),
            m.ordinal,
            detail,
            commit_id
        ],
    ))
}

fn apply_shutdown_details_compaction(
    connection: &Connection,
    commit_id: &str,
    m: &ShutdownDetailsCompactionMutation,
) -> Result<(), CommitBatchError> {
    run(connection.execute(
        "DELETE FROM shutdown_targets WHERE shutdown_id = ?1",
        params![m.key.shutdown_id],
    ))?;
    run(connection.execute(
        "DELETE FROM shutdown_recovery_snapshots WHERE shutdown_id = ?1",
        params![m.key.shutdown_id],
    ))?;
    run(connection.execute(
        "UPDATE shutdown_plans
         SET details_state = 'compacted', revision = ?2, commit_id = ?3
         WHERE shutdown_id = ?1",
        params![m.key.shutdown_id, m.revision.value(), commit_id],
    ))
}

fn apply_shutdown_pointer(
    connection: &Connection,
    m: &ShutdownLatestPointerMutation,
) -> Result<(), CommitBatchError> {
    run(connection.execute(
        "UPDATE store_metadata SET
            current_shutdown_id = ?1,
            shutdown_pointer_revision = shutdown_pointer_revision + 1
         WHERE id = 1",
        params![m.new.as_ref().map(|key| key.shutdown_id.clone())],
    ))
}
