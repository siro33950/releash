use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

use crate::app_data_dir::resolve_data_dir;
use crate::domain::agent_session::CODEX_FIXED_MODELS;
use crate::infrastructure::agent_session::runtime::bridge_common::{
    start_agent_session_internal, start_agent_turn, write_bridge_command, AgentProcessMap,
    CODEX_BACKEND_ID,
};
use crate::infrastructure::agent_session::runtime::{
    AgentBackend, AgentMessage, BackendRuntimeConfig, PermissionResponse, SessionConfig,
    SessionHandle,
};
use crate::usecase::agent_session::session::SessionStore;

/// Codex SDK Bridge バックエンド。
/// 実際のプロセス制御は AgentProcess bridge runtime に委譲する。
/// モデル選択肢は `CODEX_FIXED_MODELS` で完全固定する。
pub struct CodexBackend {
    #[allow(dead_code)]
    runtime: Option<Arc<dyn CodexBackendRuntime>>,
    cli_path: Option<String>,
}

pub(crate) fn configured_cli_path<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<String> {
    app.try_state::<std::sync::Arc<crate::config::AppConfig>>()
        .and_then(|cfg_state| cfg_state.get_config().ok())
        .and_then(|cfg| cfg.agents.codex.cli_path)
        .filter(|path| !path.trim().is_empty())
}

fn configured_cli_path_from_config(
    app_config: Option<&crate::config::AppConfig>,
) -> Option<String> {
    app_config
        .and_then(|cfg_state| cfg_state.get_config().ok())
        .and_then(|cfg| cfg.agents.codex.cli_path)
        .filter(|path| !path.trim().is_empty())
}

#[allow(dead_code)]
impl CodexBackend {
    pub fn new() -> Self {
        Self {
            runtime: None,
            cli_path: None,
        }
    }

    pub fn with_agent_process_runtime(
        app: AppHandle,
        handles: Arc<Mutex<AgentProcessMap>>,
        session_store: Arc<SessionStore>,
    ) -> Self {
        let cli_path = configured_cli_path(&app);
        Self {
            runtime: Some(Arc::new(AgentProcessCodexRuntime {
                app,
                handles,
                session_store,
            })),
            cli_path,
        }
    }

    fn runtime(&self) -> Result<Arc<dyn CodexBackendRuntime>, String> {
        self.runtime.clone().ok_or_else(|| {
            "CodexBackend runtime is not attached; build the registry with app runtime".to_string()
        })
    }

    fn cli_path(&self) -> String {
        self.cli_path.clone().unwrap_or_else(|| "codex".to_string())
    }
}

#[allow(dead_code)]
#[async_trait]
trait CodexBackendRuntime: Send + Sync {
    async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String>;
    async fn send_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String>;
    async fn interrupt(&self, session: &SessionHandle) -> Result<(), String>;
    async fn respond_permission(
        &self,
        session: &SessionHandle,
        response: PermissionResponse,
    ) -> Result<(), String>;
    async fn close_session(&self, session: &SessionHandle) -> Result<(), String>;
}

#[allow(dead_code)]
struct AgentProcessCodexRuntime {
    app: AppHandle,
    handles: Arc<Mutex<AgentProcessMap>>,
    session_store: Arc<SessionStore>,
}

#[async_trait]
impl CodexBackendRuntime for AgentProcessCodexRuntime {
    async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String> {
        start_agent_session_internal(
            &self.app,
            &self.handles,
            &self.session_store,
            &config.chat_session_id,
            &config.cwd,
            config.permission_mode,
            config.system_prompt,
        )
        .await?;

        Ok(SessionHandle {
            chat_session_id: config.chat_session_id,
            backend_id: CODEX_BACKEND_ID.to_string(),
        })
    }

    async fn send_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String> {
        ensure_codex_session(session)?;
        let data_dir = resolve_data_dir(&self.app)?;
        let stored_session = self
            .session_store
            .get_session(&data_dir, &session.chat_session_id)?
            .ok_or_else(|| format!("Session not found: {}", session.chat_session_id))?;

        start_agent_turn(
            &self.app,
            &self.handles,
            &self.session_store,
            &session.chat_session_id,
            &stored_session.worktree_path,
            &message.permission_mode,
            &message.content,
            &message.streaming_message_id,
            &message.images,
        )
        .await
    }

    async fn interrupt(&self, session: &SessionHandle) -> Result<(), String> {
        ensure_codex_session(session)?;
        write_bridge_command(
            &self.handles,
            &session.chat_session_id,
            json!({"type": "interrupt"}),
        )
        .await
    }

    async fn respond_permission(
        &self,
        session: &SessionHandle,
        response: PermissionResponse,
    ) -> Result<(), String> {
        ensure_codex_session(session)?;
        if response.behavior != "allow" && response.behavior != "deny" {
            return Err(format!("Invalid behavior: {}", response.behavior));
        }

        let mut result = json!({ "behavior": response.behavior });
        if let Some(message) = response.message {
            result["message"] = serde_json::Value::String(message);
        }
        if let Some(updated_input) = response.updated_input {
            match serde_json::from_str::<serde_json::Value>(&updated_input) {
                Ok(parsed) => result["updatedInput"] = parsed,
                Err(e) => log::warn!("Failed to parse updated_input JSON: {e}"),
            }
        }

        write_bridge_command(
            &self.handles,
            &session.chat_session_id,
            json!({
                "type": "permission_response",
                "request_id": response.request_id,
                "result": result,
            }),
        )
        .await
    }

    async fn close_session(&self, session: &SessionHandle) -> Result<(), String> {
        ensure_codex_session(session)?;
        write_bridge_command(
            &self.handles,
            &session.chat_session_id,
            json!({"type": "close"}),
        )
        .await
    }
}

#[allow(dead_code)]
fn ensure_codex_session(session: &SessionHandle) -> Result<(), String> {
    if session.backend_id == CODEX_BACKEND_ID {
        return Ok(());
    }
    Err(format!(
        "Session {} belongs to backend {}, not {}",
        session.chat_session_id, session.backend_id, CODEX_BACKEND_ID
    ))
}

#[async_trait]
impl AgentBackend for CodexBackend {
    fn id(&self) -> &str {
        CODEX_BACKEND_ID
    }

    fn name(&self) -> &str {
        "Codex"
    }

    async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String> {
        self.runtime()?.start_session(config).await
    }

    async fn send_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String> {
        self.runtime()?.send_message(session, message).await
    }

    async fn interrupt(&self, session: &SessionHandle) -> Result<(), String> {
        self.runtime()?.interrupt(session).await
    }

    async fn respond_permission(
        &self,
        session: &SessionHandle,
        response: PermissionResponse,
    ) -> Result<(), String> {
        self.runtime()?.respond_permission(session, response).await
    }

    fn fixed_models(&self) -> Option<Vec<String>> {
        Some(CODEX_FIXED_MODELS.iter().map(|s| s.to_string()).collect())
    }

    fn runtime_config(
        &self,
        app_config: Option<&crate::config::AppConfig>,
    ) -> BackendRuntimeConfig {
        let mut bridge_init_options = serde_json::Map::new();
        bridge_init_options.insert(
            "codexCliPath".to_string(),
            serde_json::Value::String(
                configured_cli_path_from_config(app_config).unwrap_or_else(|| "codex".to_string()),
            ),
        );

        BackendRuntimeConfig {
            bridge_init_options,
        }
    }

    async fn close_session(&self, session: &SessionHandle) -> Result<(), String> {
        self.runtime()?.close_session(session).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_models_returns_codex_fixed_list_in_order() {
        let backend = CodexBackend::new();
        let expected: Vec<String> = CODEX_FIXED_MODELS.iter().map(|s| s.to_string()).collect();
        assert_eq!(backend.fixed_models(), Some(expected));
    }

    #[tokio::test]
    async fn runtime_methods_require_attached_runtime() {
        let backend = CodexBackend::new();
        let session = SessionHandle {
            chat_session_id: "session-1".to_string(),
            backend_id: CODEX_BACKEND_ID.to_string(),
        };
        let message = AgentMessage {
            content: "hello".to_string(),
            streaming_message_id: "message-1".to_string(),
            images: vec![],
            permission_mode: "edit".to_string(),
        };

        assert!(backend.send_message(&session, message).await.is_err());
        assert!(backend.interrupt(&session).await.is_err());
        assert!(backend.close_session(&session).await.is_err());
    }
}
