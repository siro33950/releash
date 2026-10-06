pub(crate) mod agent_session_display_name_tests {
    use super::super::*;

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
