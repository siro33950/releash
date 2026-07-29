use std::collections::HashMap;

use serde_json::Value;

use crate::infrastructure::agent_session::stdout_line_reader::StdoutDiagnostics;

#[derive(Debug)]
pub(crate) struct ClaudeRuntimeState {
    pub(crate) session_id: String,
    pub(crate) backend_session_id: Option<String>,
    pub(crate) cwd: String,
    pub(crate) model: String,
    pub(crate) permission_mode: String,
    pub(crate) plan_mode: bool,
    pub(crate) system_prompt: Option<String>,
    pub(crate) resume: Option<String>,
    pub(crate) base_branch: Option<String>,
    pub(crate) startup_timeout: Option<std::time::Duration>,
    pub(crate) startup_max_retries: Option<u32>,
    pub(crate) stale_timeout: Option<std::time::Duration>,
    pub(crate) extra_env: Vec<(String, String)>,
    pub(crate) turn_active: bool,
    pub(crate) pending_inputs: HashMap<String, Value>,
    pub(crate) stdout_diagnostics: StdoutDiagnostics,
}
