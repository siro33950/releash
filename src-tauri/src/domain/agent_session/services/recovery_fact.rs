use crate::domain::agent_session::aggregates::session::RecoveryFact;
use crate::domain::agent_session::events::{AgentSessionDomainEvent, BackendSessionRecoveryReason};
use crate::domain::agent_session::value_objects::SessionState;
use crate::domain::local_event::{
    ObligationRecord, ObligationStateRecord, ObligationView, RecoveryPublicationMessageRecord,
    RecoveryPublicationObligationRecord,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPublicationListDecision {
    Sessions,
    ClosedHistory,
    ArchivedHistory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryPublicationDecision {
    pub list: RecoveryPublicationListDecision,
    pub published_state: SessionState,
}

pub fn decide_recovery_publication(state: SessionState) -> RecoveryPublicationDecision {
    match state {
        SessionState::Closed => RecoveryPublicationDecision {
            list: RecoveryPublicationListDecision::ClosedHistory,
            published_state: state,
        },
        SessionState::Archived => RecoveryPublicationDecision {
            list: RecoveryPublicationListDecision::ArchivedHistory,
            published_state: state,
        },
        _ => RecoveryPublicationDecision {
            list: RecoveryPublicationListDecision::Sessions,
            published_state: SessionState::Active,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPublicationCommitDecision {
    Publish,
    AlreadyPublished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPublicationCommitRejection {
    MessageIdentityMismatch,
    ObligationIdentityMismatch,
    PendingIdentityMismatch,
    NoLongerPending,
}

#[derive(Debug, Clone, Copy)]
pub struct RecoveryPublicationCommitFacts<'a> {
    pub session_id: &'a str,
    pub recovery_id: &'a str,
    pub message_id: &'a str,
    pub candidate_message_id: &'a str,
    pub source_obligation_id: &'a str,
    pub expected_message: &'a RecoveryPublicationMessageRecord,
    pub current_obligation: Option<&'a ObligationView>,
    pub projected_pending_message: Option<&'a RecoveryPublicationMessageRecord>,
    pub message_already_exists: bool,
}

pub fn decide_recovery_publication_commit(
    facts: RecoveryPublicationCommitFacts<'_>,
) -> Result<RecoveryPublicationCommitDecision, RecoveryPublicationCommitRejection> {
    if facts.candidate_message_id != facts.message_id {
        return Err(RecoveryPublicationCommitRejection::MessageIdentityMismatch);
    }
    let publication_completed = facts
        .current_obligation
        .map(|current| match &current.record {
            ObligationRecord::RecoveryPublication {
                session_id,
                recovery_id,
                message_id,
                source_obligation_id,
                detail: RecoveryPublicationObligationRecord::Pending { pending_message },
                state: ObligationStateRecord::Pending,
            } if session_id == facts.session_id
                && recovery_id == facts.recovery_id
                && message_id == facts.message_id
                && source_obligation_id == facts.source_obligation_id
                && pending_message == facts.expected_message =>
            {
                Ok(false)
            }
            ObligationRecord::RecoveryPublication {
                session_id,
                recovery_id,
                message_id,
                source_obligation_id,
                detail: RecoveryPublicationObligationRecord::Completed { .. },
                state: ObligationStateRecord::Completed,
            } if session_id == facts.session_id
                && recovery_id == facts.recovery_id
                && message_id == facts.message_id
                && source_obligation_id == facts.source_obligation_id =>
            {
                Ok(true)
            }
            _ => Err(RecoveryPublicationCommitRejection::ObligationIdentityMismatch),
        })
        .transpose()?;

    match facts.projected_pending_message {
        Some(current)
            if current == facts.expected_message && publication_completed != Some(true) =>
        {
            Ok(RecoveryPublicationCommitDecision::Publish)
        }
        None if facts.message_already_exists && publication_completed != Some(false) => {
            Ok(RecoveryPublicationCommitDecision::AlreadyPublished)
        }
        Some(_) => Err(RecoveryPublicationCommitRejection::PendingIdentityMismatch),
        None => Err(RecoveryPublicationCommitRejection::NoLongerPending),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendRecoveryObservation {
    Missing,
    Pending,
    Succeeded,
    Ambiguous,
}

impl BackendRecoveryObservation {
    pub fn requires_owner_completion(self) -> bool {
        self == Self::Pending
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendRecoveryReadbackDecision {
    Missing,
    Pending,
    Succeeded,
    Ambiguous,
    CompleteOwner,
}

pub fn decide_backend_recovery_readback(
    observation: BackendRecoveryObservation,
    owner_completion_available: bool,
) -> BackendRecoveryReadbackDecision {
    match (observation, owner_completion_available) {
        (BackendRecoveryObservation::Missing, _) => BackendRecoveryReadbackDecision::Missing,
        (BackendRecoveryObservation::Pending, true) => {
            BackendRecoveryReadbackDecision::CompleteOwner
        }
        (BackendRecoveryObservation::Pending, false) => BackendRecoveryReadbackDecision::Pending,
        (BackendRecoveryObservation::Succeeded, _) => BackendRecoveryReadbackDecision::Succeeded,
        (BackendRecoveryObservation::Ambiguous, _) => BackendRecoveryReadbackDecision::Ambiguous,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableBackendRecovery {
    None,
    Recovering {
        recovery_id: String,
        old_provider_session_generation: u64,
        reason: BackendSessionRecoveryReason,
    },
    ReconciliationRequired {
        recovery_id: String,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendRecoveryOperationRejection {
    PublicationPending { recovery_id: String },
    Recovering { recovery_id: String },
    ReconciliationRequired { recovery_id: String, error: String },
}

pub fn project_durable_backend_recovery(
    events: &[AgentSessionDomainEvent],
) -> DurableBackendRecovery {
    let mut recovery = DurableBackendRecovery::None;
    for event in events {
        match event {
            AgentSessionDomainEvent::BackendSessionRecoveryStarted {
                recovery_id,
                old_provider_session_generation,
                reason,
                ..
            } => {
                recovery = DurableBackendRecovery::Recovering {
                    recovery_id: recovery_id.clone(),
                    old_provider_session_generation: *old_provider_session_generation,
                    reason: *reason,
                };
            }
            AgentSessionDomainEvent::BackendSessionRecoveryCompleted { recovery_id, .. }
                if matches!(
                    &recovery,
                    DurableBackendRecovery::Recovering {
                        recovery_id: active,
                        ..
                    } if active == recovery_id
                ) =>
            {
                recovery = DurableBackendRecovery::None;
            }
            AgentSessionDomainEvent::BackendSessionRecoveryFailed {
                recovery_id, error, ..
            } => {
                recovery = DurableBackendRecovery::ReconciliationRequired {
                    recovery_id: recovery_id.clone(),
                    error: error.clone(),
                };
            }
            _ => {}
        }
    }
    recovery
}

pub fn backend_recovery_may_be_incomplete(
    has_provider_session: bool,
    context_carry_failed: bool,
    has_context_reinjection_generation: bool,
) -> bool {
    !has_provider_session && context_carry_failed && !has_context_reinjection_generation
}

pub fn admit_backend_recovery_sensitive_operation(
    pending_publication_recovery_id: Option<&str>,
    recovery_may_be_incomplete: bool,
    recovery: &DurableBackendRecovery,
) -> Result<(), BackendRecoveryOperationRejection> {
    if let Some(recovery_id) = pending_publication_recovery_id {
        return Err(BackendRecoveryOperationRejection::PublicationPending {
            recovery_id: recovery_id.to_string(),
        });
    }
    if !recovery_may_be_incomplete {
        return Ok(());
    }
    match recovery {
        DurableBackendRecovery::Recovering { recovery_id, .. } => {
            Err(BackendRecoveryOperationRejection::Recovering {
                recovery_id: recovery_id.clone(),
            })
        }
        DurableBackendRecovery::ReconciliationRequired { recovery_id, error } => {
            Err(BackendRecoveryOperationRejection::ReconciliationRequired {
                recovery_id: recovery_id.clone(),
                error: error.clone(),
            })
        }
        DurableBackendRecovery::None => Ok(()),
    }
}

pub fn classify_backend_recovery(
    events: &[AgentSessionDomainEvent],
    expected_recovery_id: &str,
) -> BackendRecoveryObservation {
    let mut observation = BackendRecoveryObservation::Missing;
    for event in events {
        match event {
            AgentSessionDomainEvent::BackendSessionRecoveryStarted { recovery_id, .. }
                if recovery_id == expected_recovery_id =>
            {
                observation = BackendRecoveryObservation::Pending;
            }
            AgentSessionDomainEvent::BackendSessionRecoveryCompleted { recovery_id, .. }
                if recovery_id == expected_recovery_id
                    && observation == BackendRecoveryObservation::Pending =>
            {
                observation = BackendRecoveryObservation::Succeeded;
            }
            AgentSessionDomainEvent::BackendSessionRecoveryFailed { recovery_id, .. }
                if recovery_id == expected_recovery_id
                    && observation == BackendRecoveryObservation::Pending =>
            {
                observation = BackendRecoveryObservation::Ambiguous;
            }
            _ => {}
        }
    }
    observation
}

/// Classifies owner-scoped operation facts for the Session aggregate.
///
/// This service crosses the independent Session and obligation aggregate
/// boundaries, but deliberately does not decide whether any Session command
/// is admissible.
pub fn classify_recovery_fact<'a>(
    has_pending_recovery_projection: bool,
    obligations: impl IntoIterator<Item = (&'a str, &'a ObligationRecord)>,
) -> RecoveryFact {
    if has_pending_recovery_projection
        || obligations.into_iter().any(|(identity, record)| {
            record
                .unresolved_recovery_original_identity(identity)
                .is_some()
        })
    {
        RecoveryFact::Unresolved
    } else {
        RecoveryFact::Resolved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::local_event::{
        ObligationStateRecord, SendObligationDispositionRecord, SendObligationKindRecord,
    };

    #[test]
    fn normal_live_send_is_not_classified_as_unresolved_recovery() {
        let record = ObligationRecord::Send {
            obligation_id: "obligation".into(),
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
            state: ObligationStateRecord::Pending,
        };
        assert_eq!(
            classify_recovery_fact(false, [("obligation", &record)]),
            RecoveryFact::Resolved
        );
    }

    #[test]
    fn reconciliation_fences_the_session() {
        let record = ObligationRecord::ProviderEstablish {
            operation_id: "operation".into(),
            effect_identity: "provider-establish:operation".into(),
            session_id: "session".into(),
            state: ObligationStateRecord::ReconciliationRequired,
        };
        assert_eq!(
            classify_recovery_fact(false, [("obligation", &record)]),
            RecoveryFact::Unresolved
        );
    }

    #[test]
    fn consumed_publication_and_cleared_obligations_resolve_recovery() {
        let obligations: Vec<(String, ObligationRecord)> = Vec::new();
        assert_eq!(
            classify_recovery_fact(
                false,
                obligations
                    .iter()
                    .map(|(identity, record)| (identity.as_str(), record)),
            ),
            RecoveryFact::Resolved
        );
        assert_eq!(
            classify_recovery_fact(
                true,
                obligations
                    .iter()
                    .map(|(identity, record)| (identity.as_str(), record)),
            ),
            RecoveryFact::Unresolved
        );
    }

    #[test]
    fn backend_recovery_completion_requires_the_matching_start() {
        let completed = AgentSessionDomainEvent::BackendSessionRecoveryCompleted {
            recovery_id: "recovery".into(),
            provider_session_generation: 2,
            at: 2.0,
        };
        assert_eq!(
            classify_backend_recovery(&[completed.clone()], "recovery"),
            BackendRecoveryObservation::Missing
        );
        let started = AgentSessionDomainEvent::BackendSessionRecoveryStarted {
            recovery_id: "recovery".into(),
            old_provider_session_generation: 1,
            reason:
                crate::domain::agent_session::events::BackendSessionRecoveryReason::ResumeMismatch,
            at: 1.0,
        };
        assert_eq!(
            classify_backend_recovery(&[started, completed], "recovery"),
            BackendRecoveryObservation::Succeeded
        );
        assert_eq!(
            decide_backend_recovery_readback(BackendRecoveryObservation::Pending, true),
            BackendRecoveryReadbackDecision::CompleteOwner
        );
    }

    #[test]
    fn durable_recovery_projection_and_admission_are_decided_in_domain() {
        let events = vec![AgentSessionDomainEvent::BackendSessionRecoveryStarted {
            recovery_id: "recovery".into(),
            old_provider_session_generation: 1,
            reason: BackendSessionRecoveryReason::BackendSessionLost,
            at: 1.0,
        }];
        let recovery = project_durable_backend_recovery(&events);
        assert_eq!(
            admit_backend_recovery_sensitive_operation(None, true, &recovery),
            Err(BackendRecoveryOperationRejection::Recovering {
                recovery_id: "recovery".into()
            })
        );
        assert!(admit_backend_recovery_sensitive_operation(None, false, &recovery).is_ok());
        assert_eq!(
            admit_backend_recovery_sensitive_operation(
                Some("publication"),
                false,
                &DurableBackendRecovery::None
            ),
            Err(BackendRecoveryOperationRejection::PublicationPending {
                recovery_id: "publication".into()
            })
        );
    }

    #[test]
    fn recovery_publication_commit_is_decided_from_exact_durable_facts() {
        let expected = RecoveryPublicationMessageRecord {
            kind: crate::domain::local_event::RecoveryPublicationMessageKindRecord::Notice,
            recovery_id: "recovery".into(),
            message_id: "message".into(),
            error: None,
        };
        let facts = RecoveryPublicationCommitFacts {
            session_id: "session",
            recovery_id: "recovery",
            message_id: "message",
            candidate_message_id: "message",
            source_obligation_id: "backend-recovery:session:recovery",
            expected_message: &expected,
            current_obligation: None,
            projected_pending_message: Some(&expected),
            message_already_exists: false,
        };
        assert_eq!(
            decide_recovery_publication_commit(facts),
            Ok(RecoveryPublicationCommitDecision::Publish)
        );
        assert_eq!(
            decide_recovery_publication_commit(RecoveryPublicationCommitFacts {
                projected_pending_message: None,
                message_already_exists: true,
                ..facts
            }),
            Ok(RecoveryPublicationCommitDecision::AlreadyPublished)
        );
        assert_eq!(
            decide_recovery_publication_commit(RecoveryPublicationCommitFacts {
                candidate_message_id: "other",
                ..facts
            }),
            Err(RecoveryPublicationCommitRejection::MessageIdentityMismatch)
        );
    }
}
