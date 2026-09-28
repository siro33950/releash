pub(crate) enum PushMessage {
    FileChange {
        watcher_id: u64,
        path: String,
        kind: String,
    },
}

pub(crate) trait PushOutput: Send + Sync {
    fn publish(&self, message: PushMessage);
}
