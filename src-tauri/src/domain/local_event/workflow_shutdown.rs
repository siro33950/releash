//! Rules for the workflow shutdown quiesce effect obligation.
//!
//! One durable obligation record exists per effect identity. The record's
//! owner revision names the claimant that reserved the effect, but it is not
//! part of the effect's identity: a later durable claimant of the same effect
//! may adopt a completed result or take over a stale reservation.

use super::{
    ObligationMutation, ObligationRecord, ObligationStateRecord, ObligationView, PendingPartition,
    Revision, RevisionGuard, ShutdownTargetKindRecord, ShutdownTargetRecord,
    ShutdownTargetRecoveryRecord,
};

/// Durable resolution of one quiesce effect readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowShutdownEffectResolution {
    /// A durable record proves this effect completed, regardless of which
    /// owner revision completed it.
    Completed,
    /// The effect has no completion record. A bare reservation also resolves
    /// here: it cannot prove completion, and the effect is idempotent, so it
    /// may be re-executed under the caller's owner revision.
    NotStarted,
    /// The stored record does not belong to this effect.
    Unresolved,
}

/// How an executor may reserve the effect before running it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowShutdownReservationStep {
    /// A durable record proves this effect completed; do not run it again.
    AlreadyCompleted,
    /// The caller already holds the reservation; continue from it.
    ContinueOwn { reservation: Revision },
    /// Commit a reservation under the caller's owner revision: fresh when no
    /// record exists, a takeover when a stale owner's reservation remains.
    Reserve {
        expected: RevisionGuard,
        reservation: Revision,
    },
    /// The record cannot be owned by this effect; do not start it.
    Reject,
}

fn record_identity(
    record: &ObligationView,
    operation_id: &str,
    effect_identity: &str,
    execution_id: &str,
) -> Option<(ObligationStateRecord, i64)> {
    let ObligationRecord::WorkflowShutdown {
        operation_id: stored_operation_id,
        effect_identity: stored_effect_identity,
        owner_revision: stored_owner_revision,
        execution_id: stored_execution_id,
        state,
    } = &record.record
    else {
        return None;
    };
    (stored_operation_id == operation_id
        && stored_effect_identity == effect_identity
        && stored_execution_id == execution_id)
        .then_some((*state, *stored_owner_revision))
}

pub fn read_resolution(
    record: Option<&ObligationView>,
    operation_id: &str,
    effect_identity: &str,
    execution_id: &str,
) -> WorkflowShutdownEffectResolution {
    let Some(record) = record else {
        return WorkflowShutdownEffectResolution::NotStarted;
    };
    match record_identity(record, operation_id, effect_identity, execution_id) {
        Some((ObligationStateRecord::Completed, _)) => WorkflowShutdownEffectResolution::Completed,
        Some((ObligationStateRecord::EffectReserved, _)) => {
            WorkflowShutdownEffectResolution::NotStarted
        }
        _ => WorkflowShutdownEffectResolution::Unresolved,
    }
}

pub fn reservation_step(
    record: Option<&ObligationView>,
    operation_id: &str,
    effect_identity: &str,
    execution_id: &str,
    owner_revision: i64,
) -> WorkflowShutdownReservationStep {
    let Some(record) = record else {
        return WorkflowShutdownReservationStep::Reserve {
            expected: RevisionGuard::Absent,
            reservation: Revision::new(0).expect("zero revision"),
        };
    };
    match record_identity(record, operation_id, effect_identity, execution_id) {
        Some((ObligationStateRecord::Completed, _)) => {
            WorkflowShutdownReservationStep::AlreadyCompleted
        }
        Some((ObligationStateRecord::EffectReserved, stored_owner_revision)) => {
            if stored_owner_revision == owner_revision {
                WorkflowShutdownReservationStep::ContinueOwn {
                    reservation: record.revision,
                }
            } else {
                match record.revision.next() {
                    Some(reservation) => WorkflowShutdownReservationStep::Reserve {
                        expected: RevisionGuard::Expected(record.revision),
                        reservation,
                    },
                    None => WorkflowShutdownReservationStep::Reject,
                }
            }
        }
        _ => WorkflowShutdownReservationStep::Reject,
    }
}

/// The durable anchor admitting a workflow shutdown obligation commit under
/// closed admission: the target row at the obligation's owner revision either
/// holds the plan's own effect reservation or a claimed recovery attempt
/// re-owning the same effect.
pub fn obligation_anchor_matches(
    detail: &ShutdownTargetRecord,
    revision: i64,
    operation_id: &str,
    effect_identity: &str,
    execution_id: &str,
    owner_revision: i64,
) -> bool {
    use super::ShutdownTargetStateRecord;
    matches!(
        detail,
        ShutdownTargetRecord::Target {
            target_id,
            kind: ShutdownTargetKindRecord::WorkflowExecution,
            state,
            effect_identity: stored_effect,
            owner_operation_id: Some(owner),
            recovery_action,
            ..
        } if target_id.as_str() == execution_id
            && stored_effect.as_str() == effect_identity
            && owner.as_str() == operation_id
            && revision == owner_revision
            && (*state == ShutdownTargetStateRecord::EffectReserved
                || matches!(
                    recovery_action,
                    Some(ShutdownTargetRecoveryRecord {
                        state: ObligationStateRecord::EffectReserved,
                        ..
                    })
                ))
    )
}

pub fn obligation_guard_matches(
    obligation: &ObligationMutation,
    state: ObligationStateRecord,
    effect_identity: &str,
    execution_id: &str,
) -> bool {
    let reserved_pending_matches = || {
        obligation.pending.as_ref().is_some_and(|pending| {
            pending.owner == execution_id
                && pending.partition == PendingPartition::Owner
                && pending.shutdown_plan.is_none()
                && pending.ordered_key == format!("workflow-shutdown-{effect_identity}")
        })
    };
    match (obligation.expected, state) {
        (RevisionGuard::Absent, ObligationStateRecord::EffectReserved) => {
            obligation.expected.inserts_zero(obligation.revision) && reserved_pending_matches()
        }
        // A claimed retry takes over a stale reservation by advancing the
        // same record under its new owner revision.
        (RevisionGuard::Expected(_), ObligationStateRecord::EffectReserved) => {
            obligation.expected.advances_to(obligation.revision) && reserved_pending_matches()
        }
        (RevisionGuard::Expected(_), ObligationStateRecord::Completed) => {
            obligation.expected.advances_to(obligation.revision) && obligation.pending.is_none()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{PendingIndexEntry, RecoveryActionKind, ShutdownTargetStateRecord};
    use super::*;

    fn record(state: ObligationStateRecord, owner_revision: i64) -> ObligationView {
        ObligationView {
            obligation_id: "workflow-shutdown-record".to_string(),
            record: ObligationRecord::WorkflowShutdown {
                operation_id: "quit-operation".to_string(),
                effect_identity: "quit-operation:0:workflow-1".to_string(),
                owner_revision,
                execution_id: "workflow-1".to_string(),
                state,
            },
            record_sha256: [0; 32],
            pending: None,
            revision: Revision::new(1).unwrap(),
        }
    }

    fn resolution(record: Option<&ObligationView>) -> WorkflowShutdownEffectResolution {
        read_resolution(
            record,
            "quit-operation",
            "quit-operation:0:workflow-1",
            "workflow-1",
        )
    }

    #[test]
    fn read_resolution_adopts_completion_and_reopens_stale_reservation() {
        assert_eq!(
            resolution(Some(&record(ObligationStateRecord::Completed, 7))),
            WorkflowShutdownEffectResolution::Completed
        );
        assert_eq!(
            resolution(Some(&record(ObligationStateRecord::EffectReserved, 7))),
            WorkflowShutdownEffectResolution::NotStarted
        );
        assert_eq!(
            resolution(Some(&record(
                ObligationStateRecord::ReconciliationRequired,
                7
            ))),
            WorkflowShutdownEffectResolution::Unresolved
        );
        assert_eq!(
            resolution(None),
            WorkflowShutdownEffectResolution::NotStarted
        );
    }

    #[test]
    fn read_resolution_is_bound_to_exact_effect_not_owner_revision() {
        let stored = record(ObligationStateRecord::Completed, 7);
        assert_eq!(
            read_resolution(
                Some(&stored),
                "unrelated-terminal-operation",
                "quit-operation:0:workflow-1",
                "workflow-1",
            ),
            WorkflowShutdownEffectResolution::Unresolved
        );
        assert_eq!(
            read_resolution(
                Some(&stored),
                "quit-operation",
                "quit-operation:0:workflow-2",
                "workflow-1",
            ),
            WorkflowShutdownEffectResolution::Unresolved
        );
        assert_eq!(
            read_resolution(
                Some(&stored),
                "quit-operation",
                "quit-operation:0:workflow-1",
                "workflow-2",
            ),
            WorkflowShutdownEffectResolution::Unresolved
        );
    }

    fn step(
        record: Option<&ObligationView>,
        owner_revision: i64,
    ) -> WorkflowShutdownReservationStep {
        reservation_step(
            record,
            "quit-operation",
            "quit-operation:0:workflow-1",
            "workflow-1",
            owner_revision,
        )
    }

    #[test]
    fn reservation_step_continues_own_and_takes_over_stale_owner() {
        assert_eq!(
            step(None, 9),
            WorkflowShutdownReservationStep::Reserve {
                expected: RevisionGuard::Absent,
                reservation: Revision::new(0).unwrap(),
            }
        );
        assert_eq!(
            step(Some(&record(ObligationStateRecord::Completed, 3)), 9),
            WorkflowShutdownReservationStep::AlreadyCompleted
        );
        assert_eq!(
            step(Some(&record(ObligationStateRecord::EffectReserved, 9)), 9),
            WorkflowShutdownReservationStep::ContinueOwn {
                reservation: Revision::new(1).unwrap(),
            }
        );
        assert_eq!(
            step(Some(&record(ObligationStateRecord::EffectReserved, 3)), 9),
            WorkflowShutdownReservationStep::Reserve {
                expected: RevisionGuard::Expected(Revision::new(1).unwrap()),
                reservation: Revision::new(2).unwrap(),
            }
        );
        let mut exhausted = record(ObligationStateRecord::EffectReserved, 3);
        exhausted.revision = Revision::new(i64::MAX).unwrap();
        assert_eq!(
            step(Some(&exhausted), 9),
            WorkflowShutdownReservationStep::Reject
        );
        assert_eq!(
            step(
                Some(&record(ObligationStateRecord::ReconciliationRequired, 9)),
                9
            ),
            WorkflowShutdownReservationStep::Reject
        );
        assert_eq!(
            reservation_step(
                Some(&record(ObligationStateRecord::EffectReserved, 3)),
                "unrelated-terminal-operation",
                "quit-operation:0:workflow-1",
                "workflow-1",
                9,
            ),
            WorkflowShutdownReservationStep::Reject
        );
    }

    fn target(
        state: ShutdownTargetStateRecord,
        recovery_state: Option<ObligationStateRecord>,
    ) -> ShutdownTargetRecord {
        ShutdownTargetRecord::Target {
            target_id: "workflow-1".to_string(),
            kind: ShutdownTargetKindRecord::WorkflowExecution,
            state,
            effect_identity: "quit-operation:0:workflow-1".to_string(),
            owner_operation_id: Some("quit-operation".to_string()),
            failure: None,
            recovery_action: recovery_state.map(|state| ShutdownTargetRecoveryRecord {
                action_id: "action-1".to_string(),
                origin_revision: 4,
                action: RecoveryActionKind::RetrySameEffect,
                state,
            }),
        }
    }

    fn anchor_matches(detail: &ShutdownTargetRecord, revision: i64) -> bool {
        obligation_anchor_matches(
            detail,
            revision,
            "quit-operation",
            "quit-operation:0:workflow-1",
            "workflow-1",
            5,
        )
    }

    #[test]
    fn obligation_anchor_accepts_reservation_and_claimed_recovery() {
        assert!(anchor_matches(
            &target(ShutdownTargetStateRecord::EffectReserved, None),
            5
        ));
        assert!(anchor_matches(
            &target(
                ShutdownTargetStateRecord::ReconciliationRequired,
                Some(ObligationStateRecord::EffectReserved),
            ),
            5,
        ));
        assert!(anchor_matches(
            &target(
                ShutdownTargetStateRecord::Prepared,
                Some(ObligationStateRecord::EffectReserved),
            ),
            5,
        ));
        assert!(!anchor_matches(
            &target(ShutdownTargetStateRecord::ReconciliationRequired, None),
            5
        ));
        assert!(!anchor_matches(
            &target(
                ShutdownTargetStateRecord::ReconciliationRequired,
                Some(ObligationStateRecord::Completed),
            ),
            5,
        ));
        assert!(!anchor_matches(
            &target(ShutdownTargetStateRecord::EffectReserved, None),
            6
        ));
    }

    #[test]
    fn obligation_anchor_is_bound_to_target_identity() {
        let detail = target(ShutdownTargetStateRecord::EffectReserved, None);
        assert!(!obligation_anchor_matches(
            &detail,
            5,
            "unrelated-operation",
            "quit-operation:0:workflow-1",
            "workflow-1",
            5,
        ));
        assert!(!obligation_anchor_matches(
            &detail,
            5,
            "quit-operation",
            "quit-operation:0:workflow-2",
            "workflow-1",
            5,
        ));
        assert!(!obligation_anchor_matches(
            &detail,
            5,
            "quit-operation",
            "quit-operation:0:workflow-1",
            "workflow-2",
            5,
        ));
    }

    fn obligation(
        state: ObligationStateRecord,
        expected: RevisionGuard,
        revision: i64,
        pending: Option<PendingIndexEntry>,
    ) -> ObligationMutation {
        ObligationMutation {
            obligation_id: "workflow-shutdown-abc".to_string(),
            record: ObligationRecord::WorkflowShutdown {
                operation_id: "quit-operation".to_string(),
                effect_identity: "quit-operation:0:workflow-1".to_string(),
                owner_revision: 5,
                execution_id: "workflow-1".to_string(),
                state,
            },
            pending,
            expected,
            revision: Revision::new(revision).unwrap(),
        }
    }

    fn reserved_pending() -> PendingIndexEntry {
        PendingIndexEntry {
            ordered_key: "workflow-shutdown-quit-operation:0:workflow-1".to_string(),
            owner: "workflow-1".to_string(),
            partition: PendingPartition::Owner,
            shutdown_plan: None,
        }
    }

    fn guard_matches(obligation: &ObligationMutation, state: ObligationStateRecord) -> bool {
        obligation_guard_matches(
            obligation,
            state,
            "quit-operation:0:workflow-1",
            "workflow-1",
        )
    }

    #[test]
    fn obligation_guard_admits_reservation_takeover_and_completion() {
        let reservation = obligation(
            ObligationStateRecord::EffectReserved,
            RevisionGuard::Absent,
            0,
            Some(reserved_pending()),
        );
        assert!(guard_matches(
            &reservation,
            ObligationStateRecord::EffectReserved
        ));

        let takeover = obligation(
            ObligationStateRecord::EffectReserved,
            RevisionGuard::Expected(Revision::new(0).unwrap()),
            1,
            Some(reserved_pending()),
        );
        assert!(guard_matches(
            &takeover,
            ObligationStateRecord::EffectReserved
        ));

        let completion = obligation(
            ObligationStateRecord::Completed,
            RevisionGuard::Expected(Revision::new(1).unwrap()),
            2,
            None,
        );
        assert!(guard_matches(&completion, ObligationStateRecord::Completed));
    }

    #[test]
    fn obligation_guard_rejects_malformed_shapes() {
        let skipped_revision = obligation(
            ObligationStateRecord::EffectReserved,
            RevisionGuard::Expected(Revision::new(0).unwrap()),
            2,
            Some(reserved_pending()),
        );
        assert!(!guard_matches(
            &skipped_revision,
            ObligationStateRecord::EffectReserved
        ));

        let reservation_without_pending = obligation(
            ObligationStateRecord::EffectReserved,
            RevisionGuard::Expected(Revision::new(0).unwrap()),
            1,
            None,
        );
        assert!(!guard_matches(
            &reservation_without_pending,
            ObligationStateRecord::EffectReserved
        ));

        let completion_with_pending = obligation(
            ObligationStateRecord::Completed,
            RevisionGuard::Expected(Revision::new(1).unwrap()),
            2,
            Some(reserved_pending()),
        );
        assert!(!guard_matches(
            &completion_with_pending,
            ObligationStateRecord::Completed
        ));

        let inserted_completion = obligation(
            ObligationStateRecord::Completed,
            RevisionGuard::Absent,
            0,
            None,
        );
        assert!(!guard_matches(
            &inserted_completion,
            ObligationStateRecord::Completed
        ));

        let mut foreign_pending = reserved_pending();
        foreign_pending.owner = "workflow-2".to_string();
        let foreign_owner = obligation(
            ObligationStateRecord::EffectReserved,
            RevisionGuard::Expected(Revision::new(0).unwrap()),
            1,
            Some(foreign_pending),
        );
        assert!(!guard_matches(
            &foreign_owner,
            ObligationStateRecord::EffectReserved
        ));
    }
}
