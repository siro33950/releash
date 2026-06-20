use crate::domain::mcp::value_objects::McpConnectionInfo;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct McpServer {
    running: bool,
    port: Option<u16>,
    token: Option<String>,
}

impl McpServer {
    pub fn stopped() -> Self {
        Self::default()
    }

    pub fn running(port: u16, token: String) -> Self {
        Self {
            running: true,
            port: Some(port),
            token: Some(token),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn connection_info(&self) -> Option<McpConnectionInfo> {
        if !self.running {
            return None;
        }
        Some(McpConnectionInfo {
            url: format!("http://127.0.0.1:{}/mcp", self.port?),
            token: self.token.clone()?,
        })
    }
}

#[cfg(test)]
mod mcp_server_tests {
    use super::*;

    #[test]
    fn test_mcpサーバ状態_停止中は接続情報なし() {
        // Given
        let server = McpServer::stopped();

        // When / Then
        assert!(!server.is_running());
        assert!(server.connection_info().is_none());
    }

    #[test]
    fn test_mcpサーバ状態_起動中は接続情報を返す() {
        // Given
        let server = McpServer::running(19801, "token".to_string());

        // When
        let info = server.connection_info().unwrap();

        // Then
        assert_eq!(info.url, "http://127.0.0.1:19801/mcp");
        assert_eq!(info.token, "token");
    }
}
