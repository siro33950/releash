#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClaudeModelSpec {
    pub id: &'static str,
    pub display_name: &'static str,
}

pub(crate) const BACKEND_ID: &str = "claude";
pub(crate) const BACKEND_NAME: &str = "Claude";
pub(crate) const DEFAULT_CLI_PATH: &str = "claude";

pub(crate) const FIXED_MODELS: &[ClaudeModelSpec] = &[
    ClaudeModelSpec {
        id: "claude-opus-5",
        display_name: "Opus 5",
    },
    ClaudeModelSpec {
        id: "claude-fable-5",
        display_name: "Fable 5",
    },
    ClaudeModelSpec {
        id: "claude-sonnet-5",
        display_name: "Sonnet 5",
    },
    ClaudeModelSpec {
        id: "claude-haiku-4-5-20251001",
        display_name: "Haiku 4.5",
    },
];
