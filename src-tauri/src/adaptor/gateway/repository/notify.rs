//! repo_paths 変更通知 gateway（`RepoPathsNotifier` の Tauri 実装）。
//!
//! usecase が決めた「成功時に現在の一覧 payload で通知する」gating の結果を、
//! Tauri の `repo-paths-changed` イベントとしてフロントへ emit する送信 infra。
//! 送信手段の差し替え（テストでの fake、将来の WS 送信等）を可能にするため、
//! domain の `RepoPathsNotifier` port 経由で usecase から呼ばれる。

use tauri::{Emitter, Runtime};

use crate::domain::repository::RepoPathsNotifier;

/// repo_paths 変更時にフロントへ emit する Tauri イベント名。
///
/// design.md の不変契約。リテラル散在による typo を防ぐため定数化する。
pub const REPO_PATHS_CHANGED_EVENT: &str = "repo-paths-changed";

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
        let _ = self.app.emit(REPO_PATHS_CHANGED_EVENT, &paths);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// イベント名は design.md の不変契約。値が変わると既存フロントの
    /// `listen("repo-paths-changed")` と齟齬が出るため pin する。
    #[test]
    fn repo_paths_changed_event_name_is_pinned() {
        assert_eq!(REPO_PATHS_CHANGED_EVENT, "repo-paths-changed");
    }
}
