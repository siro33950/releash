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
            value: "sonnet".to_string(),
            display_name: "Claude Sonnet".to_string(),
        },
        ModelInfo {
            value: "opus".to_string(),
            display_name: "Claude Opus".to_string(),
        },
        ModelInfo {
            value: "haiku".to_string(),
            display_name: "Claude Haiku".to_string(),
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
        assert!(models.iter().any(|m| m.value == "sonnet"));
        assert!(models.iter().any(|m| m.value == "opus"));
    }
}
