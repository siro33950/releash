#[allow(dead_code)] // issues-1301 G-1: event-log turn id vocabulary is retained for the domain boundary.
pub type TurnId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnResult {
    Completed {
        stop_reason: Option<TurnStopReason>,
        token_usage: Option<TokenUsage>,
    },
    Failed {
        error: String,
        token_usage: Option<TokenUsage>,
    },
    Interrupted {
        reason: InterruptReason,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStopReason {
    #[allow(dead_code)]
    // issues-1301 G-1: refusal is emitted by backend conversion fixtures and workflow failure projection, not every production path.
    Refusal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptReason {
    Abort,
    Timeout,
    Crash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: Option<u64>,
    pub context_window_tokens: Option<u64>,
}
