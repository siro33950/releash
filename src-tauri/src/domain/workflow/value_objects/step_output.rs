#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct StepHistoryEntry {
    pub step_name: String,
    pub completed_at: f64,
    pub result: Option<String>,
    pub session_id: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub structured_output: Option<serde_json::Value>,
    pub run_index: u32,
    pub child_outputs: Option<Vec<ChildOutputSnapshot>>,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChildOutputSnapshot {
    pub step_name: String,
    pub session_id: Option<String>,
    pub result: Option<String>,
    pub run_index: u32,
    pub completed_at: f64,
    pub structured_output: Option<serde_json::Value>,
    pub output_contract: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParallelStepState {
    pub step_name: String,
    pub state: String,
    pub session_id: Option<String>,
    pub result: Option<String>,
    pub run_index: u32,
    pub completed_at: Option<f64>,
    pub structured_output: Option<serde_json::Value>,
    pub output_contract: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepOutput {
    pub step_name: String,
    pub run_index: u32,
    pub session_id: Option<String>,
    pub result: Option<String>,
    pub structured_output: Option<serde_json::Value>,
    pub output_contract: Option<String>,
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
    fn test_default_step_entry_stateはcompletedを返す() {
        assert_eq!(default_step_entry_state(), "completed");
    }
}
