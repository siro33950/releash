pub type WatchChangeHandler = std::sync::Arc<dyn Fn(Result<(), String>) + Send + Sync>;

pub trait FileWatchGateway: Send + Sync {
    /// `path` 以下の木を監視し、変化のたびに `on_change` を呼ぶ。存在しない path は最も近い
    /// 既存の祖先から監視し、開始直後にも `on_change` を 1 回呼ぶ。
    fn start_tree(&self, path: &str, on_change: WatchChangeHandler) -> Result<u64, String>;
    fn stop(&self, watcher_id: u64) -> Result<(), String>;
}
