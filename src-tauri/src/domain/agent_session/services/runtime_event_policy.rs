use crate::domain::agent_session::events::BackendSessionRecoveryReason;
use crate::domain::agent_session::gateway::AgentRuntimeEvent;
use crate::domain::agent_session::value_objects::ContextCarryState;
use crate::domain::local_event::sha256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalCommitIdentity {
    pub turn_id: u64,
    pub message_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingTerminalCommitIdentity;

pub fn require_terminal_commit_identity(
    turn_id: Option<u64>,
    message_id: Option<String>,
) -> Result<TerminalCommitIdentity, MissingTerminalCommitIdentity> {
    match (turn_id, message_id) {
        (Some(turn_id), Some(message_id)) => Ok(TerminalCommitIdentity {
            turn_id,
            message_id,
        }),
        _ => Err(MissingTerminalCommitIdentity),
    }
}

fn deterministic_uuid(material: Vec<u8>) -> String {
    let digest = sha256(material);
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

pub fn runtime_error_message_id(
    session_id: &str,
    runtime_epoch: u64,
    event_received_at: f64,
    message: &str,
) -> String {
    let mut exact = Vec::with_capacity(session_id.len() + message.len() + 48);
    exact.extend_from_slice(b"runtime_error_message_v1");
    exact.extend_from_slice(&(session_id.len() as u64).to_be_bytes());
    exact.extend_from_slice(session_id.as_bytes());
    exact.extend_from_slice(&runtime_epoch.to_be_bytes());
    exact.extend_from_slice(&event_received_at.to_bits().to_be_bytes());
    exact.extend_from_slice(&(message.len() as u64).to_be_bytes());
    exact.extend_from_slice(message.as_bytes());
    deterministic_uuid(exact)
}

pub fn runtime_event_recovery_id(
    session_id: &str,
    runtime_epoch: u64,
    event_received_at: f64,
    reason: BackendSessionRecoveryReason,
    event_identity: &str,
) -> String {
    let reason_tag = match reason {
        BackendSessionRecoveryReason::ResumeMismatch => b"resume_mismatch".as_slice(),
        BackendSessionRecoveryReason::BackendSessionLost => b"backend_session_lost".as_slice(),
    };
    let mut exact = Vec::with_capacity(session_id.len() + event_identity.len() + 80);
    exact.extend_from_slice(b"runtime_event_recovery_v1");
    exact.extend_from_slice(&(session_id.len() as u64).to_be_bytes());
    exact.extend_from_slice(session_id.as_bytes());
    exact.extend_from_slice(&runtime_epoch.to_be_bytes());
    exact.extend_from_slice(&event_received_at.to_bits().to_be_bytes());
    exact.extend_from_slice(&(reason_tag.len() as u64).to_be_bytes());
    exact.extend_from_slice(reason_tag);
    exact.extend_from_slice(&(event_identity.len() as u64).to_be_bytes());
    exact.extend_from_slice(event_identity.as_bytes());
    deterministic_uuid(exact)
}

pub fn runtime_provider_session_observation_id(
    session_id: &str,
    runtime_epoch: u64,
    event_received_at: f64,
    backend_session_id: &str,
    context_carry: Option<&ContextCarryState>,
) -> String {
    let context_carry_tag = match context_carry {
        None => b"not_requested".as_slice(),
        Some(ContextCarryState::Resumed) => b"resumed".as_slice(),
        Some(ContextCarryState::Reinjected) => b"reinjected".as_slice(),
        Some(ContextCarryState::Failed) => b"failed".as_slice(),
    };
    let mut exact = Vec::with_capacity(session_id.len() + backend_session_id.len() + 80);
    exact.extend_from_slice(b"runtime_provider_session_observation_v1");
    exact.extend_from_slice(&(session_id.len() as u64).to_be_bytes());
    exact.extend_from_slice(session_id.as_bytes());
    exact.extend_from_slice(&runtime_epoch.to_be_bytes());
    exact.extend_from_slice(&event_received_at.to_bits().to_be_bytes());
    exact.extend_from_slice(&(backend_session_id.len() as u64).to_be_bytes());
    exact.extend_from_slice(backend_session_id.as_bytes());
    exact.extend_from_slice(&(context_carry_tag.len() as u64).to_be_bytes());
    exact.extend_from_slice(context_carry_tag);
    deterministic_uuid(exact)
}

pub fn runtime_event_targets_current_turn(event: &AgentRuntimeEvent) -> bool {
    matches!(
        event,
        AgentRuntimeEvent::PartsMerged(_)
            | AgentRuntimeEvent::PermissionRequested(_)
            | AgentRuntimeEvent::TokenUsageUpdated(_)
            | AgentRuntimeEvent::KeepAlive
            | AgentRuntimeEvent::TurnCompleted(_)
            | AgentRuntimeEvent::Fatal { .. }
    )
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeEventAdmissionFacts<'a> {
    pub event: &'a AgentRuntimeEvent,
    pub current_runtime: bool,
    pub terminal_committed: bool,
    pub provider_establishment_in_flight: bool,
    pub recovery_completion_in_flight: bool,
    pub recovery_failure_in_flight: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEventAdmission {
    Apply,
    DropStaleRuntime,
    DropAfterTerminal,
    RejectRecoveryFailureSettling,
    RejectProviderEstablishmentSettling,
    RejectRecoveryCompletionSettling,
}

pub fn decide_runtime_event_admission(
    facts: RuntimeEventAdmissionFacts<'_>,
) -> RuntimeEventAdmission {
    if !facts.current_runtime {
        return RuntimeEventAdmission::DropStaleRuntime;
    }
    if facts.terminal_committed && runtime_event_targets_current_turn(facts.event) {
        return RuntimeEventAdmission::DropAfterTerminal;
    }
    if facts.recovery_failure_in_flight {
        return RuntimeEventAdmission::RejectRecoveryFailureSettling;
    }
    let requires_provider_settlement = matches!(
        facts.event,
        AgentRuntimeEvent::TurnCompleted(_)
            | AgentRuntimeEvent::Fatal { .. }
            | AgentRuntimeEvent::BackendSessionCleared
            | AgentRuntimeEvent::SessionEstablished {
                resume: crate::domain::agent_session::gateway::ResumeOutcome::Mismatch { .. },
                ..
            }
    );
    if facts.provider_establishment_in_flight && requires_provider_settlement {
        return RuntimeEventAdmission::RejectProviderEstablishmentSettling;
    }
    if facts.recovery_completion_in_flight && requires_provider_settlement {
        return RuntimeEventAdmission::RejectRecoveryCompletionSettling;
    }
    RuntimeEventAdmission::Apply
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEstablishedEventDecision {
    RecoverResumeMismatch,
    IgnoreAlreadyEstablished,
    Observe,
}

pub fn decide_session_established_event(
    resume: &crate::domain::agent_session::gateway::ResumeOutcome,
    provider_session_already_established: bool,
    recovery_active: bool,
) -> SessionEstablishedEventDecision {
    if matches!(
        resume,
        crate::domain::agent_session::gateway::ResumeOutcome::Mismatch { .. }
    ) {
        SessionEstablishedEventDecision::RecoverResumeMismatch
    } else if provider_session_already_established && !recovery_active {
        SessionEstablishedEventDecision::IgnoreAlreadyEstablished
    } else {
        SessionEstablishedEventDecision::Observe
    }
}

pub fn context_carry_for_established_resume(
    resume: &crate::domain::agent_session::gateway::ResumeOutcome,
) -> Result<Option<ContextCarryState>, ()> {
    match resume {
        crate::domain::agent_session::gateway::ResumeOutcome::Resumed => {
            Ok(Some(ContextCarryState::Resumed))
        }
        crate::domain::agent_session::gateway::ResumeOutcome::NotRequested => Ok(None),
        crate::domain::agent_session::gateway::ResumeOutcome::Mismatch { .. } => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_identities_bind_every_fencing_fact() {
        assert_ne!(
            runtime_error_message_id("session", 1, 2.0, "error"),
            runtime_error_message_id("session", 2, 2.0, "error")
        );
        assert_ne!(
            runtime_event_recovery_id(
                "session",
                1,
                2.0,
                BackendSessionRecoveryReason::ResumeMismatch,
                "event"
            ),
            runtime_event_recovery_id(
                "session",
                1,
                2.0,
                BackendSessionRecoveryReason::BackendSessionLost,
                "event"
            )
        );
        assert_ne!(
            runtime_provider_session_observation_id(
                "session",
                1,
                2.0,
                "provider",
                Some(&ContextCarryState::Resumed)
            ),
            runtime_provider_session_observation_id(
                "session",
                1,
                2.0,
                "provider",
                Some(&ContextCarryState::Failed)
            )
        );
    }

    #[test]
    fn terminal_commit_requires_both_durable_identities() {
        assert_eq!(
            require_terminal_commit_identity(Some(7), Some("message".into())),
            Ok(TerminalCommitIdentity {
                turn_id: 7,
                message_id: "message".into(),
            })
        );
        assert_eq!(
            require_terminal_commit_identity(Some(7), None),
            Err(MissingTerminalCommitIdentity)
        );
    }

    #[test]
    fn runtime_event_admission_orders_all_fences_in_domain() {
        let event = AgentRuntimeEvent::Fatal {
            message: "crash".into(),
        };
        let facts = RuntimeEventAdmissionFacts {
            event: &event,
            current_runtime: true,
            terminal_committed: false,
            provider_establishment_in_flight: false,
            recovery_completion_in_flight: false,
            recovery_failure_in_flight: false,
        };
        assert_eq!(
            decide_runtime_event_admission(RuntimeEventAdmissionFacts {
                current_runtime: false,
                ..facts
            }),
            RuntimeEventAdmission::DropStaleRuntime
        );
        assert_eq!(
            decide_runtime_event_admission(RuntimeEventAdmissionFacts {
                terminal_committed: true,
                ..facts
            }),
            RuntimeEventAdmission::DropAfterTerminal
        );
        assert_eq!(
            decide_runtime_event_admission(RuntimeEventAdmissionFacts {
                recovery_failure_in_flight: true,
                ..facts
            }),
            RuntimeEventAdmission::RejectRecoveryFailureSettling
        );
        assert_eq!(
            decide_runtime_event_admission(RuntimeEventAdmissionFacts {
                provider_establishment_in_flight: true,
                ..facts
            }),
            RuntimeEventAdmission::RejectProviderEstablishmentSettling
        );
        assert_eq!(
            decide_runtime_event_admission(RuntimeEventAdmissionFacts {
                recovery_completion_in_flight: true,
                ..facts
            }),
            RuntimeEventAdmission::RejectRecoveryCompletionSettling
        );
        assert_eq!(
            decide_runtime_event_admission(facts),
            RuntimeEventAdmission::Apply
        );
    }

    #[test]
    fn provider_establishment_routing_is_domain_owned() {
        use crate::domain::agent_session::gateway::ResumeOutcome;

        assert_eq!(
            decide_session_established_event(
                &ResumeOutcome::Mismatch {
                    actual: "new".into(),
                },
                false,
                false,
            ),
            SessionEstablishedEventDecision::RecoverResumeMismatch
        );
        assert_eq!(
            decide_session_established_event(&ResumeOutcome::Resumed, true, false),
            SessionEstablishedEventDecision::IgnoreAlreadyEstablished
        );
        assert_eq!(
            decide_session_established_event(&ResumeOutcome::NotRequested, true, true),
            SessionEstablishedEventDecision::Observe
        );
        assert_eq!(
            context_carry_for_established_resume(&ResumeOutcome::Resumed),
            Ok(Some(ContextCarryState::Resumed))
        );
        assert_eq!(
            context_carry_for_established_resume(&ResumeOutcome::Mismatch {
                actual: "new".into(),
            }),
            Err(())
        );
    }
}
