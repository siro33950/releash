use std::time::Duration;

use crate::domain::pty_session::services::OUTPUT_BUFFER_CAPACITY;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtyLifecycleConfig {
    pub per_worktree_cap: usize,
    pub max_panes_total: usize,
    pub idle_timeout: Duration,
    pub output_buffer_cap: usize,
    pub sweep_interval: Duration,
}

impl Default for PtyLifecycleConfig {
    fn default() -> Self {
        Self {
            per_worktree_cap: 32,
            max_panes_total: 64,
            idle_timeout: Duration::from_secs(300),
            output_buffer_cap: OUTPUT_BUFFER_CAPACITY,
            sweep_interval: Duration::from_secs(60),
        }
    }
}
