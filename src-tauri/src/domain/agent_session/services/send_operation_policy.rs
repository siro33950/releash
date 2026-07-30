use std::time::Duration;

use crate::domain::agent_session::events::{PromptInput, SendDisposition};
use crate::domain::local_event::{hex_lower, sha256};

pub const INTERNAL_WORKFLOW_OPERATION_PRINCIPAL: &str = "workflow-runtime";
pub const WORKFLOW_SEND_RETRY_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowSendTargetRejection {
    NotWorkflowSession,
    AuthorityMismatch,
}

pub fn admit_workflow_send_target(
    workflow_node_session: bool,
    stored_permission_mode: &str,
    requested_permission_mode: &str,
) -> Result<(), WorkflowSendTargetRejection> {
    if !workflow_node_session {
        return Err(WorkflowSendTargetRejection::NotWorkflowSession);
    }
    if stored_permission_mode != requested_permission_mode {
        return Err(WorkflowSendTargetRejection::AuthorityMismatch);
    }
    Ok(())
}

pub fn workflow_send_receipt_matches(
    expected_session_id: &str,
    actual_session_id: &str,
    disposition: &SendDisposition,
) -> bool {
    expected_session_id == actual_session_id
        && matches!(disposition, SendDisposition::StartedTurn { .. })
}

pub fn workflow_send_should_retry(retryable: bool, attempt: usize) -> bool {
    retryable && attempt.saturating_add(1) < WORKFLOW_SEND_RETRY_ATTEMPTS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptedSendTarget<'a> {
    Direct {
        session_id: Option<&'a str>,
        worktree_path: &'a str,
    },
    WorkflowApproval,
    WorkflowTurn {
        session_id: &'a str,
    },
}

pub fn accepted_send_target_matches(
    target: AcceptedSendTarget<'_>,
    accepted_session_id: &str,
    accepted_worktree_path: &str,
) -> bool {
    match target {
        AcceptedSendTarget::Direct {
            session_id: Some(session_id),
            worktree_path,
        } => session_id == accepted_session_id && worktree_path == accepted_worktree_path,
        AcceptedSendTarget::Direct {
            session_id: None,
            worktree_path,
        } => worktree_path == accepted_worktree_path,
        AcceptedSendTarget::WorkflowApproval => true,
        AcceptedSendTarget::WorkflowTurn { session_id } => session_id == accepted_session_id,
    }
}

pub fn accepted_worktree_matches(queued_worktree_path: &str, session_worktree_path: &str) -> bool {
    queued_worktree_path == session_worktree_path
}

pub fn accepted_prompt_matches(committed: &PromptInput, accepted: &PromptInput) -> bool {
    committed == accepted
}

pub fn workflow_turn_principal_is_authorized(principal: &str) -> bool {
    principal == INTERNAL_WORKFLOW_OPERATION_PRINCIPAL
}

pub fn accepted_send_artifact_identity_material(
    principal: &str,
    operation_id: &str,
    canonical_payload: &str,
) -> Vec<u8> {
    let mut identity = b"accepted-send-artifact/v1\0".to_vec();
    for value in [principal, operation_id, canonical_payload] {
        identity.extend_from_slice(&(value.len() as u64).to_be_bytes());
        identity.extend_from_slice(value.as_bytes());
    }
    identity
}

pub fn accepted_send_artifact_digest(
    principal: &str,
    operation_id: &str,
    canonical_payload: &str,
) -> [u8; 32] {
    sha256(accepted_send_artifact_identity_material(
        principal,
        operation_id,
        canonical_payload,
    ))
}

pub fn durable_workflow_turn_identity_material(
    node_execution_id: &str,
    turn_role: &str,
) -> Vec<u8> {
    let mut identity = b"durable-workflow-turn/v1".to_vec();
    for value in [node_execution_id, turn_role] {
        identity.extend_from_slice(&(value.len() as u64).to_be_bytes());
        identity.extend_from_slice(value.as_bytes());
    }
    identity
}

pub fn durable_workflow_turn_operation_id(node_execution_id: &str, turn_role: &str) -> String {
    format!(
        "workflow-send-{}",
        hex_lower(sha256(durable_workflow_turn_identity_material(
            node_execution_id,
            turn_role
        )))
    )
}

pub fn accepted_send_retry_delay(attempt: usize) -> Duration {
    let shift = u32::try_from(attempt.min(5)).unwrap_or(5);
    Duration::from_millis(25_u64.saturating_mul(1_u64 << shift))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptedQueuedEffectIdentity<'a> {
    pub queue_item_id: &'a str,
    pub human_message_id: Option<&'a str>,
    pub reserved_turn_id: Option<u64>,
    pub operation_id: Option<&'a str>,
    pub obligation_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalQueuedEffectIdentity<'a> {
    pub queue_item_id: &'a str,
    pub human_message_id: &'a str,
    pub reserved_turn_id: &'a str,
}

pub fn accepted_queued_effect_has_durable_identity(
    effect: AcceptedQueuedEffectIdentity<'_>,
) -> bool {
    effect.operation_id.is_some() || effect.obligation_id.is_some()
}

pub fn accepted_queued_effect_matches(
    effect: AcceptedQueuedEffectIdentity<'_>,
    canonical: CanonicalQueuedEffectIdentity<'_>,
) -> bool {
    effect.queue_item_id == canonical.queue_item_id
        && effect.human_message_id == Some(canonical.human_message_id)
        && effect.reserved_turn_id == canonical.reserved_turn_id.parse::<u64>().ok()
}

pub fn accepted_queued_effect_identity_is_consistent(
    existing: AcceptedQueuedEffectIdentity<'_>,
    observed: AcceptedQueuedEffectIdentity<'_>,
) -> bool {
    existing.queue_item_id == observed.queue_item_id
        && existing.operation_id == observed.operation_id
        && existing.obligation_id == observed.obligation_id
        && existing.human_message_id == observed.human_message_id
        && existing.reserved_turn_id == observed.reserved_turn_id
}

pub fn accepted_effect_execution_matches(
    operation_id: Option<&str>,
    obligation_id: Option<&str>,
    expected_operation_id: &str,
    expected_obligation_id: &str,
) -> bool {
    operation_id == Some(expected_operation_id) && obligation_id == Some(expected_obligation_id)
}

pub fn accepted_effect_has_durable_execution_identity(
    operation_id: Option<&str>,
    obligation_id: Option<&str>,
) -> bool {
    operation_id.is_some() && obligation_id.is_some()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeTurnRecoveryDecision {
    RetainAccepted {
        turn_id: u64,
        assistant_message_id: String,
    },
    Requeue,
}

pub fn decide_runtime_turn_recovery(
    active_turn_id: Option<u64>,
    operation_id: Option<&str>,
    obligation_id: Option<&str>,
    assistant_message_id: Option<&str>,
) -> RuntimeTurnRecoveryDecision {
    match (
        active_turn_id,
        accepted_effect_has_durable_execution_identity(operation_id, obligation_id),
        assistant_message_id,
    ) {
        (Some(turn_id), true, Some(assistant_message_id)) => {
            RuntimeTurnRecoveryDecision::RetainAccepted {
                turn_id,
                assistant_message_id: assistant_message_id.to_string(),
            }
        }
        _ => RuntimeTurnRecoveryDecision::Requeue,
    }
}

pub fn accepted_effect_is_process_owned(
    has_active_turn: bool,
    operation_id: Option<&str>,
    obligation_id: Option<&str>,
    expected_operation_id: &str,
    expected_obligation_id: &str,
) -> bool {
    has_active_turn
        && accepted_effect_execution_matches(
            operation_id,
            obligation_id,
            expected_operation_id,
            expected_obligation_id,
        )
}

pub fn accepted_queued_effect_reservation_conflicts(
    existing: AcceptedQueuedEffectIdentity<'_>,
    observed: AcceptedQueuedEffectIdentity<'_>,
) -> bool {
    existing.queue_item_id != observed.queue_item_id
        && existing.reserved_turn_id == observed.reserved_turn_id
        && (existing.operation_id != observed.operation_id
            || existing.obligation_id != observed.obligation_id)
}

pub fn queue_item_identity_matches(observed: &str, expected: &str) -> bool {
    observed == expected
}

pub fn queued_effect_remains_unstarted(has_active_turn: bool, queue_item_is_cached: bool) -> bool {
    !has_active_turn && queue_item_is_cached
}

pub fn accepted_queued_effect_should_retain<'a>(
    effect: AcceptedQueuedEffectIdentity<'_>,
    canonical: impl IntoIterator<Item = CanonicalQueuedEffectIdentity<'a>>,
) -> bool {
    !accepted_queued_effect_has_durable_identity(effect)
        || canonical
            .into_iter()
            .any(|entry| accepted_queued_effect_matches(effect, entry))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptedQueuedEffectQueueDecision {
    Start,
    AwaitCanonicalFront,
    DiscardStale,
}

pub fn decide_accepted_queued_effect_queue<'a>(
    effect: AcceptedQueuedEffectIdentity<'_>,
    canonical: impl IntoIterator<Item = CanonicalQueuedEffectIdentity<'a>>,
) -> AcceptedQueuedEffectQueueDecision {
    if !accepted_queued_effect_has_durable_identity(effect) {
        return AcceptedQueuedEffectQueueDecision::Start;
    }
    let mut canonical = canonical.into_iter();
    let Some(front) = canonical.next() else {
        return AcceptedQueuedEffectQueueDecision::DiscardStale;
    };
    if accepted_queued_effect_matches(effect, front) {
        return AcceptedQueuedEffectQueueDecision::Start;
    }
    if canonical.any(|entry| accepted_queued_effect_matches(effect, entry)) {
        AcceptedQueuedEffectQueueDecision::AwaitCanonicalFront
    } else {
        AcceptedQueuedEffectQueueDecision::DiscardStale
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReservedTurnIdentity<'a> {
    pub queue_item_id: &'a str,
    pub turn_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnIdentityAllocationError {
    InvalidReservedIdentity { queue_item_id: String },
    NonAdvancingReservedIdentity { queue_item_id: String },
    CapacityExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptedEffectIdentityRejection {
    InvalidTurn,
    MissingReservedTurn,
    MissingAssistantMessage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptedEffectExecutionIdentity {
    Queued {
        queue_item_id: String,
    },
    StartedTurn {
        turn_id: u64,
        assistant_message_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedEffectRuntimeIdentity {
    pub reserved_turn_id: Option<u64>,
    pub execution: AcceptedEffectExecutionIdentity,
}

pub fn validate_accepted_effect_runtime_identity(
    disposition: &SendDisposition,
    reserved_turn_id: Option<&str>,
    assistant_message_id: Option<&str>,
) -> Result<AcceptedEffectRuntimeIdentity, AcceptedEffectIdentityRejection> {
    let reserved_turn_id = reserved_turn_id
        .map(str::parse::<u64>)
        .transpose()
        .map_err(|_| AcceptedEffectIdentityRejection::InvalidTurn)?;
    let execution = match disposition {
        SendDisposition::Queued { queue_item_id } => {
            if reserved_turn_id.is_none() {
                return Err(AcceptedEffectIdentityRejection::MissingReservedTurn);
            }
            AcceptedEffectExecutionIdentity::Queued {
                queue_item_id: queue_item_id.clone(),
            }
        }
        SendDisposition::StartedTurn { turn_id } => {
            let turn_id = turn_id
                .parse::<u64>()
                .map_err(|_| AcceptedEffectIdentityRejection::InvalidTurn)?;
            let assistant_message_id = assistant_message_id
                .ok_or(AcceptedEffectIdentityRejection::MissingAssistantMessage)?;
            AcceptedEffectExecutionIdentity::StartedTurn {
                turn_id,
                assistant_message_id: assistant_message_id.to_string(),
            }
        }
    };
    Ok(AcceptedEffectRuntimeIdentity {
        reserved_turn_id,
        execution,
    })
}

pub fn allocate_next_turn_identity<'a>(
    last_turn_id: u64,
    reservations: impl IntoIterator<Item = ReservedTurnIdentity<'a>>,
) -> Result<u64, TurnIdentityAllocationError> {
    let mut previous = last_turn_id;
    for reservation in reservations {
        let reserved = reservation.turn_id.parse::<u64>().map_err(|_| {
            TurnIdentityAllocationError::InvalidReservedIdentity {
                queue_item_id: reservation.queue_item_id.to_string(),
            }
        })?;
        if reserved <= previous {
            return Err(TurnIdentityAllocationError::NonAdvancingReservedIdentity {
                queue_item_id: reservation.queue_item_id.to_string(),
            });
        }
        previous = reserved;
    }
    if previous >= i64::MAX as u64 {
        return Err(TurnIdentityAllocationError::CapacityExceeded);
    }
    previous
        .checked_add(1)
        .ok_or(TurnIdentityAllocationError::CapacityExceeded)
}

pub fn turn_identity_advances(previous: u64, candidate: u64) -> bool {
    candidate > previous
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnPreclaimFailureDisposition {
    PreserveOriginal,
    FailAcceptedEffect,
}

pub fn turn_preclaim_failure_disposition(
    accepted_execution: bool,
) -> TurnPreclaimFailureDisposition {
    if accepted_execution {
        TurnPreclaimFailureDisposition::FailAcceptedEffect
    } else {
        TurnPreclaimFailureDisposition::PreserveOriginal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_send_target_requires_workflow_ownership_and_exact_authority() {
        assert_eq!(
            admit_workflow_send_target(false, "edit", "edit"),
            Err(WorkflowSendTargetRejection::NotWorkflowSession)
        );
        assert_eq!(
            admit_workflow_send_target(true, "edit", "plan"),
            Err(WorkflowSendTargetRejection::AuthorityMismatch)
        );
        assert_eq!(admit_workflow_send_target(true, "edit", "edit"), Ok(()));
    }

    #[test]
    fn workflow_receipt_requires_the_accepted_session_and_immediate_turn() {
        assert!(workflow_send_receipt_matches(
            "session",
            "session",
            &SendDisposition::StartedTurn {
                turn_id: "1".into()
            }
        ));
        assert!(!workflow_send_receipt_matches(
            "session",
            "other",
            &SendDisposition::StartedTurn {
                turn_id: "1".into()
            }
        ));
        assert!(!workflow_send_receipt_matches(
            "session",
            "session",
            &SendDisposition::Queued {
                queue_item_id: "queue".into()
            }
        ));
    }

    #[test]
    fn workflow_retry_is_bounded_by_the_domain_policy() {
        assert!(workflow_send_should_retry(true, 0));
        assert!(workflow_send_should_retry(true, 1));
        assert!(!workflow_send_should_retry(true, 2));
        assert!(!workflow_send_should_retry(false, 0));
    }

    #[test]
    fn accepted_target_matching_preserves_each_target_contract() {
        assert!(accepted_send_target_matches(
            AcceptedSendTarget::Direct {
                session_id: Some("session"),
                worktree_path: "/repo",
            },
            "session",
            "/repo",
        ));
        assert!(!accepted_send_target_matches(
            AcceptedSendTarget::Direct {
                session_id: None,
                worktree_path: "/other",
            },
            "session",
            "/repo",
        ));
        assert!(accepted_send_target_matches(
            AcceptedSendTarget::WorkflowApproval,
            "session",
            "/repo",
        ));
        assert!(!accepted_send_target_matches(
            AcceptedSendTarget::WorkflowTurn {
                session_id: "other"
            },
            "session",
            "/repo",
        ));
    }

    #[test]
    fn artifact_identity_material_binds_all_length_delimited_fields() {
        assert_ne!(
            accepted_send_artifact_identity_material("ab", "c", "d"),
            accepted_send_artifact_identity_material("a", "bc", "d")
        );
        assert_ne!(
            accepted_send_artifact_digest("principal", "operation", "payload"),
            accepted_send_artifact_digest("principal", "other", "payload")
        );
    }

    #[test]
    fn workflow_turn_identity_material_binds_role_and_node() {
        assert_ne!(
            durable_workflow_turn_identity_material("ab", "c"),
            durable_workflow_turn_identity_material("a", "bc")
        );
        assert_ne!(
            durable_workflow_turn_operation_id("ab", "c"),
            durable_workflow_turn_operation_id("a", "bc")
        );
    }

    #[test]
    fn retry_delay_is_exponential_and_capped() {
        assert_eq!(accepted_send_retry_delay(0), Duration::from_millis(25));
        assert_eq!(accepted_send_retry_delay(3), Duration::from_millis(200));
        assert_eq!(accepted_send_retry_delay(5), Duration::from_millis(800));
        assert_eq!(accepted_send_retry_delay(99), Duration::from_millis(800));
    }

    #[test]
    fn accepted_queue_cache_is_fenced_by_the_exact_canonical_identity() {
        let accepted = AcceptedQueuedEffectIdentity {
            queue_item_id: "queue",
            human_message_id: Some("human"),
            reserved_turn_id: Some(7),
            operation_id: Some("operation"),
            obligation_id: Some("obligation"),
        };
        let canonical = CanonicalQueuedEffectIdentity {
            queue_item_id: "queue",
            human_message_id: "human",
            reserved_turn_id: "7",
        };
        assert!(accepted_queued_effect_matches(accepted, canonical));
        assert_eq!(
            decide_accepted_queued_effect_queue(accepted, [canonical]),
            AcceptedQueuedEffectQueueDecision::Start
        );
        assert_eq!(
            decide_accepted_queued_effect_queue(
                accepted,
                [
                    CanonicalQueuedEffectIdentity {
                        queue_item_id: "front",
                        human_message_id: "other",
                        reserved_turn_id: "6",
                    },
                    canonical,
                ],
            ),
            AcceptedQueuedEffectQueueDecision::AwaitCanonicalFront
        );
        assert_eq!(
            decide_accepted_queued_effect_queue(accepted, []),
            AcceptedQueuedEffectQueueDecision::DiscardStale
        );
        assert!(accepted_queued_effect_should_retain(accepted, [canonical]));
        assert!(!accepted_queued_effect_should_retain(
            accepted,
            [CanonicalQueuedEffectIdentity {
                reserved_turn_id: "8",
                ..canonical
            }]
        ));
        assert!(!accepted_queued_effect_identity_is_consistent(
            accepted,
            AcceptedQueuedEffectIdentity {
                operation_id: Some("other"),
                ..accepted
            }
        ));
    }

    #[test]
    fn next_turn_identity_validates_the_complete_reserved_sequence() {
        assert_eq!(
            allocate_next_turn_identity(
                3,
                [
                    ReservedTurnIdentity {
                        queue_item_id: "a",
                        turn_id: "4",
                    },
                    ReservedTurnIdentity {
                        queue_item_id: "b",
                        turn_id: "6",
                    },
                ],
            ),
            Ok(7)
        );
        assert_eq!(
            allocate_next_turn_identity(
                3,
                [ReservedTurnIdentity {
                    queue_item_id: "a",
                    turn_id: "3",
                }],
            ),
            Err(TurnIdentityAllocationError::NonAdvancingReservedIdentity {
                queue_item_id: "a".to_string(),
            })
        );
        assert_eq!(
            allocate_next_turn_identity(i64::MAX as u64, []),
            Err(TurnIdentityAllocationError::CapacityExceeded)
        );
    }

    #[test]
    fn accepted_runtime_effect_requires_complete_durable_identity() {
        assert_eq!(
            validate_accepted_effect_runtime_identity(
                &SendDisposition::Queued {
                    queue_item_id: "queue".into(),
                },
                None,
                None,
            ),
            Err(AcceptedEffectIdentityRejection::MissingReservedTurn)
        );
        assert_eq!(
            validate_accepted_effect_runtime_identity(
                &SendDisposition::StartedTurn {
                    turn_id: "7".into(),
                },
                None,
                Some("assistant"),
            ),
            Ok(AcceptedEffectRuntimeIdentity {
                reserved_turn_id: None,
                execution: AcceptedEffectExecutionIdentity::StartedTurn {
                    turn_id: 7,
                    assistant_message_id: "assistant".into(),
                },
            })
        );
    }

    #[test]
    fn runtime_recovery_retains_only_a_complete_accepted_turn() {
        assert_eq!(
            decide_runtime_turn_recovery(
                Some(7),
                Some("operation"),
                Some("obligation"),
                Some("assistant"),
            ),
            RuntimeTurnRecoveryDecision::RetainAccepted {
                turn_id: 7,
                assistant_message_id: "assistant".into(),
            }
        );
        assert_eq!(
            decide_runtime_turn_recovery(Some(7), Some("operation"), None, Some("assistant")),
            RuntimeTurnRecoveryDecision::Requeue
        );
    }

    #[test]
    fn preclaim_failure_disposition_distinguishes_accepted_effects() {
        assert_eq!(
            turn_preclaim_failure_disposition(true),
            TurnPreclaimFailureDisposition::FailAcceptedEffect
        );
        assert_eq!(
            turn_preclaim_failure_disposition(false),
            TurnPreclaimFailureDisposition::PreserveOriginal
        );
    }
}
