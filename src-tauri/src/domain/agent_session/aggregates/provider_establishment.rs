#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderEstablishmentObservation {
    Start,
    AlreadyEstablished,
    AlreadyPending,
    Conflict,
}

/// Process-local ownership aggregate for one provider-session establishment
/// observation. Durable provider identity remains owned by the session
/// projection; this type only fences retries within the current runtime epoch.
#[derive(Debug, Default)]
struct ProviderEstablishmentTracker {
    established: bool,
    pending_observation_id: Option<String>,
}

impl ProviderEstablishmentTracker {
    pub fn reset(&mut self) {
        self.established = false;
        self.pending_observation_id = None;
    }

    pub fn observe(&mut self, observation_id: &str) -> ProviderEstablishmentObservation {
        if self.established {
            return ProviderEstablishmentObservation::AlreadyEstablished;
        }
        match self.pending_observation_id.as_deref() {
            Some(current) if current == observation_id => {
                ProviderEstablishmentObservation::AlreadyPending
            }
            Some(_) => ProviderEstablishmentObservation::Conflict,
            None => {
                self.pending_observation_id = Some(observation_id.to_string());
                ProviderEstablishmentObservation::Start
            }
        }
    }

    pub fn is_established(&self) -> bool {
        self.established
    }

    pub fn has_pending_observation(&self) -> bool {
        self.pending_observation_id.is_some()
    }

    pub fn owns(&self, observation_id: &str) -> bool {
        self.pending_observation_id.as_deref() == Some(observation_id)
    }

    pub fn clear_if_owned(&mut self, observation_id: &str) -> bool {
        if !self.owns(observation_id) {
            return false;
        }
        self.pending_observation_id = None;
        true
    }

    pub fn settle_if_owned(&mut self, observation_id: &str) -> bool {
        if !self.clear_if_owned(observation_id) {
            return false;
        }
        self.established = true;
        true
    }

    pub fn mark_established(&mut self) {
        self.pending_observation_id = None;
        self.established = true;
    }
}

/// Process-local provider runtime aggregate. The epoch fences every async
/// provider observation, while the establishment tracker owns the retry state
/// within that epoch.
#[derive(Debug, Default)]
pub struct ProviderRuntime {
    epoch: u64,
    establishment: ProviderEstablishmentTracker,
}

impl ProviderRuntime {
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn owns_epoch(&self, expected: u64) -> bool {
        self.epoch == expected
    }

    pub fn bump_epoch(&mut self) -> u64 {
        self.epoch = self.epoch.saturating_add(1);
        self.establishment.reset();
        self.epoch
    }

    pub fn session_is_established(&self) -> bool {
        self.establishment.is_established()
    }

    pub fn has_pending_establishment(&self) -> bool {
        self.establishment.has_pending_observation()
    }

    pub fn establishment_is_current(&self, observation_id: &str) -> bool {
        self.establishment.owns(observation_id)
    }

    pub fn observe_establishment(
        &mut self,
        observation_id: &str,
    ) -> ProviderEstablishmentObservation {
        self.establishment.observe(observation_id)
    }

    pub fn clear_establishment_if_current(&mut self, observation_id: &str) -> bool {
        self.establishment.clear_if_owned(observation_id)
    }

    pub fn settle_establishment_if_current(&mut self, observation_id: &str) -> bool {
        self.establishment.settle_if_owned(observation_id)
    }

    pub fn mark_session_established(&mut self) {
        self.establishment.mark_established();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_identity_is_idempotent_and_conflict_closed() {
        let mut tracker = ProviderEstablishmentTracker::default();
        assert_eq!(
            tracker.observe("one"),
            ProviderEstablishmentObservation::Start
        );
        assert_eq!(
            tracker.observe("one"),
            ProviderEstablishmentObservation::AlreadyPending
        );
        assert_eq!(
            tracker.observe("two"),
            ProviderEstablishmentObservation::Conflict
        );
        assert!(tracker.settle_if_owned("one"));
        assert!(tracker.is_established());
        assert_eq!(
            tracker.observe("one"),
            ProviderEstablishmentObservation::AlreadyEstablished
        );
        tracker.reset();
        assert!(!tracker.is_established());
        assert!(!tracker.has_pending_observation());
    }

    #[test]
    fn epoch_change_invalidates_pending_provider_establishment() {
        let mut runtime = ProviderRuntime::default();
        assert_eq!(
            runtime.observe_establishment("one"),
            ProviderEstablishmentObservation::Start
        );
        assert!(runtime.establishment_is_current("one"));

        assert_eq!(runtime.bump_epoch(), 1);
        assert!(runtime.owns_epoch(1));
        assert!(!runtime.establishment_is_current("one"));
        assert!(!runtime.has_pending_establishment());
    }
}
