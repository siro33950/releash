#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeTurnOwnership {
    Current,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeTurnStartCommit {
    Commit,
    Interrupted,
    Paused,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFatalObservation {
    CompleteCurrentTurn,
    MatchesCompletedCrash,
    Unrelated,
}

/// Process-local ownership aggregate for one provider turn.
///
/// Runtime handles and streaming buffers remain outside this type. It alone
/// advances the generation fence and decides whether interrupt/terminal work
/// still belongs to the current turn.
#[derive(Debug, Default)]
pub struct RuntimeTurn {
    active_turn_id: Option<u64>,
    last_turn_id: Option<u64>,
    terminal_turn_id: Option<u64>,
    generation: u64,
    interrupt_requested_generation: Option<u64>,
    pending_trailing_fatal_message: Option<String>,
}

impl RuntimeTurn {
    pub fn register_start(&mut self, turn_id: u64) -> u64 {
        self.active_turn_id = Some(turn_id);
        self.last_turn_id = Some(turn_id);
        self.terminal_turn_id = None;
        self.generation = self.generation.saturating_add(1);
        self.generation
    }

    pub fn observe_canonical_identity(&mut self, turn_id: u64) {
        self.active_turn_id = Some(turn_id);
        self.last_turn_id = Some(turn_id);
    }

    pub fn release(&mut self) {
        self.active_turn_id = None;
    }

    pub fn rollback_start(&mut self) {
        self.active_turn_id = None;
        self.terminal_turn_id = None;
        self.pending_trailing_fatal_message = None;
    }

    pub fn mark_terminal(&mut self, turn_id: u64) -> RuntimeTurnOwnership {
        if self.active_turn_id != Some(turn_id) {
            return RuntimeTurnOwnership::Superseded;
        }
        self.terminal_turn_id = Some(turn_id);
        RuntimeTurnOwnership::Current
    }

    pub fn seal_terminal(&mut self, turn_id: u64) -> RuntimeTurnOwnership {
        if self.active_turn_id.or(self.last_turn_id) != Some(turn_id) {
            return RuntimeTurnOwnership::Superseded;
        }
        self.active_turn_id = None;
        self.terminal_turn_id = Some(turn_id);
        self.interrupt_requested_generation = None;
        RuntimeTurnOwnership::Current
    }

    pub fn request_interrupt(&mut self) -> RuntimeTurnOwnership {
        if self.active_turn_id.is_none() {
            return RuntimeTurnOwnership::Superseded;
        }
        self.interrupt_requested_generation = Some(self.generation);
        RuntimeTurnOwnership::Current
    }

    pub fn clear_interrupt_request(&mut self) {
        self.interrupt_requested_generation = None;
    }

    pub fn clear_trailing_fatal(&mut self) {
        self.pending_trailing_fatal_message = None;
    }

    pub fn admits_trailing_fatal_wait(&self, has_trailing_fatal_message: bool) -> bool {
        has_trailing_fatal_message && self.has_active_turn()
    }

    pub fn defer_trailing_fatal(&mut self, message: Option<String>) {
        self.pending_trailing_fatal_message = message;
    }

    pub fn observe_fatal(&mut self, message: &str) -> RuntimeFatalObservation {
        let observation = if self.has_active_turn() {
            RuntimeFatalObservation::CompleteCurrentTurn
        } else if self.pending_trailing_fatal_message.as_deref() == Some(message) {
            RuntimeFatalObservation::MatchesCompletedCrash
        } else {
            RuntimeFatalObservation::Unrelated
        };
        self.pending_trailing_fatal_message = None;
        observation
    }

    pub fn active_turn_id(&self) -> Option<u64> {
        self.active_turn_id
    }

    #[cfg(test)]
    pub fn last_turn_id(&self) -> Option<u64> {
        self.last_turn_id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn has_active_turn(&self) -> bool {
        self.active_turn_id.is_some()
    }

    pub fn owns_generation(&self, generation: u64) -> bool {
        self.has_active_turn() && self.generation == generation
    }

    pub fn matches_generation(&self, generation: u64) -> bool {
        self.generation == generation
    }

    pub fn owns_optional_generation(&self, expected: Option<u64>) -> bool {
        self.has_active_turn() && expected.is_none_or(|generation| self.generation == generation)
    }

    pub fn owns_turn(&self, generation: u64, turn_id: u64) -> bool {
        self.owns_generation(generation) && self.active_turn_id == Some(turn_id)
    }

    pub fn owns_active_turn_id(&self, turn_id: u64) -> bool {
        self.active_turn_id == Some(turn_id)
    }

    pub fn interrupt_requested_for(&self, generation: u64) -> bool {
        self.interrupt_requested_generation == Some(generation)
    }

    pub fn interrupt_requested_for_current(&self) -> bool {
        self.interrupt_requested_for(self.generation)
    }

    pub fn interrupt_requested_for_optional_generation(&self, expected: Option<u64>) -> bool {
        self.interrupt_requested_for_current()
            && expected.is_none_or(|generation| self.generation == generation)
    }

    #[cfg(test)]
    pub fn repeated_interrupt(&self, generation: u64, queue_paused: bool) -> bool {
        self.interrupt_requested_for(generation) && queue_paused
    }

    pub fn admits_provider_effect(&self, generation: u64, queue_paused: bool) -> bool {
        self.owns_generation(generation)
            && !queue_paused
            && !self.interrupt_requested_for(generation)
    }

    pub fn should_rollback_start(&self, generation: u64) -> bool {
        self.generation == generation && !self.interrupt_requested_for(generation)
    }

    pub fn decide_start_commit(
        &self,
        generation: u64,
        turn_id: u64,
        queue_paused: bool,
    ) -> RuntimeTurnStartCommit {
        if !self.owns_generation(generation) || self.active_turn_id != Some(turn_id) {
            return RuntimeTurnStartCommit::Superseded;
        }
        if self.interrupt_requested_for(generation) {
            return RuntimeTurnStartCommit::Interrupted;
        }
        if queue_paused {
            return RuntimeTurnStartCommit::Paused;
        }
        RuntimeTurnStartCommit::Commit
    }

    pub fn terminal_matches_current_or_last(&self) -> bool {
        self.active_turn_id
            .or(self.last_turn_id)
            .is_some_and(|turn_id| self.terminal_turn_id == Some(turn_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_fences_interrupt_and_terminal_transitions() {
        let mut turn = RuntimeTurn::default();
        let first = turn.register_start(7);
        assert_eq!(first, 1);
        assert_eq!(turn.request_interrupt(), RuntimeTurnOwnership::Current);
        assert!(turn.interrupt_requested_for_current());
        assert_eq!(turn.mark_terminal(8), RuntimeTurnOwnership::Superseded);
        assert!(turn.has_active_turn());
        assert_eq!(turn.mark_terminal(7), RuntimeTurnOwnership::Current);
        assert!(turn.has_active_turn());
        assert_eq!(turn.seal_terminal(7), RuntimeTurnOwnership::Current);
        assert!(!turn.has_active_turn());
        assert!(turn.terminal_matches_current_or_last());

        let second = turn.register_start(8);
        assert_eq!(second, 2);
        assert!(!turn.interrupt_requested_for_current());
        assert!(!turn.terminal_matches_current_or_last());
    }

    #[test]
    fn stale_generation_never_owns_a_new_turn() {
        let mut turn = RuntimeTurn::default();
        let stale = turn.register_start(1);
        turn.register_start(2);
        assert!(!turn.owns_generation(stale));
        assert!(turn.owns_generation(2));
        assert_eq!(turn.active_turn_id(), Some(2));
    }

    #[test]
    fn provider_effect_and_start_commit_share_the_turn_fence() {
        let mut turn = RuntimeTurn::default();
        let generation = turn.register_start(4);
        assert!(turn.admits_provider_effect(generation, false));
        assert_eq!(
            turn.decide_start_commit(generation, 4, false),
            RuntimeTurnStartCommit::Commit
        );
        turn.request_interrupt();
        assert!(!turn.admits_provider_effect(generation, false));
        assert_eq!(
            turn.decide_start_commit(generation, 4, false),
            RuntimeTurnStartCommit::Interrupted
        );
        assert!(!turn.should_rollback_start(generation));
        assert!(turn.repeated_interrupt(generation, true));
    }

    #[test]
    fn trailing_fatal_is_correlated_with_the_completed_crash() {
        let mut turn = RuntimeTurn::default();
        turn.register_start(4);
        assert!(turn.admits_trailing_fatal_wait(true));
        turn.release();
        turn.defer_trailing_fatal(Some("boom".into()));
        assert_eq!(
            turn.observe_fatal("other"),
            RuntimeFatalObservation::Unrelated
        );

        turn.defer_trailing_fatal(Some("boom".into()));
        assert_eq!(
            turn.observe_fatal("boom"),
            RuntimeFatalObservation::MatchesCompletedCrash
        );

        turn.register_start(5);
        assert_eq!(
            turn.observe_fatal("new crash"),
            RuntimeFatalObservation::CompleteCurrentTurn
        );
    }
}
