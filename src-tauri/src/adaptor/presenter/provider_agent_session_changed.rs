use tauri::Emitter;

use crate::usecase::agent_session::ProviderAgentSessionChangeNotifier;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderAgentSessionChangedPayload {
    worktree_path: String,
}

/// standalone AgentSession 読み取りモデルの変化を
/// `provider-agent-session-changed` Tauri イベントとして通知する。
pub(crate) struct TauriProviderAgentSessionChangeNotifier<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> TauriProviderAgentSessionChangeNotifier<R> {
    pub(crate) fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: tauri::Runtime> ProviderAgentSessionChangeNotifier
    for TauriProviderAgentSessionChangeNotifier<R>
{
    fn provider_agent_session_changed(&self, worktree_path: &str) {
        let _ = self.app.emit(
            "provider-agent-session-changed",
            ProviderAgentSessionChangedPayload {
                worktree_path: worktree_path.to_string(),
            },
        );
    }
}
