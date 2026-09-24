pub(crate) trait FileWatchGateway: Send + Sync {
    fn start(&self, path: &str) -> Result<u64, String>;
    fn start_tree(&self, path: &str) -> Result<u64, String> {
        self.start(path)
    }
    fn stop(&self, watcher_id: u64) -> Result<(), String>;
}
