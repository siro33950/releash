//! Workflow history entry construction rules.

use crate::domain::workflow::value_objects::{NodeHistoryEntry, TokenUsage, NODE_STATUS_ABORTED};

pub fn aborted_node_history_entry(
    node_name: String,
    attempt: u32,
    session_id: Option<String>,
    token_usage: TokenUsage,
    timestamp: f64,
) -> NodeHistoryEntry {
    NodeHistoryEntry {
        node_name,
        completed_at: timestamp,
        result: None,
        session_id,
        token_usage: Some(token_usage),
        artifact: None,
        attempt,
        fanout_children: None,
        state: NODE_STATUS_ABORTED.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
