use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
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

pub const STEP_STATE_ABORTED: &str = "aborted";
pub const STEP_STATE_COMPLETED: &str = "completed";
pub const STEP_STATE_FAILED: &str = "failed";
pub const STEP_STATE_INTERRUPTED: &str = "interrupted";
pub const STEP_STATE_PENDING: &str = "pending";
pub const STEP_STATE_RUNNING: &str = "running";
pub const STEP_STATE_WAITING_APPROVAL: &str = "waiting_approval";

pub fn default_step_entry_state() -> String {
    STEP_STATE_COMPLETED.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StepHistoryEntry {
    pub step_name: String,
    pub completed_at: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token_usage: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output: Option<serde_json::Value>,
    #[serde(default)]
    pub run_index: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub child_outputs: Option<Vec<ChildOutputSnapshot>>,
    #[serde(default = "default_step_entry_state")]
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChildOutputSnapshot {
    pub step_name: String,
    pub session_id: Option<String>,
    pub result: Option<String>,
    pub run_index: u32,
    pub completed_at: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub output_contract: Option<String>,
    #[serde(default = "default_step_entry_state")]
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ParallelStepState {
    pub step_name: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<String>,
    pub run_index: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub completed_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub output_contract: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StepOutput {
    pub step_name: String,
    pub run_index: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub output_contract: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token_usage: Option<TokenUsage>,
    pub completed_at: f64,
}

#[cfg(test)]
mod step_output_tests {
    use super::*;

    #[test]
    fn test_token_usage_addは入力出力を加算する() {
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

    #[test]
    fn test_step_history_state欠落はcompletedに戻す() {
        let entry: StepHistoryEntry = serde_json::from_value(serde_json::json!({
            "stepName": "review",
            "completedAt": 1.0,
            "runIndex": 0
        }))
        .unwrap();
        assert_eq!(entry.state, "completed");
    }
}
