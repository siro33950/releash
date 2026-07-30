use crate::domain::agent_session::gateway::ResumeOutcome;
use crate::domain::agent_session::value_objects::{SessionState, TurnPhase};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionActivity {
    Running,
    Waiting,
    Error,
    Done,
}

pub fn classify_session_activity(
    turn_phase: TurnPhase,
    session_state: SessionState,
) -> SessionActivity {
    match turn_phase {
        TurnPhase::Streaming => SessionActivity::Running,
        TurnPhase::WaitingPermission => SessionActivity::Waiting,
        TurnPhase::Idle if session_state.is_error() => SessionActivity::Error,
        TurnPhase::Idle if session_state == SessionState::Idle => SessionActivity::Waiting,
        TurnPhase::Idle => SessionActivity::Done,
    }
}

pub fn backend_selection_is_presented_as_changeable(
    has_messages: bool,
    has_provider_session: bool,
    turn_phase: TurnPhase,
) -> bool {
    backend_selection_change_is_admitted(BackendSelectionChangeFacts {
        has_messages,
        has_provider_session,
        turn_phase,
        has_pending_permission: false,
        has_accepted_effects: false,
        has_backend_recovery: false,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendSelectionChangeFacts {
    pub has_messages: bool,
    pub has_provider_session: bool,
    pub turn_phase: TurnPhase,
    pub has_pending_permission: bool,
    pub has_accepted_effects: bool,
    pub has_backend_recovery: bool,
}

pub fn backend_selection_change_is_admitted(facts: BackendSelectionChangeFacts) -> bool {
    !facts.has_messages
        && !facts.has_provider_session
        && facts.turn_phase.is_idle()
        && !facts.has_pending_permission
        && !facts.has_accepted_effects
        && !facts.has_backend_recovery
}

pub fn project_runtime_turn_phase(
    has_active_turn: bool,
    has_pending_permission: bool,
) -> TurnPhase {
    if !has_active_turn {
        TurnPhase::Idle
    } else if has_pending_permission {
        TurnPhase::WaitingPermission
    } else {
        TurnPhase::Streaming
    }
}

pub fn classify_resume_outcome(requested: Option<&str>, actual: &str) -> ResumeOutcome {
    match requested {
        Some(requested) if requested == actual => ResumeOutcome::Resumed,
        Some(_) => ResumeOutcome::Mismatch {
            actual: actual.to_string(),
        },
        None => ResumeOutcome::NotRequested,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_turn_phase_precedes_terminal_history_projection() {
        assert_eq!(
            classify_session_activity(TurnPhase::Streaming, SessionState::Error),
            SessionActivity::Running
        );
        assert_eq!(
            classify_session_activity(TurnPhase::WaitingPermission, SessionState::Done),
            SessionActivity::Waiting
        );
    }

    #[test]
    fn backend_selection_hint_requires_an_empty_quiescent_session() {
        assert!(backend_selection_is_presented_as_changeable(
            false,
            false,
            TurnPhase::Idle
        ));
        assert!(!backend_selection_is_presented_as_changeable(
            true,
            false,
            TurnPhase::Idle
        ));
        assert!(!backend_selection_is_presented_as_changeable(
            false,
            false,
            TurnPhase::Streaming
        ));
        assert!(!backend_selection_change_is_admitted(
            BackendSelectionChangeFacts {
                has_messages: false,
                has_provider_session: false,
                turn_phase: TurnPhase::Idle,
                has_pending_permission: false,
                has_accepted_effects: true,
                has_backend_recovery: false,
            }
        ));
    }

    #[test]
    fn resume_outcome_classification_is_domain_owned() {
        assert_eq!(
            classify_resume_outcome(Some("same"), "same"),
            ResumeOutcome::Resumed
        );
        assert_eq!(
            classify_resume_outcome(Some("old"), "new"),
            ResumeOutcome::Mismatch {
                actual: "new".into()
            }
        );
        assert_eq!(
            classify_resume_outcome(None, "new"),
            ResumeOutcome::NotRequested
        );
    }
}
