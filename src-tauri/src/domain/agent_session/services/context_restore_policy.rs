use crate::domain::agent_session::value_objects::{ContextCarryState, SessionState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextRestorePreparationDecision {
    Skip {
        expected_provider_session_generation: u64,
    },
    Restore {
        reinjection_required: bool,
        expected_provider_session_generation: u64,
    },
}

pub fn decide_context_restore_preparation(
    had_runtime: bool,
    provider_session_generation: u64,
    context_reinjection_generation: Option<u64>,
) -> ContextRestorePreparationDecision {
    let reinjection_required = context_reinjection_generation == Some(provider_session_generation);
    if had_runtime && !reinjection_required {
        ContextRestorePreparationDecision::Skip {
            expected_provider_session_generation: provider_session_generation,
        }
    } else {
        ContextRestorePreparationDecision::Restore {
            reinjection_required,
            expected_provider_session_generation: provider_session_generation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextRestoreCompletionFacts {
    pub session_state: SessionState,
    pub pending_recovery_failure: bool,
    pub has_recovery_publication_snapshot: bool,
    pub provider_session_generation: u64,
    pub context_reinjection_generation: Option<u64>,
    pub last_turn_id: Option<u64>,
    pub backend_recovery_observation: bool,
    pub has_pending_recovery_message: bool,
    pub context_carry: Option<ContextCarryState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextRestoreCompletionCommand {
    pub expected_provider_session_generation: u64,
    pub expected_turn_id: Option<u64>,
    pub reinjected: bool,
    pub clear_context_carry: bool,
    pub recovery_restore_required: bool,
}

impl ContextRestoreCompletionCommand {
    pub fn after_started_turn(
        expected_provider_session_generation: u64,
        expected_turn_id: u64,
        reinjected: bool,
        clear_context_carry: bool,
        recovery_restore_required: bool,
    ) -> Self {
        Self {
            expected_provider_session_generation,
            expected_turn_id: Some(expected_turn_id),
            reinjected,
            clear_context_carry,
            recovery_restore_required,
        }
    }

    pub fn requests_change(self) -> bool {
        self.reinjected || self.clear_context_carry || self.recovery_restore_required
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextCarryChange {
    Keep,
    Replace(Option<ContextCarryState>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextRestoreCompletionDecision {
    pub clear_context_reinjection_generation: bool,
    pub context_carry: ContextCarryChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextRestoreCompletionRejection {
    Fenced,
    Unchanged,
}

pub fn decide_context_restore_completion(
    facts: ContextRestoreCompletionFacts,
    command: ContextRestoreCompletionCommand,
) -> Result<ContextRestoreCompletionDecision, ContextRestoreCompletionRejection> {
    if command.recovery_restore_required {
        if facts.session_state.fences_context_restore()
            || facts.pending_recovery_failure
            || facts.has_recovery_publication_snapshot
            || facts.provider_session_generation != command.expected_provider_session_generation
            || facts.context_reinjection_generation
                != Some(command.expected_provider_session_generation)
        {
            return Err(ContextRestoreCompletionRejection::Fenced);
        }
        return Ok(ContextRestoreCompletionDecision {
            clear_context_reinjection_generation: true,
            context_carry: if command.reinjected {
                ContextCarryChange::Replace(Some(ContextCarryState::Reinjected))
            } else {
                ContextCarryChange::Keep
            },
        });
    }

    let next_generation = command.expected_provider_session_generation.checked_add(1);
    let generation_matches = facts.provider_session_generation
        == command.expected_provider_session_generation
        || next_generation == Some(facts.provider_session_generation);
    if facts.session_state.fences_context_restore()
        || !generation_matches
        || command.expected_turn_id.is_none()
        || facts.last_turn_id != command.expected_turn_id
        || facts.backend_recovery_observation
        || facts.has_recovery_publication_snapshot
        || facts.has_pending_recovery_message
        || facts.context_reinjection_generation.is_some()
        || facts
            .context_carry
            .is_some_and(ContextCarryState::is_failed)
    {
        return Err(ContextRestoreCompletionRejection::Fenced);
    }
    if !command.reinjected && !command.clear_context_carry {
        return Err(ContextRestoreCompletionRejection::Unchanged);
    }
    let context_carry = command.reinjected.then_some(ContextCarryState::Reinjected);
    if facts.context_carry == context_carry {
        return Err(ContextRestoreCompletionRejection::Unchanged);
    }
    Ok(ContextRestoreCompletionDecision {
        clear_context_reinjection_generation: false,
        context_carry: ContextCarryChange::Replace(context_carry),
    })
}

pub fn context_restore_completion_is_settled(
    facts: ContextRestoreCompletionFacts,
    command: ContextRestoreCompletionCommand,
) -> bool {
    if command.recovery_restore_required {
        return facts.provider_session_generation == command.expected_provider_session_generation
            && facts.context_reinjection_generation.is_none()
            && !facts.has_recovery_publication_snapshot
            && !facts.pending_recovery_failure
            && !facts.session_state.fences_context_restore()
            && (!command.reinjected
                || facts
                    .context_carry
                    .is_some_and(ContextCarryState::is_reinjected));
    }
    let next_generation = command.expected_provider_session_generation.checked_add(1);
    let generation_matches = facts.provider_session_generation
        == command.expected_provider_session_generation
        || next_generation == Some(facts.provider_session_generation);
    generation_matches
        && command.expected_turn_id.is_some()
        && facts.last_turn_id == command.expected_turn_id
        && !facts.backend_recovery_observation
        && facts.context_carry == command.reinjected.then_some(ContextCarryState::Reinjected)
        && !facts.has_recovery_publication_snapshot
        && !facts.has_pending_recovery_message
        && facts.context_reinjection_generation.is_none()
        && !facts.session_state.fences_context_restore()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> ContextRestoreCompletionFacts {
        ContextRestoreCompletionFacts {
            session_state: SessionState::Active,
            pending_recovery_failure: false,
            has_recovery_publication_snapshot: false,
            provider_session_generation: 4,
            context_reinjection_generation: None,
            last_turn_id: Some(7),
            backend_recovery_observation: false,
            has_pending_recovery_message: false,
            context_carry: None,
        }
    }

    #[test]
    fn preparation_reuses_only_a_runtime_without_a_reinjection_fence() {
        assert_eq!(
            decide_context_restore_preparation(true, 4, None),
            ContextRestorePreparationDecision::Skip {
                expected_provider_session_generation: 4
            }
        );
        assert_eq!(
            decide_context_restore_preparation(true, 4, Some(4)),
            ContextRestorePreparationDecision::Restore {
                reinjection_required: true,
                expected_provider_session_generation: 4
            }
        );
        assert_eq!(
            decide_context_restore_preparation(false, 4, None),
            ContextRestorePreparationDecision::Restore {
                reinjection_required: false,
                expected_provider_session_generation: 4
            }
        );
    }

    #[test]
    fn ordinary_completion_requires_the_current_turn_and_generation() {
        let decision = decide_context_restore_completion(
            facts(),
            ContextRestoreCompletionCommand {
                expected_provider_session_generation: 4,
                expected_turn_id: Some(7),
                reinjected: true,
                clear_context_carry: false,
                recovery_restore_required: false,
            },
        )
        .unwrap();
        assert_eq!(
            decision.context_carry,
            ContextCarryChange::Replace(Some(ContextCarryState::Reinjected))
        );
        assert!(
            ContextRestoreCompletionCommand::after_started_turn(4, 7, true, false, false)
                .requests_change()
        );

        let mut stale = facts();
        stale.last_turn_id = Some(8);
        assert_eq!(
            decide_context_restore_completion(
                stale,
                ContextRestoreCompletionCommand {
                    expected_provider_session_generation: 4,
                    expected_turn_id: Some(7),
                    reinjected: true,
                    clear_context_carry: false,
                    recovery_restore_required: false,
                }
            ),
            Err(ContextRestoreCompletionRejection::Fenced)
        );
    }

    #[test]
    fn recovery_completion_consumes_only_the_matching_reinjection_marker() {
        let mut recovery = facts();
        recovery.context_reinjection_generation = Some(4);
        let decision = decide_context_restore_completion(
            recovery,
            ContextRestoreCompletionCommand {
                expected_provider_session_generation: 4,
                expected_turn_id: None,
                reinjected: false,
                clear_context_carry: false,
                recovery_restore_required: true,
            },
        )
        .unwrap();
        assert!(decision.clear_context_reinjection_generation);
        assert_eq!(decision.context_carry, ContextCarryChange::Keep);
    }

    #[test]
    fn settled_classification_uses_the_same_domain_fences() {
        let mut settled = facts();
        settled.context_carry = Some(ContextCarryState::Reinjected);
        assert!(context_restore_completion_is_settled(
            settled,
            ContextRestoreCompletionCommand {
                expected_provider_session_generation: 4,
                expected_turn_id: Some(7),
                reinjected: true,
                clear_context_carry: false,
                recovery_restore_required: false,
            }
        ));
        settled.session_state = SessionState::Closed;
        assert!(!context_restore_completion_is_settled(
            settled,
            ContextRestoreCompletionCommand {
                expected_provider_session_generation: 4,
                expected_turn_id: Some(7),
                reinjected: true,
                clear_context_carry: false,
                recovery_restore_required: false,
            }
        ));
    }
}
