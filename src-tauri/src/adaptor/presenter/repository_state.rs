use std::sync::Arc;

use crate::usecase::push::{PushMessage, PushOutput};
use crate::usecase::repository_state::worktree::{RepositoryStateNotifier, SnapshotNotification};
use crate::usecase::state_subscription::{StateChangeSource, StateSubscriptionOutputRef};

pub struct ClientRepositoryStateNotifier {
    publisher: StateSubscriptionOutputRef,
    sink: Arc<dyn PushOutput>,
}

impl ClientRepositoryStateNotifier {
    pub fn new(sink: Arc<dyn PushOutput>, publisher: StateSubscriptionOutputRef) -> Self {
        Self { sink, publisher }
    }
}

impl RepositoryStateNotifier for ClientRepositoryStateNotifier {
    fn snapshot_changed(&self, notification: SnapshotNotification) {
        for worktree_path in &notification.worktree_paths {
            self.sink.publish(PushMessage::GitStatusChanged {
                repo_path: worktree_path.clone(),
            });
        }
        self.publisher.invalidate(StateChangeSource::Repository(
            notification.worktree_paths.clone(),
        ));
        if notification.reason.file_change {
            let path = notification.reason.path.unwrap_or_else(|| {
                notification
                    .worktree_paths
                    .first()
                    .cloned()
                    .unwrap_or_default()
            });
            for watcher_id in notification.file_watcher_ids {
                self.sink.publish(PushMessage::FileChange {
                    watcher_id,
                    path: path.clone(),
                    kind: "change".into(),
                });
            }
        }
    }
}
