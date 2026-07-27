use crate::domain::agent_session::entities::SessionState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkflowTurnAdmissionFacts {
    pub(crate) session_state: SessionState,
    pub(crate) has_active_turn: bool,
    pub(crate) has_pending_queue: bool,
    pub(crate) has_unresolved_recovery: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkflowTurnAdmissionRejection {
    SessionClosed,
    NotQuiescent,
    UnresolvedRecovery,
}

pub(crate) fn decide_workflow_turn_admission(
    facts: WorkflowTurnAdmissionFacts,
) -> Result<(), WorkflowTurnAdmissionRejection> {
    if matches!(
        facts.session_state,
        SessionState::Closed | SessionState::Archived
    ) {
        return Err(WorkflowTurnAdmissionRejection::SessionClosed);
    }
    if facts.has_active_turn || facts.has_pending_queue {
        return Err(WorkflowTurnAdmissionRejection::NotQuiescent);
    }
    if facts.has_unresolved_recovery {
        return Err(WorkflowTurnAdmissionRejection::UnresolvedRecovery);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_session_state_matches_the_workflow_turn_sendability_matrix() {
        for state in [
            SessionState::Idle,
            SessionState::Active,
            SessionState::Done,
            SessionState::Error,
        ] {
            assert_eq!(
                decide_workflow_turn_admission(WorkflowTurnAdmissionFacts {
                    session_state: state,
                    has_active_turn: false,
                    has_pending_queue: false,
                    has_unresolved_recovery: false,
                }),
                Ok(())
            );
        }

        for state in [SessionState::Closed, SessionState::Archived] {
            assert_eq!(
                decide_workflow_turn_admission(WorkflowTurnAdmissionFacts {
                    session_state: state,
                    has_active_turn: false,
                    has_pending_queue: false,
                    has_unresolved_recovery: false,
                }),
                Err(WorkflowTurnAdmissionRejection::SessionClosed)
            );
        }
    }

    #[test]
    fn every_open_state_requires_quiescence_and_resolved_recovery() {
        for state in [
            SessionState::Idle,
            SessionState::Active,
            SessionState::Done,
            SessionState::Error,
        ] {
            assert_eq!(
                decide_workflow_turn_admission(WorkflowTurnAdmissionFacts {
                    session_state: state.clone(),
                    has_active_turn: true,
                    has_pending_queue: false,
                    has_unresolved_recovery: false,
                }),
                Err(WorkflowTurnAdmissionRejection::NotQuiescent)
            );
            assert_eq!(
                decide_workflow_turn_admission(WorkflowTurnAdmissionFacts {
                    session_state: state.clone(),
                    has_active_turn: false,
                    has_pending_queue: true,
                    has_unresolved_recovery: false,
                }),
                Err(WorkflowTurnAdmissionRejection::NotQuiescent)
            );
            assert_eq!(
                decide_workflow_turn_admission(WorkflowTurnAdmissionFacts {
                    session_state: state,
                    has_active_turn: false,
                    has_pending_queue: false,
                    has_unresolved_recovery: true,
                }),
                Err(WorkflowTurnAdmissionRejection::UnresolvedRecovery)
            );
        }
    }
}
