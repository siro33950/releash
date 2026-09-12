//! repo_paths 変更通知 gateway（`RepoPathsNotifier` の Tauri 実装）。
//!
//! usecase が決めた「成功時に現在の一覧 payload で通知する」gating の結果を、
//! Tauri の `repo-paths-changed` イベントとしてフロントへ emit する送信 infra。
//! 送信手段の差し替え（テストでの fake、将来の WS 送信等）を可能にするため、
//! domain の `RepoPathsNotifier` port 経由で usecase から呼ばれる。

use crate::adaptor::gateway::push::BackendPush;
use tauri::Runtime;

use crate::domain::repository::RepoPathsNotifier;

pub struct RepoPathsNotifyGateway<R: Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: Runtime> RepoPathsNotifyGateway<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> RepoPathsNotifier for RepoPathsNotifyGateway<R> {
    fn notify_changed(&self, paths: Vec<String>) {
        BackendPush::RepoPathsChanged(&paths).emit(&self.app);
    }
}
