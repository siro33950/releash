use crate::domain::mcp::error::McpError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    Claude,
    Codex,
    Gemini,
    Cursor,
}

impl AgentKind {
    pub fn parse(value: &str) -> Result<Self, McpError> {
        match value {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "gemini" => Ok(Self::Gemini),
            "cursor" => Ok(Self::Cursor),
            _ => Err(McpError::InvalidInput(format!(
                "Unknown agent type: {value}"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Cursor => "cursor",
        }
    }

    pub fn global_path(self) -> &'static str {
        match self {
            Self::Claude => ".claude.json",
            Self::Codex => ".codex/config.toml",
            Self::Gemini => ".gemini/settings.json",
            Self::Cursor => ".cursor/mcp.json",
        }
    }
}

#[cfg(test)]
mod agent_kind_tests {
    use super::*;

    #[test]
    fn test_エージェント種別_文字列から復元できる() {
        // Given / When / Then
        assert_eq!(AgentKind::parse("claude").unwrap(), AgentKind::Claude);
        assert_eq!(AgentKind::parse("codex").unwrap(), AgentKind::Codex);
        assert_eq!(AgentKind::parse("gemini").unwrap(), AgentKind::Gemini);
        assert_eq!(AgentKind::parse("cursor").unwrap(), AgentKind::Cursor);
    }

    #[test]
    fn test_エージェント種別_グローバル設定パスを返す() {
        // Given / When / Then
        assert_eq!(AgentKind::Claude.global_path(), ".claude.json");
        assert_eq!(AgentKind::Codex.global_path(), ".codex/config.toml");
        assert_eq!(AgentKind::Gemini.global_path(), ".gemini/settings.json");
        assert_eq!(AgentKind::Cursor.global_path(), ".cursor/mcp.json");
    }
}
