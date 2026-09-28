pub(crate) type WatchChangeHandler = std::sync::Arc<dyn Fn() + Send + Sync>;

pub(crate) trait FileWatchGateway: Send + Sync {
    fn start(&self, path: &str) -> Result<u64, String>;
    /// `path` 以下の木を監視し、変化のたびに `on_change` を呼ぶ。存在しない path は最も近い
    /// 既存の祖先から監視し、開始直後にも `on_change` を 1 回呼ぶ。
    fn start_tree(&self, path: &str, _on_change: WatchChangeHandler) -> Result<u64, String> {
        self.start(path)
    }
    fn stop(&self, watcher_id: u64) -> Result<(), String>;
    /// 登録結果を返せない監視の資源を破棄する。存在しないidも許容する。
    fn release(&self, watcher_id: u64);
}
