#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSurfaceLifecycleConfig {
    pub per_worktree_cap: usize,
    pub max_panes_total: usize,
}

impl Default for TerminalSurfaceLifecycleConfig {
    fn default() -> Self {
        Self {
            per_worktree_cap: 32,
            max_panes_total: 64,
        }
    }
}
