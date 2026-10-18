pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn aborted_node_history_entry_keeps_session_and_token_usage() {
        let entry = aborted_node_history_entry(
            "review".to_string(),
            3,
            Some("session-review".to_string()),
            TokenUsage {
                input_tokens: 5,
                output_tokens: 8,
            },
            12.0,
        );

        assert_eq!(entry.node_name, "review");
        assert_eq!(entry.attempt, 3);
        assert_eq!(entry.session_id.as_deref(), Some("session-review"));
        assert_eq!(entry.token_usage.unwrap().input_tokens, 5);
        assert_eq!(entry.state, "aborted");
    }
}
