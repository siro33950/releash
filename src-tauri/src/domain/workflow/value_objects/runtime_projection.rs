use super::failure::{FailureDisposition, NodeExecutionFailureKind};

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl TokenUsage {
    pub fn add(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

pub const NODE_STATUS_ABORTED: &str = "aborted";
pub const NODE_STATUS_COMPLETED: &str = "completed";
pub const NODE_STATUS_FAILED: &str = "failed";
pub const NODE_STATUS_INTERRUPTED: &str = "interrupted";
pub const NODE_STATUS_RUNNING: &str = "running";
#[cfg(test)]
pub const NODE_STATUS_WAITING_APPROVAL: &str = "waiting_approval";

pub fn default_node_history_status() -> String {
    NODE_STATUS_COMPLETED.to_string()
}

/// Private runtime transition history. Public history is `NodeExecution`.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeHistoryEntry {
    pub node_name: String,
    pub completed_at: f64,
    pub result: Option<String>,
    pub session_id: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub artifact: Option<serde_json::Value>,
    pub attempt: u32,
    pub fanout_children: Option<Vec<FanoutChildSnapshot>>,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanoutChildSnapshot {
    pub node_name: String,
    pub session_id: Option<String>,
    pub result: Option<String>,
    pub attempt: u32,
    pub completed_at: f64,
    pub artifact: Option<serde_json::Value>,
    pub contract: Option<String>,
    pub state: String,
    pub failure_kind: Option<NodeExecutionFailureKind>,
    pub failure_disposition: Option<FailureDisposition>,
}

/// Private runtime artifact slot used while an execution is transitioning.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeArtifact {
    pub node_name: String,
    pub attempt: u32,
    pub session_id: Option<String>,
    pub result: Option<String>,
    pub artifact: Option<serde_json::Value>,
    pub contract: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub completed_at: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_usage_adds_input_and_output() {
        let mut usage = TokenUsage {
            input_tokens: 1,
            output_tokens: 2,
        };
        usage.add(&TokenUsage {
            input_tokens: 3,
            output_tokens: 4,
        });
        assert_eq!(usage.input_tokens, 4);
        assert_eq!(usage.output_tokens, 6);
    }
}
