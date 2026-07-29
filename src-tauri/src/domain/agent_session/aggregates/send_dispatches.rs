use std::collections::HashMap;

/// Process-local ownership count for one accepted provider effect.
///
/// The registry is a bounded domain aggregate: callers may retain leases, but
/// only this type decides how duplicate claims and final release change the
/// canonical in-process ownership fact.
#[derive(Debug, Default)]
pub struct SendClaimRegistry {
    counts: HashMap<String, usize>,
}

impl SendClaimRegistry {
    pub fn acquire(&mut self, obligation_id: &str) {
        let count = self.counts.entry(obligation_id.to_string()).or_default();
        *count = count.saturating_add(1);
    }

    pub fn release(&mut self, obligation_id: &str) {
        let should_remove = self.counts.get_mut(obligation_id).is_some_and(|count| {
            *count = count.saturating_sub(1);
            *count == 0
        });
        if should_remove {
            self.counts.remove(obligation_id);
        }
    }

    pub fn owns(&self, obligation_id: &str) -> bool {
        self.counts.contains_key(obligation_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispatchState {
    Active { redrive_requested: bool },
    Parked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendDispatchAdmission {
    Acquired,
    RedriveParked,
    AlreadyActive,
}

/// Process-local dispatch aggregate for accepted send obligations.
///
/// It owns duplicate suppression, parked redrive activation, and the
/// one-shot retry latch. The gateway only stores this aggregate behind a
/// mutex and executes the returned decision.
#[derive(Debug, Default)]
pub struct SendDispatchRegistry {
    states: HashMap<String, DispatchState>,
}

impl SendDispatchRegistry {
    #[cfg(test)]
    pub fn try_acquire(&mut self, obligation_id: &str) -> bool {
        if self.states.contains_key(obligation_id) {
            return false;
        }
        self.states.insert(
            obligation_id.to_string(),
            DispatchState::Active {
                redrive_requested: false,
            },
        );
        true
    }

    pub fn admit(&mut self, obligation_id: &str, queued_effect: bool) -> SendDispatchAdmission {
        match self.states.get_mut(obligation_id) {
            None => {
                self.states.insert(
                    obligation_id.to_string(),
                    DispatchState::Active {
                        redrive_requested: false,
                    },
                );
                SendDispatchAdmission::Acquired
            }
            Some(DispatchState::Active { redrive_requested }) => {
                if queued_effect {
                    *redrive_requested = true;
                }
                SendDispatchAdmission::AlreadyActive
            }
            Some(state @ DispatchState::Parked) if queued_effect => {
                *state = DispatchState::Active {
                    redrive_requested: false,
                };
                SendDispatchAdmission::RedriveParked
            }
            Some(DispatchState::Parked) => SendDispatchAdmission::AlreadyActive,
        }
    }

    /// Returns whether a concurrent recovery request requires one more drain
    /// attempt. Otherwise the dispatch is retired.
    pub fn finish_no_work(&mut self, obligation_id: &str) -> bool {
        match self.states.get_mut(obligation_id) {
            Some(DispatchState::Active { redrive_requested }) if *redrive_requested => {
                *redrive_requested = false;
                true
            }
            _ => {
                self.states.remove(obligation_id);
                false
            }
        }
    }

    /// Returns whether a concurrent recovery request requires one more drain
    /// attempt. Otherwise the accepted item remains parked.
    pub fn finish_blocked(&mut self, obligation_id: &str) -> bool {
        match self.states.get_mut(obligation_id) {
            Some(DispatchState::Active { redrive_requested }) if *redrive_requested => {
                *redrive_requested = false;
                true
            }
            Some(state @ DispatchState::Active { .. }) => {
                *state = DispatchState::Parked;
                false
            }
            Some(DispatchState::Parked) | None => false,
        }
    }

    pub fn release(&mut self, obligation_id: &str) {
        self.states.remove(obligation_id);
    }

    pub fn owns(&self, obligation_id: &str) -> bool {
        self.states.contains_key(obligation_id)
    }

    pub fn is_parked(&self, obligation_id: &str) -> bool {
        self.states.get(obligation_id) == Some(&DispatchState::Parked)
    }

    pub fn release_if_parked(&mut self, obligation_id: &str) {
        if self.is_parked(obligation_id) {
            self.states.remove(obligation_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_count_releases_only_the_last_owner() {
        let mut registry = SendClaimRegistry::default();
        registry.acquire("obligation");
        registry.acquire("obligation");
        registry.release("obligation");
        assert!(registry.owns("obligation"));
        registry.release("obligation");
        assert!(!registry.owns("obligation"));
    }

    #[test]
    fn parked_dispatch_is_reactivated_only_for_a_queued_effect() {
        let mut registry = SendDispatchRegistry::default();
        assert_eq!(
            registry.admit("obligation", true),
            SendDispatchAdmission::Acquired
        );
        assert!(!registry.finish_blocked("obligation"));
        assert!(registry.is_parked("obligation"));
        assert_eq!(
            registry.admit("obligation", false),
            SendDispatchAdmission::AlreadyActive
        );
        assert!(registry.is_parked("obligation"));
        assert_eq!(
            registry.admit("obligation", true),
            SendDispatchAdmission::RedriveParked
        );
        assert!(!registry.is_parked("obligation"));
    }

    #[test]
    fn concurrent_redrive_is_consumed_once_before_parking_or_retirement() {
        let mut registry = SendDispatchRegistry::default();
        assert_eq!(
            registry.admit("obligation", true),
            SendDispatchAdmission::Acquired
        );
        assert_eq!(
            registry.admit("obligation", true),
            SendDispatchAdmission::AlreadyActive
        );
        assert!(registry.finish_no_work("obligation"));
        assert!(!registry.finish_no_work("obligation"));
        assert!(!registry.owns("obligation"));
    }
}
