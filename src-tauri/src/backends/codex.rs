use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

use crate::backends::bridge_common::{
    start_agent_session_internal, start_agent_turn, write_bridge_command, AgentProcessMap,
    CODEX_BACKEND_ID,
};
use crate::backends::{
    AgentBackend, AgentMessage, BackendRuntimeConfig, ModelInfo, PermissionResponse, SessionConfig,
    SessionHandle,
};
use crate::session::{resolve_data_dir, SessionStore};

/// Codex SDK Bridge バックエンド。
/// 実際のプロセス制御は AgentProcess bridge runtime に委譲する。
pub struct CodexBackend {
    #[allow(dead_code)]
    runtime: Option<Arc<dyn CodexBackendRuntime>>,
}

pub fn codex_supported_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            value: "gpt-5.4".to_string(),
            display_name: "GPT-5.4".to_string(),
        },
        ModelInfo {
            value: "gpt-5.3-codex".to_string(),
            display_name: "GPT-5.3 Codex".to_string(),
        },
        ModelInfo {
            value: "gpt-5.2-codex".to_string(),
            display_name: "GPT-5.2 Codex".to_string(),
        },
        ModelInfo {
            value: "gpt-5-codex".to_string(),
            display_name: "GPT-5 Codex".to_string(),
        },
        ModelInfo {
            value: "o3".to_string(),
            display_name: "o3".to_string(),
        },
    ]
}

pub(crate) fn configured_initial_model(app: &tauri::AppHandle) -> Option<String> {
    app.try_state::<std::sync::Arc<crate::config::AppConfig>>()
        .and_then(|cfg_state| cfg_state.get_config().ok())
        .and_then(|cfg| cfg.agents.codex.model)
}

pub(crate) fn configured_cli_path(app: &tauri::AppHandle) -> Option<String> {
    app.try_state::<std::sync::Arc<crate::config::AppConfig>>()
        .and_then(|cfg_state| cfg_state.get_config().ok())
        .and_then(|cfg| cfg.agents.codex.cli_path)
        .filter(|path| !path.trim().is_empty())
}

#[allow(dead_code)]
impl CodexBackend {
    pub fn new() -> Self {
        Self { runtime: None }
    }

    pub fn with_agent_process_runtime(
        app: AppHandle,
        handles: Arc<Mutex<AgentProcessMap>>,
        session_store: Arc<SessionStore>,
    ) -> Self {
        Self {
            runtime: Some(Arc::new(AgentProcessCodexRuntime {
                app,
                handles,
                session_store,
            })),
        }
    }

    fn runtime(&self) -> Result<Arc<dyn CodexBackendRuntime>, String> {
        self.runtime.clone().ok_or_else(|| {
            "CodexBackend runtime is not attached; build the registry with app runtime".to_string()
        })
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

    async fn available_models(&self) -> Result<Vec<ModelInfo>, String> {
        Ok(codex_supported_models())
    }

    fn runtime_config(&self, app: &tauri::AppHandle) -> BackendRuntimeConfig {
        let mut bridge_init_options = serde_json::Map::new();
        bridge_init_options.insert(
            "codexCliPath".to_string(),
            serde_json::Value::String(
                configured_cli_path(app).unwrap_or_else(|| "codex".to_string()),
            ),
        );

        BackendRuntimeConfig {
            initial_model: configured_initial_model(app),
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

    #[tokio::test]
    async fn available_models_returns_codex_defaults() {
        let backend = CodexBackend::new();
        let models = backend.available_models().await.unwrap();
        assert!(models.iter().any(|m| m.value == "gpt-5.4"));
        assert!(models.iter().any(|m| m.value == "o3"));
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
            permission_mode: "acceptEdits".to_string(),
        };

        assert!(backend.send_message(&session, message).await.is_err());
        assert!(backend.interrupt(&session).await.is_err());
        assert!(backend.close_session(&session).await.is_err());
    }
}
