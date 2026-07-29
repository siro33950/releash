#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RuntimeQueuePause {
    paused_at: Option<f64>,
}

impl RuntimeQueuePause {
    pub fn blocked_by_durable_or_local(durable_paused: bool, local_paused: bool) -> bool {
        durable_paused || local_paused
    }

    pub fn restore(paused_at: Option<f64>) -> Self {
        Self { paused_at }
    }

    pub fn paused_at(self) -> Option<f64> {
        self.paused_at
    }

    pub fn is_paused(self) -> bool {
        self.paused_at.is_some()
    }

    pub fn pause(&mut self, at: f64) {
        self.paused_at = Some(at);
    }

    pub fn replace(&mut self, paused_at: Option<f64>) {
        self.paused_at = paused_at;
    }

    pub fn merge_durable_observation(&mut self, paused_at: Option<f64>) {
        if paused_at.is_some() {
            self.paused_at = paused_at;
        }
    }

    pub fn resume_if_matches(&mut self, expected_paused_at: f64) -> bool {
        if self.paused_at != Some(expected_paused_at) {
            return false;
        }
        self.paused_at = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_resume_cannot_clear_a_newer_pause() {
        let mut state = RuntimeQueuePause::restore(Some(2.0));
        assert!(!state.resume_if_matches(1.0));
        assert_eq!(state.paused_at(), Some(2.0));
        assert!(state.resume_if_matches(2.0));
        assert!(!state.is_paused());
    }

    #[test]
    fn missing_durable_observation_does_not_erase_a_local_commit_mirror() {
        let mut state = RuntimeQueuePause::restore(Some(1.0));
        state.merge_durable_observation(None);
        assert_eq!(state.paused_at(), Some(1.0));
        assert!(RuntimeQueuePause::blocked_by_durable_or_local(false, true));
        assert!(!RuntimeQueuePause::blocked_by_durable_or_local(
            false, false
        ));
    }
}
