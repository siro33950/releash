use crate::domain::agent_session::events::{RecoveryActionKind, RecoveryResultClassification};
use crate::domain::local_event::{
    ObligationRecord, ObligationRecoveryActionRecord, ObligationStateRecord,
    SendObligationDispositionRecord, SendObligationKindRecord,
    WorkflowTurnCompletionObligationRecord,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingRecoveryOwnerTarget {
    Session {
        session_id: String,
    },
    WorkflowExecution {
        execution_id: String,
    },
    WorkflowNode {
        execution_id: String,
        node_execution_id: String,
        workflow_name: String,
        node_name: String,
        attempt: u32,
    },
    ClosedSession {
        session_id: String,
    },
    ArchivedSession {
        session_id: String,
    },
    UnownedRuntime {
        runtime_id: String,
    },
    UnknownOwner {
        owner: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PendingRecoveryCategory {
    TurnExecution,
    QueueExecution,
    PermissionDelivery,
    ProviderEstablish,
    TerminalCommit,
    BackendRecovery,
    SessionClose,
    WorkflowShutdown,
    RecoveryPublication,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PendingRecoveryKnownStatus {
    Prepared,
    Pending,
    EffectReserved,
    Running,
    WaitingApproval,
    ReconciliationRequired,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryResourceState {
    Pending,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryActionIdentity {
    pub action_id: String,
    pub action: RecoveryActionKind,
    pub origin_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRecoveryDescriptor {
    pub category: PendingRecoveryCategory,
    pub original_identity: String,
    pub known_status: PendingRecoveryKnownStatus,
    pub safe_label: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryObservationFact {
    pub classification: RecoveryResultClassification,
    pub cancellable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCapabilities {
    pub state: RecoveryResourceState,
    pub safe_label: String,
    pub actions: Vec<RecoveryActionKind>,
    pub active_action: Option<RecoveryActionIdentity>,
}

fn bounded_identity(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| !value.is_empty() && value.len() <= 512)
        .map(str::to_string)
}

pub fn bounded_recovery_owner_component(value: &str) -> Option<String> {
    bounded_identity(Some(value))
}

fn original_obligation(record: &ObligationRecord) -> &ObligationRecord {
    record.original()
}

pub fn workflow_node_recovery_owner_target(
    record: &ObligationRecord,
) -> Option<PendingRecoveryOwnerTarget> {
    let ObligationRecord::WorkflowTurnCompletion {
        detail:
            WorkflowTurnCompletionObligationRecord::Pending {
                workflow_context, ..
            },
        ..
    } = original_obligation(record)
    else {
        return None;
    };
    let context = workflow_context.as_ref();
    Some(PendingRecoveryOwnerTarget::WorkflowNode {
        execution_id: bounded_recovery_owner_component(&context.execution_id)?,
        node_execution_id: bounded_recovery_owner_component(&context.node_execution_id)?,
        workflow_name: bounded_recovery_owner_component(&context.workflow_name)?,
        node_name: bounded_recovery_owner_component(&context.node_name)?,
        attempt: context.attempt,
    })
}

fn known_status(state: Option<ObligationStateRecord>) -> PendingRecoveryKnownStatus {
    match state {
        Some(ObligationStateRecord::Prepared) => PendingRecoveryKnownStatus::Prepared,
        Some(ObligationStateRecord::Pending) => PendingRecoveryKnownStatus::Pending,
        Some(ObligationStateRecord::EffectReserved) => PendingRecoveryKnownStatus::EffectReserved,
        Some(ObligationStateRecord::Running) => PendingRecoveryKnownStatus::Running,
        Some(ObligationStateRecord::WaitingApproval) => PendingRecoveryKnownStatus::WaitingApproval,
        Some(ObligationStateRecord::ReconciliationRequired) => {
            PendingRecoveryKnownStatus::ReconciliationRequired
        }
        Some(ObligationStateRecord::Failed) => PendingRecoveryKnownStatus::Failed,
        Some(ObligationStateRecord::OutcomeUnknown)
        | Some(ObligationStateRecord::Completed)
        | Some(ObligationStateRecord::Cancelled)
        | None => PendingRecoveryKnownStatus::Unknown,
    }
}

fn descriptor(
    category: PendingRecoveryCategory,
    identity: Option<String>,
    known_status: PendingRecoveryKnownStatus,
    safe_label: &'static str,
    obligation_id: &str,
) -> PendingRecoveryDescriptor {
    match identity {
        Some(original_identity) => PendingRecoveryDescriptor {
            category,
            original_identity,
            known_status,
            safe_label,
        },
        None => PendingRecoveryDescriptor {
            category: PendingRecoveryCategory::Unknown,
            original_identity: obligation_id.to_string(),
            known_status: PendingRecoveryKnownStatus::Unknown,
            safe_label: "Pending local operation",
        },
    }
}

pub fn pending_recovery_descriptor(
    obligation_id: &str,
    record: &ObligationRecord,
) -> PendingRecoveryDescriptor {
    let status = known_status(record.state());
    let identity = |value: &str| bounded_identity(Some(value));
    let (category, original_identity, safe_label) = match original_obligation(record) {
        ObligationRecord::Send {
            operation_id,
            kind: SendObligationKindRecord::ProviderEstablish,
            ..
        } => (
            PendingRecoveryCategory::ProviderEstablish,
            identity(operation_id),
            "Provider session establishment",
        ),
        ObligationRecord::Send {
            operation_id,
            kind: SendObligationKindRecord::TurnExecution,
            disposition,
            ..
        } => match disposition {
            SendObligationDispositionRecord::Queued => (
                PendingRecoveryCategory::QueueExecution,
                identity(operation_id),
                "Queued agent execution",
            ),
            SendObligationDispositionRecord::StartedTurn => (
                PendingRecoveryCategory::TurnExecution,
                identity(operation_id),
                "Agent turn execution",
            ),
        },
        ObligationRecord::PermissionResponse { operation_id, .. } => (
            PendingRecoveryCategory::PermissionDelivery,
            identity(operation_id),
            "Permission response delivery",
        ),
        ObligationRecord::StopInterrupt { operation_id, .. }
        | ObligationRecord::TerminalCommit { operation_id, .. } => (
            PendingRecoveryCategory::TerminalCommit,
            identity(operation_id),
            "Agent turn terminalization",
        ),
        ObligationRecord::SessionClose { operation_id, .. } => (
            PendingRecoveryCategory::SessionClose,
            identity(operation_id),
            "Session lifecycle action",
        ),
        ObligationRecord::BackendSessionRecovery { recovery_id, .. } => (
            PendingRecoveryCategory::BackendRecovery,
            identity(recovery_id),
            "Backend session recovery",
        ),
        ObligationRecord::WorkflowShutdown {
            effect_identity,
            execution_id,
            ..
        } => (
            PendingRecoveryCategory::WorkflowShutdown,
            identity(effect_identity).or_else(|| identity(execution_id)),
            "Workflow shutdown",
        ),
        ObligationRecord::WorkflowTurnCompletion {
            terminal_identity, ..
        } => (
            PendingRecoveryCategory::TurnExecution,
            identity(terminal_identity),
            "Workflow turn completion handoff",
        ),
        ObligationRecord::RecoveryPublication {
            message_id,
            recovery_id,
            ..
        } => (
            PendingRecoveryCategory::RecoveryPublication,
            identity(message_id).or_else(|| identity(recovery_id)),
            "Recovery message publication",
        ),
        ObligationRecord::ProviderEstablish {
            operation_id,
            effect_identity,
            ..
        } => (
            PendingRecoveryCategory::ProviderEstablish,
            identity(operation_id).or_else(|| identity(effect_identity)),
            "Provider session establishment",
        ),
        ObligationRecord::TurnExecution {
            operation_id,
            turn_id,
            ..
        } => (
            PendingRecoveryCategory::TurnExecution,
            identity(operation_id).or_else(|| identity(turn_id)),
            "Agent turn execution",
        ),
        ObligationRecord::RecoveryReserved {
            recovery_id,
            effect_identity,
            ..
        }
        | ObligationRecord::RecoveryCompleted {
            recovery_id,
            effect_identity,
            ..
        } => (
            PendingRecoveryCategory::BackendRecovery,
            identity(recovery_id).or_else(|| identity(effect_identity)),
            "Recovery reconciliation",
        ),
        ObligationRecord::FeedbackReservation { .. }
        | ObligationRecord::Feedback { .. }
        | ObligationRecord::WorkflowExecution { .. }
        | ObligationRecord::RecoveryTransition { .. }
        | ObligationRecord::Observed { .. } => (
            PendingRecoveryCategory::Unknown,
            Some(obligation_id.to_string()),
            "Pending local operation",
        ),
    };
    descriptor(
        category,
        original_identity,
        status,
        safe_label,
        obligation_id,
    )
}

fn recovery_action(record: &ObligationRecord) -> Option<&ObligationRecoveryActionRecord> {
    match record {
        ObligationRecord::RecoveryTransition {
            recovery_action, ..
        } => Some(recovery_action),
        ObligationRecord::Observed { original, .. } => recovery_action(original),
        _ => None,
    }
}

fn permission_payload_is_valid(
    record: &ObligationRecord,
    permission_payload_encodable: bool,
) -> bool {
    matches!(
        original_obligation(record),
        ObligationRecord::PermissionResponse {
            operation_id,
            effect_identity,
            session_id,
            turn_id,
            response,
            owner_access: true,
            state: ObligationStateRecord::Pending,
            ..
        } if !operation_id.is_empty()
            && !session_id.is_empty()
            && !turn_id.is_empty()
            && !response.request_id.is_empty()
            && effect_identity == &format!("permission-response:{operation_id}")
            && permission_payload_encodable
    )
}

pub fn decide_recovery_capabilities(
    obligation_id: &str,
    origin_revision: u64,
    record: &ObligationRecord,
    observation: Option<RecoveryObservationFact>,
    supports_read_again: bool,
    permission_payload_encodable: bool,
    derived_active_action_id: Option<&str>,
) -> RecoveryCapabilities {
    let permission_payload_valid =
        permission_payload_is_valid(record, permission_payload_encodable);
    if matches!(
        original_obligation(record),
        ObligationRecord::PermissionResponse {
            state: ObligationStateRecord::Pending,
            ..
        }
    ) && !permission_payload_valid
    {
        return RecoveryCapabilities {
            state: RecoveryResourceState::Failed,
            safe_label: "Permission response payload is unavailable".to_string(),
            actions: vec![RecoveryActionKind::KeepForManualResolution],
            active_action: None,
        };
    }

    let nonterminal_action = recovery_action(record).filter(|action| {
        matches!(
            action.state,
            ObligationStateRecord::Prepared
                | ObligationStateRecord::EffectReserved
                | ObligationStateRecord::OutcomeUnknown
                | ObligationStateRecord::ReconciliationRequired
        )
    });
    let active_action = nonterminal_action.and_then(|action| {
        (action.effect_identity == obligation_id
            && derived_active_action_id == Some(action.action_id.as_str()))
        .then(|| RecoveryActionIdentity {
            action_id: action.action_id.clone(),
            action: action.action,
            origin_revision,
        })
    });
    if nonterminal_action.is_some() && active_action.is_none() {
        return RecoveryCapabilities {
            state: RecoveryResourceState::Failed,
            safe_label: "Recovery action identity is incompatible".to_string(),
            actions: Vec::new(),
            active_action: None,
        };
    }
    if let Some(active_action) = active_action {
        return RecoveryCapabilities {
            state: RecoveryResourceState::Pending,
            safe_label: pending_recovery_descriptor(obligation_id, record)
                .safe_label
                .to_string(),
            actions: vec![active_action.action],
            active_action: Some(active_action),
        };
    }

    let mut actions = Vec::new();
    if supports_read_again {
        actions.push(RecoveryActionKind::ReadAgain);
    }
    if permission_payload_valid {
        actions.push(RecoveryActionKind::RetrySameEffect);
    }
    if observation.is_some() {
        actions.push(RecoveryActionKind::UseObservedResult);
    }
    if observation.is_some_and(|fact| {
        fact.classification == RecoveryResultClassification::ConfirmedNoEffect && fact.cancellable
    }) {
        actions.push(RecoveryActionKind::CancelIfSafe);
    }
    actions.push(RecoveryActionKind::KeepForManualResolution);
    RecoveryCapabilities {
        state: RecoveryResourceState::Pending,
        safe_label: pending_recovery_descriptor(obligation_id, record)
            .safe_label
            .to_string(),
        actions,
        active_action: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::entities::{PermissionResponse, PermissionResponseDecision};

    fn permission_record(state: ObligationStateRecord) -> ObligationRecord {
        ObligationRecord::PermissionResponse {
            operation_id: "permission-op".into(),
            effect_identity: "permission-response:permission-op".into(),
            session_id: "session".into(),
            turn_id: "7".into(),
            response: PermissionResponse {
                request_id: "request".into(),
                decision: PermissionResponseDecision::Allow {
                    updated_input: None,
                    answers: None,
                },
            },
            owner_access: true,
            from_runtime_state: false,
            state,
        }
    }

    #[test]
    fn permission_retry_requires_the_exact_saved_payload() {
        let valid = decide_recovery_capabilities(
            "permission-response:permission-op",
            1,
            &permission_record(ObligationStateRecord::Pending),
            None,
            false,
            true,
            None,
        );
        assert_eq!(valid.state, RecoveryResourceState::Pending);
        assert!(valid.actions.contains(&RecoveryActionKind::RetrySameEffect));

        let invalid = decide_recovery_capabilities(
            "permission-response:permission-op",
            1,
            &permission_record(ObligationStateRecord::Pending),
            None,
            false,
            false,
            None,
        );
        assert_eq!(invalid.state, RecoveryResourceState::Failed);
        assert_eq!(
            invalid.actions,
            vec![RecoveryActionKind::KeepForManualResolution]
        );
    }

    #[test]
    fn confirmed_no_effect_is_the_only_cancellable_observation() {
        let record = ObligationRecord::ProviderEstablish {
            operation_id: "operation".into(),
            effect_identity: "effect".into(),
            session_id: "session".into(),
            state: ObligationStateRecord::OutcomeUnknown,
        };
        let decision = decide_recovery_capabilities(
            "effect",
            2,
            &record,
            Some(RecoveryObservationFact {
                classification: RecoveryResultClassification::ConfirmedNoEffect,
                cancellable: true,
            }),
            true,
            false,
            None,
        );
        assert!(decision.actions.contains(&RecoveryActionKind::ReadAgain));
        assert!(decision
            .actions
            .contains(&RecoveryActionKind::UseObservedResult));
        assert!(decision.actions.contains(&RecoveryActionKind::CancelIfSafe));
    }
}
