#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeAdmissionRejection {
    ShutdownInProgress,
}

/// Aggregate for process-wide agent-session command admission.
///
/// Synchronization and waiting stay in the caller; this aggregate alone owns
/// the acceptance rule and active-operation transition.
#[derive(Debug, Default)]
pub struct RuntimeAdmission {
    shutting_down: bool,
    active_operations: usize,
}

#[derive(Debug, Default)]
pub struct RuntimeSessionAdmission {
    closing: bool,
}

impl RuntimeSessionAdmission {
    #[cfg(test)]
    pub fn begin_closing(&mut self) {
        self.closing = true;
    }

    #[cfg(test)]
    pub fn cancel_closing(&mut self) {
        self.closing = false;
    }

    pub fn is_closing(&self) -> bool {
        self.closing
    }

    pub fn accepts_work(&self) -> bool {
        !self.closing
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueDrainFacts {
    pub closing: bool,
    pub backend_recovery_active: bool,
    pub active_turn: bool,
    pub queue_paused: bool,
}

pub fn queue_drain_is_admitted(facts: QueueDrainFacts) -> bool {
    !facts.closing && !facts.backend_recovery_active && !facts.active_turn && !facts.queue_paused
}

pub fn accepted_effect_delivery_is_admitted(
    closing: bool,
    backend_recovery_active: bool,
    canonical_head: bool,
) -> bool {
    !closing && !backend_recovery_active && canonical_head
}

#[cfg(test)]
pub fn provider_session_is_confirmed(
    has_runtime: bool,
    provider_session_established: bool,
) -> bool {
    has_runtime && provider_session_established
}

impl RuntimeAdmission {
    pub fn admit(&mut self) -> Result<(), RuntimeAdmissionRejection> {
        if self.shutting_down {
            return Err(RuntimeAdmissionRejection::ShutdownInProgress);
        }
        self.active_operations = self.active_operations.saturating_add(1);
        Ok(())
    }

    pub fn release(&mut self) -> bool {
        self.active_operations = self.active_operations.saturating_sub(1);
        self.active_operations == 0
    }

    #[cfg(test)]
    pub fn begin_shutdown(&mut self) {
        self.shutting_down = true;
    }

    #[cfg(test)]
    pub fn cancel_shutdown(&mut self) {
        self.shutting_down = false;
    }

    #[cfg(test)]
    pub fn is_idle(&self) -> bool {
        self.active_operations == 0
    }

    #[cfg(test)]
    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_rejects_new_work_and_waits_for_existing_work() {
        let mut admission = RuntimeAdmission::default();
        admission.admit().unwrap();
        admission.begin_shutdown();
        assert_eq!(
            admission.admit(),
            Err(RuntimeAdmissionRejection::ShutdownInProgress)
        );
        assert!(!admission.is_idle());
        assert!(admission.release());
        assert!(admission.is_idle());
    }

    #[test]
    fn session_close_fences_new_process_local_work_until_cancelled() {
        let mut admission = RuntimeSessionAdmission::default();
        assert!(!admission.is_closing());
        admission.begin_closing();
        assert!(admission.is_closing());
        admission.cancel_closing();
        assert!(!admission.is_closing());
    }

    #[test]
    fn queue_drain_and_accepted_effect_share_the_close_and_recovery_fences() {
        assert!(queue_drain_is_admitted(QueueDrainFacts {
            closing: false,
            backend_recovery_active: false,
            active_turn: false,
            queue_paused: false,
        }));
        assert!(!queue_drain_is_admitted(QueueDrainFacts {
            closing: false,
            backend_recovery_active: true,
            active_turn: false,
            queue_paused: false,
        }));
        assert!(!accepted_effect_delivery_is_admitted(false, false, false));
        assert!(accepted_effect_delivery_is_admitted(false, false, true));
    }
}
