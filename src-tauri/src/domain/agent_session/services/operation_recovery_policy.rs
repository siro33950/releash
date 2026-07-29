use std::time::Duration;

use crate::domain::agent_session::events::{RecoveryActionKind, RecoveryResultClassification};
use crate::domain::local_event::{
    hex_lower, sha256, validate_operation_identity, BackendSessionRecoveryObligationRecord,
    ObligationRecord, ObligationStateRecord, RecoveryResultOutcomeRecord, SendObligationKindRecord,
    SessionLifecycleRecordAction,
};

pub fn next_recovery_retry_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(Duration::from_secs(1))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyProviderEstablishRecovery {
    ContinueTurnExecution,
    RequiresManualResolution,
}

pub fn classify_legacy_provider_establish(
    has_provider_session: bool,
    backend_id: &str,
) -> LegacyProviderEstablishRecovery {
    if has_provider_session || backend_id == "claude" {
        LegacyProviderEstablishRecovery::ContinueTurnExecution
    } else {
        LegacyProviderEstablishRecovery::RequiresManualResolution
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryReadbackTarget {
    Stop {
        effect_identity: String,
        operation_id: String,
        session_id: String,
        turn_id: String,
    },
    SessionClose {
        effect_identity: String,
        operation_id: String,
        session_id: String,
    },
    BackendRecovery {
        effect_identity: String,
        session_id: String,
        recovery_id: String,
    },
    SendTurn {
        effect_identity: String,
        operation_id: String,
        session_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryReadbackRejection {
    Unsupported,
    InvalidOperationIdentity,
    EffectIdentityMismatch,
}

pub fn classify_recovery_readback(
    obligation_id: &str,
    immutable_obligation: &ObligationRecord,
    expected_stop_identity: Option<&str>,
    expected_session_close_identity: Option<&str>,
) -> Result<RecoveryReadbackTarget, RecoveryReadbackRejection> {
    match immutable_obligation.original() {
        ObligationRecord::StopInterrupt {
            operation_id,
            session_id,
            turn_id,
            ..
        } => {
            validate_operation_identity(operation_id)
                .map_err(|_| RecoveryReadbackRejection::InvalidOperationIdentity)?;
            if expected_stop_identity != Some(obligation_id) {
                return Err(RecoveryReadbackRejection::EffectIdentityMismatch);
            }
            Ok(RecoveryReadbackTarget::Stop {
                effect_identity: obligation_id.to_string(),
                operation_id: operation_id.clone(),
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
            })
        }
        ObligationRecord::SessionClose {
            obligation_id: stored_obligation_id,
            operation_id,
            session_id,
            action: SessionLifecycleRecordAction::Close,
            ..
        } => {
            validate_operation_identity(operation_id)
                .map_err(|_| RecoveryReadbackRejection::InvalidOperationIdentity)?;
            if stored_obligation_id != obligation_id
                || expected_session_close_identity != Some(obligation_id)
            {
                return Err(RecoveryReadbackRejection::EffectIdentityMismatch);
            }
            Ok(RecoveryReadbackTarget::SessionClose {
                effect_identity: obligation_id.to_string(),
                operation_id: operation_id.clone(),
                session_id: session_id.clone(),
            })
        }
        ObligationRecord::BackendSessionRecovery {
            session_id,
            recovery_id,
            ..
        } => {
            let expected = format!("backend-recovery:{session_id}:{recovery_id}");
            if obligation_id != expected {
                return Err(RecoveryReadbackRejection::EffectIdentityMismatch);
            }
            Ok(RecoveryReadbackTarget::BackendRecovery {
                effect_identity: expected,
                session_id: session_id.clone(),
                recovery_id: recovery_id.clone(),
            })
        }
        ObligationRecord::Send {
            obligation_id: stored_obligation_id,
            operation_id,
            session_id,
            kind: SendObligationKindRecord::TurnExecution,
            ..
        } => {
            validate_operation_identity(operation_id)
                .map_err(|_| RecoveryReadbackRejection::InvalidOperationIdentity)?;
            let expected = format!("{operation_id}.exec");
            if stored_obligation_id != obligation_id || obligation_id != expected {
                return Err(RecoveryReadbackRejection::EffectIdentityMismatch);
            }
            Ok(RecoveryReadbackTarget::SendTurn {
                effect_identity: expected,
                operation_id: operation_id.clone(),
                session_id: session_id.clone(),
            })
        }
        _ => Err(RecoveryReadbackRejection::Unsupported),
    }
}

pub fn recovery_handoff_matches(
    origin_revision: u64,
    expected_record: &ObligationRecord,
    expected_owner: Option<&str>,
    actual_revision: u64,
    actual_record: &ObligationRecord,
    actual_owner: Option<&str>,
) -> bool {
    actual_revision == origin_revision
        && actual_record == expected_record
        && actual_owner == expected_owner
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryActionDecision {
    ReadAgain,
    UseObservedResult,
    KeepForManualResolution,
    RetryPermissionResponse { operation_id: String },
    CancelConfirmedNoEffect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryActionRejection {
    UnsupportedReadback,
    MissingAuthoritativeObservation,
    PermissionOperationIdentityUnavailable,
    MissingCancellationProof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionResponseRecoveryObservation {
    Completed,
    AwaitingProviderResponse,
    ReconciliationRequired,
}

pub fn classify_permission_response_recovery(
    observation: PermissionResponseRecoveryObservation,
) -> RecoveryResultClassification {
    match observation {
        PermissionResponseRecoveryObservation::Completed => RecoveryResultClassification::Succeeded,
        PermissionResponseRecoveryObservation::AwaitingProviderResponse => {
            RecoveryResultClassification::Pending
        }
        PermissionResponseRecoveryObservation::ReconciliationRequired => {
            RecoveryResultClassification::Ambiguous
        }
    }
}

pub fn recovery_classification_is_allowed(
    action: RecoveryActionKind,
    observation: Option<(RecoveryResultClassification, bool)>,
    classification: RecoveryResultClassification,
) -> bool {
    match action {
        RecoveryActionKind::ReadAgain => matches!(
            classification,
            RecoveryResultClassification::Pending
                | RecoveryResultClassification::Succeeded
                | RecoveryResultClassification::ConfirmedNoEffect
                | RecoveryResultClassification::Ambiguous
        ),
        RecoveryActionKind::RetrySameEffect => matches!(
            classification,
            RecoveryResultClassification::Pending
                | RecoveryResultClassification::Succeeded
                | RecoveryResultClassification::Ambiguous
        ),
        RecoveryActionKind::UseObservedResult => {
            observation.is_some_and(|(observed, _)| observed == classification)
        }
        RecoveryActionKind::CancelIfSafe => {
            classification == RecoveryResultClassification::CancelledBeforeEffect
                && observation == Some((RecoveryResultClassification::ConfirmedNoEffect, true))
        }
        RecoveryActionKind::KeepForManualResolution => {
            classification == RecoveryResultClassification::Unchanged
        }
    }
}

pub fn recovery_result_outcome(
    classification: RecoveryResultClassification,
) -> RecoveryResultOutcomeRecord {
    match classification {
        RecoveryResultClassification::Pending
        | RecoveryResultClassification::ConfirmedNoEffect
        | RecoveryResultClassification::Ambiguous => RecoveryResultOutcomeRecord::Pending,
        RecoveryResultClassification::Succeeded
        | RecoveryResultClassification::CancelledBeforeEffect => {
            RecoveryResultOutcomeRecord::Terminal
        }
        RecoveryResultClassification::Unchanged => RecoveryResultOutcomeRecord::Unchanged,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendRecoveryReservation {
    pub old_provider_session_generation: u64,
    pub reserved_at_bits: u64,
}

pub fn backend_recovery_reservation(
    record: &ObligationRecord,
    expected_session_id: &str,
    expected_recovery_id: &str,
) -> Option<BackendRecoveryReservation> {
    match record.original() {
        ObligationRecord::BackendSessionRecovery {
            session_id,
            recovery_id,
            detail:
                crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                    old_provider_session_generation,
                    reserved_at_bits,
                    ..
                },
            state: ObligationStateRecord::EffectReserved,
        } if session_id == expected_session_id && recovery_id == expected_recovery_id => {
            Some(BackendRecoveryReservation {
                old_provider_session_generation: *old_provider_session_generation,
                reserved_at_bits: *reserved_at_bits,
            })
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BackendRecoveryDurableCompletionFacts<'a> {
    pub session_id: &'a str,
    pub recovery_id: &'a str,
    pub old_provider_session_generation: u64,
    pub reserved_at_bits: u64,
    pub projected_provider_session_generation: u64,
    pub context_reinjection_generation: Option<u64>,
    pub backend_session_id: Option<&'a str>,
    pub publication_recovery_id: Option<&'a str>,
    pub publication_session_id: Option<&'a str>,
    pub completed_at_bits: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRecoveryReadbackCompletion {
    pub old_provider_session_generation: u64,
    pub provider_session_generation: u64,
    pub backend_session_id: String,
    pub completed_at_bits: u64,
    pub publication_message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendRecoveryDurableCompletionDecision {
    NotReady,
    Complete(BackendRecoveryReadbackCompletion),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendRecoveryDurableCompletionRejection {
    ProviderSessionGenerationCapacityExceeded,
    InvalidCompletionTimestamp,
}

pub fn decide_backend_recovery_durable_completion(
    facts: BackendRecoveryDurableCompletionFacts<'_>,
) -> Result<BackendRecoveryDurableCompletionDecision, BackendRecoveryDurableCompletionRejection> {
    let provider_session_generation = facts.old_provider_session_generation.checked_add(1).ok_or(
        BackendRecoveryDurableCompletionRejection::ProviderSessionGenerationCapacityExceeded,
    )?;
    let Some(backend_session_id) = facts.backend_session_id.filter(|value| !value.is_empty())
    else {
        return Ok(BackendRecoveryDurableCompletionDecision::NotReady);
    };
    if facts.projected_provider_session_generation != provider_session_generation
        || facts.context_reinjection_generation != Some(provider_session_generation)
        || facts.publication_recovery_id != Some(facts.recovery_id)
        || facts.publication_session_id != Some(facts.session_id)
    {
        return Ok(BackendRecoveryDurableCompletionDecision::NotReady);
    }

    let reserved_at = f64::from_bits(facts.reserved_at_bits);
    let completed_at = f64::from_bits(facts.completed_at_bits);
    if !reserved_at.is_finite() || !completed_at.is_finite() || completed_at < reserved_at {
        return Err(BackendRecoveryDurableCompletionRejection::InvalidCompletionTimestamp);
    }

    let digest = sha256(
        format!(
            "backend-recovery-readback-publication/v1\0{}\0{}\0{provider_session_generation}\0{backend_session_id}",
            facts.session_id, facts.recovery_id
        )
        .as_bytes(),
    );
    Ok(BackendRecoveryDurableCompletionDecision::Complete(
        BackendRecoveryReadbackCompletion {
            old_provider_session_generation: facts.old_provider_session_generation,
            provider_session_generation,
            backend_session_id: backend_session_id.to_string(),
            completed_at_bits: facts.completed_at_bits,
            publication_message_id: format!("backend-recovery-readback-{}", hex_lower(digest)),
        },
    ))
}

pub fn backend_recovery_effect_identity(session_id: &str, recovery_id: &str) -> String {
    format!("backend-recovery:{session_id}:{recovery_id}")
}

pub fn backend_recovery_effect_identity_matches(
    effect_identity: &str,
    session_id: &str,
    recovery_id: &str,
) -> bool {
    effect_identity == backend_recovery_effect_identity(session_id, recovery_id)
}

pub fn runtime_stop_request_id(session_id: &str, turn_id: u64) -> String {
    let digest = sha256(format!("runtime-stop-request/v1\0{session_id}\0{turn_id}").as_bytes());
    format!("runtime-stop-{}", hex_lower(digest))
}

pub fn stop_readback_obligation_id(session_id: &str, turn_id: &str) -> String {
    let digest = sha256(format!("stop-target-obligation/v1\0{session_id}\0{turn_id}").as_bytes());
    format!("stop-target-{}", hex_lower(digest))
}

pub fn session_close_readback_obligation_id(session_id: &str) -> String {
    let digest = sha256(format!("session-lifecycle-target/v1\0{session_id}").as_bytes());
    format!("session-lifecycle-target-{}", hex_lower(digest))
}

pub fn backend_recovery_obligation_id(session_id: &str, recovery_id: &str) -> String {
    format!("backend-recovery:{session_id}:{recovery_id}")
}

pub fn backend_recovery_provider_observation_id(recovery_id: &str) -> String {
    format!("backend-recovery/v1:{recovery_id}")
}

pub fn backend_recovery_failure_message_id(session_id: &str, recovery_id: &str) -> String {
    let digest =
        sha256(format!("backend-recovery-failure-v1\0{session_id}\0{recovery_id}").as_bytes());
    format!("backend-recovery-failure-{}", hex_lower(digest))
}

pub fn backend_recovery_error_digest(error: &str) -> [u8; 32] {
    sha256(error.as_bytes())
}

pub fn recovery_publication_obligation_id(
    session_id: &str,
    recovery_id: &str,
    message_id: &str,
) -> String {
    let digest = sha256(
        format!("recovery-publication/v1\0{session_id}\0{recovery_id}\0{message_id}").as_bytes(),
    );
    format!("recovery-publication-{}", hex_lower(digest))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendRecoveryReservationDecision {
    Apply,
    AlreadyApplied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendRecoveryReservationRejection {
    Missing,
    IdentityMismatch,
    AlreadyResolved,
    NotPending,
}

pub fn admit_backend_recovery_start(
    current: Option<&ObligationRecord>,
    session_id: &str,
    recovery_id: &str,
) -> Result<BackendRecoveryReservationDecision, BackendRecoveryReservationRejection> {
    let Some(current) = current else {
        return Ok(BackendRecoveryReservationDecision::Apply);
    };
    match current {
        ObligationRecord::BackendSessionRecovery {
            session_id: stored_session_id,
            recovery_id: stored_recovery_id,
            detail: BackendSessionRecoveryObligationRecord::EffectReserved { .. },
            state: ObligationStateRecord::EffectReserved,
        } if stored_session_id == session_id && stored_recovery_id == recovery_id => {
            Ok(BackendRecoveryReservationDecision::AlreadyApplied)
        }
        ObligationRecord::BackendSessionRecovery {
            session_id: stored_session_id,
            recovery_id: stored_recovery_id,
            detail:
                BackendSessionRecoveryObligationRecord::Completed { .. }
                | BackendSessionRecoveryObligationRecord::Failed { .. },
            ..
        } if stored_session_id == session_id && stored_recovery_id == recovery_id => {
            Err(BackendRecoveryReservationRejection::AlreadyResolved)
        }
        _ => Err(BackendRecoveryReservationRejection::IdentityMismatch),
    }
}

pub fn admit_backend_recovery_completion(
    current: Option<&ObligationRecord>,
    session_id: &str,
    recovery_id: &str,
    old_provider_session_generation: u64,
) -> Result<BackendRecoveryReservationDecision, BackendRecoveryReservationRejection> {
    let current = current.ok_or(BackendRecoveryReservationRejection::Missing)?;
    match current {
        ObligationRecord::BackendSessionRecovery {
            session_id: stored_session_id,
            recovery_id: stored_recovery_id,
            detail: BackendSessionRecoveryObligationRecord::Completed { .. },
            state: ObligationStateRecord::Completed,
        } if stored_session_id == session_id && stored_recovery_id == recovery_id => {
            Ok(BackendRecoveryReservationDecision::AlreadyApplied)
        }
        ObligationRecord::BackendSessionRecovery {
            session_id: stored_session_id,
            recovery_id: stored_recovery_id,
            detail:
                BackendSessionRecoveryObligationRecord::EffectReserved {
                    old_provider_session_generation: stored_generation,
                    ..
                },
            state: ObligationStateRecord::EffectReserved,
        } if stored_session_id == session_id
            && stored_recovery_id == recovery_id
            && *stored_generation == old_provider_session_generation =>
        {
            Ok(BackendRecoveryReservationDecision::Apply)
        }
        ObligationRecord::BackendSessionRecovery {
            session_id: stored_session_id,
            recovery_id: stored_recovery_id,
            detail: BackendSessionRecoveryObligationRecord::Failed { .. },
            ..
        } if stored_session_id == session_id && stored_recovery_id == recovery_id => {
            Err(BackendRecoveryReservationRejection::NotPending)
        }
        _ => Err(BackendRecoveryReservationRejection::IdentityMismatch),
    }
}

pub fn admit_backend_recovery_failure(
    current: Option<&ObligationRecord>,
    session_id: &str,
    recovery_id: &str,
) -> Result<BackendRecoveryReservationDecision, BackendRecoveryReservationRejection> {
    let current = current.ok_or(BackendRecoveryReservationRejection::Missing)?;
    match current {
        ObligationRecord::BackendSessionRecovery {
            session_id: stored_session_id,
            recovery_id: stored_recovery_id,
            detail: BackendSessionRecoveryObligationRecord::Failed { .. },
            state: ObligationStateRecord::Failed,
        } if stored_session_id == session_id && stored_recovery_id == recovery_id => {
            Ok(BackendRecoveryReservationDecision::AlreadyApplied)
        }
        ObligationRecord::BackendSessionRecovery {
            session_id: stored_session_id,
            recovery_id: stored_recovery_id,
            detail: BackendSessionRecoveryObligationRecord::EffectReserved { .. },
            state: ObligationStateRecord::EffectReserved,
        } if stored_session_id == session_id && stored_recovery_id == recovery_id => {
            Ok(BackendRecoveryReservationDecision::Apply)
        }
        ObligationRecord::BackendSessionRecovery {
            session_id: stored_session_id,
            recovery_id: stored_recovery_id,
            detail: BackendSessionRecoveryObligationRecord::Completed { .. },
            ..
        } if stored_session_id == session_id && stored_recovery_id == recovery_id => {
            Err(BackendRecoveryReservationRejection::NotPending)
        }
        _ => Err(BackendRecoveryReservationRejection::IdentityMismatch),
    }
}

pub fn decide_recovery_action(
    action: RecoveryActionKind,
    obligation: &ObligationRecord,
    readback_supported: bool,
    observation: Option<(RecoveryResultClassification, bool)>,
) -> Result<RecoveryActionDecision, RecoveryActionRejection> {
    match action {
        RecoveryActionKind::ReadAgain if readback_supported => {
            Ok(RecoveryActionDecision::ReadAgain)
        }
        RecoveryActionKind::ReadAgain => Err(RecoveryActionRejection::UnsupportedReadback),
        RecoveryActionKind::UseObservedResult if observation.is_some() => {
            Ok(RecoveryActionDecision::UseObservedResult)
        }
        RecoveryActionKind::UseObservedResult => {
            Err(RecoveryActionRejection::MissingAuthoritativeObservation)
        }
        RecoveryActionKind::KeepForManualResolution => {
            Ok(RecoveryActionDecision::KeepForManualResolution)
        }
        RecoveryActionKind::RetrySameEffect => {
            let operation_id = match obligation.original() {
                ObligationRecord::PermissionResponse {
                    operation_id,
                    state: ObligationStateRecord::Pending,
                    ..
                } => Some(operation_id.clone()),
                _ => None,
            }
            .ok_or(RecoveryActionRejection::PermissionOperationIdentityUnavailable)?;
            Ok(RecoveryActionDecision::RetryPermissionResponse { operation_id })
        }
        RecoveryActionKind::CancelIfSafe
            if observation == Some((RecoveryResultClassification::ConfirmedNoEffect, true)) =>
        {
            Ok(RecoveryActionDecision::CancelConfirmedNoEffect)
        }
        RecoveryActionKind::CancelIfSafe => Err(RecoveryActionRejection::MissingCancellationProof),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::entities::{PermissionResponse, PermissionResponseDecision};
    use crate::domain::local_event::SendObligationDispositionRecord;

    #[test]
    fn legacy_provider_establishment_classification_is_domain_owned() {
        assert_eq!(
            classify_legacy_provider_establish(false, "claude"),
            LegacyProviderEstablishRecovery::ContinueTurnExecution
        );
        assert_eq!(
            classify_legacy_provider_establish(false, "codex"),
            LegacyProviderEstablishRecovery::RequiresManualResolution
        );
        assert_eq!(
            classify_legacy_provider_establish(true, "codex"),
            LegacyProviderEstablishRecovery::ContinueTurnExecution
        );
    }

    #[test]
    fn send_readback_requires_the_exact_accepted_effect_identity() {
        let record = ObligationRecord::Send {
            obligation_id: "operation.exec".into(),
            operation_id: "operation".into(),
            session_id: "session".into(),
            kind: SendObligationKindRecord::TurnExecution,
            disposition: SendObligationDispositionRecord::StartedTurn,
            human_message_id: None,
            assistant_message_id: None,
            reserved_turn_id: None,
            turn_id: Some("1".into()),
            dependency_obligation_ids: Vec::new(),
            canonical_payload: "{}".into(),
            state: ObligationStateRecord::Running,
        };
        assert!(matches!(
            classify_recovery_readback("operation.exec", &record, None, None),
            Ok(RecoveryReadbackTarget::SendTurn { .. })
        ));
        assert_eq!(
            classify_recovery_readback("another.exec", &record, None, None),
            Err(RecoveryReadbackRejection::EffectIdentityMismatch)
        );
    }

    #[test]
    fn retry_same_effect_is_admitted_only_for_pending_permission_response() {
        let response = PermissionResponse {
            request_id: "request".into(),
            decision: PermissionResponseDecision::Deny { message: None },
        };
        let pending = ObligationRecord::PermissionResponse {
            operation_id: "permission-op".into(),
            effect_identity: "permission-op.effect".into(),
            session_id: "session".into(),
            turn_id: "1".into(),
            response,
            owner_access: true,
            from_runtime_state: false,
            state: ObligationStateRecord::Pending,
        };
        assert_eq!(
            decide_recovery_action(RecoveryActionKind::RetrySameEffect, &pending, false, None),
            Ok(RecoveryActionDecision::RetryPermissionResponse {
                operation_id: "permission-op".into()
            })
        );
    }

    #[test]
    fn runtime_and_readback_identities_are_stable_and_target_bound() {
        assert_eq!(
            runtime_stop_request_id("session", 7),
            runtime_stop_request_id("session", 7)
        );
        assert_ne!(
            runtime_stop_request_id("session", 7),
            runtime_stop_request_id("session", 8)
        );
        assert_ne!(
            stop_readback_obligation_id("session", "7"),
            stop_readback_obligation_id("session", "8")
        );
        assert_ne!(
            session_close_readback_obligation_id("session"),
            session_close_readback_obligation_id("other")
        );
    }

    #[test]
    fn permission_recovery_status_has_one_domain_classification() {
        assert_eq!(
            classify_permission_response_recovery(
                PermissionResponseRecoveryObservation::AwaitingProviderResponse
            ),
            RecoveryResultClassification::Pending
        );
        assert_eq!(
            classify_permission_response_recovery(
                PermissionResponseRecoveryObservation::ReconciliationRequired
            ),
            RecoveryResultClassification::Ambiguous
        );
    }

    #[test]
    fn action_and_result_classification_compatibility_is_closed() {
        assert!(recovery_classification_is_allowed(
            RecoveryActionKind::CancelIfSafe,
            Some((RecoveryResultClassification::ConfirmedNoEffect, true)),
            RecoveryResultClassification::CancelledBeforeEffect
        ));
        assert!(!recovery_classification_is_allowed(
            RecoveryActionKind::CancelIfSafe,
            Some((RecoveryResultClassification::ConfirmedNoEffect, false)),
            RecoveryResultClassification::CancelledBeforeEffect
        ));
        assert_eq!(
            recovery_result_outcome(RecoveryResultClassification::Succeeded),
            RecoveryResultOutcomeRecord::Terminal
        );
    }

    #[test]
    fn backend_recovery_reservation_requires_exact_owner_and_reserved_state() {
        let record = ObligationRecord::BackendSessionRecovery {
            session_id: "session".into(),
            recovery_id: "recovery".into(),
            detail:
                crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                    reason:
                        crate::domain::agent_session::events::BackendSessionRecoveryReason::BackendSessionLost,
                    old_provider_session_generation: 7,
                    reserved_at_bits: 11,
                },
            state: ObligationStateRecord::EffectReserved,
        };
        assert_eq!(
            backend_recovery_reservation(&record, "session", "recovery"),
            Some(BackendRecoveryReservation {
                old_provider_session_generation: 7,
                reserved_at_bits: 11,
            })
        );
        assert_eq!(
            backend_recovery_reservation(&record, "other", "recovery"),
            None
        );
        assert!(backend_recovery_effect_identity_matches(
            "backend-recovery:session:recovery",
            "session",
            "recovery"
        ));
    }

    #[test]
    fn backend_recovery_reservation_admission_is_domain_owned() {
        let reserved = ObligationRecord::BackendSessionRecovery {
            session_id: "session".into(),
            recovery_id: "recovery".into(),
            detail: BackendSessionRecoveryObligationRecord::EffectReserved {
                reason:
                    crate::domain::agent_session::events::BackendSessionRecoveryReason::BackendSessionLost,
                old_provider_session_generation: 7,
                reserved_at_bits: 11,
            },
            state: ObligationStateRecord::EffectReserved,
        };
        assert_eq!(
            admit_backend_recovery_start(Some(&reserved), "session", "recovery"),
            Ok(BackendRecoveryReservationDecision::AlreadyApplied)
        );
        assert_eq!(
            admit_backend_recovery_completion(Some(&reserved), "session", "recovery", 7),
            Ok(BackendRecoveryReservationDecision::Apply)
        );
        assert_eq!(
            admit_backend_recovery_completion(Some(&reserved), "session", "recovery", 8),
            Err(BackendRecoveryReservationRejection::IdentityMismatch)
        );
        assert_eq!(
            admit_backend_recovery_failure(Some(&reserved), "session", "recovery"),
            Ok(BackendRecoveryReservationDecision::Apply)
        );
    }

    fn ready_backend_recovery_readback_facts() -> BackendRecoveryDurableCompletionFacts<'static> {
        BackendRecoveryDurableCompletionFacts {
            session_id: "session",
            recovery_id: "recovery",
            old_provider_session_generation: 7,
            reserved_at_bits: 10.0_f64.to_bits(),
            projected_provider_session_generation: 8,
            context_reinjection_generation: Some(8),
            backend_session_id: Some("provider-session"),
            publication_recovery_id: Some("recovery"),
            publication_session_id: Some("session"),
            completed_at_bits: 11.0_f64.to_bits(),
        }
    }

    #[test]
    fn backend_recovery_readback_completion_is_domain_decided() {
        let facts = ready_backend_recovery_readback_facts();
        let expected_digest = sha256(
            b"backend-recovery-readback-publication/v1\0session\0recovery\08\0provider-session",
        );
        assert_eq!(
            decide_backend_recovery_durable_completion(facts),
            Ok(BackendRecoveryDurableCompletionDecision::Complete(
                BackendRecoveryReadbackCompletion {
                    old_provider_session_generation: 7,
                    provider_session_generation: 8,
                    backend_session_id: "provider-session".into(),
                    completed_at_bits: 11.0_f64.to_bits(),
                    publication_message_id: format!(
                        "backend-recovery-readback-{}",
                        hex_lower(expected_digest)
                    ),
                }
            ))
        );
    }

    #[test]
    fn backend_recovery_readback_waits_for_exact_durable_evidence() {
        let mut facts = ready_backend_recovery_readback_facts();
        facts.publication_session_id = Some("other");
        assert_eq!(
            decide_backend_recovery_durable_completion(facts),
            Ok(BackendRecoveryDurableCompletionDecision::NotReady)
        );

        let mut facts = ready_backend_recovery_readback_facts();
        facts.backend_session_id = Some("");
        assert_eq!(
            decide_backend_recovery_durable_completion(facts),
            Ok(BackendRecoveryDurableCompletionDecision::NotReady)
        );
    }

    #[test]
    fn backend_recovery_readback_rejects_invalid_calculation_inputs() {
        let mut facts = ready_backend_recovery_readback_facts();
        facts.completed_at_bits = 9.0_f64.to_bits();
        assert_eq!(
            decide_backend_recovery_durable_completion(facts),
            Err(BackendRecoveryDurableCompletionRejection::InvalidCompletionTimestamp)
        );

        let mut facts = ready_backend_recovery_readback_facts();
        facts.old_provider_session_generation = u64::MAX;
        assert_eq!(
            decide_backend_recovery_durable_completion(facts),
            Err(
                BackendRecoveryDurableCompletionRejection::ProviderSessionGenerationCapacityExceeded
            )
        );
    }
}
