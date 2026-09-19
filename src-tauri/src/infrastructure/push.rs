use std::sync::Arc;
use tokio::sync::broadcast;

pub(crate) struct PushSink {
    sender: broadcast::Sender<Arc<[u8]>>,
}
impl PushSink {
    pub(crate) fn new() -> Self {
        Self {
            sender: broadcast::channel(64).0,
        }
    }
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Arc<[u8]>> {
        self.sender.subscribe()
    }
    #[cfg(feature = "desktop")]
    pub(crate) fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
    pub(crate) fn send(&self, frame: Vec<u8>) {
        let _ = self.sender.send(frame.into());
    }
}
