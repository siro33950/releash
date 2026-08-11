use tauri::Emitter;

use crate::usecase::agent_session::AgentSessionChangeNotifier;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSessionChangedPayload {
    worktree_path: String,
}

/// standalone AgentSession 読み取りモデルの変化を
/// `agent-session-changed` Tauri イベントとして通知する。
pub(crate) struct TauriAgentSessionChangeNotifier<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> TauriAgentSessionChangeNotifier<R> {
    pub(crate) fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: tauri::Runtime> AgentSessionChangeNotifier for TauriAgentSessionChangeNotifier<R> {
    fn agent_session_changed(&self, worktree_path: &str) {
        let _ = self.app.emit(
            "agent-session-changed",
            AgentSessionChangedPayload {
                worktree_path: worktree_path.to_string(),
            },
        );
    }
}
