use crate::domain::mcp::error::McpError;
use crate::domain::mcp::value_objects::AgentKind;

pub const ALL_AGENTS: [AgentKind; 4] = [
    AgentKind::Claude,
    AgentKind::Codex,
    AgentKind::Gemini,
    AgentKind::Cursor,
];

pub fn mcp_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

pub fn is_authorized_bearer(header: Option<&str>, expected_token: &str) -> bool {
    if expected_token.trim().is_empty() {
        return false;
    }
    let Some(header) = header else {
        return false;
    };
    let mut parts = header.splitn(2, ' ');
    let scheme = parts.next().unwrap_or_default();
    let token = parts.next().unwrap_or_default().trim();
    scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() && token == expected_token
}

pub fn normalize_agent_types(agent_types: Vec<String>) -> Result<Vec<String>, McpError> {
    let mut normalized = Vec::new();
    for raw in agent_types {
        let candidate = raw.trim().to_lowercase();
        if candidate.is_empty() {
            continue;
        }
        let agent = AgentKind::parse(&candidate)?;
        let value = agent.as_str().to_string();
        if !normalized.contains(&value) {
            normalized.push(value);
        }
    }
    Ok(normalized)
}

pub fn validate_generation_credentials(
    has_desired_agents: bool,
    port: u16,
    token: &str,
) -> Result<(), McpError> {
    if !has_desired_agents {
        return Ok(());
    }
    if port == 0 {
        return Err(McpError::InvalidInput(
            "mcp_port must be between 1 and 65535".to_string(),
        ));
    }
    if token.trim().is_empty() {
        return Err(McpError::InvalidInput(
            "mcp_token must not be empty".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod services_tests {
    use super::*;

    #[test]
    fn test_bearer認証_正しいトークンだけ許可する() {
        // Given / When / Then
        assert!(is_authorized_bearer(Some("Bearer abc"), "abc"));
        assert!(is_authorized_bearer(Some("bearer abc"), "abc"));
        assert!(!is_authorized_bearer(Some("Bearer other"), "abc"));
        assert!(!is_authorized_bearer(None, "abc"));
    }

    #[test]
    fn test_bearer認証_期待トークン空は拒否する() {
        assert!(!is_authorized_bearer(Some("Bearer "), ""));
    }

    #[test]
    fn test_bearer認証_空トークンは拒否する() {
        assert!(!is_authorized_bearer(Some("Bearer "), "abc"));
    }

    #[test]
    fn test_エージェント種別正規化_空白重複を除去する() {
        // Given
        let raw = vec![
            " Claude ".to_string(),
            "codex".to_string(),
            "claude".to_string(),
            String::new(),
        ];

        // When
        let normalized = normalize_agent_types(raw).unwrap();

        // Then
        assert_eq!(normalized, vec!["claude", "codex"]);
    }
}
