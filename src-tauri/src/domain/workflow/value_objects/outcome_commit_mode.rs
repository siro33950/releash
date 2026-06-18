#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeCommitMode {
    EmitProgressEvents,
    ProgressEventsAlreadyCommitted,
}

impl OutcomeCommitMode {
    pub fn should_emit_progress_events(self) -> bool {
        matches!(self, Self::EmitProgressEvents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_outcome_commit_mode_progress_event発行要否() {
        assert!(OutcomeCommitMode::EmitProgressEvents.should_emit_progress_events());
        assert!(!OutcomeCommitMode::ProgressEventsAlreadyCommitted.should_emit_progress_events());
    }
}
