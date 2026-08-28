//! Fault injection hooks for the writer worker and reply path.
//!
//! Production composition uses the default no-op injector; tests arm faults
//! to simulate storage errors before COMMIT, crashes between COMMIT and the
//! fresh readback, dropped reply channels, and a stopped writer worker.

use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(test)]
use std::sync::{Arc, Condvar, Mutex};
#[cfg(test)]
use std::time::Duration;

#[cfg(test)]
#[derive(Debug, Default)]
struct NodeEventAppendStallState {
    armed: bool,
    arrived: bool,
    released: bool,
}

#[cfg(test)]
#[derive(Debug, Default)]
struct NodeEventAppendStall {
    state: Mutex<NodeEventAppendStallState>,
    arrived: Condvar,
    released: Condvar,
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct NodeEventAppendStallGuard {
    stall: Arc<NodeEventAppendStall>,
}

#[cfg(test)]
impl NodeEventAppendStallGuard {
    pub(crate) fn wait_until_arrived(&self) {
        let state = self
            .stall
            .state
            .lock()
            .expect("node event append stall poisoned");
        let (state, _) = self
            .stall
            .arrived
            .wait_timeout_while(state, Duration::from_secs(10), |state| !state.arrived)
            .expect("node event append stall poisoned");
        assert!(
            state.arrived,
            "writer did not reach the node event append stall"
        );
    }

    pub(crate) fn release(&self) {
        let mut state = self
            .stall
            .state
            .lock()
            .expect("node event append stall poisoned");
        state.released = true;
        drop(state);
        self.stall.released.notify_all();
    }
}

#[cfg(test)]
impl Drop for NodeEventAppendStallGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialCreateFaultPoint {
    BeforeEvidenceCreate = 1,
    AfterPartialEvidenceWrite = 2,
    AfterEvidenceFileSync = 3,
    AfterEvidenceDirectorySync = 4,
    AfterSqliteFileCreate = 5,
    BeforeInitializationCommit = 6,
    AfterInitializationCommitReplyLoss = 7,
    AfterDatabaseSync = 8,
    BeforeEvidenceUnlink = 9,
    AfterEvidenceUnlink = 10,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceFaultPoint {
    BeforeVacuumInto = 1,
    BeforeOutputValidation = 2,
    BeforeOutputPermission = 3,
    BeforeOutputSync = 4,
    BeforeCanonicalSidecarCleanup = 5,
    BeforeReplace = 6,
    AfterReplace = 7,
}

#[derive(Debug, Default)]
pub struct FaultInjector {
    fail_before_begin: AtomicUsize,
    /// One-based participant-write countdown. Zero means disabled; a value
    /// of N trips immediately after the Nth event/state participant write.
    fail_after_participant_write: AtomicUsize,
    fail_before_commit: AtomicUsize,
    crash_after_commit_before_readback: AtomicUsize,
    drop_reply: AtomicUsize,
    schema_fail_before_begin: AtomicUsize,
    schema_fail_before_commit: AtomicUsize,
    schema_commit_reply_loss: AtomicUsize,
    schema_fail_before_readback: AtomicUsize,
    initial_create_fault_point: AtomicUsize,
    maintenance_fault_point: AtomicUsize,
    #[cfg(test)]
    node_event_append_stall: Arc<NodeEventAppendStall>,
    #[cfg(test)]
    initial_create_process_crash_point: AtomicUsize,
    #[cfg(test)]
    initial_installation_id: Mutex<Option<String>>,
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

    pub fn take_fail_before_begin(&self) -> bool {
        Self::take(&self.fail_before_begin)
    }

    pub fn take_fail_after_participant_write(&self) -> bool {
        self.fail_after_participant_write
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_sub(1)
            })
            .is_ok_and(|previous| previous == 1)
    }

    #[cfg(test)]
    pub fn arm_fail_after_participant_write(&self, participant: usize) {
        self.fail_after_participant_write
            .store(participant, Ordering::SeqCst);
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

    pub fn take_drop_reply(&self) -> bool {
        Self::take(&self.drop_reply)
    }

    #[cfg(test)]
    pub fn arm_drop_reply(&self) {
        self.drop_reply.fetch_add(1, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn arm_node_event_append_stall(&self) -> NodeEventAppendStallGuard {
        let mut state = self
            .node_event_append_stall
            .state
            .lock()
            .expect("node event append stall poisoned");
        assert!(!state.armed, "node event append stall is already armed");
        *state = NodeEventAppendStallState {
            armed: true,
            arrived: false,
            released: false,
        };
        drop(state);
        NodeEventAppendStallGuard {
            stall: Arc::clone(&self.node_event_append_stall),
        }
    }

    #[cfg(test)]
    pub(crate) fn wait_before_node_event_append_if_armed(&self) {
        let mut state = self
            .node_event_append_stall
            .state
            .lock()
            .expect("node event append stall poisoned");
        if !state.armed {
            return;
        }
        state.arrived = true;
        self.node_event_append_stall.arrived.notify_all();
        while !state.released {
            state = self
                .node_event_append_stall
                .released
                .wait(state)
                .expect("node event append stall poisoned");
        }
        *state = NodeEventAppendStallState::default();
    }

    #[cfg(not(test))]
    pub(crate) fn wait_before_node_event_append_if_armed(&self) {}

    pub fn take_schema_fail_before_begin(&self) -> bool {
        Self::take(&self.schema_fail_before_begin)
    }

    pub fn take_schema_fail_before_commit(&self) -> bool {
        Self::take(&self.schema_fail_before_commit)
    }

    #[cfg(test)]
    pub fn arm_schema_fail_before_commit(&self) {
        self.schema_fail_before_commit
            .fetch_add(1, Ordering::SeqCst);
    }

    pub fn take_schema_commit_reply_loss(&self) -> bool {
        Self::take(&self.schema_commit_reply_loss)
    }

    pub fn take_schema_fail_before_readback(&self) -> bool {
        Self::take(&self.schema_fail_before_readback)
    }

    pub fn take_initial_create_fault(&self, point: InitialCreateFaultPoint) -> bool {
        self.initial_create_fault_point
            .compare_exchange(point as usize, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub fn take_maintenance_fault(&self, point: MaintenanceFaultPoint) -> bool {
        self.maintenance_fault_point
            .compare_exchange(point as usize, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    #[cfg(test)]
    pub fn arm_maintenance_fault(&self, point: MaintenanceFaultPoint) {
        self.maintenance_fault_point
            .store(point as usize, Ordering::SeqCst);
    }

    /// Terminate the subprocess without unwinding after the production fault
    /// hook has completed the boundary's filesystem or transaction action.
    ///
    /// This is test-only so production fault behavior remains an ordinary
    /// typed error. `abort` deliberately skips Rust destructors and SQLite
    /// connection cleanup, leaving the same artifacts as an abrupt process
    /// loss for the parent acceptance test to recover.
    #[cfg(test)]
    pub fn crash_initial_create_process_if_armed(&self, point: InitialCreateFaultPoint) {
        if self
            .initial_create_process_crash_point
            .compare_exchange(point as usize, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            std::process::abort();
        }
    }

    #[cfg(test)]
    pub fn set_initial_installation_id(&self, installation_id: &str) {
        *self
            .initial_installation_id
            .lock()
            .expect("initial installation identity fault lock") = Some(installation_id.to_string());
    }

    #[cfg(test)]
    pub fn initial_installation_id(&self) -> Option<String> {
        self.initial_installation_id
            .lock()
            .expect("initial installation identity fault lock")
            .clone()
    }

    #[cfg(not(test))]
    pub fn initial_installation_id(&self) -> Option<String> {
        None
    }
}
