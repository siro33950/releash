//! Rules deciding which commits the store still admits while an application
//! shutdown plan owns admission.
//!
//! The store fetches and decodes the durable rows a rule needs and passes
//! them in as facts; every judgment on those facts lives here.

use super::{
    workflow_shutdown, ApplicationDomainEvent, ApplicationShutdownPhase, CommitOperationKind,
    LocalAtomicBatch, LocalDomainEvent, LocalStateMutation, ObligationMutation, ObligationRecord,
    ObligationStateRecord, OperationKind, OperationReceiptRecord, OperationStatusValue,
    RecoveryAttemptRecord, RecoveryResultRecord, RevisionGuard, SessionProjectionRecord,
    ShutdownPlanKey, ShutdownPlanRecord, ShutdownTargetRecord, ShutdownTargetStateRecord, StreamId,
    WorkflowExecutionProjectionRecord,
};

/// Which commit lanes a closed shutdown admission rejects outright. The
/// exempt lanes must each have proven their binding to the current plan.
pub fn closed_admission_rejects(
    kind: CommitOperationKind,
    shutdown_target_recovery: bool,
    internal_progress: bool,
    application_quit_progress: bool,
) -> bool {
    matches!(
        kind,
        CommitOperationKind::UserMutation
            | CommitOperationKind::Recovery
            | CommitOperationKind::Workflow
            | CommitOperationKind::Projection
            | CommitOperationKind::ApplicationQuit
    ) && !shutdown_target_recovery
        && !internal_progress
        && !application_quit_progress
}

pub fn operation_progress_is_structurally_valid(mutations: &[LocalStateMutation]) -> bool {
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
pub fn internal_progress_is_anchored_to_existing_owner(batch: &LocalAtomicBatch) -> bool {
    let mutations = &batch.state_mutations;
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
                if !operation.expected.advances_to(operation.revision) {
                    return false;
                }
                advances_existing_owner = true;
            }
            LocalStateMutation::Obligation(obligation) => {
                if !obligation.expected.advances_to(obligation.revision) {
                    return false;
                }
                advances_existing_owner = true;
            }
            LocalStateMutation::RecoveryAction(action) => {
                if !action.expected.advances_to(action.revision) {
                    return false;
                }
                advances_existing_owner = true;
            }
            LocalStateMutation::SessionProjection(session) => {
                if session.expected.advances_to(session.revision) {
                    advances_existing_owner = true;
                } else if !session.expected.inserts_zero(session.revision) {
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
                if !workflow.expected.advances_to(workflow.revision)
                    && !workflow.expected.inserts_zero(workflow.revision)
                {
                    return false;
                }
            }
            LocalStateMutation::WorkflowExecutionNodeProjection(nodes) => {
                if !nodes.expected.advances_to(nodes.revision)
                    && !nodes.expected.inserts_zero(nodes.revision)
                {
                    return false;
                }
            }
        }
    }
    if !advances_existing_owner {
        return false;
    }
    match batch.idempotency.operation_kind {
        CommitOperationKind::Projection => false,
        CommitOperationKind::Workflow => workflow_progress_has_one_execution_scope(batch),
        CommitOperationKind::ApplicationQuit
        | CommitOperationKind::Recovery
        | CommitOperationKind::UserMutation
        | CommitOperationKind::OperationProgress => false,
    }
}

fn workflow_progress_has_one_execution_scope(batch: &LocalAtomicBatch) -> bool {
    let mut execution_id = None;
    for mutation in &batch.state_mutations {
        if let LocalStateMutation::Obligation(ObligationMutation {
            record: ObligationRecord::WorkflowExecution { execution },
            expected,
            revision,
            ..
        }) = mutation
        {
            if !expected.advances_to(*revision)
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
    if batch
        .expected_heads
        .iter()
        .any(|head| head.stream_id != expected_stream)
        || batch.events.iter().any(|event| {
            event.stream_id != expected_stream
                || !matches!(
                    &event.event,
                    LocalDomainEvent::Workflow(workflow)
                        if workflow.execution_id() == execution_id
                )
        })
    {
        return false;
    }
    batch.state_mutations.iter().all(|mutation| match mutation {
        LocalStateMutation::Obligation(ObligationMutation {
            record: ObligationRecord::WorkflowExecution { execution },
            ..
        }) => execution.execution_id == execution_id,
        LocalStateMutation::SessionProjection(projection) => match &projection.projection {
            SessionProjectionRecord::WorkflowExecution(
                WorkflowExecutionProjectionRecord::Present(execution),
            ) => {
                projection.session_id == format!("workflow:{execution_id}")
                    && execution.execution_id == execution_id
            }
            SessionProjectionRecord::WorkflowExecution(
                WorkflowExecutionProjectionRecord::Deleted {
                    execution_id: deleted,
                },
            ) => {
                projection.session_id == format!("workflow:{execution_id}")
                    && deleted == &execution_id
            }
            SessionProjectionRecord::WorkflowWorktreeOwner(owner) => {
                owner.execution_id == execution_id
            }
            SessionProjectionRecord::AgentSession(_)
            | SessionProjectionRecord::ProviderSessionOwnership(_)
            | SessionProjectionRecord::ProviderHookHealth(_) => false,
        },
        LocalStateMutation::WorkflowExecutionProjection(projection) => {
            batch.state_mutations.iter().any(|candidate| {
                matches!(
                    candidate,
                    LocalStateMutation::SessionProjection(source)
                        if source.session_id == format!("workflow:{execution_id}")
                            && source.expected == projection.expected
                            && source.revision == projection.revision
                )
            }) && match &projection.projection {
                WorkflowExecutionProjectionRecord::Present(execution) => {
                    execution.execution_id == execution_id
                }
                WorkflowExecutionProjectionRecord::Deleted {
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
                && batch.state_mutations.iter().any(|candidate| {
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

/// Durable rows the store fetched and decoded for one ApplicationQuit-kind
/// batch under closed admission.
pub struct QuitProgressFacts<'a> {
    pub plan_id: &'a str,
    pub plan_summary: &'a ShutdownPlanRecord,
    /// Canonical obligation id derived from the batch obligation's effect
    /// identity (an identity-authority computation owned by the store).
    pub workflow_shutdown_obligation_id: Option<&'a str>,
    /// Every target row of the current plan: (detail, row revision). Fetched
    /// when the batch is a single workflow-shutdown obligation commit.
    pub plan_targets: &'a [(ShutdownTargetRecord, i64)],
    /// The stored row at the ordinal of the batch's ShutdownTarget mutation:
    /// (detail, row revision).
    pub existing_target: Option<&'a (ShutdownTargetRecord, i64)>,
}

pub fn application_quit_progress_is_bound_to_current_plan(
    batch: &LocalAtomicBatch,
    facts: &QuitProgressFacts<'_>,
) -> bool {
    let operation_id = facts.plan_summary.operation_id.as_str();
    if operation_id.is_empty() || batch.state_mutations.is_empty() {
        return false;
    }

    // A caller joining the already accepted flight may add exactly one
    // immutable binding to the current operation. It cannot smuggle any
    // state or stream participant into that join commit.
    if let [LocalStateMutation::OperationBinding(binding)] = batch.state_mutations.as_slice() {
        return batch.expected_heads.is_empty()
            && batch.events.is_empty()
            && binding.key.kind == OperationKind::ApplicationQuit
            && binding.key.installation_id == batch.idempotency.installation_id
            && !binding.key.principal.is_empty()
            && !binding.key.caller_request_id.is_empty()
            && binding.operation_id == operation_id
            && batch.idempotency.idempotency_key
                == format!("{operation_id}.join.{}", binding.key.caller_request_id);
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
            return false;
        };
        let mut target_matches = false;
        for (detail, revision) in facts.plan_targets {
            if workflow_shutdown::obligation_anchor_matches(
                detail,
                *revision,
                operation_id,
                effect_identity,
                execution_id,
                *owner_revision,
            ) {
                if target_matches {
                    return false;
                }
                target_matches = true;
            }
        }
        let Some(expected_obligation_id) = facts.workflow_shutdown_obligation_id else {
            return false;
        };
        let guard_matches = workflow_shutdown::obligation_guard_matches(
            obligation,
            *state,
            effect_identity,
            execution_id,
        );
        return target_matches
            && batch.expected_heads.is_empty()
            && batch.events.is_empty()
            && stored_operation_id == operation_id
            && obligation.obligation_id == expected_obligation_id
            && batch.idempotency.idempotency_key
                == format!("{expected_obligation_id}.{}", obligation.revision.value())
            && guard_matches;
    }

    let current_plan = ShutdownPlanKey {
        shutdown_id: facts.plan_id.to_string(),
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
                    || !operation.expected.advances_to(operation.revision)
                {
                    return false;
                }
                operation_status = Some(&operation.latest_status.value);
            }
            LocalStateMutation::ShutdownPlan(plan) => {
                if saw_plan
                    || plan.key != current_plan
                    || plan.summary.operation_id != operation_id
                    || !plan.expected.advances_to(plan.revision)
                {
                    return false;
                }
                saw_plan = true;
                plan_phase = Some(plan.phase);
            }
            LocalStateMutation::ShutdownTarget(target) => {
                if target_transition.replace(target).is_some()
                    || target.key != current_plan
                    || !target.expected.advances_to(target.revision)
                {
                    return false;
                }
                let Some((existing, revision)) = facts.existing_target else {
                    return false;
                };
                let RevisionGuard::Expected(expected) = target.expected else {
                    return false;
                };
                if expected.value() != *revision {
                    return false;
                }
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
                ) = (existing, &target.detail)
                else {
                    return false;
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
                    return false;
                }
            }
            LocalStateMutation::ShutdownLatestPointer(pointer) => {
                if saw_pointer
                    || pointer.expected.as_ref() != Some(&current_plan)
                    || pointer.new.is_some()
                {
                    return false;
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
            | LocalStateMutation::ShutdownDetailsCompaction(_) => return false,
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
            return false;
        }
        return true;
    }
    let Some(operation_revision) = operation_revision else {
        return false;
    };
    if !saw_plan
        || batch.expected_heads.len() != 1
        || batch.expected_heads[0].stream_id != StreamId::application()
        || batch.events.len() != 1
    {
        return false;
    }
    let event = &batch.events[0];
    let event_matches = event.stream_id == StreamId::application()
        && matches!(
            &event.event,
            LocalDomainEvent::Application(
                ApplicationDomainEvent::ShutdownPhaseAdvanced {
                    shutdown_id,
                    phase,
                    ..
                }
            ) if shutdown_id == facts.plan_id
                && Some(*phase) == plan_phase
        );
    let activation = batch.idempotency.idempotency_key == format!("{operation_id}.activate")
        && operation_status == Some(&OperationStatusValue::Activated)
        && plan_phase == Some(ApplicationShutdownPhase::Activated)
        && !saw_pointer
        && batch.state_mutations.len() == 2;
    let finish_identity = batch.idempotency.idempotency_key
        == format!("{operation_id}.finish.{}", operation_revision.value());
    let finish = finish_identity
        && match (operation_status, plan_phase) {
            (Some(OperationStatusValue::Completed), Some(ApplicationShutdownPhase::Completed)) => {
                saw_pointer && batch.state_mutations.len() == 3
            }
            (
                Some(OperationStatusValue::FailedBeforeActivation { .. }),
                Some(ApplicationShutdownPhase::Failed),
            )
            | (
                Some(OperationStatusValue::ReconciliationRequired { .. }),
                Some(ApplicationShutdownPhase::ReconciliationRequired),
            ) => !saw_pointer && batch.state_mutations.len() == 2,
            _ => false,
        };
    event_matches && (activation || finish)
}

/// The stored recovery action row at the batch action's id.
pub struct ExistingRecoveryAction<'a> {
    pub attempt: &'a RecoveryAttemptRecord,
    pub has_completed_result: bool,
    pub revision: i64,
}

/// Durable rows the store fetched and decoded for one Recovery-kind batch
/// under closed admission.
pub struct ShutdownRecoveryFacts<'a> {
    pub plan_id: &'a str,
    pub plan_summary: &'a ShutdownPlanRecord,
    /// The stored row at the batch target's (plan, ordinal): (detail, row
    /// revision).
    pub existing_target: Option<&'a (ShutdownTargetRecord, i64)>,
    /// Canonical target key of the existing row's (kind, target id), an
    /// identity-authority computation owned by the store.
    pub existing_target_key: Option<&'a str>,
    /// SHA-256 of the existing row's effect identity, computed by the store.
    pub existing_effect_identity_sha256: Option<[u8; 32]>,
    pub existing_action: Option<ExistingRecoveryAction<'a>>,
    /// The batch's completed recovery result references this exact target,
    /// validated against the canonical result payload by the store codec.
    pub completed_result_targets_target: bool,
}

pub fn shutdown_target_recovery_is_bound_to_current_plan(
    mutations: &[LocalStateMutation],
    facts: &ShutdownRecoveryFacts<'_>,
) -> bool {
    let mut recovery_action = None;
    let mut shutdown_target = None;
    let mut closure_operation = None;
    let mut closure_plan = None;
    let mut closure_pointer = None;
    for mutation in mutations {
        match mutation {
            LocalStateMutation::RecoveryAction(action) => {
                if recovery_action.replace(action).is_some() {
                    return false;
                }
            }
            LocalStateMutation::ShutdownTarget(target) => {
                if shutdown_target.replace(target).is_some() {
                    return false;
                }
            }
            LocalStateMutation::OperationRecord(operation)
                if operation.kind == OperationKind::ApplicationQuit =>
            {
                if closure_operation.replace(operation).is_some() {
                    return false;
                }
            }
            LocalStateMutation::ShutdownPlan(plan) => {
                if closure_plan.replace(plan).is_some() {
                    return false;
                }
            }
            LocalStateMutation::ShutdownLatestPointer(pointer) => {
                if closure_pointer.replace(pointer).is_some() {
                    return false;
                }
            }
            _ => return false,
        }
    }
    let (Some(action), Some(target)) = (recovery_action, shutdown_target) else {
        return false;
    };
    let has_plan_closure = match (closure_operation, closure_plan, closure_pointer) {
        (None, None, None) => false,
        (Some(operation), Some(plan), Some(pointer))
            if operation.operation_id == facts.plan_summary.operation_id
                && matches!(operation.expected, RevisionGuard::Expected(_))
                && plan.key.shutdown_id == facts.plan_id
                && plan.phase == ApplicationShutdownPhase::Completed
                && plan.summary.operation_id == facts.plan_summary.operation_id
                && plan.summary.intent == facts.plan_summary.intent
                && matches!(plan.expected, RevisionGuard::Expected(_))
                && pointer
                    .expected
                    .as_ref()
                    .is_some_and(|key| key.shutdown_id == facts.plan_id)
                && pointer.new.is_none() =>
        {
            true
        }
        _ => return false,
    };
    if target.key.shutdown_id != facts.plan_id || target.ordinal < 0 {
        return false;
    }
    let RevisionGuard::Expected(expected_target_revision) = target.expected else {
        // Shutdown recovery may only mutate a member of the plan's fixed
        // target set. In particular, Recovery is never an insert path.
        return false;
    };
    let Some((existing_target, existing_target_revision)) = facts.existing_target else {
        return false;
    };
    if expected_target_revision.value() != *existing_target_revision
        || target.revision.value()
            != match existing_target_revision.checked_add(1) {
                Some(next) => next,
                None => return false,
            }
    {
        return false;
    }
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
    ) = (existing_target, &target.detail)
    else {
        return false;
    };
    if target_id != existing_target_id
        || kind != existing_kind
        || effect_identity != existing_effect_identity
        || owner_operation_id != existing_owner_operation_id
    {
        // A recovery transition may change target state/failure, but never
        // the executor-owned identity at the admitted ordinal.
        return false;
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
        "shutdown-target:{}:{}:{target_key}",
        facts.plan_id, target.ordinal
    );
    if plan.shutdown_id != facts.plan_id
        || *ordinal != target.ordinal
        || facts.existing_target_key != Some(target_key.as_str())
        || resource_ref != &expected_resource_ref
        || target_recovery.action_id != action.action_id
        || target_recovery.origin_revision != *origin_revision
        || target_recovery.action != *attempted_action
        || target_recovery.state != *attempt_state
        || facts.existing_effect_identity_sha256 != Some(*effect_identity_sha256)
        || *intent != facts.plan_summary.intent
        || attempt_failure.is_some()
    {
        return false;
    }

    match action.expected {
        RevisionGuard::Absent => {
            action.revision.value() == 0
                && action.completed.is_none()
                && *origin_revision == *existing_target_revision as u64
                && *attempt_state == ObligationStateRecord::EffectReserved
                && *state == *existing_state
                && *failure == *existing_failure
        }
        RevisionGuard::Expected(expected_action_revision) => {
            let Some(existing_action) = facts.existing_action.as_ref() else {
                return false;
            };
            if expected_action_revision.value() != existing_action.revision
                || action.revision.value()
                    != match existing_action.revision.checked_add(1) {
                        Some(next) => next,
                        None => return false,
                    }
                || existing_action.has_completed_result
            {
                return false;
            }
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
            } = existing_action.attempt;
            let Some(existing_target_recovery) = existing_target_recovery else {
                return false;
            };
            if existing_resource_ref != resource_ref
                || existing_plan != plan
                || existing_ordinal != ordinal
                || existing_target_key != target_key
                || existing_origin_revision != origin_revision
                || existing_action_kind != attempted_action
                || existing_effect_identity_sha256 != effect_identity_sha256
                || existing_intent != intent
                || *existing_attempt_state != ObligationStateRecord::EffectReserved
                || existing_attempt_failure.is_some()
                || existing_target_recovery.action_id != action.action_id
                || existing_target_recovery.origin_revision != *origin_revision
                || existing_target_recovery.action != *attempted_action
                || existing_target_recovery.state != ObligationStateRecord::EffectReserved
                || *attempt_state != ObligationStateRecord::Completed
                || target_recovery.state != ObligationStateRecord::Completed
            {
                return false;
            }
            let Some(RecoveryResultRecord::Action(completed)) = action.completed.as_ref() else {
                return false;
            };
            (!has_plan_closure || *state == ShutdownTargetStateRecord::Completed)
                && completed.resource_revision == target.revision.value() as u64
                && facts.completed_result_targets_target
        }
    }
}
