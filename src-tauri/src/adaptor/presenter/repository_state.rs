use crate::usecase::repository_state::worktree::{RepositoryStateNotifier, SnapshotNotification};
use crate::usecase::state_subscription::{StateChangeSource, StateSubscriptionOutputRef};

pub struct ClientRepositoryStateNotifier {
    publisher: StateSubscriptionOutputRef,
}

impl ClientRepositoryStateNotifier {
    pub fn new(publisher: StateSubscriptionOutputRef) -> Self {
        Self { publisher }
    }
}

impl RepositoryStateNotifier for ClientRepositoryStateNotifier {
    fn snapshot_changed(&self, notification: SnapshotNotification) {
        self.publisher
            .invalidate(StateChangeSource::Repository(notification.worktree_paths));
    }
}

#[cfg(test)]
#[path = "repository_state_test.rs"]
mod repository_state_tests;
