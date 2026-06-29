use async_trait::async_trait;

use crate::domain::agent_session::CLAUDE_FIXED_MODELS;
use crate::infrastructure::agent_session::runtime::bridge_common::CLAUDE_BACKEND_ID;
use crate::infrastructure::agent_session::runtime::{
    AgentBackend, AgentMessage, BackendRuntimeConfig, PermissionResponse, SessionConfig,
    SessionHandle,
};

/// Claude Agent SDK Bridge バックエンド。
/// Node.js ブリッジプロセスを経由して Claude Agent SDK と通信する。
/// モデル選択肢は `CLAUDE_FIXED_MODELS` で完全固定する。
pub struct ClaudeBackend;

impl ClaudeBackend {
    pub fn new() -> Self {
        Self
    }

    /// 実アプリ用コンストラクタ。`build_registry_with_runtime` から呼ばれる。
    /// モデル一覧は固定リストで供給するため AppHandle は使用しない。
    pub fn with_app(_app: &tauri::AppHandle) -> Self {
        Self
    }
}

#[async_trait]
impl AgentBackend for ClaudeBackend {
    fn id(&self) -> &str {
        CLAUDE_BACKEND_ID
    }

    fn name(&self) -> &str {
        "Claude"
    }

    async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String> {
        Ok(SessionHandle {
            chat_session_id: config.chat_session_id,
            backend_id: CLAUDE_BACKEND_ID.to_string(),
        })
    }

    async fn send_message(
        &self,
        _session: &SessionHandle,
        _message: AgentMessage,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn interrupt(&self, _session: &SessionHandle) -> Result<(), String> {
        Ok(())
    }

    async fn respond_permission(
        &self,
        _session: &SessionHandle,
        _response: PermissionResponse,
    ) -> Result<(), String> {
        Ok(())
    }

    fn fixed_models(&self) -> Option<Vec<String>> {
        Some(CLAUDE_FIXED_MODELS.iter().map(|s| s.to_string()).collect())
    }

    fn runtime_config(
        &self,
        _app_config: Option<&dyn crate::domain::app_config::AgentConfigRepository>,
    ) -> BackendRuntimeConfig {
        BackendRuntimeConfig {
            bridge_init_options: serde_json::Map::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_models_returns_claude_fixed_list_in_order() {
        let backend = ClaudeBackend::new();
        let expected: Vec<String> = CLAUDE_FIXED_MODELS.iter().map(|s| s.to_string()).collect();
        assert_eq!(backend.fixed_models(), Some(expected));
    }
}
