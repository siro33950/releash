pub(crate) trait FileWatchGateway: Send + Sync {
    fn start(&self, path: &str) -> Result<u64, String>;
    fn start_tree(&self, path: &str) -> Result<u64, String> {
        self.start(path)
    }
    fn stop(&self, watcher_id: u64) -> Result<(), String>;
    /// 登録結果を返せない監視の資源を破棄する。存在しないidも許容する。
    fn release(&self, watcher_id: u64);
}
