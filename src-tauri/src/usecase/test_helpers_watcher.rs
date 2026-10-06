use super::*;
use crate::domain::repository::file_watcher::WatchChangeHandler;
use std::sync::Mutex;

#[derive(Default)]
pub struct Files(pub Mutex<Vec<String>>);
impl FileWatchGateway for Files {
    fn start_tree(&self, path: &str, _on_change: WatchChangeHandler) -> Result<u64, String> {
        self.0.lock().unwrap().push(path.into());
        if path == "/missing" {
            Err("missing path".into())
        } else {
            Ok(42)
        }
    }
    fn stop(&self, watcher_id: u64) -> Result<(), String> {
        self.0.lock().unwrap().push(watcher_id.to_string());
        if watcher_id == 42 {
            Ok(())
        } else {
            Err("unknown watcher".into())
        }
    }
}
