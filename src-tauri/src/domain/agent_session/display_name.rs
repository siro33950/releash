#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentSessionDisplayName(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionDisplayNameError {
    Empty,
}

impl AgentSessionDisplayName {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, AgentSessionDisplayNameError> {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() {
            return Err(AgentSessionDisplayNameError::Empty);
        }
        Ok(Self(value.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod agent_session_display_name_tests {
    use super::*;

    #[test]
    fn test_agent_session表示名_前後の空白を除いて保持する() {
        let name = AgentSessionDisplayName::new("  release review  ").unwrap();

        assert_eq!(name.as_str(), "release review");
    }

    #[test]
    fn test_agent_session表示名_空文字と空白だけを拒否する() {
        for value in ["", "  \t\n "] {
            assert_eq!(
                AgentSessionDisplayName::new(value),
                Err(AgentSessionDisplayNameError::Empty)
            );
        }
    }
}
