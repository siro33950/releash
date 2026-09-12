use crate::adaptor::gateway::push::{AgentSessionChangedPayload, BackendPush};

use crate::usecase::agent_session::AgentSessionChangeNotifier;

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
        BackendPush::AgentSessionChanged(AgentSessionChangedPayload { worktree_path })
            .emit(&self.app);
    }
}
