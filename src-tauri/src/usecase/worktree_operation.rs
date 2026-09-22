use std::collections::HashMap;
use std::sync::{Arc, Weak};

use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::domain::repository::worktree_operation::{
    WorktreeOperationLease, WorktreeOperationLocks, WorktreeOperationState,
};
use crate::domain::repository::RepositoryError;

pub(crate) struct WorktreeOperations {
    worktrees: Mutex<HashMap<String, Weak<OperationSlot>>>,
    locks: Arc<dyn WorktreeOperationLocks>,
}

#[derive(Default)]
struct OperationSlot {
    state: Mutex<WorktreeOperationState>,
    changed: Notify,
}

pub struct WorktreeMutationGuard {
    slot: Arc<OperationSlot>,
    _lease: Box<dyn WorktreeOperationLease>,
}
pub struct WorktreeDeletionGuard(
    Vec<Arc<OperationSlot>>,
    Vec<Box<dyn WorktreeOperationLease>>,
);

impl WorktreeOperations {
    pub(crate) fn new(locks: Arc<dyn WorktreeOperationLocks>) -> Self {
        Self {
            worktrees: Mutex::default(),
            locks,
        }
    }

    fn slot(&self, identity: &str) -> Arc<OperationSlot> {
        let mut slots = self.worktrees.lock();
        slots.retain(|_, slot| slot.strong_count() > 0);
        if let Some(slot) = slots.get(identity).and_then(Weak::upgrade) {
            return slot;
        }
        let slot = Arc::new(OperationSlot::default());
        slots.insert(identity.to_string(), Arc::downgrade(&slot));
        slot
    }

    pub(crate) fn mutate(&self, identity: &str) -> Result<WorktreeMutationGuard, RepositoryError> {
        let lease = self.locks.mutation(identity)?;
        let slot = self.slot(identity);
        slot.state.lock().begin_mutation()?;
        Ok(WorktreeMutationGuard {
            slot,
            _lease: lease,
        })
    }

    #[cfg(test)]
    pub(crate) async fn delete(
        &self,
        identity: &str,
    ) -> Result<WorktreeDeletionGuard, RepositoryError> {
        self.delete_many(&[identity.to_string()]).await
    }

    pub(crate) async fn delete_many(
        &self,
        identities: &[String],
    ) -> Result<WorktreeDeletionGuard, RepositoryError> {
        let mut guard = WorktreeDeletionGuard(Vec::new(), Vec::new());
        for identity in identities {
            let slot = self.slot(identity);
            slot.state.lock().begin_deletion()?;
            guard.0.push(slot);
        }
        for identity in identities {
            guard.1.push(self.locks.deletion(identity).await?);
        }
        for slot in &guard.0 {
            loop {
                let changed = slot.changed.notified();
                if slot.state.lock().ready_to_delete() {
                    break;
                }
                changed.await;
            }
        }
        Ok(guard)
    }
}

impl Drop for WorktreeMutationGuard {
    fn drop(&mut self) {
        self.slot.state.lock().finish_mutation();
        self.slot.changed.notify_one();
    }
}

impl Drop for WorktreeDeletionGuard {
    fn drop(&mut self) {
        for slot in &self.0 {
            slot.state.lock().finish_deletion();
        }
    }
}

#[cfg(test)]
#[path = "worktree_operation_test.rs"]
mod worktree_operation_tests;

#[cfg(test)]
impl Default for WorktreeOperations {
    fn default() -> Self {
        struct Locks;
        impl WorktreeOperationLease for Locks {}
        #[async_trait::async_trait]
        impl WorktreeOperationLocks for Locks {
            fn mutation(
                &self,
                _: &str,
            ) -> Result<Box<dyn WorktreeOperationLease>, RepositoryError> {
                Ok(Box::new(Locks))
            }
            async fn deletion(
                &self,
                _: &str,
            ) -> Result<Box<dyn WorktreeOperationLease>, RepositoryError> {
                Ok(Box::new(Locks))
            }
        }
        Self::new(Arc::new(Locks))
    }
}
