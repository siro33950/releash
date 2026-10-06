use crate::domain::repository::file_watcher::{FileWatchGateway, WatchChangeHandler};
use std::sync::Mutex;

#[derive(Default)]
pub struct SubscriptionFiles {
    pub(crate) next: std::sync::atomic::AtomicU64,
    pub active: Mutex<std::collections::HashSet<u64>>,
    pub(crate) fail_stop: std::sync::atomic::AtomicBool,
}
impl FileWatchGateway for SubscriptionFiles {
    fn start_tree(&self, path: &str, _on_change: WatchChangeHandler) -> Result<u64, String> {
        if path == "/missing" {
            return Err("missing path".into());
        }
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.active.lock().unwrap().insert(id);
        Ok(id)
    }
    fn stop(&self, id: u64) -> Result<(), String> {
        if self.fail_stop.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("stop failed".into());
        }
        self.active.lock().unwrap().remove(&id);
        Ok(())
    }
}
