use async_trait::async_trait;

use crate::backends::bridge_common::CLAUDE_BACKEND_ID;
use crate::backends::{
    AgentBackend, AgentMessage, ModelInfo, PermissionResponse, SessionConfig, SessionHandle,
};

/// Claude Agent SDK Bridge バックエンド。
/// Node.js ブリッジプロセスを経由して Claude Agent SDK と通信する。
pub struct ClaudeBackend {
    _private: (),
}

impl ClaudeBackend {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

pub fn claude_supported_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            value: "claude-sonnet-4-6".to_string(),
            display_name: "claude-sonnet-4-6".to_string(),
        },
        ModelInfo {
            value: "claude-opus-4-7".to_string(),
            display_name: "claude-opus-4-7".to_string(),
        },
        ModelInfo {
            value: "claude-haiku-4-5-20251001".to_string(),
            display_name: "claude-haiku-4-5-20251001".to_string(),
        },
    ]
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

    async fn available_models(&self) -> Result<Vec<ModelInfo>, String> {
        Ok(claude_supported_models())
    }

    async fn close_session(&self, _session: &SessionHandle) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn available_models_returns_claude_defaults() {
        let backend = ClaudeBackend::new();
        let models = backend.available_models().await.unwrap();
        assert!(models.iter().any(|m| m.value == "claude-sonnet-4-6"));
        assert!(models.iter().any(|m| m.value == "claude-opus-4-7"));
    }
}
