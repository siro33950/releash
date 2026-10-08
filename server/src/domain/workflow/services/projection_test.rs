pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn total_token_usage_sums_history_entries_and_skips_missing_usage() {
        let usage = total_token_usage(&[
            NodeHistoryEntry {
                node_name: "plan".to_string(),
                completed_at: 1.0,
                result: None,
                session_id: None,
                token_usage: Some(TokenUsage {
                    input_tokens: 3,
                    output_tokens: 5,
                }),
                artifact: None,
                attempt: 1,
                fanout_children: None,
                state: NODE_STATUS_COMPLETED.to_string(),
            },
            NodeHistoryEntry {
                node_name: "review".to_string(),
                completed_at: 2.0,
                result: None,
                session_id: None,
                token_usage: None,
                artifact: None,
                attempt: 1,
                fanout_children: None,
                state: NODE_STATUS_COMPLETED.to_string(),
            },
            NodeHistoryEntry {
                node_name: "fix".to_string(),
                completed_at: 3.0,
                result: None,
                session_id: None,
                token_usage: Some(TokenUsage {
                    input_tokens: 7,
                    output_tokens: 11,
                }),
                artifact: None,
                attempt: 1,
                fanout_children: None,
                state: NODE_STATUS_COMPLETED.to_string(),
            },
        ]);
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 16);
    }
}
