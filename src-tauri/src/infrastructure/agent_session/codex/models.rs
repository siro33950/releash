#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CodexModelSpec {
    pub id: &'static str,
    pub display_name: &'static str,
}

pub(crate) const BACKEND_ID: &str = "codex";
pub(crate) const BACKEND_NAME: &str = "Codex";
pub(crate) const DEFAULT_CLI_PATH: &str = "codex";

pub(crate) const FIXED_MODELS: &[CodexModelSpec] = &[
    CodexModelSpec {
        id: "gpt-5.6-sol",
        display_name: "GPT-5.6 Sol",
    },
    CodexModelSpec {
        id: "gpt-5.6-terra",
        display_name: "GPT-5.6 Terra",
    },
    CodexModelSpec {
        id: "gpt-5.6-luna",
        display_name: "GPT-5.6 Luna",
    },
];
