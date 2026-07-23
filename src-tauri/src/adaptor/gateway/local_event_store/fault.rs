//! Fault injection hooks for the writer worker and reply path.
//!
//! Production composition uses the default no-op injector; tests arm faults
//! to simulate storage errors before COMMIT, crashes between COMMIT and the
//! fresh readback, dropped reply channels, and a stopped writer worker.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::adaptor::gateway::local_event_store::authority::AuthorityFaultPoint;

#[derive(Debug, Default)]
pub struct FaultInjector {
    fail_before_begin: AtomicUsize,
    /// One-based participant-write countdown. Zero means disabled; a value
    /// of N trips immediately after the Nth event/state participant write.
    fail_after_participant_write: AtomicUsize,
    fail_before_commit: AtomicUsize,
    crash_after_commit_before_readback: AtomicUsize,
    drop_reply: AtomicUsize,
    stop_worker: AtomicBool,
    authority_cutover_fault: AtomicUsize,
}

impl FaultInjector {
    pub fn new() -> Self {
        Self::default()
    }

    fn take(counter: &AtomicUsize) -> bool {
        counter
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_sub(1)
            })
            .is_ok()
    }

    /// Storage failure before `BEGIN IMMEDIATE` (nothing written).
    #[cfg(test)]
    pub fn arm_fail_before_begin(&self) {
        self.fail_before_begin.fetch_add(1, Ordering::SeqCst);
    }

    pub fn take_fail_before_begin(&self) -> bool {
        Self::take(&self.fail_before_begin)
    }

    /// Storage failure after participant writes but before COMMIT
    /// (transaction rolls back).
    #[cfg(test)]
    pub fn arm_fail_after_participant_write(&self) {
        self.arm_fail_after_participant_write_number(1);
    }

    /// Storage failure immediately after the selected one-based participant
    /// write. This makes the atomicity oracle cover every partial prefix of a
    /// heterogeneous batch, rather than only the final pre-COMMIT boundary.
    #[cfg(test)]
    pub fn arm_fail_after_participant_write_number(&self, write_number: usize) {
        assert!(write_number > 0, "participant write number is one-based");
        self.fail_after_participant_write
            .store(write_number, Ordering::SeqCst);
    }

    pub fn take_fail_after_participant_write(&self) -> bool {
        self.fail_after_participant_write
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_sub(1)
            })
            .is_ok_and(|previous| previous == 1)
    }

    /// Storage failure at COMMIT boundary before COMMIT executes.
    #[cfg(test)]
    pub fn arm_fail_before_commit(&self) {
        self.fail_before_commit.fetch_add(1, Ordering::SeqCst);
    }

    pub fn take_fail_before_commit(&self) -> bool {
        Self::take(&self.fail_before_commit)
    }

    /// Crash equivalent between COMMIT and the fresh readback: the commit is
    /// durable but the caller only sees `OutcomeUnknown`.
    #[cfg(test)]
    pub fn arm_crash_after_commit_before_readback(&self) {
        self.crash_after_commit_before_readback
            .fetch_add(1, Ordering::SeqCst);
    }

    pub fn take_crash_after_commit_before_readback(&self) -> bool {
        Self::take(&self.crash_after_commit_before_readback)
    }

    /// Drop the reply channel after the commit completed (reply loss).
    #[cfg(test)]
    pub fn arm_drop_reply(&self) {
        self.drop_reply.fetch_add(1, Ordering::SeqCst);
    }

    pub fn take_drop_reply(&self) -> bool {
        Self::take(&self.drop_reply)
    }

    /// Stop the writer worker; queued and future requests observe reply loss.
    #[cfg(test)]
    pub fn stop_worker(&self) {
        self.stop_worker.store(true, Ordering::SeqCst);
    }

    pub fn worker_stopped(&self) -> bool {
        self.stop_worker.load(Ordering::SeqCst)
    }

    /// Interrupt the one-shot migration authority CAS at one exact durable
    /// boundary. Production composition never arms this hook.
    #[cfg(test)]
    pub fn arm_authority_cutover_fault(&self, point: AuthorityFaultPoint) {
        self.authority_cutover_fault.store(
            match point {
                AuthorityFaultPoint::TempWritten => 1,
                AuthorityFaultPoint::TempSynced => 2,
                AuthorityFaultPoint::AuthorityRenamed => 3,
            },
            Ordering::SeqCst,
        );
    }

    pub fn take_authority_cutover_fault(&self) -> Option<AuthorityFaultPoint> {
        match self.authority_cutover_fault.swap(0, Ordering::SeqCst) {
            1 => Some(AuthorityFaultPoint::TempWritten),
            2 => Some(AuthorityFaultPoint::TempSynced),
            3 => Some(AuthorityFaultPoint::AuthorityRenamed),
            _ => None,
        }
    }
}
